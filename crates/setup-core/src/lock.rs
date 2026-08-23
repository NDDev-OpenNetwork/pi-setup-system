//! The exclusive lock every mutation holds, and the durable write it protects.
//!
//! The lock is bound to the canonical target directory, not to a process, a
//! user or a setup id. Two managers pointed at the same target contend; two
//! pointed at different targets never do.
//!
//! Acquisition is non-blocking on purpose. A caller that waits silently on a
//! lock held by a crashed process looks identical to a caller doing slow work,
//! and the consumer's timeout would then be charged to the wrong cause.
//!
//! # Why the operating-system lock is not enough on its own
//!
//! Advisory file locks differ in what they are owned by. A POSIX record lock is
//! owned by the *process*, so a second acquisition from inside one process
//! succeeds and silently merges with the first: no contention is reported, and
//! two code paths both believe they hold the target exclusively. Other flavours
//! bind to the open file description and do report it.
//!
//! This kernel does not depend on which flavour a platform provides. Each setup
//! system ships one binary with two surfaces, so a wire command and a human
//! command can reach this lock in one process; [`HELD_TARGETS`] claims the path
//! in-process first, and only then is the operating-system lock taken for the
//! cross-process case. Both are released on drop, and the in-process refusal is
//! the same on every platform.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::error::{Error, ReasonCode, Result};

/// The file name the lock is taken on inside the control directory.
pub const LOCK_FILE_NAME: &str = "target.lock";

/// Lock paths currently claimed by this process.
static HELD_TARGETS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

fn held_targets() -> &'static Mutex<HashSet<PathBuf>> {
    HELD_TARGETS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Claim a path in-process, or report that this process already holds it.
///
/// A poisoned mutex is recovered rather than propagated: the guarded value is a
/// set of paths, and a panic elsewhere does not make the set meaningless. Losing
/// the ability to take any lock afterwards would be the worse outcome.
fn claim_in_process(path: &Path) -> bool {
    let mut held = held_targets()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    held.insert(path.to_path_buf())
}

fn release_in_process(path: &Path) {
    let mut held = held_targets()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    held.remove(path);
}

/// An exclusive, target-bound lock released when the value is dropped.
#[derive(Debug)]
pub struct TargetLock {
    file: File,
    path: PathBuf,
}

impl TargetLock {
    /// Take the lock without waiting.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::LockUnavailable`] if another holder has it, and
    /// [`ReasonCode::StateUnavailable`] if the lock file cannot be opened.
    pub fn acquire(control_directory: &Path) -> Result<Self> {
        let path = control_directory.join(LOCK_FILE_NAME);

        if !claim_in_process(&path) {
            return Err(Error::new(
                ReasonCode::LockUnavailable,
                format!("this process already holds {}", path.display()),
            ));
        }

        let acquire_os_lock = || -> Result<File> {
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .map_err(|source| {
                    Error::new(
                        ReasonCode::StateUnavailable,
                        format!("cannot open lock file {}", path.display()),
                    )
                    .with_source(source)
                })?;
            file.try_lock().map_err(|source| {
                Error::new(
                    ReasonCode::LockUnavailable,
                    format!("another process holds {}", path.display()),
                )
                .with_source(source)
            })?;
            Ok(file)
        };

        match acquire_os_lock() {
            Ok(file) => Ok(Self { file, path }),
            Err(error) => {
                // The in-process claim must not outlive a failed acquisition, or
                // the next attempt would be refused by this process forever.
                release_in_process(&path);
                Err(error)
            }
        }
    }

    /// The lock file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Record a non-secret owner note inside the lock file.
    ///
    /// The note is diagnostic only. Nothing reads it to make a decision, because
    /// a decision taken from an unverified note inside a contended file is a
    /// decision taken from whatever the previous holder left behind.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the note cannot be written.
    pub fn annotate(&mut self, note: &str) -> Result<()> {
        self.file
            .set_len(0)
            .and_then(|()| self.file.write_all(note.as_bytes()))
            .map_err(|source| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot annotate {}", self.path.display()),
                )
                .with_source(source)
            })
    }
}

impl Drop for TargetLock {
    fn drop(&mut self) {
        // A failed unlock is not actionable here: the value is going away and
        // the operating system releases the lock when the descriptor closes.
        let _ = self.file.unlock();
        release_in_process(&self.path);
    }
}

/// Replace a file's contents durably, or leave the previous contents intact.
///
/// The bytes land in a sibling temporary file that is flushed to disk before the
/// rename, so an interruption leaves either the old file or the new one — never
/// a half-written file that parses as valid state.
///
/// Missing parent directories are created. A namespace can be nested — Codex
/// routes skills to `.agents/skills`, Antigravity keeps everything under
/// subdirectories of a home it shares — so writing one is routinely the act
/// that first creates its directory. Doing it here rather than at each call
/// site is what keeps two write paths from disagreeing, which they did: the
/// bundle path created parents and the catalog path did not, so a setup with a
/// nested file installed over the wire and failed from disk.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if any step fails.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(Error::new(
            ReasonCode::StateUnavailable,
            format!("{} has no parent directory", path.display()),
        ));
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Err(Error::new(
            ReasonCode::StateUnavailable,
            format!("{} has no usable file name", path.display()),
        ));
    };
    fs::create_dir_all(parent).map_err(|source| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot create {}", parent.display()),
        )
        .with_source(source)
    })?;
    let temporary = parent.join(format!(".{file_name}.staging"));

    let write = || -> std::io::Result<()> {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    write().map_err(|source| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot stage {}", temporary.display()),
        )
        .with_source(source)
    })?;

    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        Error::new(
            ReasonCode::StateUnavailable,
            format!(
                "cannot promote {} to {}",
                temporary.display(),
                path.display()
            ),
        )
        .with_source(source)
    })?;

    sync_directory(parent);
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) {
    // The rename is only durable once the directory entry itself is flushed.
    // A failure here is not recoverable by retrying and does not invalidate the
    // rename, so it is observed and not escalated.
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {
    // Windows has no directory handle to flush in the POSIX sense; the rename
    // is ordered by the file system itself.
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("setup-core-lock-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn a_second_holder_in_this_process_is_refused_rather_than_silently_merged() {
        // The operating-system lock underneath is process-owned, so this second
        // acquisition would succeed on its own. The in-process claim is what
        // makes it a refusal, and this test fails if that claim is removed.
        let control = scratch("contended");
        let first = TargetLock::acquire(&control).unwrap();
        let error = TargetLock::acquire(&control).unwrap_err();
        assert_eq!(error.reason(), ReasonCode::LockUnavailable);
        drop(first);
    }

    #[test]
    fn a_refused_acquisition_does_not_strand_the_claim_it_failed_to_complete() {
        let control = scratch("no-leak");
        let first = TargetLock::acquire(&control).unwrap();
        assert!(TargetLock::acquire(&control).is_err());
        assert!(TargetLock::acquire(&control).is_err());
        drop(first);
        // If the refused attempts had leaked their claim, this would fail.
        assert!(TargetLock::acquire(&control).is_ok());
    }

    #[test]
    fn the_lock_is_released_when_the_holder_is_dropped() {
        let control = scratch("released");
        drop(TargetLock::acquire(&control).unwrap());
        let second = TargetLock::acquire(&control);
        assert!(second.is_ok());
    }

    #[test]
    fn two_targets_do_not_contend() {
        let one = scratch("target-one");
        let two = scratch("target-two");
        let first = TargetLock::acquire(&one).unwrap();
        let second = TargetLock::acquire(&two);
        assert!(second.is_ok());
        drop(first);
    }

    #[test]
    fn an_atomic_write_creates_the_directories_its_path_names() {
        // A nested namespace is written before its directory exists: Codex's
        // `.agents/skills` and every Antigravity namespace are the first thing
        // to create their own parent. This failed with "cannot stage" until the
        // creation moved in here, where both write paths reach it.
        let base = scratch("atomic-nested");
        let file = base.join("antigravity-cli").join("settings.json");
        atomic_write(&file, b"{}").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"{}");

        let deeper = base.join("a").join("b").join("c").join("leaf");
        atomic_write(&deeper, b"deep").unwrap();
        assert_eq!(fs::read(&deeper).unwrap(), b"deep");
    }

    #[test]
    fn an_atomic_write_replaces_contents_and_leaves_no_staging_file() {
        let base = scratch("atomic");
        let file = base.join("state.json");
        atomic_write(&file, b"first").unwrap();
        atomic_write(&file, b"second").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"second");
        assert!(!base.join(".state.json.staging").exists());
    }
}
