//! Bounded, lossless software-prefix observations for plan preconditions.
//!
//! Only validated shared writer metadata and the provider control directory are excluded. Links contribute
//! their literal destinations; no link or special file is opened for content.
//! This observation establishes neither ownership nor permission to remove files.

use std::{
    collections::BTreeSet,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Error, ReasonCode, Result};

const MAX_ENTRIES: usize = 65_536;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_PATH_BYTES: usize = 8192;
const DEADLINE: Duration = Duration::from_secs(20);

/// Complete installation inventories are metadata, never copies of file data.
pub(crate) const INVENTORY_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryKind {
    Directory,
    File,
    Link,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub mode: u32,
    pub payload: String,
}

/// Reconstruct the exact stage seal from its recorded membership.
pub(crate) fn inventory_digest(entries: &[Entry]) -> Result<String> {
    if entries.is_empty() || entries.len() > MAX_ENTRIES + 1 {
        return Err(invalid());
    }
    let mut paths = BTreeSet::new();
    let mut directories = BTreeSet::new();
    let mut hash = Sha256::new();
    field(&mut hash, b"nddev:software-stage:v1");
    for (index, entry) in entries.iter().enumerate() {
        if entry.path.len() > MAX_PATH_BYTES || entry.mode > 0o7777 || !paths.insert(&entry.path) {
            return Err(invalid());
        }
        if index == 0 {
            if !entry.path.is_empty() || entry.kind != EntryKind::Directory {
                return Err(invalid());
            }
        } else {
            let path = Path::new(&entry.path);
            if entry.path.is_empty()
                || entry.path.contains('\\')
                || !path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
                || path.components().count() > MAX_DEPTH + 1
                || path
                    .iter()
                    .map(|part| part.to_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("/")
                    != entry.path
                || !directories
                    .contains(&entry.path.rsplit_once('/').map_or("", |(parent, _)| parent))
            {
                return Err(invalid());
            }
        }
        let (kind, payload) = match entry.kind {
            EntryKind::Directory => {
                if !entry.payload.is_empty() {
                    return Err(invalid());
                }
                directories.insert(entry.path.as_str());
                (b"directory".as_slice(), Vec::new())
            }
            EntryKind::File => {
                if entry.payload.len() != 64
                    || !entry
                        .payload
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(invalid());
                }
                let bytes = entry
                    .payload
                    .as_bytes()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| {
                        let digit = |b: u8| {
                            if b.is_ascii_digit() {
                                b - b'0'
                            } else {
                                b - b'a' + 10
                            }
                        };
                        (digit(pair[0]) << 4) | digit(pair[1])
                    })
                    .collect();
                (b"file".as_slice(), bytes)
            }
            EntryKind::Link => {
                if entry.payload.len() > MAX_PATH_BYTES {
                    return Err(invalid());
                }
                (b"link".as_slice(), entry.payload.as_bytes().to_vec())
            }
        };
        for value in [
            entry.path.as_bytes(),
            kind,
            &entry.mode.to_be_bytes(),
            &payload,
        ] {
            field(&mut hash, value);
        }
    }
    Ok(format!("sha256:{}", crate::digest::hex(&hash.finalize())))
}

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
    inventory: Option<Vec<Entry>>,
    inventory_bytes: usize,
    omitted: BTreeSet<String>,
    physical_directories: bool,
    omit_bin_node: bool,
}

impl Reading {
    fn check(&self) -> Result<()> {
        if self.started.elapsed() > DEADLINE {
            return Err(invalid());
        }
        Ok(())
    }

    fn record(&mut self, name: &str, kind: &[u8], mode: u32, payload: &[u8]) -> Result<()> {
        for value in [name.as_bytes(), kind, &mode.to_be_bytes(), payload] {
            field(&mut self.hash, value);
        }
        if let Some(entries) = &mut self.inventory {
            let (kind, payload) = match kind {
                b"directory" => (EntryKind::Directory, String::new()),
                b"file" => (EntryKind::File, crate::digest::hex(payload)),
                b"link" => (
                    EntryKind::Link,
                    std::str::from_utf8(payload)
                        .map_err(|_| invalid())?
                        .to_owned(),
                ),
                _ => return Err(invalid()),
            };
            self.inventory_bytes += name.len() + payload.len() + 128;
            if self.inventory_bytes > INVENTORY_LIMIT {
                return Err(invalid());
            }
            entries.push(Entry {
                path: name.to_owned(),
                kind,
                mode,
                payload,
            });
        }
        Ok(())
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
        if !(self.omit_bin_node && relative == "bin") {
            self.record(relative, b"directory", permissions(&before), b"")?;
            if self.physical_directories {
                field(&mut self.hash, &before.dev().to_be_bytes());
                field(&mut self.hash, &before.ino().to_be_bytes());
            }
        }
        let mut names = Vec::new();
        for entry in io(dir.entries())? {
            self.check()?;
            let name = io(entry)?.file_name();
            if depth == 0
                && (name == control
                    || (!control.is_empty() && name == crate::software::writer::CONTROL))
            {
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
            if self.omitted.contains(&path) {
                continue;
            }
            let metadata = io(dir.symlink_metadata(&name))?;
            if metadata.is_symlink() {
                let target = io(dir.read_link_contents(&name))?;
                let value = text(&target)?;
                self.record(&path, b"link", permissions(&metadata), value.as_bytes())?;
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
        self.record(path, b"file", permissions(&before), &content.finalize())?;
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
        inventory: None,
        inventory_bytes: 0,
        omitted: BTreeSet::new(),
        physical_directories: false,
        omit_bin_node: false,
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
        crate::software::writer::validate_control(&dir)?;
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

/// Seal every entry below a held stage, with no excluded bookkeeping name.
pub(crate) fn digest_directory(directory: &Dir) -> Result<String> {
    let mut reading = Reading {
        started: Instant::now(),
        entries: 0,
        bytes: 0,
        hash: Sha256::new(),
        inventory: None,
        inventory_bytes: 0,
        omitted: BTreeSet::new(),
        physical_directories: false,
        omit_bin_node: false,
    };
    field(&mut reading.hash, b"nddev:software-stage:v1");
    reading.walk(directory, "", 0, "")?;
    Ok(format!(
        "sha256:{}",
        crate::digest::hex(&reading.hash.finalize())
    ))
}

/// Record the same seal and all owned members in one bounded read.
pub(crate) fn inventory_directory(directory: &Dir) -> Result<(String, Vec<Entry>)> {
    let mut reading = Reading {
        started: Instant::now(),
        entries: 0,
        bytes: 0,
        hash: Sha256::new(),
        inventory: Some(Vec::new()),
        inventory_bytes: 0,
        omitted: BTreeSet::new(),
        physical_directories: false,
        omit_bin_node: false,
    };
    field(&mut reading.hash, b"nddev:software-stage:v1");
    reading.walk(directory, "", 0, "")?;
    let digest = format!("sha256:{}", crate::digest::hex(&reading.hash.finalize()));
    let entries = reading.inventory.ok_or_else(invalid)?;
    if inventory_digest(&entries)? != digest {
        return Err(invalid());
    }
    Ok((digest, entries))
}

/// Bind all paths outside exact recorded effects, including directory identity.
/// An initially absent bin may become an empty directory; its children are
/// still observed. This private digest does not change the public prefix format.
pub(crate) fn resume_digest(
    directory: &Dir,
    control: &str,
    omitted: &[String],
    bin_was_absent: bool,
) -> Result<String> {
    let mut reading = Reading {
        started: Instant::now(),
        entries: 0,
        bytes: 0,
        hash: Sha256::new(),
        inventory: None,
        inventory_bytes: 0,
        omitted: omitted.iter().cloned().collect(),
        physical_directories: true,
        omit_bin_node: bin_was_absent,
    };
    field(&mut reading.hash, b"nddev:software-operation-scope:v1");
    reading.walk(directory, "", 0, control)?;
    Ok(format!(
        "sha256:{}",
        crate::digest::hex(&reading.hash.finalize())
    ))
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
        let held = Dir::open_ambient_dir(&root, cap_std::ambient_authority()).unwrap();
        let (seal, entries) = inventory_directory(&held).unwrap();
        assert_eq!(seal, digest_directory(&held).unwrap());
        assert_eq!(seal, inventory_digest(&entries).unwrap());
        assert!(entries.iter().any(|entry| entry.path == "cafe\u{301}"));
        let mut changed_inventory = entries.clone();
        changed_inventory.pop();
        assert_ne!(seal, inventory_digest(&changed_inventory).unwrap());
        let mut invalid_inventory = entries;
        invalid_inventory[0].path = "..".to_owned();
        assert!(inventory_digest(&invalid_inventory).is_err());
        drop(held);
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
            use std::os::unix::{
                fs::{PermissionsExt, symlink},
                net::UnixListener,
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
            // Linux permits byte names which APFS refuses before observation.
            #[cfg(target_os = "linux")]
            {
                use std::{ffi::OsString, os::unix::ffi::OsStringExt};
                let invalid = root.join(OsString::from_vec(vec![0xff]));
                fs::write(&invalid, b"unrepresentable name").unwrap();
                assert!(observe(&root, ".control").is_err());
                fs::remove_file(invalid).unwrap();
            }
            // macOS's user temporary directory can exceed sockaddr_un's
            // pathname budget. Keep this negative control in an exclusively
            // created short directory; never change the process-wide cwd.
            let socket_root = Path::new("/tmp").join(format!(
                "nddev-socket-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&socket_root).unwrap();
            assert!(observe(&socket_root, ".control").is_ok());
            let socket = UnixListener::bind(socket_root.join("socket")).unwrap();
            assert!(observe(&socket_root, ".control").is_err());
            drop(socket);
            fs::remove_file(socket_root.join("socket")).unwrap();
            assert!(observe(&socket_root, ".control").is_ok());
            fs::remove_dir(socket_root).unwrap();
        }
        fs::remove_dir_all(parent).unwrap();
    }
}
