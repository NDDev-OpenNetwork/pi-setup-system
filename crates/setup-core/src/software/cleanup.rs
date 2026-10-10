//! Delete only unchanged remaining members of a recorded installation.

use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::{Dir, Metadata};

use super::{
    exposure,
    records::{Identity, io, refuse, sync},
};
use crate::{
    Result,
    software_prefix::{self, Entry, EntryKind},
};

fn mode(metadata: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn remove_member(root: &Dir, entry: &Entry) -> Result<()> {
    let path = Path::new(&entry.path);
    let mut parent = io(root.try_clone())?;
    for component in path.parent().ok_or_else(refuse)?.components() {
        parent = io(parent.open_dir_nofollow(component.as_os_str()))?;
        Identity::of(&parent)?;
    }
    let name = path.file_name().ok_or_else(refuse)?;
    let before = io(parent.symlink_metadata(name))?;
    if mode(&before) != entry.mode {
        return Err(refuse());
    }
    match entry.kind {
        EntryKind::Directory => {
            if !before.is_dir() || before.is_symlink() {
                return Err(refuse());
            }
            let directory = io(parent.open_dir_nofollow(name))?;
            Identity::of(&directory)?;
            let held = io(directory.dir_metadata())?;
            if before.dev() != held.dev() || before.ino() != held.ino() {
                return Err(refuse());
            }
            // Empty-only deletion cannot recursively consume a newly added file.
            io(directory.remove_open_dir())?;
        }
        EntryKind::File | EntryKind::Link => {
            let observed = match entry.kind {
                EntryKind::File if before.is_file() && !before.is_symlink() => {
                    io(exposure::payload(&parent, Path::new(name)))?.2
                }
                EntryKind::Link if before.is_symlink() => io(parent.read_link_contents(name))?
                    .to_str()
                    .ok_or_else(refuse)?
                    .to_owned(),
                _ => return Err(refuse()),
            };
            let expected = if entry.kind == EntryKind::File {
                format!("sha256:{}", entry.payload)
            } else {
                entry.payload.clone()
            };
            let after = io(parent.symlink_metadata(name))?;
            if observed != expected
                || before.dev() != after.dev()
                || before.ino() != after.ino()
                || before.len() != after.len()
                || before.modified().ok() != after.modified().ok()
                || mode(&after) != entry.mode
            {
                return Err(refuse());
            }
            io(parent.remove_file_or_symlink(name))?;
        }
    }
    sync(&parent)
}

pub(super) fn inspect_remaining(directory: &Dir, owned: &[Entry]) -> Result<Vec<Entry>> {
    software_prefix::inventory_digest(owned).map_err(|_| refuse())?;
    let (_, remaining) = software_prefix::inventory_directory(directory)?;
    let expected: BTreeMap<_, _> = owned.iter().map(|entry| (&entry.path, entry)).collect();
    if remaining
        .iter()
        .any(|entry| expected.get(&entry.path).copied() != Some(entry))
    {
        return Err(refuse());
    }
    Ok(remaining)
}

pub(super) fn remaining(directory: &Dir, owned: &[Entry]) -> Result<()> {
    // Observe the complete remaining subset before making any deletion. The
    // reverse preorder visits children before their directories.
    let remaining = inspect_remaining(directory, owned)?;
    let started = Instant::now();
    for entry in remaining.iter().skip(1).rev() {
        if started.elapsed() > Duration::from_secs(20) {
            return Err(refuse());
        }
        remove_member(directory, entry)?;
    }
    Ok(())
}
