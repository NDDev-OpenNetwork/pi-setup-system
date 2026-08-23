//! The bounded backup pool, and the reference that names one slot in it.
//!
//! Every mutation captures the exact pre-operation state before its first
//! write. That capture is what `restore` restores and what recovery from
//! [`Phase::Prepared`](crate::journal::Phase::Prepared) rewinds to.
//!
//! A slot's completion marker is written **last**. A slot without one was
//! interrupted mid-capture, and [`Pool::partial_slots`] surfaces it so planning
//! can refuse instead of restoring from a half-copied tree — the failure mode a
//! marker written first would hide.
//!
//! Slots are numbered by a monotonic sequence, not by a timestamp. Ordering that
//! depends on a clock reorders itself when the clock moves, and "the last
//! backup" would then name a different slot than it did a moment earlier.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, ReasonCode, Result};
use crate::lock;

/// The pool directory name inside the control directory.
pub const POOL_DIRECTORY_NAME: &str = "backups";

/// The marker file written last inside a completed slot.
pub const SLOT_MARKER_NAME: &str = "slot.json";

/// The subdirectory holding the captured tree.
pub const SLOT_PAYLOAD_NAME: &str = "payload";

/// The schema this kernel writes and is willing to read.
pub const SLOT_SCHEMA: u32 = 1;

/// A target-bound reference to one backup slot.
///
/// The reference is meaningful only against the target it was captured from;
/// it is never a global identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BackupRef(String);

impl BackupRef {
    /// Build a reference from a sequence number.
    #[must_use]
    pub fn from_sequence(sequence: u64) -> Self {
        Self(format!("slot-{sequence:012}"))
    }

    /// Parse a reference, rejecting anything that is not one this kernel mints.
    ///
    /// The rejection matters: a reference is used as a directory name, so a
    /// value carrying separators or parent components would let a caller name a
    /// path outside the pool.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::IntegrityMismatch`] for a malformed reference.
    pub fn parse(text: &str) -> Result<Self> {
        let Some(digits) = text.strip_prefix("slot-") else {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!("{text:?} is not a backup reference"),
            ));
        };
        if digits.len() != 12 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!("{text:?} is not a backup reference"),
            ));
        }
        Ok(Self(text.to_owned()))
    }

    /// The wire spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn sequence(&self) -> u64 {
        self.0
            .strip_prefix("slot-")
            .and_then(|digits| digits.parse().ok())
            .unwrap_or(0)
    }
}

/// What one completed slot records about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotRecord {
    /// Schema of this record.
    pub schema_version: u32,
    /// The reference naming this slot.
    pub backup_ref: BackupRef,
    /// The operation that captured it.
    pub operation: String,
    /// The operation id that captured it.
    pub operation_id: String,
    /// The target identity digest at capture time.
    pub target_identity_digest: String,
    /// The setup identity in effect at capture time, when one was stamped.
    pub setup_id: Option<String>,
}

/// The bounded pool of backup slots for one target.
#[derive(Debug, Clone)]
pub struct Pool {
    root: PathBuf,
    capacity: usize,
}

impl Pool {
    /// Open the pool inside a control directory.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the pool directory cannot be
    /// created, and [`ReasonCode::IntegrityMismatch`] if `capacity` is zero — a
    /// pool that keeps nothing would let a mutation proceed with no way back.
    pub fn open(control_directory: &Path, capacity: usize) -> Result<Self> {
        if capacity == 0 {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                "a backup pool with no slots cannot support restore",
            ));
        }
        let root = control_directory.join(POOL_DIRECTORY_NAME);
        fs::create_dir_all(&root).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot create the backup pool at {}", root.display()),
            )
            .with_source(source)
        })?;
        Ok(Self { root, capacity })
    }

    /// Completed slots, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the pool cannot be listed.
    pub fn list(&self) -> Result<Vec<SlotRecord>> {
        let mut records = Vec::new();
        for slot in self.slot_directories()? {
            if let Some(record) = read_record(&slot)? {
                records.push(record);
            }
        }
        // Newest first: the reverse of the minted sequence, never a clock.
        records.sort_by_key(|record| std::cmp::Reverse(record.backup_ref.sequence()));
        Ok(records)
    }

    /// The newest completed slot, if the pool holds one.
    ///
    /// This is what `restore` without an explicit reference restores.
    ///
    /// # Errors
    ///
    /// Propagates the refusal from [`Pool::list`].
    pub fn latest(&self) -> Result<Option<SlotRecord>> {
        Ok(self.list()?.into_iter().next())
    }

    /// Slots that were interrupted before their marker was written.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the pool cannot be listed.
    pub fn partial_slots(&self) -> Result<Vec<PathBuf>> {
        let mut partial = Vec::new();
        for slot in self.slot_directories()? {
            if !slot.join(SLOT_MARKER_NAME).exists() {
                partial.push(slot);
            }
        }
        partial.sort();
        Ok(partial)
    }

    /// The payload directory of one completed slot.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::IntegrityMismatch`] if the slot is absent or
    /// incomplete. An incomplete slot is never offered as a restore source.
    pub fn payload_of(&self, backup_ref: &BackupRef) -> Result<PathBuf> {
        let slot = self.root.join(backup_ref.as_str());
        if !slot.join(SLOT_MARKER_NAME).exists() {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!("backup {} is absent or incomplete", backup_ref.as_str()),
            ));
        }
        Ok(slot.join(SLOT_PAYLOAD_NAME))
    }

    /// The reference the next capture will use.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the pool cannot be listed.
    pub fn next_ref(&self) -> Result<BackupRef> {
        let highest = self
            .slot_directories()?
            .iter()
            .filter_map(|slot| slot.file_name().and_then(|name| name.to_str()))
            .filter_map(|name| BackupRef::parse(name).ok())
            .map(|reference| reference.sequence())
            .max()
            .unwrap_or(0);
        Ok(BackupRef::from_sequence(highest.saturating_add(1)))
    }

    /// Capture `source` into a new slot and complete it.
    ///
    /// The payload is copied first and the marker written last, so an
    /// interruption leaves a slot [`Pool::partial_slots`] can name.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the copy or the marker write
    /// fails.
    pub fn capture(
        &self,
        source: &Path,
        excluded_top_level: &[&str],
        record: impl FnOnce(BackupRef) -> SlotRecord,
    ) -> Result<SlotRecord> {
        let backup_ref = self.next_ref()?;
        let slot = self.root.join(backup_ref.as_str());
        let payload = slot.join(SLOT_PAYLOAD_NAME);
        fs::create_dir_all(&payload).map_err(|source_error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot create backup slot {}", slot.display()),
            )
            .with_source(source_error)
        })?;

        copy_tree(source, &payload, excluded_top_level)?;

        let record = record(backup_ref);
        let value = serde_json::to_value(&record).map_err(|source_error| {
            Error::new(
                ReasonCode::StateUnavailable,
                "cannot encode the backup record",
            )
            .with_source(source_error)
        })?;
        let bytes = crate::canonical::to_canonical_bytes(&value)?;
        lock::atomic_write(&slot.join(SLOT_MARKER_NAME), &bytes)?;

        self.prune()?;
        Ok(record)
    }

    /// Drop the oldest completed slots beyond the pool capacity.
    ///
    /// Incomplete slots are never pruned: they are evidence recovery needs, and
    /// deleting them here would erase the reason planning is refusing.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if a slot cannot be removed.
    pub fn prune(&self) -> Result<()> {
        let records = self.list()?;
        for record in records.into_iter().skip(self.capacity) {
            let slot = self.root.join(record.backup_ref.as_str());
            fs::remove_dir_all(&slot).map_err(|source| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot prune {}", slot.display()),
                )
                .with_source(source)
            })?;
        }
        Ok(())
    }

    fn slot_directories(&self) -> Result<Vec<PathBuf>> {
        let read = match fs::read_dir(&self.root) {
            Ok(read) => read,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot list {}", self.root.display()),
                )
                .with_source(source));
            }
        };
        let mut slots = Vec::new();
        for entry in read {
            let entry = entry.map_err(|source| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot read an entry of {}", self.root.display()),
                )
                .with_source(source)
            })?;
            let path = entry.path();
            let is_slot = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| BackupRef::parse(name).is_ok());
            if is_slot && path.is_dir() {
                slots.push(path);
            }
        }
        Ok(slots)
    }
}

/// Read one slot's completion marker, or report that the slot has none.
///
/// A slot without a marker is not an error here — [`Pool::partial_slots`] is
/// what names it, and [`Pool::list`] simply does not offer it.
fn read_record(slot: &Path) -> Result<Option<SlotRecord>> {
    let marker = slot.join(SLOT_MARKER_NAME);
    let bytes = match fs::read(&marker) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot read {}", marker.display()),
            )
            .with_source(source));
        }
    };
    let record: SlotRecord = serde_json::from_slice(&bytes).map_err(|source| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("{} does not parse as a backup record", marker.display()),
        )
        .with_source(source)
    })?;
    if record.schema_version != SLOT_SCHEMA {
        return Err(Error::new(
            ReasonCode::StateUnavailable,
            format!(
                "backup schema {} is not the {SLOT_SCHEMA} this build writes",
                record.schema_version
            ),
        ));
    }
    Ok(Some(record))
}

/// Copy a directory tree, skipping the named top-level entries.
///
/// Symbolic links are not followed. A link inside the source is refused rather
/// than dereferenced, because a backup that silently inlined a link's target
/// would restore different bytes than it captured.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] on any I/O failure and
/// [`ReasonCode::IntegrityMismatch`] when a symbolic link is encountered.
pub fn copy_tree(source: &Path, destination: &Path, excluded_top_level: &[&str]) -> Result<()> {
    copy_inner(source, destination, excluded_top_level, true)
}

fn copy_inner(
    source: &Path,
    destination: &Path,
    excluded_top_level: &[&str],
    at_root: bool,
) -> Result<()> {
    fs::create_dir_all(destination).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot create {}", destination.display()),
        )
        .with_source(error)
    })?;
    let read = fs::read_dir(source).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot list {}", source.display()),
        )
        .with_source(error)
    })?;
    for entry in read {
        let entry = entry.map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot read an entry of {}", source.display()),
            )
            .with_source(error)
        })?;
        let from = entry.path();
        let Some(name) = from.file_name().and_then(|name| name.to_str()) else {
            return Err(Error::new(
                ReasonCode::StateUnavailable,
                format!("{} has a name this kernel cannot represent", from.display()),
            ));
        };
        if at_root && excluded_top_level.contains(&name) {
            continue;
        }
        let to = destination.join(name);
        let metadata = fs::symlink_metadata(&from).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot stat {}", from.display()),
            )
            .with_source(error)
        })?;
        if metadata.is_symlink() {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!("{} is a symbolic link and is not captured", from.display()),
            ));
        }
        if metadata.is_dir() {
            copy_inner(&from, &to, excluded_top_level, false)?;
        } else {
            fs::copy(&from, &to).map_err(|error| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot copy {} to {}", from.display(), to.display()),
                )
                .with_source(error)
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("setup-core-backup-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn record_for(backup_ref: BackupRef) -> SlotRecord {
        SlotRecord {
            schema_version: SLOT_SCHEMA,
            backup_ref,
            operation: "install".to_owned(),
            operation_id: "op_test".to_owned(),
            target_identity_digest: "sha256:target".to_owned(),
            setup_id: Some("full-auto".to_owned()),
        }
    }

    #[test]
    fn a_reference_that_could_escape_the_pool_is_refused() {
        for hostile in [
            "slot-../../etc",
            "slot-1",
            "../slot-000000000001",
            "slot-00000000000a",
        ] {
            assert!(BackupRef::parse(hostile).is_err(), "accepted {hostile:?}");
        }
        assert!(BackupRef::parse("slot-000000000001").is_ok());
    }

    #[test]
    fn a_pool_with_no_slots_is_refused_at_construction() {
        let control = scratch("zero");
        let error = Pool::open(&control, 0).unwrap_err();
        assert_eq!(error.reason(), ReasonCode::IntegrityMismatch);
    }

    #[test]
    fn capture_records_the_slot_and_latest_names_it() {
        let base = scratch("capture");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("a.txt"), "one").unwrap();

        let pool = Pool::open(&base.join("control"), 3).unwrap();
        let record = pool.capture(&target, &[], record_for).unwrap();

        assert_eq!(pool.latest().unwrap().unwrap(), record);
        let payload = pool.payload_of(&record.backup_ref).unwrap();
        assert_eq!(fs::read_to_string(payload.join("a.txt")).unwrap(), "one");
    }

    #[test]
    fn slots_are_ordered_by_sequence_so_latest_is_the_last_captured() {
        let base = scratch("order");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        let pool = Pool::open(&base.join("control"), 5).unwrap();

        fs::write(target.join("a.txt"), "one").unwrap();
        let first = pool.capture(&target, &[], record_for).unwrap();
        fs::write(target.join("a.txt"), "two").unwrap();
        let second = pool.capture(&target, &[], record_for).unwrap();

        assert_eq!(
            pool.latest().unwrap().unwrap().backup_ref,
            second.backup_ref
        );
        let listed = pool.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].backup_ref, first.backup_ref);

        // The chosen older slot still restores the bytes it captured.
        let payload = pool.payload_of(&first.backup_ref).unwrap();
        assert_eq!(fs::read_to_string(payload.join("a.txt")).unwrap(), "one");
    }

    #[test]
    fn a_slot_without_its_marker_is_partial_and_never_offered_for_restore() {
        let base = scratch("partial");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("a.txt"), "one").unwrap();

        let control = base.join("control");
        let pool = Pool::open(&control, 3).unwrap();
        let record = pool.capture(&target, &[], record_for).unwrap();

        // Simulate an interruption between payload copy and marker write.
        let slot = control
            .join(POOL_DIRECTORY_NAME)
            .join(record.backup_ref.as_str());
        fs::remove_file(slot.join(SLOT_MARKER_NAME)).unwrap();

        assert_eq!(pool.partial_slots().unwrap(), vec![slot]);
        assert!(pool.list().unwrap().is_empty());
        assert_eq!(
            pool.payload_of(&record.backup_ref).unwrap_err().reason(),
            ReasonCode::IntegrityMismatch
        );
    }

    #[test]
    fn pruning_bounds_completed_slots_and_leaves_partial_evidence_alone() {
        let base = scratch("prune");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        let control = base.join("control");
        let pool = Pool::open(&control, 2).unwrap();

        for index in 0..4 {
            fs::write(target.join("a.txt"), format!("{index}")).unwrap();
            pool.capture(&target, &[], record_for).unwrap();
        }
        let listed = pool.list().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].backup_ref, BackupRef::from_sequence(4));
        assert_eq!(listed[1].backup_ref, BackupRef::from_sequence(3));

        // A partial slot survives pruning because recovery still needs it.
        let partial = control.join(POOL_DIRECTORY_NAME).join("slot-000000000009");
        fs::create_dir_all(partial.join(SLOT_PAYLOAD_NAME)).unwrap();
        pool.prune().unwrap();
        assert!(partial.exists());
    }

    #[test]
    fn excluded_top_level_entries_are_not_captured() {
        let base = scratch("exclude");
        let target = base.join("target");
        fs::create_dir_all(target.join(".ctl")).unwrap();
        fs::write(target.join(".ctl").join("journal.json"), "{}").unwrap();
        fs::write(target.join("a.txt"), "one").unwrap();

        let pool = Pool::open(&base.join("control"), 2).unwrap();
        let record = pool.capture(&target, &[".ctl"], record_for).unwrap();
        let payload = pool.payload_of(&record.backup_ref).unwrap();

        assert!(payload.join("a.txt").exists());
        assert!(!payload.join(".ctl").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_refused_rather_than_inlined() {
        let base = scratch("symlink");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(base.join("outside.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), target.join("link.txt")).unwrap();

        let pool = Pool::open(&base.join("control"), 2).unwrap();
        let error = pool.capture(&target, &[], record_for).unwrap_err();
        assert_eq!(error.reason(), ReasonCode::IntegrityMismatch);
    }
}
