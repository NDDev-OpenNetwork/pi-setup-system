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
    /// The definition digest that setup was identified by, when one was stamped.
    ///
    /// Added after the schema was in use, and deliberately not a schema bump: a
    /// slot written before it is still a complete, restorable capture, and
    /// refusing to read one would trade a recoverable target for a field.
    #[serde(default)]
    pub setup_definition_digest: Option<String>,
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

    /// Read an existing pool without creating one.
    ///
    /// `open` makes the pool directory, which is right when a mutation is about
    /// to capture into it and wrong when a command is only reporting: a caller
    /// that asks what backups exist should not thereby create the place they
    /// would live. `list` and friends work on the returned value either way,
    /// because a pool whose root is absent simply has no slots.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::IntegrityMismatch`] if `capacity` is zero, for the
    /// same reason [`Pool::open`] does.
    pub fn observe(control_directory: &Path, capacity: usize) -> Result<Self> {
        if capacity == 0 {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                "a backup pool with no slots cannot support restore",
            ));
        }
        Ok(Self {
            root: control_directory.join(POOL_DIRECTORY_NAME),
            capacity,
        })
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

    /// Capture the named paths of `source` into a new slot and complete it.
    ///
    /// The payload is copied first and the marker written last, so an
    /// interruption leaves a slot [`Pool::partial_slots`] can name.
    ///
    /// # Only what the caller can undo
    ///
    /// `included` names the paths a restore can put back, and nothing else is
    /// copied. That is not an optimisation bolted on afterwards -- it is the
    /// only honest scope, because a restore reads exactly these paths out of
    /// the payload again. Anything else in the slot could never be restored by
    /// the code that wrote it.
    ///
    /// Capturing the whole directory instead was measured against a real Grok
    /// home: 517 MB copied -- the product's downloads, marketplace cache,
    /// vendored runtime and logs -- to protect four kilobytes of configuration,
    /// doubling the directory on the first operation and multiplying it by the
    /// slot count thereafter. It also failed outright, because a live install
    /// keeps a symlink at `bin/grok` and this copy refuses symlinks; the
    /// provider could not touch a real installation at all. Both problems were
    /// the same mistake, and both end here.
    ///
    /// A path that does not exist is skipped: a target legitimately holds only
    /// some of what a harness may own.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the copy or the marker write
    /// fails.
    pub fn capture(
        &self,
        source: &Path,
        included: &[&str],
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

        for relative in included {
            let from = source.join(relative);
            if !from.exists() {
                continue;
            }
            let to = payload.join(relative);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|source_error| {
                    Error::new(
                        ReasonCode::StateUnavailable,
                        format!("cannot create {}", parent.display()),
                    )
                    .with_source(source_error)
                })?;
            }
            if from.is_dir() {
                copy_tree(&from, &to, &[])?;
            } else {
                copy_file(&from, &to)?;
            }
        }

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

/// Copy one regular file, refusing a symlink for the same reason a tree does.
///
/// A symlink captured into a backup slot is a pointer, not the bytes it points
/// at: restoring it would recreate a link whose target may have moved, been
/// deleted, or been replaced by something else entirely. Refusing keeps a slot
/// a statement about content.
fn copy_file(from: &Path, to: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(from).map_err(|error| {
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
    fs::copy(from, to).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot copy {} to {}", from.display(), to.display()),
        )
        .with_source(error)
    })?;
    Ok(())
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
            setup_definition_digest: Some("sha256:definition".to_owned()),
        }
    }

    #[test]
    fn a_slot_written_before_the_definition_digest_existed_still_reads() {
        // The field was added without a schema bump, so a slot captured by an
        // earlier build must still restore. Refusing one would trade a
        // recoverable target for a field that is allowed to be absent.
        let older = serde_json::json!({
            "schema_version": SLOT_SCHEMA,
            "backup_ref": "slot-000000000001",
            "operation": "install",
            "operation_id": "op_test",
            "target_identity_digest": "sha256:target",
            "setup_id": "full-auto",
        });
        let read: SlotRecord = serde_json::from_value(older).unwrap();
        assert_eq!(read.setup_id.as_deref(), Some("full-auto"));
        assert_eq!(read.setup_definition_digest, None);
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
        let record = pool.capture(&target, &["a.txt"], record_for).unwrap();

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
        let first = pool.capture(&target, &["a.txt"], record_for).unwrap();
        fs::write(target.join("a.txt"), "two").unwrap();
        let second = pool.capture(&target, &["a.txt"], record_for).unwrap();

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
        let record = pool.capture(&target, &["a.txt"], record_for).unwrap();

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
            pool.capture(&target, &["a.txt"], record_for).unwrap();
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
    fn only_the_named_paths_are_captured() {
        // The scope is what a restore can put back, and nothing else. Capturing
        // the whole directory once copied 517 MB of a Grok home's downloads and
        // caches to protect four kilobytes of configuration -- bytes no restore
        // in this kernel would ever read again.
        let base = scratch("included");
        let target = base.join("target");
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::create_dir_all(target.join("downloads")).unwrap();
        fs::write(target.join("config.toml"), "kept").unwrap();
        fs::write(target.join("skills/a.md"), "kept too").unwrap();
        fs::write(target.join("downloads/huge.bin"), "not ours").unwrap();
        fs::write(target.join("auth.json"), "secret").unwrap();

        let pool = Pool::open(&base.join("ctl"), 3).unwrap();
        let record = pool
            .capture(&target, &["config.toml", "skills"], record_for)
            .unwrap();
        let payload = pool.payload_of(&record.backup_ref).unwrap();

        assert_eq!(
            fs::read_to_string(payload.join("config.toml")).unwrap(),
            "kept"
        );
        assert_eq!(
            fs::read_to_string(payload.join("skills/a.md")).unwrap(),
            "kept too"
        );
        assert!(!payload.join("downloads").exists());
        assert!(!payload.join("auth.json").exists());
    }

    #[test]
    fn a_path_the_target_does_not_hold_is_skipped_not_fatal() {
        // A harness may own more than any one target happens to contain.
        let base = scratch("included-absent");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("config.toml"), "here").unwrap();

        let pool = Pool::open(&base.join("ctl"), 3).unwrap();
        let record = pool
            .capture(
                &target,
                &["config.toml", "never-created", "skills"],
                record_for,
            )
            .unwrap();
        let payload = pool.payload_of(&record.backup_ref).unwrap();
        assert_eq!(
            fs::read_to_string(payload.join("config.toml")).unwrap(),
            "here"
        );
        assert!(!payload.join("skills").exists());
    }

    // Creating a symlink on Windows needs a privilege a runner does not have,
    // so this is asserted where it can be.
    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_refused_rather_than_inlined() {
        // Both shapes: named directly, and found while walking a named
        // directory. A slot has to be a statement about content, and a link is
        // a pointer whose target may since have moved or been replaced.
        let base = scratch("symlink");
        let target = base.join("target");
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(base.join("outside.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), target.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), target.join("skills/deep.md"))
            .unwrap();

        let pool = Pool::open(&base.join("control"), 2).unwrap();
        let named = pool
            .capture(&target, &["link.txt"], record_for)
            .unwrap_err();
        assert_eq!(named.reason(), ReasonCode::IntegrityMismatch);

        let walked = pool.capture(&target, &["skills"], record_for).unwrap_err();
        assert_eq!(walked.reason(), ReasonCode::IntegrityMismatch);
    }

    // Creating a symlink on Windows needs a privilege a runner does not have,
    // so this is asserted where it can be.
    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_outside_the_captured_paths_is_simply_not_seen() {
        // A live Grok install keeps a symlink at `bin/grok`, which no harness
        // here owns. Capturing the whole directory met it and refused, so the
        // provider could not touch a real installation at all. Scoped to what a
        // restore can use, the link is never reached.
        let base = scratch("symlink-outside");
        let target = base.join("target");
        fs::create_dir_all(target.join("bin")).unwrap();
        fs::write(base.join("outside.txt"), "runtime").unwrap();
        std::os::unix::fs::symlink(base.join("outside.txt"), target.join("bin/grok")).unwrap();
        fs::write(target.join("config.toml"), "ours").unwrap();

        let pool = Pool::open(&base.join("control"), 2).unwrap();
        let record = pool.capture(&target, &["config.toml"], record_for).unwrap();
        let payload = pool.payload_of(&record.backup_ref).unwrap();
        assert_eq!(
            fs::read_to_string(payload.join("config.toml")).unwrap(),
            "ours"
        );
        assert!(!payload.join("bin").exists());
    }
}
