//! Bounded local software records and checked directory identities.

use std::{
    io::Read,
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{Error, ReasonCode, Result};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
}

impl Identity {
    pub(super) fn of(directory: &Dir) -> Result<Self> {
        let metadata = io(directory.dir_metadata())?;
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt as _;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(refuse());
            }
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

pub(super) fn refuse() -> Error {
    Error::new(
        ReasonCode::RecoveryRequired,
        "software record, content or directory identity is inconsistent; recorded objects were preserved",
    )
}

pub(super) fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|error| refuse().with_source(error))
}

pub(super) fn leaf(value: &str) -> bool {
    let mut parts = Path::new(value).components();
    matches!(parts.next(), Some(Component::Normal(_)))
        && parts.next().is_none()
        && !value.contains(['/', '\\', ':'])
        && !value.ends_with(['.', ' '])
}

pub(super) fn member_valid(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':'])
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

pub(super) fn unique() -> Result<String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| refuse())?
        .as_nanos();
    Ok(format!(
        "{}-{time}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg_attr(
    not(unix),
    allow(
        clippy::unnecessary_wraps,
        reason = "Preserve the fallible Unix directory-sync interface."
    )
)]
pub(super) fn sync(directory: &Dir) -> Result<()> {
    // Windows does not offer directory fsync through this interface. File
    // contents are flushed on every platform; power-loss durability of directory
    // entries is claimed only where this directory flush succeeds.
    #[cfg(unix)]
    io(io(directory.open("."))?.sync_all())?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

pub(super) fn open_root(path: &Path) -> Result<Dir> {
    let parent = path.parent().ok_or_else(refuse)?;
    let name = path.file_name().ok_or_else(refuse)?;
    let parent = io(Dir::open_ambient_dir(parent, cap_std::ambient_authority()))?;
    io(parent.open_dir_nofollow(name))
}

pub(super) fn optional_root(path: &Path) -> Result<Option<Dir>> {
    match open_root(path) {
        Ok(root) => Ok(Some(root)),
        Err(error) => match std::fs::symlink_metadata(path) {
            Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => Ok(None),
            _ => Err(error),
        },
    }
}

pub(super) fn present(parent: &Dir, name: &str) -> Result<Option<Dir>> {
    match parent.open_dir_nofollow(name) {
        Ok(directory) => {
            Identity::of(&directory)?;
            Ok(Some(directory))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(refuse().with_source(error)),
    }
}

pub(super) fn read<T: DeserializeOwned>(root: &Dir, name: &str, limit: usize) -> Result<Option<T>> {
    read_bytes(root, name, limit)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| refuse()))
        .transpose()
}

pub(super) fn read_bytes(root: &Dir, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let mut file = match root.open_with(name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(refuse().with_source(error)),
    };
    let before = io(file.metadata())?;
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt as _;
        if before.file_attributes() & 0x400 != 0 {
            return Err(refuse());
        }
    }
    if !before.is_file() || before.nlink() != 1 || before.len() > limit as u64 {
        return Err(refuse());
    }
    let mut bytes = Vec::new();
    io(Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes))?;
    let after = io(file.metadata())?;
    let named = io(root.symlink_metadata(name))?;
    if bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || named.dev() != before.dev()
        || named.ino() != before.ino()
        || named.nlink() != 1
        || named.is_symlink()
    {
        return Err(refuse());
    }
    Ok(Some(bytes))
}
