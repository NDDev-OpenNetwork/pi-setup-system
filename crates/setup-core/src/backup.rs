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

/// The marker naming a slot that retention may not reclaim.
///
/// Beside the record rather than inside it, on purpose. The record states what
/// was *captured*; a hold states whether retention may take it back. Those are
/// two different facts about one slot, and writing the second into the first
/// would mean editing a completed capture every time someone changed their mind
/// about keeping it.
///
/// The file holds the reason the hold was placed. A pool can be held by more
/// than one run at a time, and a refusal that says only *which* slots are held
/// leaves the next caller releasing one blind — possibly one another run is
/// still depending on.
pub const SLOT_HELD_NAME: &str = "HELD";

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

    /// Keep one slot until it is explicitly released.
    ///
    /// The pool rolls: ten slots, oldest evicted. A long evidence series makes
    /// more captures than that, so the baseline it means to return to at the
    /// end is gone by the time it gets there — reported after a run of fifty
    /// captures could not restore the state it started from.
    ///
    /// A hold is the smallest thing that fixes it: one marker that eviction has
    /// to read. Against an export/import format, which is a second format to
    /// keep correct, and against a separate baseline store, which is a second
    /// place state lives. With a hold, *retention never silently changes what a
    /// `BackupRef` names* becomes a checkable statement rather than an
    /// intention.
    ///
    /// # Errors
    ///
    /// Refuses a reference this pool does not hold, and refuses a hold that
    /// would leave the pool no slot to rotate — ten held slots is a target that
    /// can never be backed up again, which is a worse failure than the eviction
    /// it was protecting against. The refusal names what is already held so a
    /// caller knows what to release.
    pub fn hold(&self, backup_ref: &BackupRef, reason: &str) -> Result<bool> {
        let slot = self.root.join(backup_ref.as_str());
        if read_record(&slot)?.is_none() {
            return Err(Error::new(
                ReasonCode::InvalidTarget,
                format!(
                    "{} is not a completed slot in this pool",
                    backup_ref.as_str()
                ),
            ));
        }
        let already = self.held()?;
        if already.iter().any(|(held, _)| held == backup_ref) {
            return Ok(false);
        }
        // One slot must stay reclaimable, or the next capture has nothing to
        // evict and the pool grows past the bound it was opened with.
        if already.len() + 1 >= self.capacity {
            return Err(Error::new(
                ReasonCode::InvalidTarget,
                format!(
                    "holding {} would leave this pool of {} no slot to rotate; release one of \
                     these first, and the reason each names is who would lose it: {}",
                    backup_ref.as_str(),
                    self.capacity,
                    already
                        .iter()
                        .map(|(held, why)| format!("{} ({why})", held.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        // The reason travels with the hold. Without it a caller reading a full
        // pool knows what to release and not what releasing it would cost.
        fs::write(slot.join(SLOT_HELD_NAME), reason.as_bytes()).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot hold {}", slot.display()),
            )
            .with_source(source)
        })?;
        Ok(true)
    }

    /// Let retention have a slot back.
    ///
    /// Answers `false` for a slot that was not held rather than refusing: a run
    /// cleaning up after itself should not have to tell "nothing to do" apart
    /// from "something is wrong", which is the same reason `remove` answers
    /// `removed: false` on a prefix it never wrote.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the marker cannot be removed.
    pub fn release(&self, backup_ref: &BackupRef) -> Result<bool> {
        let marker = self.root.join(backup_ref.as_str()).join(SLOT_HELD_NAME);
        match fs::remove_file(&marker) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot release {}", marker.display()),
            )
            .with_source(source)),
        }
    }

    /// Every slot retention may not reclaim, newest first, with why.
    ///
    /// # Errors
    ///
    /// Propagates the refusal from [`Pool::list`].
    pub fn held(&self) -> Result<Vec<(BackupRef, String)>> {
        let mut out = Vec::new();
        for record in self.list()? {
            let marker = self
                .root
                .join(record.backup_ref.as_str())
                .join(SLOT_HELD_NAME);
            if let Ok(reason) = fs::read_to_string(&marker) {
                let reason = reason.trim().to_owned();
                let reason = if reason.is_empty() {
                    // A hold placed before reasons were carried, or one whose
                    // caller gave none. Saying so beats an empty parenthesis.
                    "no reason recorded".to_owned()
                } else {
                    reason
                };
                out.push((record.backup_ref, reason));
            }
        }
        Ok(out)
    }

    /// Why one slot is held, when it is.
    ///
    /// # Errors
    ///
    /// Propagates the refusal from [`Pool::list`].
    pub fn held_reason(&self, backup_ref: &BackupRef) -> Result<Option<String>> {
        Ok(self
            .held()?
            .into_iter()
            .find(|(held, _)| held == backup_ref)
            .map(|(_, reason)| reason))
    }

    /// Whether one slot is held.
    ///
    /// # Errors
    ///
    /// Never; the signature matches its neighbours so a caller reads them alike.
    pub fn is_held(&self, backup_ref: &BackupRef) -> Result<bool> {
        Ok(self
            .root
            .join(backup_ref.as_str())
            .join(SLOT_HELD_NAME)
            .is_file())
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
        // Held slots are not counted against the bound and are never reclaimed.
        // Counting them would make a hold quietly shorten the rolling window
        // instead of protecting one capture, and the caller asked for the
        // second thing.
        let records: Vec<SlotRecord> = self
            .list()?
            .into_iter()
            .filter(|record| !self.is_held(&record.backup_ref).unwrap_or(false))
            .collect();
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

/// Every owned path a capture would meet and be unable to take.
///
/// A backup slot is a statement about *content*. A link is a pointer, and one
/// captured into a slot would restore as a link whose target may have moved,
/// been deleted, or been replaced -- so `copy_file` refuses it. That refusal is
/// right and stays; what was wrong is *when* it happened.
///
/// The refusal used to arrive mid-copy: the slot was created, files were
/// written into it, and then the walk met a link and stopped. The operation
/// became `partial` and left recovery and control artifacts behind, for a shape
/// that could have been recognised before anything moved. Reported from a real
/// Windows target where four Antigravity skills under `config/skills` were
/// Junctions.
///
/// This is that recognition, and it is a reading rather than a write: it walks
/// only what the provider owns, follows nothing, and names every relative path
/// it cannot take so a caller learns all of them at once instead of one per
/// attempt.
///
/// Windows Junctions and Unix symbolic links are both reparse-style entries and
/// `symlink_metadata` reports both through `is_symlink`, which is the same
/// question `copy_file` asks. One check, both systems, and no second opinion
/// about what is capturable.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if an owned path cannot be walked.
pub fn uncapturable(source: &Path, included: &[&str]) -> Result<Vec<String>> {
    let mut refused = Vec::new();
    for relative in included {
        let from = relative
            .split('/')
            .fold(source.to_path_buf(), |at, part| at.join(part));
        collect_uncapturable(&from, relative, &mut refused)?;
    }
    refused.sort();
    Ok(refused)
}

fn collect_uncapturable(path: &Path, relative: &str, out: &mut Vec<String>) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        // Absent is not unsupported: a namespace that is not there is simply
        // nothing to capture, which `capture` already skips.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot stat {}", path.display()),
            )
            .with_source(error));
        }
    };

    if metadata.is_symlink() {
        out.push(relative.to_owned());
        // Never descend through it. Following a link out of the target is how a
        // reading of what we own becomes a reading of someone else's disk.
        return Ok(());
    }
    if !metadata.is_dir() {
        return Ok(());
    }

    let entries = fs::read_dir(path).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot list {}", path.display()),
        )
        .with_source(error)
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot read an entry of {}", path.display()),
            )
            .with_source(error)
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        collect_uncapturable(&entry.path(), &format!("{relative}/{name}"), out)?;
    }
    Ok(())
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
        let base =
            std::env::temp_dir().join(format!("setup-core-backup-{name}-{}", std::process::id()));
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

    /// The claim `#428` asks for: a held baseline survives a run longer than
    /// the pool.
    ///
    /// The pool rolls at ten. A series that captures fifty times loses the
    /// state it started from long before it gets back to it — reported after
    /// exactly that, where the final restore had nothing to restore to.
    #[test]
    fn a_held_slot_outlives_far_more_captures_than_the_pool_holds() {
        let base = scratch("held-baseline");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("a.txt"), "the baseline").unwrap();

        let pool = Pool::open(&base.join("control"), 10).unwrap();
        let baseline = pool.capture(&target, &["a.txt"], record_for).unwrap();
        assert!(
            pool.hold(&baseline.backup_ref, "E00 baseline for the evidence series")
                .unwrap()
        );

        // Fifty more, which is five times the pool.
        for round in 0..50 {
            fs::write(target.join("a.txt"), format!("round {round}")).unwrap();
            pool.capture(&target, &["a.txt"], record_for).unwrap();
            pool.prune().unwrap();
        }

        let held = pool.held().unwrap();
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].0, baseline.backup_ref);
        assert_eq!(held[0].1, "E00 baseline for the evidence series");
        assert!(
            pool.payload_of(&baseline.backup_ref).is_ok(),
            "the held baseline was reclaimed by retention"
        );
        assert_eq!(
            fs::read_to_string(pool.payload_of(&baseline.backup_ref).unwrap().join("a.txt"))
                .unwrap(),
            "the baseline",
            "the held slot survived but no longer names what it named"
        );

        // And the hold did not quietly shorten the rolling window: ten
        // unheld slots are still there beside it.
        let unheld: Vec<_> = pool
            .list()
            .unwrap()
            .into_iter()
            .filter(|r| !pool.is_held(&r.backup_ref).unwrap())
            .collect();
        assert_eq!(unheld.len(), 10);

        // Released, it becomes ordinary and the next prune reclaims it.
        assert!(pool.release(&baseline.backup_ref).unwrap());
        assert!(!pool.release(&baseline.backup_ref).unwrap());
        pool.prune().unwrap();
        assert!(pool.payload_of(&baseline.backup_ref).is_err());
    }

    /// A pool that is entirely held is a target that can never be backed up
    /// again, which is a worse failure than the eviction a hold prevents.
    #[test]
    fn a_hold_that_would_leave_nothing_to_rotate_is_refused_naming_what_to_release() {
        let base = scratch("held-full");
        let target = base.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("a.txt"), "x").unwrap();

        let pool = Pool::open(&base.join("control"), 3).unwrap();
        let first = pool.capture(&target, &["a.txt"], record_for).unwrap();
        let second = pool.capture(&target, &["a.txt"], record_for).unwrap();
        let third = pool.capture(&target, &["a.txt"], record_for).unwrap();

        assert!(pool.hold(&first.backup_ref, "series A baseline").unwrap());
        assert!(pool.hold(&second.backup_ref, "series B baseline").unwrap());
        // Holding a third of three would leave nothing to evict.
        let error = pool.hold(&third.backup_ref, "series C").unwrap_err();
        assert!(error.to_string().contains("no slot to rotate"), "{error}");
        assert!(
            error.to_string().contains(first.backup_ref.as_str()),
            "the refusal does not say what to release: {error}"
        );
        // And what releasing it would cost, so nobody releases blind.
        assert!(
            error.to_string().contains("series A baseline"),
            "the refusal does not say who holds it: {error}"
        );

        // Holding one that is already held is not an error and not a second
        // hold: a run that re-runs its own setup should not have to check.
        assert!(!pool.hold(&first.backup_ref, "series A again").unwrap());
    }

    /// A reference this pool never minted is refused rather than marked.
    #[test]
    fn holding_a_slot_that_is_not_here_is_refused() {
        let base = scratch("held-absent");
        let pool = Pool::open(&base.join("control"), 3).unwrap();
        let absent = BackupRef::parse("slot-000000000009").unwrap();
        let error = pool.hold(&absent, "nothing").unwrap_err();
        assert!(error.to_string().contains("slot-000000000009"), "{error}");
        // And releasing one that is not here is still not an error.
        assert!(!pool.release(&absent).unwrap());
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
