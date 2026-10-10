//! One held writer for every software command sharing a prefix.

use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;

use super::{Artifact, Installed, Software, VerifiedArtifact, records};
use crate::{Result, lock::TargetLock};
use records::{Identity, io, open_root, refuse, sync};

pub(crate) const CONTROL: &str = ".nddev-software";

pub(crate) use super::store::validate_control;

/// A prefix writer held across observation and all software effects.
///
/// Every harness uses the same lock. A caller with a plan must revalidate its
/// prefix precondition after acquiring this writer and before invoking a method.
pub struct Writer {
    root: PathBuf,
    identity: Identity,
    control_identity: Identity,
    lock: TargetLock,
}

impl Writer {
    /// Lock an existing plain prefix without waiting for another writer.
    ///
    /// # Errors
    /// Refuses contention, aliases, unrecognized control data or changed roots.
    pub fn acquire(root: &Path) -> Result<Self> {
        let directory = open_root(root)?;
        let identity = Identity::of(&directory)?;
        let created = match directory.create_dir(CONTROL) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(refuse().with_source(error)),
        };
        let control = io(directory.open_dir_nofollow(CONTROL))?;
        let control_identity = Identity::of(&control)?;
        // Only the shared lock and bounded metadata store are recognized.
        // An empty control directory can resume interrupted initialization.
        if !created {
            validate_control(&directory)?;
        }
        let lock = TargetLock::acquire(&root.join(CONTROL))?;
        if Identity::of(&open_root(root)?)? != identity
            || Identity::of(&io(open_root(root)?.open_dir_nofollow(CONTROL))?)? != control_identity
        {
            return Err(refuse());
        }
        sync(&control)?;
        if created {
            sync(&directory)?;
        }
        validate_control(&directory)?;
        Ok(Self {
            root: root.to_owned(),
            identity,
            control_identity,
            lock,
        })
    }

    fn revalidate(&self) -> Result<()> {
        let root = open_root(&self.root)?;
        if Identity::of(&root)? != self.identity
            || Identity::of(&io(root.open_dir_nofollow(CONTROL))?)? != self.control_identity
        {
            return Err(refuse());
        }
        validate_control(&root)?;
        self.lock.revalidate()
    }

    /// Install a held verified artifact while retaining this writer.
    ///
    /// # Errors
    /// Refuses pending transactions, changed input, ownership conflicts or I/O failures.
    pub fn install_verified(
        &self,
        software: &Software,
        artifact: &Artifact,
        source: VerifiedArtifact,
    ) -> Result<Installed> {
        self.revalidate()?;
        super::install_locked(software, artifact, source, &self.root)
    }

    /// Remove an unchanged recorded version while retaining this writer.
    ///
    /// # Errors
    /// Refuses pending transactions, changed owned entries or I/O failures.
    pub fn remove(&self, software: &Software) -> Result<bool> {
        self.revalidate()?;
        super::remove_locked(software, &self.root)
    }

    /// Complete only the recorded kernel transaction while retaining this writer.
    ///
    /// # Errors
    /// Refuses conflicting records, changed owned entries or I/O failures.
    pub fn recover(&self, software: &Software) -> Result<Vec<String>> {
        self.revalidate()?;
        super::store::recover(&open_root(&self.root)?)?;
        super::recover_locked(software, &self.root)
    }

    /// Select an installed version while retaining this writer.
    ///
    /// # Errors
    /// Refuses pending transactions, unavailable versions or I/O failures.
    pub fn rollback(&self, software: &Software, to: &str) -> Result<Installed> {
        self.revalidate()?;
        super::require_idle(software, &self.root)?;
        super::rollback_locked(software, &self.root, to)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::{
        ReasonCode,
        software::{self, Delivery},
        software_prefix,
    };
    use std::fs;

    #[test]
    fn one_prefix_writer_excludes_other_commands_and_refuses_substituted_metadata() {
        let root =
            std::env::temp_dir().join(format!("software-writer-{}", records::unique().unwrap()));
        fs::create_dir(&root).unwrap();
        let before = software_prefix::observe(&root, ".first-provider")
            .unwrap()
            .digest;
        let other = Software {
            version: "1.2.3",
            command: "other",
            delivery: Delivery::Artifacts(&[]),
            unsupported: &[],
            previous: None,
        };
        let writer = Writer::acquire(&root).unwrap();
        assert_eq!(
            before,
            software_prefix::observe(&root, ".first-provider")
                .unwrap()
                .digest
        );
        assert!(Writer::acquire(&root).is_err());
        assert_eq!(
            software::remove(&other, &root).unwrap_err().reason(),
            ReasonCode::LockUnavailable
        );
        assert_eq!(
            software::rollback(&other, &root, "1.2.3")
                .unwrap_err()
                .reason(),
            ReasonCode::LockUnavailable
        );
        let pending = root.join(".nddev-software-other.transaction.json");
        fs::write(&pending, b"generated incomplete transaction").unwrap();
        assert_eq!(
            software::recover(&other, &root).unwrap_err().reason(),
            ReasonCode::LockUnavailable
        );
        assert_eq!(
            fs::read(&pending).unwrap(),
            b"generated incomplete transaction"
        );
        fs::remove_file(pending).unwrap();
        assert!(!writer.remove(&other).unwrap());
        let held = open_root(&root).unwrap();
        let pending = super::super::store::Key::Installation("first".into());
        super::super::store::write(&held, &pending, &"generated pending operation", 1024).unwrap();
        assert!(software::require_idle(&other, &root).is_err());
        assert!(writer.remove(&other).is_err());
        super::super::store::remove(&held, &pending).unwrap();
        drop(held);
        #[cfg(unix)]
        {
            let lock = root.join(CONTROL).join(crate::lock::LOCK_FILE_NAME);
            let previous = root.join("held-lock");
            fs::rename(&lock, &previous).unwrap();
            fs::write(&lock, b"replacement lock").unwrap();
            assert!(writer.remove(&other).is_err());
            assert_eq!(fs::read(&lock).unwrap(), b"replacement lock");
            fs::remove_file(&lock).unwrap();
            fs::rename(previous, lock).unwrap();
        }
        drop(writer);
        drop(Writer::acquire(&root).unwrap());
        // Windows byte-range locks may reject content reads. The seal test
        // concerns reserved names in an artifact, not a concurrently held lock.
        let held = open_root(&root).unwrap();
        let (_, inventory) = software_prefix::inventory_directory(&held).unwrap();
        assert!(
            inventory
                .iter()
                .any(|entry| entry.path == ".nddev-software/target.lock")
        );
        drop(held);
        let unknown = root.join(CONTROL).join("personal");
        fs::write(&unknown, b"preserve").unwrap();
        assert!(Writer::acquire(&root).is_err());
        assert!(software_prefix::observe(&root, ".first-provider").is_err());
        assert_eq!(fs::read(&unknown).unwrap(), b"preserve");
        fs::remove_file(unknown).unwrap();
        // Empty interrupted initialization resumes without a format receipt.
        let lock = root.join(CONTROL).join(crate::lock::LOCK_FILE_NAME);
        fs::remove_file(&lock).unwrap();
        drop(Writer::acquire(&root).unwrap());
        assert!(lock.is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
