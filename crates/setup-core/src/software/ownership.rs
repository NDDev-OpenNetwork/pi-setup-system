//! Installation receipts bind a version tree, never just its directory name.

use std::path::Path;

use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::records::{self, Identity, io, leaf, member_valid, open_root, present, refuse, sync};
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

fn name(command: &str, version: &str) -> Result<String> {
    if !leaf(command) || !leaf(version) {
        return Err(refuse());
    }
    Ok(format!(
        ".nddev-software-{command}-{version}.installed.json"
    ))
}

impl Receipt {
    pub(super) fn read(root: &Dir, command: &str, version: &str) -> Result<Option<Self>> {
        let Some(record): Option<Self> =
            records::read(root, &name(command, version)?, INVENTORY_LIMIT)?
        else {
            return Ok(None);
        };
        if record.schema_version != 1
            || record.command != command
            || record.version != version
            || record.root_identity != Identity::of(root)?
            || !digest_valid(&record.artifact_sha256)
            || !digest_valid(&record.tree_digest)
            || !member_valid(&record.member)
            || software_prefix::inventory_digest(&record.entries).map_err(|_| refuse())?
                != record.tree_digest
        {
            return Err(refuse());
        }
        Ok(Some(record))
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
    records::write(root, &name(command, version)?, &record, INVENTORY_LIMIT)
}

/// Refuse unknown or modified trees before deleting any version content.
pub(super) fn remove(root_path: &Path, command: &str, version: &str) -> Result<bool> {
    let root = open_root(root_path)?;
    let Some(directory) = present(&root, version)? else {
        return Ok(false);
    };
    let record = Receipt::read(&root, command, version)?.ok_or_else(|| {
        crate::Error::new(
            crate::ReasonCode::RecoveryRequired,
            "software version has no installation receipt; reinstall its exact artifact to adopt unchanged legacy bytes",
        )
    })?;
    record.verify(&directory)?;
    if Identity::of(&open_root(root_path)?)? != record.root_identity {
        return Err(refuse());
    }
    // Consume the held directory, so a changed name cannot select another tree.
    io(directory.remove_open_dir_all())?;
    if Receipt::read(&root, command, version)?.as_ref() != Some(&record) {
        return Err(refuse());
    }
    io(root.remove_file(name(command, version)?))?;
    sync(&root)?;
    Ok(true)
}
