//! Recorded version removal with resumable, inventory-bound cleanup.

use std::path::{Path, PathBuf};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::{Software, cleanup, launch, ownership::Receipt, records};
use crate::{Result, software_prefix::INVENTORY_LIMIT};
use records::{Identity, io, leaf, open_root, present, refuse, sync, unique};

const RECORD_LIMIT: usize = INVENTORY_LIMIT + 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Quarantined,
    Cleaned,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    command: String,
    version: String,
    quarantine: String,
    root_identity: Identity,
    receipt: Receipt,
    exposure: Option<launch::Record>,
    phase: Phase,
}

pub(super) struct Removal {
    root: Dir,
    path: PathBuf,
    journal: super::store::Key,
    record: Record,
}

pub(super) fn journal(command: &str) -> super::store::Key {
    super::store::Key::Removal(command.to_owned())
}

impl Removal {
    pub(super) fn begin(path: &Path, software: &Software) -> Result<Option<Self>> {
        super::require_idle(software, path)?;
        let root = open_root(path)?;
        let Some(directory) = present(&root, software.version)? else {
            return Ok(None);
        };
        let receipt = Receipt::read(&root, software.command, software.version)?.ok_or_else(|| {
            crate::Error::new(crate::ReasonCode::RecoveryRequired,
                "software version has no installation receipt; reinstall its exact artifact to adopt unchanged legacy bytes")
        })?;
        receipt.verify(&directory)?;
        drop(directory);
        let exposure =
            launch::Retraction::prepare(path, software)?.map(launch::Retraction::into_record);
        let removal = Self {
            record: Record {
                schema_version: 1,
                command: software.command.to_owned(),
                version: software.version.to_owned(),
                quarantine: format!(".removing-{}-{}", software.command, unique()?),
                root_identity: Identity::of(&root)?,
                receipt,
                exposure,
                phase: Phase::Prepared,
            },
            root,
            path: path.to_owned(),
            journal: journal(software.command),
        };
        removal.save()?;
        Ok(Some(removal))
    }

    pub(super) fn load(path: &Path, command: &str) -> Result<Option<Self>> {
        if !leaf(command) {
            return Err(refuse());
        }
        let Some(root) = records::optional_root(path)? else {
            return Ok(None);
        };
        let Some(record): Option<Record> =
            super::store::read(&root, &journal(command), RECORD_LIMIT)?
        else {
            return Ok(None);
        };
        if record.schema_version != 1
            || record.command != command
            || !leaf(&record.version)
            || !leaf(&record.quarantine)
            || !record
                .quarantine
                .starts_with(&format!(".removing-{command}-"))
            || record.root_identity != Identity::of(&root)?
        {
            return Err(refuse());
        }
        record.receipt.validate(&root, command, &record.version)?;
        Ok(Some(Self {
            root,
            path: path.to_owned(),
            journal: journal(command),
            record,
        }))
    }

    pub(super) fn version(&self) -> &str {
        &self.record.version
    }

    fn check_root(&self) -> Result<()> {
        if Identity::of(&open_root(&self.path)?)? != self.record.root_identity {
            return Err(refuse());
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        self.check_root()?;
        super::store::write(&self.root, &self.journal, &self.record, RECORD_LIMIT)
    }

    fn quarantine(&mut self) -> Result<()> {
        let current = present(&self.root, &self.record.version)?;
        let quarantined = present(&self.root, &self.record.quarantine)?;
        match (current, quarantined) {
            (Some(directory), None) => {
                self.record.receipt.verify(&directory)?;
                drop(directory);
                io(self
                    .root
                    .rename(&self.record.version, &self.root, &self.record.quarantine))?;
                let directory = present(&self.root, &self.record.quarantine)?.ok_or_else(refuse)?;
                self.record.receipt.verify(&directory)?;
                sync(&self.root)?;
            }
            (None, Some(directory)) => {
                self.record.receipt.verify(&directory)?;
            }
            _ => return Err(refuse()),
        }
        self.record.phase = Phase::Quarantined;
        self.save()
    }

    pub(super) fn complete(mut self) -> Result<()> {
        self.check_root()?;
        let receipt = Receipt::read(&self.root, &self.record.command, &self.record.version)?;
        if receipt.as_ref() != Some(&self.record.receipt)
            && !(receipt.is_none() && self.record.phase == Phase::Cleaned)
        {
            return Err(refuse());
        }
        if self.record.phase == Phase::Prepared {
            self.quarantine()?;
        }
        if present(&self.root, &self.record.version)?.is_some() {
            return Err(refuse());
        }
        let remaining = present(&self.root, &self.record.quarantine)?;
        if remaining
            .as_ref()
            .map(Identity::of)
            .transpose()?
            .is_some_and(|identity| identity != self.record.receipt.tree_identity())
        {
            return Err(refuse());
        }
        if self.record.phase == Phase::Cleaned {
            if remaining.is_some() {
                return Err(refuse());
            }
        } else {
            let exposure = self
                .record
                .exposure
                .as_ref()
                .map(|exposure| {
                    launch::Retraction::resume(
                        &self.path,
                        &self.record.command,
                        &self.record.version,
                        self.record.receipt.member(),
                        exposure.clone(),
                    )
                })
                .transpose()?;
            if let Some(directory) = remaining {
                cleanup::remaining(&directory, self.record.receipt.entries())?;
                self.check_root()?;
                io(directory.remove_open_dir())?;
                sync(&self.root)?;
            }
            if let Some(exposure) = exposure {
                exposure.complete()?;
            }
            self.record.phase = Phase::Cleaned;
            self.save()?;
        }
        self.record.receipt.remove_record(&self.root)?;
        self.check_root()?;
        super::store::remove(&self.root, &self.journal)?;
        sync(&self.root)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::{
        archive::build::{Dialect, Item, gzip_tar},
        digest,
        software::{self, Artifact, Delivery, Shape},
    };
    use std::fs;

    #[test]
    fn interrupted_removal_preserves_foreign_entries_and_resumes_owned_subsets() {
        let at = std::env::temp_dir().join(format!("software-removal-{}", unique().unwrap()));
        fs::create_dir(&at).unwrap();
        let root = at.join("prefix");
        let bytes = gzip_tar(
            &[
                Item::file("codex", b"payload", 0o755),
                Item::file("docs/a", b"first", 0o644),
                Item::file("docs/b", b"second", 0o644),
            ],
            Dialect::Gnu,
        );
        let archive = at.join("artifact.tgz");
        fs::write(&archive, &bytes).unwrap();
        let artifact = Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/tool.tgz",
            bytes: bytes.len() as u64,
            sha256: Box::leak(digest::of_bytes(&bytes).into_boxed_str()),
            shape: Shape::GzipTar,
            member: "codex",
        };
        let declared = Software {
            version: "1.2.3",
            command: "codex",
            delivery: Delivery::Artifacts(&[]),
            unsupported: &[],
            previous: None,
        };
        for interruption in 0..4 {
            software::install(&declared, &artifact, &archive, &root).unwrap();
            let mut removal = Removal::begin(&root, &declared).unwrap().unwrap();
            let quarantine = root.join(&removal.record.quarantine);
            assert!(software::remove(&declared, &root).is_err());
            assert!(software::install(&declared, &artifact, &archive, &root).is_err());
            if interruption > 0 {
                removal.quarantine().unwrap();
            }
            if interruption == 1 {
                // Interruption halfway through version and launch-entry cleanup.
                fs::remove_file(quarantine.join("docs/a")).unwrap();
                fs::remove_file(root.join("bin/.codex.version")).unwrap();
                fs::write(quarantine.join("personal"), b"preserve").unwrap();
                assert!(software::recover(&declared, &root).is_err());
                assert_eq!(fs::read(quarantine.join("docs/b")).unwrap(), b"second");
                assert_eq!(fs::read(quarantine.join("personal")).unwrap(), b"preserve");
                fs::remove_file(quarantine.join("personal")).unwrap();
                fs::write(quarantine.join("docs/b"), b"changed").unwrap();
                assert!(software::recover(&declared, &root).is_err());
                assert_eq!(fs::read(quarantine.join("docs/b")).unwrap(), b"changed");
                fs::write(quarantine.join("docs/b"), b"second").unwrap();
            } else if interruption == 2 {
                let moved = root.join("retained-tree");
                fs::rename(&quarantine, &moved).unwrap();
                fs::create_dir(&quarantine).unwrap();
                fs::write(quarantine.join("personal"), b"foreign directory").unwrap();
                assert!(software::recover(&declared, &root).is_err());
                assert_eq!(
                    fs::read(quarantine.join("personal")).unwrap(),
                    b"foreign directory"
                );
                fs::remove_file(quarantine.join("personal")).unwrap();
                fs::remove_dir(&quarantine).unwrap();
                fs::rename(moved, &quarantine).unwrap();
            } else if interruption == 3 {
                let directory = present(&removal.root, &removal.record.quarantine)
                    .unwrap()
                    .unwrap();
                cleanup::remaining(&directory, removal.record.receipt.entries()).unwrap();
                directory.remove_open_dir().unwrap();
                launch::Retraction::resume(
                    &root,
                    "codex",
                    "1.2.3",
                    "codex",
                    removal.record.exposure.clone().unwrap(),
                )
                .unwrap()
                .complete()
                .unwrap();
                removal.record.phase = Phase::Cleaned;
                removal.save().unwrap();
                removal.record.receipt.remove_record(&removal.root).unwrap();
            }
            drop(removal);
            assert_eq!(software::recover(&declared, &root).unwrap().len(), 1);
            assert!(software::recover(&declared, &root).unwrap().is_empty());
            software::require_idle(&declared, &root).unwrap();
            assert!(!root.join("1.2.3").exists() && !quarantine.exists());
            assert!(
                !super::super::store::exists(
                    &open_root(&root).unwrap(),
                    &super::super::store::Key::Receipt {
                        command: "codex".into(),
                        version: "1.2.3".into()
                    }
                )
                .unwrap()
            );
            assert_eq!(fs::read_dir(root.join("bin")).unwrap().count(), 0);
        }
        fs::remove_dir_all(at).unwrap();
    }
}
