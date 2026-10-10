//! Installation receipts bind a version tree, never just its directory name.

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::records::{Identity, leaf, member_valid, present, refuse, sync};
use crate::{
    Result,
    software_prefix::{self, Entry, INVENTORY_LIMIT},
};

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    schema_version: u32,
    command: String,
    version: String,
    member: String,
    artifact_sha256: String,
    tree_digest: String,
    entries: Vec<Entry>,
    root_identity: Identity,
    tree_identity: Identity,
}

pub(super) fn digest_valid(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn name(command: &str, version: &str) -> Result<super::store::Key> {
    if !leaf(command) || !leaf(version) {
        return Err(refuse());
    }
    Ok(super::store::Key::Receipt {
        command: command.to_owned(),
        version: version.to_owned(),
    })
}

impl Receipt {
    pub(super) fn digest(&self) -> Result<String> {
        crate::digest::of_domain_canonical_json(
            "nddev-software-receipt/1",
            &serde_json::to_value(self).map_err(|_| refuse())?,
        )
    }

    pub(super) fn member(&self) -> &str {
        &self.member
    }

    pub(super) fn tree_identity(&self) -> Identity {
        self.tree_identity
    }

    pub(super) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub(super) fn remove_record(&self, root: &Dir) -> Result<()> {
        match Self::read(root, &self.command, &self.version)? {
            Some(ref current) if current == self => {
                super::store::remove(root, &name(&self.command, &self.version)?)?;
                sync(root)
            }
            None => Ok(()),
            _ => Err(refuse()),
        }
    }

    pub(super) fn read(root: &Dir, command: &str, version: &str) -> Result<Option<Self>> {
        let Some(record): Option<Self> =
            super::store::read(root, &name(command, version)?, INVENTORY_LIMIT)?
        else {
            return Ok(None);
        };
        record.validate(root, command, version)?;
        Ok(Some(record))
    }

    pub(super) fn validate(&self, root: &Dir, command: &str, version: &str) -> Result<()> {
        if self.schema_version != 1
            || self.command != command
            || self.version != version
            || self.root_identity != Identity::of(root)?
            || !digest_valid(&self.artifact_sha256)
            || !digest_valid(&self.tree_digest)
            || !member_valid(&self.member)
            || software_prefix::inventory_digest(&self.entries).map_err(|_| refuse())?
                != self.tree_digest
        {
            return Err(refuse());
        }
        Ok(())
    }

    pub(super) fn verify(&self, directory: &Dir) -> Result<&str> {
        if Identity::of(directory)? != self.tree_identity
            || crate::software_prefix::digest_directory(directory)? != self.tree_digest
        {
            return Err(refuse());
        }
        Ok(&self.tree_digest)
    }
}

pub(super) fn record_installation(
    root: &Dir,
    command: &str,
    version: &str,
    member: &str,
    artifact_sha256: &str,
    tree_digest: &str,
    tree_identity: Identity,
) -> Result<()> {
    let directory = present(root, version)?.ok_or_else(refuse)?;
    let (observed_digest, entries) = software_prefix::inventory_directory(&directory)?;
    if Identity::of(&directory)? != tree_identity
        || !digest_valid(artifact_sha256)
        || observed_digest != tree_digest
    {
        return Err(refuse());
    }
    let record = Receipt {
        schema_version: 1,
        command: command.to_owned(),
        version: version.to_owned(),
        member: member.to_owned(),
        artifact_sha256: artifact_sha256.to_owned(),
        tree_digest: tree_digest.to_owned(),
        entries,
        root_identity: Identity::of(root)?,
        tree_identity,
    };
    super::store::write(root, &name(command, version)?, &record, INVENTORY_LIMIT)
}
