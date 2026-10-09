//! A recorded staging transaction; directory names never establish ownership.

use std::path::{Path, PathBuf};

use cap_fs_ext::DirExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::ownership;
use super::records::{
    self, Identity, io, leaf, member_valid, open_root, present, refuse, sync, unique,
};
use crate::{Result, archive::Destination};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
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
    stage_identity: Identity,
    previous_identity: Option<Identity>,
    previous_digest: Option<String>,
    legacy: bool,
    artifact_sha256: String,
    sealed_digest: Option<String>,
    phase: Phase,
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
        let previous_digest = if let Some(receipt) = &receipt {
            Some(
                receipt
                    .verify(previous.as_ref().ok_or_else(refuse)?)?
                    .to_owned(),
            )
        } else {
            previous
                .as_ref()
                .map(crate::software_prefix::digest_directory)
                .transpose()?
        };
        let legacy = previous.is_some() && receipt.is_none();
        let previous_identity = previous.as_ref().map(Identity::of).transpose()?;
        drop(previous);
        let nonce = unique()?;
        let stage = format!(".incoming-{command}-{nonce}");
        let quarantine = format!(".replaced-{command}-{nonce}");
        io(directory.create_dir(&stage))?;
        let stage_identity = Identity::of(&io(directory.open_dir_nofollow(&stage))?)?;
        let transaction = Self {
            record: Record {
                schema_version: 1,
                command: command.to_owned(),
                version: version.to_owned(),
                member: member.to_owned(),
                stage,
                quarantine,
                root_identity: Identity::of(&directory)?,
                stage_identity,
                previous_identity,
                previous_digest,
                legacy,
                artifact_sha256: artifact_sha256.to_owned(),
                sealed_digest: None,
                phase: Phase::Extracting,
            },
            root: directory,
            path: root.to_owned(),
            journal,
        };
        transaction.save()?;
        Ok(transaction)
    }

    pub(super) fn load(path: &Path, command: &str) -> Result<Option<Self>> {
        if !leaf(command) {
            return Err(refuse());
        }
        let root = match open_root(path) {
            Ok(root) => root,
            Err(error) => match std::fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                _ => return Err(error),
            },
        };
        let journal = format!(".nddev-software-{command}.transaction.json");
        let Some(record): Option<Record> = records::read(&root, &journal, 16 * 1024)? else {
            return Ok(None);
        };
        if record.schema_version != 1
            || record.command != command
            || !ownership::digest_valid(&record.artifact_sha256)
            || record.previous_identity.is_some() != record.previous_digest.is_some()
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
        Ok(Some(Self {
            root,
            path: path.to_owned(),
            journal,
            record,
        }))
    }

    fn save(&self) -> Result<()> {
        self.check_root()?;
        records::write(&self.root, &self.journal, &self.record, 16 * 1024)
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
            self.checked(&self.record.stage, self.record.stage_identity)?,
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

    pub(super) fn promote(&mut self) -> Result<()> {
        self.check_root()?;
        let stage = self.checked(&self.record.stage, self.record.stage_identity)?;
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
        let promoted = self.checked(&self.record.version, self.record.stage_identity)?;
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
        if identify(&stage)?.is_some_and(|id| id != self.record.stage_identity)
            || identify(&quarantine)?.is_some_and(|id| Some(id) != self.record.previous_identity)
        {
            return Err(refuse());
        }
        let current = identify(&version)?;
        if stage.is_none()
            && current == Some(self.record.stage_identity)
            && self.record.phase != Phase::Extracting
        {
            let digest =
                crate::software_prefix::digest_directory(version.as_ref().ok_or_else(refuse)?)?;
            if self.record.sealed_digest.as_deref() != Some(digest.as_str()) {
                return Err(refuse());
            }
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

    pub(super) fn complete(self) -> Result<()> {
        self.check_root()?;
        self.checked(&self.record.version, self.record.stage_identity)?;
        ownership::record_installation(
            &self.root,
            &self.record.command,
            &self.record.version,
            &self.record.member,
            &self.record.artifact_sha256,
            self.record.sealed_digest.as_deref().ok_or_else(refuse)?,
            self.record.stage_identity,
        )?;
        if let Some(quarantine) = present(&self.root, &self.record.quarantine)? {
            if Some(Identity::of(&quarantine)?) != self.record.previous_identity
                || Some(crate::software_prefix::digest_directory(&quarantine)?)
                    != self.record.previous_digest
            {
                return Err(refuse());
            }
            io(quarantine.remove_open_dir_all())?;
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

        let transaction = Staging::begin(&root, "codex", "1.2.3", "codex", ARTIFACT).unwrap();
        let partial = transaction.stage_path();
        fs::write(partial.join("codex"), b"partial").unwrap();
        drop(transaction);
        assert_eq!(software::recover(&declared, &root).unwrap().len(), 1);
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
        transaction.promote().unwrap();
        transaction.record.phase = Phase::Promoting;
        transaction.save().unwrap();
        drop(transaction);
        fs::write(root.join("1.2.3/codex"), b"changed after promotion").unwrap();
        assert!(software::recover(&declared, &root).is_err());
        assert!(!root.join("bin/codex").exists());
        fs::write(root.join("1.2.3/codex"), b"new").unwrap();
        software::recover(&declared, &root).unwrap();
        assert_eq!(fs::read(root.join("bin/codex")).unwrap(), b"new");
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
        assert_eq!(fs::read(root.join("bin/codex")).unwrap(), b"new");
        assert_eq!(fs::read(&unrelated).unwrap(), b"unrelated");
        assert!(root.join(".replaced-unrelated").is_dir());
        assert_eq!(
            fs::read(root.join("unrelated.incoming")).unwrap(),
            b"unrelated"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
