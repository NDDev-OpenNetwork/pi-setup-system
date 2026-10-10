//! Admit a missing prefix in private operational state before publishing it.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use cap_std::fs::{Dir, DirBuilder};
use serde::{Deserialize, Serialize};

use super::{
    Software, Writer,
    operation::{Intent, State},
    records, store,
    writer::CONTROL,
};
use crate::{Result, digest, software_prefix};
use records::{Identity, io, refuse};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    parent: Identity,
    leaf: String,
    stage: String,
}

fn stage_name(leaf: &str) -> String {
    format!(
        ".nddev-prefix-{}",
        digest::of_bytes(leaf.as_bytes()).trim_start_matches("sha256:")
    )
}

impl Binding {
    pub(super) fn validate(&self) -> Result<()> {
        if !records::leaf(&self.leaf) || self.stage != stage_name(&self.leaf) {
            return Err(refuse());
        }
        Ok(())
    }

    pub(super) fn location(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(refuse)?;
        let parent = io(Dir::open_ambient_dir(
            path.parent().ok_or_else(refuse)?,
            cap_std::ambient_authority(),
        ))?;
        if ![self.leaf.as_str(), self.stage.as_str()].contains(&name)
            || Identity::of(&parent)? != self.parent
        {
            return Err(refuse());
        }
        Ok(())
    }
}

/// A held parent and one deterministic private reservation for its missing child.
///
/// Opening and reading are non-mutating. The runtime validates all original
/// first-admission preconditions before acquiring a staging writer. Publication
/// moves only the admitted empty prefix and never replaces an existing entry.
pub struct Allocation {
    parent: Dir,
    root: PathBuf,
    stage: PathBuf,
    binding: Binding,
}

impl Allocation {
    /// Hold the existing parent without creating it or the requested prefix.
    ///
    /// # Errors
    /// Refuses an invalid path, an unavailable parent or a present final entry.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(refuse());
        }
        let parent_path = root.parent().ok_or_else(refuse)?;
        let leaf = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(refuse)?;
        if !records::leaf(leaf) {
            return Err(refuse());
        }
        let parent = io(Dir::open_ambient_dir(
            parent_path,
            cap_std::ambient_authority(),
        ))?;
        let binding = Binding {
            parent: Identity::of(&parent)?,
            leaf: leaf.to_owned(),
            stage: stage_name(leaf),
        };
        let reservation = Self {
            parent,
            root: root.to_owned(),
            stage: parent_path.join(&binding.stage),
            binding,
        };
        reservation.revalidate()?;
        Ok(reservation)
    }

    fn revalidate(&self) -> Result<()> {
        self.binding.location(&self.root)?;
        if Identity::of(&self.parent)? != self.binding.parent {
            return Err(refuse());
        }
        match self.parent.symlink_metadata(&self.binding.leaf) {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            _ => Err(refuse()),
        }
    }

    fn directory(&self) -> Result<Option<Dir>> {
        self.revalidate()?;
        let directory = records::present(&self.parent, &self.binding.stage)?;
        if let Some(directory) = &directory {
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            {
                use cap_std::fs::{MetadataExt, PermissionsExt};
                let metadata = io(directory.dir_metadata())?;
                if metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.permissions().mode() & 0o777 != 0o700
                {
                    return Err(refuse());
                }
            }
            // No software may be written until publication. An interrupted
            // metadata initialization is recognizable; any other content stays.
            for entry in io(directory.entries())? {
                if io(entry)?.file_name() != CONTROL {
                    return Err(refuse());
                }
            }
            store::validate_control(directory)?;
        }
        Ok(directory)
    }

    /// Read only this reservation's original caller admission.
    ///
    /// # Errors
    /// Refuses a different caller, changed parent or foreign reservation state.
    pub fn state(&self, intent: &Intent) -> Result<Option<State>> {
        let Some(directory) = self.directory()? else {
            return Ok(None);
        };
        let Some((record, outcome)) = store::operation(&directory, &intent.binding)? else {
            return Ok(None);
        };
        if record.intent != *intent
            || record.root_identity != Identity::of(&directory)?
            || record.allocation.as_ref() != Some(&self.binding)
            || outcome.is_some()
        {
            return Err(refuse());
        }
        Ok(Some(State::Pending))
    }

    /// Create only temporary operational state and hold its common writer.
    ///
    /// # Errors
    /// Refuses contention, foreign entries or a changed parent/final prefix.
    pub fn writer(&self) -> Result<Writer> {
        self.directory()?;
        #[cfg(unix)]
        let builder = {
            use cap_std::fs::DirBuilderExt;
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = DirBuilder::new();
        match self.parent.create_dir_with(&self.binding.stage, &builder) {
            Ok(()) => records::sync(&self.parent)?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(refuse().with_source(error)),
        }
        self.directory()?.ok_or_else(refuse)?;
        Writer::acquire(&self.stage)
    }

    /// Persist the exact caller admission before making the final prefix exist.
    ///
    /// The runtime rechecks expiry, original absence and all immutable inputs
    /// under this writer before calling. Existing admitted requests use publish.
    ///
    /// # Errors
    /// Refuses foreign history, other requests, changed parents or owned entries.
    pub fn admit(
        &self,
        writer: &Writer,
        intent: &Intent,
        software: &Software,
        control: &str,
    ) -> Result<()> {
        self.revalidate()?;
        if writer.root != self.stage {
            return Err(refuse());
        }
        let directory = self.directory()?.ok_or_else(refuse)?;
        if !store::vacant(&directory)? {
            return Err(refuse());
        }
        let expected = software_prefix::observe(&self.stage, control)?.digest;
        writer.admit_at(
            intent,
            software,
            control,
            &expected,
            Some(self.binding.clone()),
        )
    }

    /// Atomically publish the admitted prefix, refusing every existing entry.
    ///
    /// # Errors
    /// Refuses changed identities, nonempty reservations and publication failure.
    pub fn publish(&self, writer: Writer, intent: &Intent) -> Result<Writer> {
        self.revalidate()?;
        if writer.root != self.stage {
            return Err(refuse());
        }
        let (record, outcome) = writer.recorded_operation(intent)?.ok_or_else(refuse)?;
        if record.allocation.as_ref() != Some(&self.binding) || outcome.is_some() {
            return Err(refuse());
        }
        let directory = self.directory()?.ok_or_else(refuse)?;
        if Identity::of(&directory)? != record.root_identity
            || store::pending_for(&directory, Some(&intent.binding))?
        {
            return Err(refuse());
        }
        record.check_initial(&writer)?;
        records::sync(&directory)?;
        // Windows directory handles block a rename. No program effect is
        // permitted while this reservation exists; the final writer serializes
        // all effects after the one no-replace publication succeeds.
        drop(directory);
        drop(writer);
        self.revalidate()?;
        publish(&self.parent, &self.binding.stage, &self.binding.leaf)?;
        records::sync(&self.parent)?;
        let published = records::open_root(&self.root)?;
        if Identity::of(&published)? != record.root_identity {
            return Err(refuse());
        }
        drop(published);
        let writer = Writer::acquire(&self.root)?;
        writer.recorded_operation(intent)?.ok_or_else(refuse)?;
        Ok(writer)
    }
}

fn publish(parent: &Dir, stage: &str, leaf: &str) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        io(rustix::fs::renameat_with(
            parent,
            stage,
            parent,
            leaf,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from))
    }
    #[cfg(windows)]
    {
        // cap-std delegates to std::fs::rename, whose Windows fallback can
        // replace an empty directory. Call MoveFileEx without replacement or
        // cross-volume copy flags, using the held parent's current path.
        let held = io(parent.try_clone())?.into_std_file();
        let parent_path = io(winx::file::get_file_path(&held))?;
        let source = parent_path.join(stage);
        let destination = parent_path.join(leaf);
        winsafe::MoveFileEx(
            source.to_str().ok_or_else(refuse)?,
            Some(destination.to_str().ok_or_else(refuse)?),
            winsafe::co::MOVEFILE::WRITE_THROUGH,
        )
        .map_err(|_| refuse())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (parent, stage, leaf);
        Err(crate::Error::new(
            crate::ReasonCode::UnsupportedPlatform,
            "atomic prefix publication is unavailable on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::super::{
        Delivery,
        operation::{self, Kind, Outcome},
    };
    use super::*;
    use std::fs;

    #[cfg(unix)]
    fn parent_substitution_is_refused(at: &Path, allocation: &Allocation, intent: &Intent) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&allocation.stage, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(allocation.state(intent).is_err());
        fs::set_permissions(&allocation.stage, fs::Permissions::from_mode(0o700)).unwrap();
        let moved = at.with_extension("held");
        fs::rename(at, &moved).unwrap();
        fs::create_dir(at).unwrap();
        assert!(allocation.state(intent).is_err());
        fs::remove_dir(at).unwrap();
        fs::rename(moved, at).unwrap();
        assert_eq!(allocation.state(intent).unwrap(), Some(State::Pending));
    }

    #[test]
    fn admission_precedes_publication_and_foreign_entries_are_never_replaced() {
        let at =
            std::env::temp_dir().join(format!("prefix-allocation-{}", records::unique().unwrap()));
        fs::create_dir(&at).unwrap();
        let root = at.join("programs");
        let software = Software {
            command: "tool",
            version: "1.2.3",
            delivery: Delivery::Artifacts(&[]),
            unsupported: &[],
            previous: None,
        };
        let intent = Intent::new(
            "first",
            &digest::of_bytes(b"exact missing prefix plan"),
            &software,
            Kind::Remove,
        )
        .unwrap();
        let allocation = Allocation::open(&root).unwrap();
        assert_eq!(allocation.state(&intent).unwrap(), None);
        assert!(!allocation.stage.exists());
        let writer = allocation.writer().unwrap();
        writer.operation_state(&intent).unwrap();
        allocation
            .admit(&writer, &intent, &software, ".test-provider")
            .unwrap();
        let stage = allocation.stage.clone();
        drop(writer);
        drop(allocation);
        assert!(!root.exists());

        // Resume the durable reservation after losing all original handles.
        let allocation = Allocation::open(&root).unwrap();
        let database = stage.join(CONTROL).join("records.sqlite3");
        let before = fs::read(&database).unwrap();
        assert_eq!(allocation.state(&intent).unwrap(), Some(State::Pending));
        assert_eq!(fs::read(&database).unwrap(), before);
        #[cfg(unix)]
        parent_substitution_is_refused(&at, &allocation, &intent);
        let conflict = Intent::new(
            "first",
            &digest::of_bytes(b"changed plan"),
            &software,
            Kind::Remove,
        )
        .unwrap();
        assert!(allocation.state(&conflict).is_err());
        fs::write(stage.join("foreign"), b"preserve stage content").unwrap();
        assert!(allocation.writer().is_err());
        assert_eq!(
            fs::read(stage.join("foreign")).unwrap(),
            b"preserve stage content"
        );
        fs::remove_file(stage.join("foreign")).unwrap();
        let writer = allocation.writer().unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("foreign"), b"preserve final content").unwrap();
        assert!(allocation.publish(writer, &intent).is_err());
        assert_eq!(
            fs::read(root.join("foreign")).unwrap(),
            b"preserve final content"
        );
        // Test the primitive too: even an empty late destination must survive.
        fs::remove_file(root.join("foreign")).unwrap();
        let identity = Identity::of(&records::open_root(&root).unwrap()).unwrap();
        assert!(
            publish(
                &allocation.parent,
                &allocation.binding.stage,
                &allocation.binding.leaf
            )
            .is_err()
        );
        assert_eq!(
            Identity::of(&records::open_root(&root).unwrap()).unwrap(),
            identity
        );
        fs::remove_dir(&root).unwrap();
        let writer = allocation.writer().unwrap();
        let writer = allocation.publish(writer, &intent).unwrap();
        assert!(!stage.exists());
        assert_eq!(
            writer.operation_state(&intent).unwrap(),
            Some(State::Pending)
        );
        assert_eq!(
            writer.remove_operation(&intent, &software).unwrap(),
            Outcome::Removed { removed: false }
        );
        drop(writer);
        assert_eq!(
            operation::state(&root, &intent).unwrap(),
            Some(State::Completed(Outcome::Removed { removed: false }))
        );
        drop(allocation);
        fs::remove_dir_all(at).unwrap();
    }
}
