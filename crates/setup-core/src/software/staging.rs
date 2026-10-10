//! A recorded staging transaction; directory names never establish ownership.

use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::records::{
    self, Identity, io, leaf, member_valid, open_root, present, refuse, sync, unique,
};
use super::{cleanup, launch, ownership};
use crate::{
    Result,
    archive::Destination,
    software_prefix::{self, Entry, INVENTORY_LIMIT},
};

const RECORD_LIMIT: usize = INVENTORY_LIMIT + 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Preparing,
    Extracting,
    Promoting,
    Promoted,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    command: String,
    version: String,
    member: String,
    stage: String,
    quarantine: String,
    root_identity: Identity,
    stage_identity: Option<Identity>,
    previous_identity: Option<Identity>,
    previous_digest: Option<String>,
    previous_entries: Option<Vec<Entry>>,
    legacy: bool,
    artifact_sha256: String,
    sealed_digest: Option<String>,
    phase: Phase,
    launch: launch::Snapshot,
}

pub(super) struct Staging {
    root: Dir,
    path: PathBuf,
    journal: String,
    record: Record,
}

impl Staging {
    pub(super) fn begin(
        root: &Path,
        command: &str,
        version: &str,
        member: &str,
        artifact_sha256: &str,
    ) -> Result<Self> {
        let mut transaction = Self::prepare(root, command, version, member, artifact_sha256)?;
        transaction.create_stage()?;
        Ok(transaction)
    }

    fn prepare(
        root: &Path,
        command: &str,
        version: &str,
        member: &str,
        artifact_sha256: &str,
    ) -> Result<Self> {
        if !leaf(command)
            || !leaf(version)
            || !member_valid(member)
            || !ownership::digest_valid(artifact_sha256)
        {
            return Err(refuse());
        }
        io(std::fs::create_dir_all(root))?;
        let directory = open_root(root)?;
        let journal = format!(".nddev-software-{command}.transaction.json");
        match directory.symlink_metadata(&journal) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refuse()),
        }
        let previous = present(&directory, version)?;
        let receipt = ownership::Receipt::read(&directory, command, version)?;
        let previous_inventory = if let Some(receipt) = &receipt {
            Some((
                receipt
                    .verify(previous.as_ref().ok_or_else(refuse)?)?
                    .to_owned(),
                receipt.entries().to_vec(),
            ))
        } else {
            previous
                .as_ref()
                .map(software_prefix::inventory_directory)
                .transpose()?
        };
        let (previous_digest, previous_entries) = match previous_inventory {
            Some((digest, entries)) => (Some(digest), Some(entries)),
            None => (None, None),
        };
        let legacy = previous.is_some() && receipt.is_none();
        let previous_identity = previous.as_ref().map(Identity::of).transpose()?;
        drop(previous);
        let launch = launch::Snapshot::prepare(root, command, member)?;
        let nonce = unique()?;
        let stage = format!(".incoming-{command}-{nonce}");
        let quarantine = format!(".replaced-{command}-{nonce}");
        let transaction = Self {
            record: Record {
                schema_version: 4,
                command: command.to_owned(),
                version: version.to_owned(),
                member: member.to_owned(),
                stage,
                quarantine,
                root_identity: Identity::of(&directory)?,
                stage_identity: None,
                previous_identity,
                previous_digest,
                previous_entries,
                legacy,
                artifact_sha256: artifact_sha256.to_owned(),
                sealed_digest: None,
                phase: Phase::Preparing,
                launch,
            },
            root: directory,
            path: root.to_owned(),
            journal,
        };
        transaction.save()?;
        Ok(transaction)
    }

    fn create_stage(&mut self) -> Result<()> {
        self.check_root()?;
        // The durable name precedes mkdir. No artifact byte is written until
        // the created physical directory is durably bound to that record.
        io(self.root.create_dir(&self.record.stage))?;
        let stage = io(self.root.open_dir_nofollow(&self.record.stage))?;
        self.record.stage_identity = Some(Identity::of(&stage)?);
        sync(&stage)?;
        sync(&self.root)?;
        self.record.phase = Phase::Extracting;
        self.save()
    }

    fn stage_identity(&self) -> Result<Identity> {
        self.record.stage_identity.ok_or_else(refuse)
    }

    pub(super) fn load(path: &Path, command: &str) -> Result<Option<Self>> {
        if !leaf(command) {
            return Err(refuse());
        }
        let Some(root) = records::optional_root(path)? else {
            return Ok(None);
        };
        let journal = format!(".nddev-software-{command}.transaction.json");
        let Some(record): Option<Record> = records::read(&root, &journal, RECORD_LIMIT)? else {
            return Ok(None);
        };
        if record.schema_version != 4
            || (record.phase == Phase::Preparing) != record.stage_identity.is_none()
            || record.command != command
            || !ownership::digest_valid(&record.artifact_sha256)
            || record.previous_identity.is_some() != record.previous_digest.is_some()
            || record.previous_identity.is_some() != record.previous_entries.is_some()
            || record
                .previous_digest
                .as_deref()
                .is_some_and(|d| !ownership::digest_valid(d))
            || (record.legacy && record.previous_identity.is_none())
            || !leaf(&record.version)
            || !leaf(&record.stage)
            || !leaf(&record.quarantine)
            || !record.stage.starts_with(&format!(".incoming-{command}-"))
            || !record
                .quarantine
                .starts_with(&format!(".replaced-{command}-"))
            || !member_valid(&record.member)
            || Identity::of(&root)? != record.root_identity
        {
            return Err(refuse());
        }
        if record
            .previous_entries
            .as_deref()
            .map(software_prefix::inventory_digest)
            .transpose()?
            != record.previous_digest
        {
            return Err(refuse());
        }
        record.launch.validate(path, command, &record.member)?;
        Ok(Some(Self {
            root,
            path: path.to_owned(),
            journal,
            record,
        }))
    }

    fn save(&self) -> Result<()> {
        self.check_root()?;
        records::write(&self.root, &self.journal, &self.record, RECORD_LIMIT)
    }

    fn check_root(&self) -> Result<()> {
        if Identity::of(&open_root(&self.path)?)? != self.record.root_identity {
            return Err(refuse());
        }
        Ok(())
    }

    fn checked(&self, name: &str, expected: Identity) -> Result<Dir> {
        let directory = present(&self.root, name)?.ok_or_else(refuse)?;
        if Identity::of(&directory)? != expected {
            return Err(refuse());
        }
        Ok(directory)
    }

    pub(super) fn destination(&self) -> Result<Destination> {
        self.check_root()?;
        Ok(Destination::from_directory(
            self.checked(&self.record.stage, self.stage_identity()?)?,
        ))
    }

    pub(super) fn stage_path(&self) -> PathBuf {
        self.path.join(&self.record.stage)
    }

    pub(super) fn version(&self) -> &str {
        &self.record.version
    }

    pub(super) fn executable(&self) -> PathBuf {
        self.path
            .join(&self.record.version)
            .join(&self.record.member)
    }

    pub(super) fn expose(&self) -> Result<()> {
        let executable = self.executable();
        let exposed = self.path.join("bin").join(super::exposed_name(
            &self.record.command,
            &self.record.member,
        ));
        super::exposure::expose(
            &executable,
            &exposed,
            &self.record.version,
            &self.record.command,
            &self.record.launch,
        )
    }

    pub(super) fn promote(&mut self) -> Result<()> {
        self.check_root()?;
        self.record
            .launch
            .revalidate(&self.path, &self.record.command, &self.record.member)?;
        let stage = self.checked(&self.record.stage, self.stage_identity()?)?;
        if present(&self.root, &self.record.quarantine)?.is_some() {
            return Err(refuse());
        }
        let current = present(&self.root, &self.record.version)?;
        if current.as_ref().map(Identity::of).transpose()? != self.record.previous_identity {
            return Err(refuse());
        }
        let sealed = crate::software_prefix::digest_directory(&stage)?;
        let previous = current
            .as_ref()
            .map(crate::software_prefix::digest_directory)
            .transpose()?;
        if previous != self.record.previous_digest
            || (self.record.legacy && previous.as_deref() != Some(sealed.as_str()))
        {
            return Err(refuse());
        }
        self.record.sealed_digest = Some(sealed);
        // Windows directory handles deny renaming their open directory. Keep
        // the verified identities and release these handles before the rename.
        drop(stage);
        drop(current);
        self.record.phase = Phase::Promoting;
        self.save()?;
        if self.record.previous_identity.is_some() {
            io(self
                .root
                .rename(&self.record.version, &self.root, &self.record.quarantine))?;
            self.checked(
                &self.record.quarantine,
                self.record.previous_identity.ok_or_else(refuse)?,
            )?;
            sync(&self.root)?;
        }
        io(self
            .root
            .rename(&self.record.stage, &self.root, &self.record.version))?;
        let promoted = self.checked(&self.record.version, self.stage_identity()?)?;
        let digest = crate::software_prefix::digest_directory(&promoted)?;
        if self.record.sealed_digest.as_deref() != Some(digest.as_str()) {
            return Err(refuse());
        }
        drop(promoted);
        sync(&self.root)?;
        self.record.phase = Phase::Promoted;
        self.save()
    }

    /// Returns true only for the exact promoted stage, which must be exposed
    /// before completion. An incomplete promotion is rolled back by identity.
    pub(super) fn recover(&self) -> Result<bool> {
        self.check_root()?;
        let stage = present(&self.root, &self.record.stage)?;
        let version = present(&self.root, &self.record.version)?;
        let quarantine = present(&self.root, &self.record.quarantine)?;
        let identify = |dir: &Option<Dir>| dir.as_ref().map(Identity::of).transpose();
        if self.record.phase == Phase::Preparing {
            // Only an absent or empty plain stage can precede its identity.
            // A nonempty or substituted shape remains untouched for inspection.
            self.record
                .launch
                .revalidate(&self.path, &self.record.command, &self.record.member)?;
            if quarantine.is_some()
                || identify(&version)? != self.record.previous_identity
                || version
                    .as_ref()
                    .map(software_prefix::digest_directory)
                    .transpose()?
                    != self.record.previous_digest
            {
                return Err(refuse());
            }
            if let Some(stage) = stage {
                if io(stage.entries())?.next().is_some() {
                    return Err(refuse());
                }
                // This operation can remove only an empty directory; a later
                // added entry causes removal to refuse without deleting it.
                io(stage.remove_open_dir())?;
                sync(&self.root)?;
            }
            io(self.root.remove_file(&self.journal))?;
            sync(&self.root)?;
            return Ok(false);
        }
        if identify(&stage)?.is_some_and(|id| Some(id) != self.record.stage_identity)
            || identify(&quarantine)?.is_some_and(|id| Some(id) != self.record.previous_identity)
        {
            return Err(refuse());
        }
        let current = identify(&version)?;
        if stage.is_none()
            && current == self.record.stage_identity
            && self.record.phase != Phase::Extracting
        {
            let digest =
                crate::software_prefix::digest_directory(version.as_ref().ok_or_else(refuse)?)?;
            if self.record.sealed_digest.as_deref() != Some(digest.as_str()) {
                return Err(refuse());
            }
            self.check_predecessor()?;
            return Ok(true);
        }
        if self.record.phase == Phase::Promoted || stage.is_none() {
            return Err(refuse());
        }
        if let Some(directory) = &quarantine {
            if Some(crate::software_prefix::digest_directory(directory)?)
                != self.record.previous_digest
            {
                return Err(refuse());
            }
            if current.is_some() || self.record.phase != Phase::Promoting {
                return Err(refuse());
            }
            drop(quarantine);
            io(self
                .root
                .rename(&self.record.quarantine, &self.root, &self.record.version))?;
            self.checked(
                &self.record.version,
                self.record.previous_identity.ok_or_else(refuse)?,
            )?;
            sync(&self.root)?;
        } else if current != self.record.previous_identity {
            return Err(refuse());
        }
        io(stage.ok_or_else(refuse)?.remove_open_dir_all())?;
        io(self.root.remove_file(&self.journal))?;
        sync(&self.root)?;
        Ok(false)
    }

    fn check_predecessor(&self) -> Result<Option<Dir>> {
        let remaining = present(&self.root, &self.record.quarantine)?;
        if let Some(directory) = &remaining {
            if Some(Identity::of(directory)?) != self.record.previous_identity {
                return Err(refuse());
            }
            cleanup::inspect_remaining(
                directory,
                self.record.previous_entries.as_deref().ok_or_else(refuse)?,
            )?;
        }
        Ok(remaining)
    }

    pub(super) fn complete(self) -> Result<()> {
        self.check_root()?;
        self.checked(&self.record.version, self.stage_identity()?)?;
        let predecessor = self.check_predecessor()?;
        ownership::record_installation(
            &self.root,
            &self.record.command,
            &self.record.version,
            &self.record.member,
            &self.record.artifact_sha256,
            self.record.sealed_digest.as_deref().ok_or_else(refuse)?,
            self.stage_identity()?,
        )?;
        if let Some(quarantine) = predecessor {
            cleanup::remaining(
                &quarantine,
                self.record.previous_entries.as_deref().ok_or_else(refuse)?,
            )?;
            self.check_root()?;
            io(quarantine.remove_open_dir())?;
            sync(&self.root)?;
        }
        io(self.root.remove_file(&self.journal))?;
        sync(&self.root)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use crate::software::{self, Delivery, Software};
    use std::fs;

    const ARTIFACT: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "One persisted lifecycle drives the interruption and foreign-entry controls."
    )]
    fn recovery_changes_only_recorded_directories_across_interruption_windows() {
        let root = std::env::temp_dir().join(format!("software-staging-{}", unique().unwrap()));
        fs::create_dir(&root).unwrap();
        let declared = Software {
            version: "1.2.3",
            command: "codex",
            delivery: Delivery::Artifacts(&[]),
            unsupported: &[],
            previous: None,
        };
        let unrelated = root.join(".incoming-unrelated/owner.txt");
        fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
        fs::write(&unrelated, b"unrelated").unwrap();
        fs::create_dir(root.join(".replaced-unrelated")).unwrap();
        fs::write(root.join("unrelated.incoming"), b"unrelated").unwrap();
        assert!(software::recover(&declared, &root).unwrap().is_empty());
        software::require_idle(&declared, &root).unwrap();

        for created in [false, true] {
            let transaction = Staging::prepare(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
            let stage = transaction.stage_path();
            assert!(!stage.exists());
            assert!(root.join(&transaction.journal).is_file());
            if created {
                fs::create_dir(&stage).unwrap();
                fs::write(stage.join("foreign"), b"preserve").unwrap();
                assert!(software::recover(&declared, &root).is_err());
                assert_eq!(fs::read(stage.join("foreign")).unwrap(), b"preserve");
                fs::remove_file(stage.join("foreign")).unwrap();
            }
            drop(transaction);
            assert_eq!(software::recover(&declared, &root).unwrap().len(), 1);
            assert!(!stage.exists());
            software::require_idle(&declared, &root).unwrap();
        }

        let transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        let partial = transaction.stage_path();
        fs::write(partial.join("codex"), b"partial").unwrap();
        drop(transaction);
        assert_eq!(
            software::require_idle(&declared, &root)
                .unwrap_err()
                .reason(),
            crate::ReasonCode::RecoveryRequired
        );
        assert!(software::remove(&declared, &root).is_err());
        assert_eq!(fs::read(partial.join("codex")).unwrap(), b"partial");
        assert_eq!(software::recover(&declared, &root).unwrap().len(), 1);
        software::require_idle(&declared, &root).unwrap();
        assert!(!partial.exists());
        assert!(!root.join("1.2.3").exists());

        // Interruption after moving the previous tree aside, before promotion.
        fs::create_dir(root.join("1.2.3")).unwrap();
        fs::write(root.join("1.2.3/codex"), b"previous").unwrap();
        let mut transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        fs::write(transaction.stage_path().join("codex"), b"new").unwrap();
        transaction.record.phase = Phase::Promoting;
        transaction.save().unwrap();
        transaction
            .root
            .rename("1.2.3", &transaction.root, &transaction.record.quarantine)
            .unwrap();
        drop(transaction);
        software::recover(&declared, &root).unwrap();
        assert_eq!(fs::read(root.join("1.2.3/codex")).unwrap(), b"previous");

        // The retained previous tree came from a completed installation.
        let directory = open_root(&root).unwrap();
        let previous = present(&directory, "1.2.3").unwrap().unwrap();
        let sealed = crate::software_prefix::digest_directory(&previous).unwrap();
        let previous_identity = Identity::of(&previous).unwrap();
        drop(previous);
        ownership::record_installation(
            &directory,
            "codex",
            "1.2.3",
            "codex",
            ARTIFACT,
            &sealed,
            previous_identity,
        )
        .unwrap();
        drop(directory);

        // Promotion landed, but its phase update and exposure did not.
        let mut transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        fs::write(transaction.stage_path().join("codex"), b"new").unwrap();
        let entry = root.join("bin/codex");
        fs::write(&entry, b"foreign during extraction").unwrap();
        assert!(transaction.promote().is_err());
        assert_eq!(fs::read(root.join("1.2.3/codex")).unwrap(), b"previous");
        assert_eq!(fs::read(&entry).unwrap(), b"foreign during extraction");
        fs::remove_file(&entry).unwrap();
        transaction.promote().unwrap();
        transaction.record.phase = Phase::Promoting;
        transaction.save().unwrap();
        drop(transaction);
        fs::write(root.join("1.2.3/codex"), b"changed after promotion").unwrap();
        assert!(software::recover(&declared, &root).is_err());
        assert!(!root.join("bin/codex").exists());
        fs::write(root.join("1.2.3/codex"), b"new").unwrap();
        fs::write(&entry, b"foreign after promotion").unwrap();
        assert!(software::recover(&declared, &root).is_err());
        assert_eq!(fs::read(&entry).unwrap(), b"foreign after promotion");
        assert!(!root.join("bin/.codex.version").exists());
        fs::remove_file(&entry).unwrap();
        // A matching entry without a sealed preparation is not a recorded rename.
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("1.2.3/codex"), &entry).unwrap();
        #[cfg(not(unix))]
        fs::copy(root.join("1.2.3/codex"), &entry).unwrap();
        assert!(software::recover(&declared, &root).is_err());
        fs::remove_file(&entry).unwrap();
        software::recover(&declared, &root).unwrap();
        assert_eq!(fs::read(root.join("bin/codex")).unwrap(), b"new");
        assert!(software::recover(&declared, &root).unwrap().is_empty());

        // Predecessor cleanup was interrupted after deleting a recorded member.
        let mut transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        fs::write(transaction.stage_path().join("codex"), b"newer").unwrap();
        transaction.promote().unwrap();
        transaction.expose().unwrap();
        let quarantine = root.join(&transaction.record.quarantine);
        fs::remove_file(quarantine.join("codex")).unwrap();
        let foreign = quarantine.join("foreign");
        fs::write(&foreign, b"preserve").unwrap();
        drop(transaction);
        let before = crate::software_prefix::observe(&root, ".unused")
            .unwrap()
            .digest;
        let launch_before =
            serde_json::to_value(launch::Snapshot::prepare(&root, "codex", "codex").unwrap())
                .unwrap();
        assert!(software::recover(&declared, &root).is_err());
        assert_eq!(
            before,
            crate::software_prefix::observe(&root, ".unused")
                .unwrap()
                .digest
        );
        assert_eq!(
            launch_before,
            serde_json::to_value(launch::Snapshot::prepare(&root, "codex", "codex").unwrap(),)
                .unwrap()
        );
        assert_eq!(fs::read(&foreign).unwrap(), b"preserve");
        fs::remove_file(foreign).unwrap();
        assert_eq!(software::recover(&declared, &root).unwrap().len(), 1);
        assert!(!quarantine.exists());
        assert_eq!(fs::read(root.join("bin/codex")).unwrap(), b"newer");
        assert!(software::recover(&declared, &root).unwrap().is_empty());

        // Matching names are insufficient when the actual directory moved.
        let mut transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        let original = transaction.stage_path();
        let moved = root.join("retained-stage");
        fs::write(original.join("codex"), b"recorded").unwrap();
        fs::rename(&original, &moved).unwrap();
        fs::create_dir(&original).unwrap();
        fs::write(original.join("foreign"), b"foreign").unwrap();
        assert!(transaction.promote().is_err());
        drop(transaction);
        assert!(software::recover(&declared, &root).is_err());
        assert_eq!(fs::read(original.join("foreign")).unwrap(), b"foreign");
        assert_eq!(fs::read(moved.join("codex")).unwrap(), b"recorded");
        assert_eq!(fs::read(root.join("bin/codex")).unwrap(), b"newer");
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated");
        assert!(root.join(".replaced-unrelated").is_dir());
        assert_eq!(
            fs::read(root.join("unrelated.incoming")).unwrap(),
            b"unrelated"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
