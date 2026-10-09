//! Bounded, lossless software-prefix observations for plan preconditions.
//!
//! Only the provider's top-level control directory is excluded. Links contribute
//! their literal destinations; no link or special file is opened for content.
//! This observation establishes neither ownership nor permission to remove files.

use std::{
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use sha2::{Digest, Sha256};

use crate::{Error, ReasonCode, Result};

const MAX_ENTRIES: usize = 65_536;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_PATH_BYTES: usize = 8192;
const DEADLINE: Duration = Duration::from_secs(20);

/// The observed prefix state, excluding provider lock and journal bookkeeping.
pub struct Observation {
    /// Versioned SHA-256 binding of presence, paths, kinds, permissions and bytes.
    pub digest: String,
    /// Whether the prefix existed as a plain directory.
    pub present: bool,
    /// Whether no entries outside the provider control directory were present.
    pub empty: bool,
}

fn invalid() -> Error {
    Error::new(
        ReasonCode::StateUnavailable,
        "software prefix is changing, inaccessible or exceeds its observation limits",
    )
}

fn io<T>(value: std::io::Result<T>) -> Result<T> {
    value.map_err(|source| invalid().with_source(source))
}

fn text(path: &Path) -> Result<&str> {
    path.to_str()
        .filter(|s| s.len() <= MAX_PATH_BYTES)
        .ok_or_else(invalid)
}

fn field(hash: &mut Sha256, value: &[u8]) {
    hash.update((value.len() as u64).to_be_bytes());
    hash.update(value);
}

fn permissions(meta: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        meta.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(meta.permissions().readonly())
    }
}

fn same(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && permissions(left) == permissions(right)
}

fn reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

struct Reading {
    started: Instant,
    entries: usize,
    bytes: u64,
    hash: Sha256,
}

impl Reading {
    fn check(&self) -> Result<()> {
        if self.started.elapsed() > DEADLINE {
            return Err(invalid());
        }
        Ok(())
    }

    fn record(&mut self, name: &str, kind: &[u8], mode: u32, payload: &[u8]) {
        for value in [name.as_bytes(), kind, &mode.to_be_bytes(), payload] {
            field(&mut self.hash, value);
        }
    }

    fn walk(&mut self, dir: &Dir, relative: &str, depth: usize, control: &str) -> Result<()> {
        self.check()?;
        if depth > MAX_DEPTH {
            return Err(invalid());
        }
        let before = io(dir.dir_metadata())?;
        if reparse(&before) {
            return Err(invalid());
        }
        self.record(relative, b"directory", permissions(&before), b"");
        let mut names = Vec::new();
        for entry in io(dir.entries())? {
            self.check()?;
            let name = io(entry)?.file_name();
            if depth == 0 && name == control {
                continue;
            }
            self.entries += 1;
            if self.entries > MAX_ENTRIES {
                return Err(invalid());
            }
            names.push(name);
        }
        names.sort();
        for name in names {
            self.check()?;
            let leaf = text(Path::new(&name))?;
            let path = if relative.is_empty() {
                leaf.to_owned()
            } else {
                format!("{relative}/{leaf}")
            };
            if path.len() > MAX_PATH_BYTES {
                return Err(invalid());
            }
            let metadata = io(dir.symlink_metadata(&name))?;
            if metadata.is_symlink() {
                let target = io(dir.read_link_contents(&name))?;
                let value = text(&target)?;
                self.record(&path, b"link", permissions(&metadata), value.as_bytes());
                if io(dir.read_link_contents(&name))? != target {
                    return Err(invalid());
                }
            } else if reparse(&metadata) {
                return Err(invalid());
            } else if metadata.is_dir() {
                let child = io(dir.open_dir_nofollow(&name))?;
                self.walk(&child, &path, depth + 1, control)?;
                let reopened = io(dir.open_dir_nofollow(&name))?;
                if !same(&io(child.dir_metadata())?, &io(reopened.dir_metadata())?) {
                    return Err(invalid());
                }
            } else if metadata.is_file() {
                self.file(dir, Path::new(&name), &path)?;
            } else {
                return Err(invalid());
            }
        }
        if !same(&before, &io(dir.dir_metadata())?) {
            return Err(invalid());
        }
        Ok(())
    }

    fn file(&mut self, dir: &Dir, name: &Path, path: &str) -> Result<()> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let mut file = io(dir.open_with(name, &options))?;
        let before = io(file.metadata())?;
        if !before.is_file() || reparse(&before) || before.len() > MAX_BYTES - self.bytes {
            return Err(invalid());
        }
        let mut content = Sha256::new();
        let mut read = 0u64;
        let mut buffer = [0; 16 * 1024];
        loop {
            self.check()?;
            let count = io(file.read(&mut buffer))?;
            if count == 0 {
                break;
            }
            self.bytes += count as u64;
            read += count as u64;
            if self.bytes > MAX_BYTES || read > before.len() {
                return Err(invalid());
            }
            content.update(&buffer[..count]);
        }
        let reopened = io(dir.open_with(name, &options))?;
        if read != before.len()
            || !same(&before, &io(file.metadata())?)
            || !same(&before, &io(reopened.metadata())?)
        {
            return Err(invalid());
        }
        self.record(path, b"file", permissions(&before), &content.finalize());
        Ok(())
    }
}

/// Observe the prefix without writes or following links inside it.
///
/// Missing and empty directories have distinct digests. Paths must be Unicode;
/// their spelling is preserved without normalization. The walk is bounded to
/// 65,536 entries, 8 GiB, 64 levels, 8 KiB paths and a 20-second read budget.
///
/// # Errors
/// Refuses unreadable or changing trees, aliases at the root, special files,
/// unrepresentable paths and exceeded limits. No partial digest is returned.
pub fn observe(root: &Path, control: &str) -> Result<Observation> {
    if !root.is_absolute()
        || root.file_name().is_none()
        || Path::new(control).components().count() != 1
        || !matches!(
            Path::new(control).components().next(),
            Some(std::path::Component::Normal(_))
        )
        || matches!(control, "" | "." | "..")
    {
        return Err(invalid());
    }
    text(root)?;
    let mut reading = Reading {
        started: Instant::now(),
        entries: 0,
        bytes: 0,
        hash: Sha256::new(),
    };
    field(&mut reading.hash, b"nddev:software-prefix:v1");
    let present = match std::fs::symlink_metadata(root) {
        Ok(meta) if meta.is_dir() && !meta.is_symlink() => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        _ => return Err(invalid()),
    };
    field(
        &mut reading.hash,
        if present { b"present" } else { b"absent" },
    );
    if present {
        let parent = io(Dir::open_ambient_dir(
            root.parent().ok_or_else(invalid)?,
            cap_std::ambient_authority(),
        ))?;
        let name = root.file_name().ok_or_else(invalid)?;
        let dir = io(parent.open_dir_nofollow(name))?;
        reading.walk(&dir, "", 0, control)?;
        let reopened = io(parent.open_dir_nofollow(name))?;
        if !same(&io(dir.dir_metadata())?, &io(reopened.dir_metadata())?) {
            return Err(invalid());
        }
    }
    Ok(Observation {
        digest: format!("sha256:{}", crate::digest::hex(&reading.hash.finalize())),
        present,
        empty: reading.entries == 0,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use std::fs;

    #[test]
    fn observations_bind_exact_content_and_refuse_unreadable_shapes() {
        let parent = std::env::temp_dir().join(format!(
            "software-prefix-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&parent).unwrap();
        let root = parent.join("prefix");
        let missing = observe(&root, ".control").unwrap();
        assert!(!missing.present);
        fs::create_dir(&root).unwrap();
        let empty = observe(&root, ".control").unwrap();
        assert!(empty.present && empty.empty);
        assert_ne!(missing.digest, empty.digest);
        fs::create_dir(root.join(".control")).unwrap();
        fs::write(root.join(".control/lock"), b"bookkeeping").unwrap();
        assert_eq!(observe(&root, ".control").unwrap().digest, empty.digest);
        fs::write(root.join("cafe\u{301}"), b"payload").unwrap();
        let original = observe(&root, ".control").unwrap();
        fs::rename(root.join("cafe\u{301}"), root.join("cafe\u{301}-renamed")).unwrap();
        assert_ne!(original.digest, observe(&root, ".control").unwrap().digest);
        fs::rename(root.join("cafe\u{301}-renamed"), root.join("cafe\u{301}")).unwrap();
        fs::write(root.join("cafe\u{301}"), b"changed").unwrap();
        assert_ne!(original.digest, observe(&root, ".control").unwrap().digest);
        fs::remove_file(root.join("cafe\u{301}")).unwrap();
        let mut deep = root.clone();
        for _ in 0..=MAX_DEPTH {
            deep.push("d");
            fs::create_dir(&deep).unwrap();
        }
        assert!(observe(&root, ".control").is_err());
        fs::remove_dir_all(root.join("d")).unwrap();
        let large = fs::File::create(root.join("large")).unwrap();
        large.set_len(MAX_BYTES + 1).unwrap();
        assert!(observe(&root, ".control").is_err());
        drop(large);
        fs::remove_file(root.join("large")).unwrap();
        #[cfg(unix)]
        {
            use std::{
                ffi::OsString,
                os::unix::{
                    ffi::OsStringExt,
                    fs::{PermissionsExt, symlink},
                    net::UnixListener,
                },
            };
            fs::write(parent.join("outside"), b"outside").unwrap();
            symlink(parent.join("outside"), root.join("link")).unwrap();
            let linked = observe(&root, ".control").unwrap().digest;
            fs::write(parent.join("outside"), b"outside changed").unwrap();
            assert_eq!(linked, observe(&root, ".control").unwrap().digest);
            fs::remove_file(root.join("link")).unwrap();
            symlink(parent.join("elsewhere"), root.join("link")).unwrap();
            assert_ne!(linked, observe(&root, ".control").unwrap().digest);
            fs::remove_file(root.join("link")).unwrap();
            fs::write(root.join("executable"), b"file").unwrap();
            fs::set_permissions(root.join("executable"), fs::Permissions::from_mode(0o600))
                .unwrap();
            let unexecutable = observe(&root, ".control").unwrap().digest;
            fs::set_permissions(root.join("executable"), fs::Permissions::from_mode(0o700))
                .unwrap();
            assert_ne!(unexecutable, observe(&root, ".control").unwrap().digest);
            let invalid = root.join(OsString::from_vec(vec![0xff]));
            fs::write(&invalid, b"unrepresentable name").unwrap();
            assert!(observe(&root, ".control").is_err());
            fs::remove_file(invalid).unwrap();
            let socket = UnixListener::bind(root.join("socket")).unwrap();
            assert!(observe(&root, ".control").is_err());
            drop(socket);
        }
        fs::remove_dir_all(parent).unwrap();
    }
}
