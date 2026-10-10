//! Observe coherent launch ownership and retain exact entry preconditions.

use std::path::{Path, PathBuf};

use cap_fs_ext::MetadataExt;
use cap_std::fs::Dir;
use serde::{Deserialize, Serialize};

use super::{Manifest, Software, exposure, records};
use crate::Result;
use records::{Identity, io, open_root, present, refuse, sync};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Stamp {
    device: u64,
    inode: u64,
    length: u64,
    mode: u32,
    link: bool,
    content: String,
}

pub(super) fn stamp(bin: &Dir, name: &str, limit: u64) -> Result<Option<Stamp>> {
    let before = match bin.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(refuse().with_source(error)),
    };
    if before.len() > limit {
        return Err(refuse());
    }
    let link = before.is_symlink();
    let content = if link {
        io(bin.read_link_contents(name))?
            .to_str()
            .ok_or_else(refuse)?
            .to_owned()
    } else {
        io(exposure::payload(bin, Path::new(name)))?.2
    };
    let after = io(bin.symlink_metadata(name))?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || before.permissions() != after.permissions()
        || link != after.is_symlink()
    {
        return Err(refuse());
    }
    #[cfg(unix)]
    let mode = {
        use cap_std::fs::PermissionsExt;
        before.permissions().mode() & 0o7777
    };
    #[cfg(not(unix))]
    let mode = u32::from(before.permissions().readonly());
    Ok(Some(Stamp {
        device: before.dev(),
        inode: before.ino(),
        length: before.len(),
        mode,
        link,
        content,
    }))
}

fn names(command: &str, member: &str) -> [String; 3] {
    [
        super::exposed_name(command, member),
        format!(".{command}.version"),
        format!(".{command}.manifest.json"),
    ]
}

struct Observed {
    entries: Vec<(String, Stamp)>,
    manifest: Manifest,
}

fn inspect(
    path: &Path,
    root: &Dir,
    bin: &Dir,
    command: &str,
    member: &str,
) -> Result<Option<Observed>> {
    let names = names(command, member);
    let observed = names
        .iter()
        .zip([8 * 1024 * 1024 * 1024, 1024, 16 * 1024])
        .map(|(name, limit)| stamp(bin, name, limit))
        .collect::<Result<Vec<_>>>()?;
    if observed.iter().all(Option::is_none) {
        return Ok(None);
    }
    let entries = names
        .into_iter()
        .zip(observed)
        .map(|(name, held)| Ok((name, held.ok_or_else(refuse)?)))
        .collect::<Result<Vec<_>>>()?;
    let marker = records::read_bytes(bin, &entries[1].0, 1024)?.ok_or_else(refuse)?;
    let manifest: Manifest = records::read(bin, &entries[2].0, 16 * 1024)?.ok_or_else(refuse)?;
    if manifest.schema_version != 1
        || !records::leaf(&manifest.version)
        || marker != manifest.version.as_bytes()
        || !super::ownership::digest_valid(&manifest.executable_sha256)
        || !records::member_valid(&manifest.executable)
        || !manifest
            .executable
            .starts_with(&format!("{}/", manifest.version))
    {
        return Err(refuse());
    }
    let executable = Path::new(&manifest.executable);
    let (_, _, digest) = io(exposure::payload(root, executable))?;
    if digest != manifest.executable_sha256 {
        return Err(refuse());
    }
    let command_entry = &entries[0].1;
    #[cfg(unix)]
    {
        let valid = if command_entry.link {
            Path::new(&command_entry.content) == path.join(executable)
        } else {
            command_entry.content == digest
        };
        if !valid {
            return Err(refuse());
        }
    }
    #[cfg(not(unix))]
    {
        let expected = io(exposure::wrapper(&path.join(executable)))?;
        let valid = if let Some(bytes) = expected {
            records::read_bytes(bin, &entries[0].0, 16 * 1024)? == Some(bytes)
        } else {
            !command_entry.link && command_entry.content == digest
        };
        if !valid {
            return Err(refuse());
        }
    }
    Ok(Some(Observed { entries, manifest }))
}

/// A captured predecessor, retained before any version-tree promotion.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    root_identity: Identity,
    bin_identity: Identity,
    entries: Vec<(String, Option<Stamp>)>,
}

impl Snapshot {
    pub(super) fn prepare(path: &Path, command: &str, member: &str) -> Result<Self> {
        if !records::leaf(command) || !records::member_valid(member) {
            return Err(refuse());
        }
        let root = open_root(path)?;
        let bin = if let Some(bin) = present(&root, "bin")? {
            bin
        } else {
            io(root.create_dir("bin"))?;
            sync(&root)?;
            present(&root, "bin")?.ok_or_else(refuse)?
        };
        let entries = match inspect(path, &root, &bin, command, member)? {
            Some(Observed { entries, .. }) => entries
                .into_iter()
                .map(|(name, stamp)| (name, Some(stamp)))
                .collect(),
            None => names(command, member)
                .into_iter()
                .map(|name| (name, None))
                .collect(),
        };
        let snapshot = Self {
            root_identity: Identity::of(&root)?,
            bin_identity: Identity::of(&bin)?,
            entries,
        };
        snapshot.revalidate(path, command, member)?;
        Ok(snapshot)
    }

    pub(super) fn validate(&self, path: &Path, command: &str, member: &str) -> Result<()> {
        let root = open_root(path)?;
        let bin = present(&root, "bin")?.ok_or_else(refuse)?;
        if Identity::of(&root)? != self.root_identity
            || Identity::of(&bin)? != self.bin_identity
            || self.entries.len() != 3
        {
            return Err(refuse());
        }
        let populated = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.is_some())
            .count();
        if populated != 0 && populated != 3 {
            return Err(refuse());
        }
        for (index, ((name, original), expected)) in
            self.entries.iter().zip(names(command, member)).enumerate()
        {
            let limit = [8 * 1024 * 1024 * 1024, 1024, 16 * 1024][index];
            if *name != expected
                || original.as_ref().is_some_and(|entry| {
                    entry.mode > 0o7777
                        || entry.length > limit
                        || (entry.link && index != 0)
                        || (!entry.link && !super::ownership::digest_valid(&entry.content))
                })
            {
                return Err(refuse());
            }
        }
        Ok(())
    }

    pub(super) fn revalidate(&self, path: &Path, command: &str, member: &str) -> Result<()> {
        self.validate(path, command, member)?;
        let bin = present(&open_root(path)?, "bin")?.ok_or_else(refuse)?;
        for ((name, original), limit) in
            self.entries
                .iter()
                .zip([8 * 1024 * 1024 * 1024, 1024, 16 * 1024])
        {
            if stamp(&bin, name, limit)? != *original {
                return Err(refuse());
            }
        }
        Ok(())
    }

    pub(super) fn check_replacement(
        &self,
        bin: &Dir,
        index: usize,
        replacement: &Stamp,
    ) -> Result<()> {
        let (name, original) = self.entries.get(index).ok_or_else(refuse)?;
        let limit = [8 * 1024 * 1024 * 1024, 1024, 16 * 1024]
            .get(index)
            .copied()
            .ok_or_else(refuse)?;
        let current = stamp(bin, name, limit)?;
        if current == *original {
            return Ok(());
        }
        // A recorded promotion may have completed any prefix of the three
        // renames. Accept only the exact intended payload on a later attempt.
        if current.as_ref().is_some_and(|found| {
            found.length == replacement.length
                && found.mode == replacement.mode
                && found.link == replacement.link
                && found.content == replacement.content
        }) {
            return Ok(());
        }
        Err(refuse())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    root_identity: Identity,
    bin_identity: Identity,
    entries: Vec<(String, Stamp)>,
}

pub(super) struct Retraction {
    path: PathBuf,
    bin: Dir,
    record: Record,
}

impl Retraction {
    /// All three records must agree. Absence or ambiguity grants no ownership.
    pub(super) fn prepare(path: &Path, software: &Software) -> Result<Option<Self>> {
        let root = open_root(path)?;
        let Some(bin) = present(&root, "bin")? else {
            return Ok(None);
        };
        let Some(Observed { entries, manifest }) =
            inspect(path, &root, &bin, software.command, software.member_here())?
        else {
            return Ok(None);
        };
        // Another coherent exposure is preserved in its entirety.
        if manifest.version != software.version {
            return Ok(None);
        }
        let receipt = super::ownership::Receipt::read(&root, software.command, software.version)?
            .ok_or_else(refuse)?;
        if manifest.executable != format!("{}/{}", software.version, receipt.member()) {
            return Err(refuse());
        }
        let retraction = Self {
            path: path.to_owned(),
            record: Record {
                root_identity: Identity::of(&root)?,
                bin_identity: Identity::of(&bin)?,
                entries,
            },
            bin,
        };
        retraction.revalidate(false)?;
        Ok(Some(retraction))
    }

    pub(super) fn into_record(self) -> Record {
        self.record
    }

    pub(super) fn resume(
        path: &Path,
        command: &str,
        version: &str,
        member: &str,
        record: Record,
    ) -> Result<Self> {
        let names = [
            super::exposed_name(command, member),
            format!(".{command}.version"),
            format!(".{command}.manifest.json"),
        ];
        if record.entries.len() != 3 {
            return Err(refuse());
        }
        for ((name, stamp), (expected, limit)) in
            record.entries.iter().zip(names.into_iter().zip([
                8 * 1024 * 1024 * 1024,
                1024,
                16 * 1024,
            ]))
        {
            if *name != expected || stamp.length > limit || stamp.mode > 0o7777 {
                return Err(refuse());
            }
            if stamp.link {
                if *name != super::exposed_name(command, member)
                    || Path::new(&stamp.content) != path.join(version).join(member)
                {
                    return Err(refuse());
                }
            } else if !super::ownership::digest_valid(&stamp.content) {
                return Err(refuse());
            }
        }
        let root = open_root(path)?;
        let bin = present(&root, "bin")?.ok_or_else(refuse)?;
        let retraction = Self {
            path: path.to_owned(),
            bin,
            record,
        };
        retraction.revalidate(true)?;
        Ok(retraction)
    }

    fn revalidate(&self, allow_missing: bool) -> Result<()> {
        let root = open_root(&self.path)?;
        if Identity::of(&root)? != self.record.root_identity
            || Identity::of(&present(&root, "bin")?.ok_or_else(refuse)?)?
                != self.record.bin_identity
        {
            return Err(refuse());
        }
        for (name, expected) in &self.record.entries {
            match stamp(&self.bin, name, expected.length)? {
                Some(found) if found == *expected => {}
                None if allow_missing => {}
                _ => return Err(refuse()),
            }
        }
        Ok(())
    }

    pub(super) fn complete(self) -> Result<()> {
        self.revalidate(true)?;
        for (name, expected) in self.record.entries {
            match stamp(&self.bin, &name, expected.length)? {
                Some(found) if found == expected => {
                    io(self.bin.remove_file(name))?;
                    sync(&self.bin)?;
                }
                None => {}
                _ => return Err(refuse()),
            }
        }
        Ok(())
    }
}
