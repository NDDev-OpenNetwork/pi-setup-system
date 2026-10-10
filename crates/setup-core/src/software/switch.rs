//! A durable launch switch to an unchanged, already installed version.

use std::path::{Path, PathBuf};

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::{exposure, launch, ownership::Receipt, records};
use crate::Result;
use records::{Identity, io, leaf, open_root, present, refuse, sync};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    command: String,
    version: String,
    member: String,
    receipt_digest: String,
    predecessor: launch::Snapshot,
}

pub(super) struct Switch {
    root: Dir,
    path: PathBuf,
    record: Record,
}

pub(super) fn journal(command: &str) -> String {
    format!(".nddev-software-{command}.switch.json")
}

impl Switch {
    pub(super) fn begin(path: &Path, command: &str, version: &str) -> Result<Self> {
        if !leaf(command) || !leaf(version) {
            return Err(refuse());
        }
        let root = open_root(path)?;
        match root.symlink_metadata(journal(command)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(refuse()),
        }
        let receipt = Receipt::read(&root, command, version)?.ok_or_else(refuse)?;
        let predecessor = launch::Snapshot::prepare(path, command, receipt.member())?;
        let switch = Self {
            record: Record {
                schema_version: 1,
                command: command.to_owned(),
                version: version.to_owned(),
                member: receipt.member().to_owned(),
                receipt_digest: receipt.digest()?,
                predecessor,
            },
            root,
            path: path.to_owned(),
        };
        switch.verify()?;
        records::write(&switch.root, &journal(command), &switch.record, 16 * 1024)?;
        Ok(switch)
    }

    pub(super) fn load(path: &Path, command: &str) -> Result<Option<Self>> {
        if !leaf(command) {
            return Err(refuse());
        }
        let Some(root) = records::optional_root(path)? else {
            return Ok(None);
        };
        let Some(record): Option<Record> = records::read(&root, &journal(command), 16 * 1024)?
        else {
            return Ok(None);
        };
        if record.schema_version != 1
            || record.command != command
            || !leaf(&record.version)
            || !records::member_valid(&record.member)
            || !super::ownership::digest_valid(&record.receipt_digest)
        {
            return Err(refuse());
        }
        let switch = Self {
            root,
            path: path.to_owned(),
            record,
        };
        switch
            .record
            .predecessor
            .validate(path, command, &switch.record.member)?;
        Ok(Some(switch))
    }

    pub(super) fn version(&self) -> &str {
        &self.record.version
    }

    pub(super) fn executable(&self) -> PathBuf {
        self.path.join("bin").join(super::exposed_name(
            &self.record.command,
            &self.record.member,
        ))
    }

    fn verify(&self) -> Result<()> {
        if Identity::of(&self.root)? != Identity::of(&open_root(&self.path)?)? {
            return Err(refuse());
        }
        self.record
            .predecessor
            .validate(&self.path, &self.record.command, &self.record.member)?;
        let receipt = Receipt::read(&self.root, &self.record.command, &self.record.version)?
            .ok_or_else(refuse)?;
        if receipt.digest()? != self.record.receipt_digest || receipt.member() != self.record.member
        {
            return Err(refuse());
        }
        receipt.verify(&present(&self.root, &self.record.version)?.ok_or_else(refuse)?)?;
        Ok(())
    }

    pub(super) fn complete(self) -> Result<()> {
        self.verify()?;
        exposure::expose(
            &self
                .path
                .join(&self.record.version)
                .join(&self.record.member),
            &self.executable(),
            &self.record.version,
            &self.record.command,
            &self.record.predecessor,
        )?;
        self.verify()?;
        io(self.root.remove_file(journal(&self.record.command)))?;
        sync(&self.root)
    }
}
