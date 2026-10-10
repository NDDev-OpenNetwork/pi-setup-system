//! Prepare launch files before atomically replacing their directory entries.

use std::{
    io::{self, Read, Write},
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};
use sha2::{Digest, Sha256};

use super::{Manifest, launch};
use crate::{Error, ReasonCode, Result, digest};

fn invalid() -> io::Error {
    io::Error::other("software exposure is not a stable regular payload and plain bin directory")
}

fn plain(directory: &Dir) -> io::Result<()> {
    let metadata = directory.dir_metadata()?;
    if !metadata.is_dir() {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt as _;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg_attr(
    not(unix),
    allow(
        clippy::unnecessary_wraps,
        reason = "Preserve the fallible Unix directory-sync interface."
    )
)]
fn sync(directory: &Dir) -> io::Result<()> {
    #[cfg(unix)]
    directory.open(".")?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

fn temporary(name: &str) -> io::Result<String> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let name = name.trim_start_matches('.');
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    Ok(format!(
        ".{name}.incoming-{}-{time}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ))
}

struct Prepared<'a> {
    directory: &'a Dir,
    name: String,
    identity: (u64, u64),
    stamp: Option<launch::Stamp>,
    pending: bool,
}

impl<'a> Prepared<'a> {
    fn created(directory: &'a Dir, name: String, metadata: &Metadata) -> Self {
        Self {
            directory,
            name,
            identity: (metadata.dev(), metadata.ino()),
            stamp: None,
            pending: true,
        }
    }

    fn seal(mut self) -> io::Result<Self> {
        let stamp = launch::stamp(self.directory, &self.name, 8 * 1024 * 1024 * 1024)
            .map_err(|_| invalid())?
            .ok_or_else(invalid)?;
        if !stamp.has_identity(self.identity) {
            return Err(invalid());
        }
        self.stamp = Some(stamp);
        Ok(self)
    }

    fn unchanged(&self) -> io::Result<bool> {
        let Some(expected) = &self.stamp else {
            return Ok(false);
        };
        Ok(
            launch::stamp(self.directory, &self.name, 8 * 1024 * 1024 * 1024)
                .map_err(|_| invalid())?
                .as_ref()
                == Some(expected),
        )
    }

    fn bytes(directory: &'a Dir, name: &str, bytes: &[u8]) -> io::Result<Self> {
        let name = temporary(name)?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = directory.open_with(&name, &options)?;
        let prepared = Self::created(directory, name, &file.metadata()?);
        file.write_all(bytes)?;
        file.sync_all()?;
        prepared.seal()
    }

    fn commit(mut self, name: &str) -> io::Result<()> {
        if !self.unchanged()? {
            return Err(invalid());
        }
        self.directory.rename(&self.name, self.directory, name)?;
        self.pending = false;
        sync(self.directory)
    }
}

impl Drop for Prepared<'_> {
    fn drop(&mut self) {
        if self.pending && self.unchanged().unwrap_or(false) {
            // Exclusive creation does not authorize deleting a later replacement.
            let _ = self.directory.remove_file(&self.name);
        }
    }
}

pub(super) fn payload(root: &Dir, relative: &Path) -> io::Result<(Dir, File, String)> {
    let mut parent = root.try_clone()?;
    for part in relative.parent().ok_or_else(invalid)?.components() {
        let Component::Normal(name) = part else {
            return Err(invalid());
        };
        parent = parent.open_dir_nofollow(name)?;
        plain(&parent)?;
    }
    let name = relative.file_name().ok_or_else(invalid)?;
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let mut file = parent.open_with(name, &options)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > 8 * 1024 * 1024 * 1024 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt as _;
        if before.file_attributes() & 0x400 != 0 {
            return Err(invalid());
        }
    }
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0; 16 * 1024];
    let started = Instant::now();
    loop {
        if started.elapsed() > Duration::from_secs(20) {
            return Err(invalid());
        }
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > before.len() {
            return Err(invalid());
        }
        hash.update(&buffer[..read]);
    }
    let after = file.metadata()?;
    let reopened = parent.open_with(name, &options)?.metadata()?;
    if bytes != before.len()
        || before.modified().ok() != after.modified().ok()
        || after.len() != bytes
        || before.dev() != reopened.dev()
        || before.ino() != reopened.ino()
        || reopened.len() != bytes
        || reopened.modified().ok() != before.modified().ok()
    {
        return Err(invalid());
    }
    Ok((
        parent,
        file,
        format!("sha256:{}", digest::hex(&hash.finalize())),
    ))
}

#[cfg(not(unix))]
pub(super) fn wrapper(executable: &Path) -> io::Result<Option<Vec<u8>>> {
    use super::{MemberKind, member_kind};
    let path = executable.to_str().ok_or_else(invalid)?;
    let invocation = match member_kind(path, true) {
        MemberKind::JavaScript => "node",
        MemberKind::CommandScript => "call",
        MemberKind::Native => return Ok(None),
    };
    Ok(Some(
        format!("@{invocation} \"{path}\" %*\r\n").into_bytes(),
    ))
}

fn metadata_destination(directory: &Dir, name: &str) -> io::Result<()> {
    match directory.symlink_metadata(name) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {
            #[cfg(windows)]
            {
                use cap_std::fs::MetadataExt as _;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(invalid());
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(invalid()),
    }
}

pub(super) fn expose(
    executable: &Path,
    exposed: &Path,
    version: &str,
    command: &str,
    predecessor: &launch::Snapshot,
) -> Result<()> {
    let perform = || -> io::Result<()> {
        let root_path = exposed
            .parent()
            .and_then(Path::parent)
            .ok_or_else(invalid)?;
        let parent = Dir::open_ambient_dir(
            root_path.parent().ok_or_else(invalid)?,
            cap_std::ambient_authority(),
        )?;
        let root = parent.open_dir_nofollow(root_path.file_name().ok_or_else(invalid)?)?;
        plain(&root)?;
        let relative = executable.strip_prefix(root_path).map_err(|_| invalid())?;
        let member = relative
            .strip_prefix(version)
            .map_err(|_| invalid())?
            .to_str()
            .ok_or_else(invalid)?;
        predecessor
            .validate(root_path, command, member)
            .map_err(|_| invalid())?;
        let (source_parent, source, sha256) = payload(&root, relative)?;
        let manifest = Manifest {
            schema_version: 1,
            version: version.to_owned(),
            executable: relative.to_string_lossy().replace('\\', "/"),
            executable_sha256: sha256,
        };
        let body = serde_json::to_vec(&manifest).map_err(io::Error::other)?;

        let bin = root.open_dir_nofollow("bin")?;
        plain(&bin)?;
        let name = exposed
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(invalid)?;
        let marker_name = format!(".{command}.version");
        let manifest_name = format!(".{command}.manifest.json");
        metadata_destination(&bin, &marker_name)?;
        metadata_destination(&bin, &manifest_name)?;
        let marker = Prepared::bytes(&bin, &marker_name, version.as_bytes())?;
        let manifest = Prepared::bytes(&bin, &manifest_name, &body)?;
        let entry = prepare_entry(&bin, name, executable, &source_parent, source)?;
        let check_identity = || -> io::Result<()> {
            let current_parent = Dir::open_ambient_dir(
                root_path.parent().ok_or_else(invalid)?,
                cap_std::ambient_authority(),
            )?;
            let current =
                current_parent.open_dir_nofollow(root_path.file_name().ok_or_else(invalid)?)?;
            let current_bin = current.open_dir_nofollow("bin")?;
            for (held, found) in [(&root, &current), (&bin, &current_bin)] {
                let held = held.dir_metadata()?;
                let found = found.dir_metadata()?;
                if held.dev() != found.dev() || held.ino() != found.ino() {
                    return Err(invalid());
                }
            }
            Ok(())
        };
        check_identity()?;
        let prepared = [
            (entry, name),
            (marker, marker_name.as_str()),
            (manifest, manifest_name.as_str()),
        ];
        let replacements = prepared
            .iter()
            .map(|(entry, _)| entry.stamp.clone().ok_or_else(invalid))
            .collect::<io::Result<Vec<_>>>()?;
        // Refuse every foreign entry before replacing any member. During a
        // recorded recovery, a member may already contain this exact result.
        for (index, replacement) in replacements.iter().enumerate() {
            predecessor
                .check_replacement(&bin, index, replacement)
                .map_err(|_| invalid())?;
        }
        for (index, (entry, name)) in prepared.into_iter().enumerate() {
            check_identity()?;
            predecessor
                .check_replacement(&bin, index, &replacements[index])
                .map_err(|_| invalid())?;
            entry.commit(name)?;
        }
        check_identity()
    };
    perform().map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            "software entry point could not be atomically exposed",
        )
        .with_source(error)
    })
}

fn prepare_entry<'a>(
    bin: &'a Dir,
    name: &str,
    executable: &Path,
    source_parent: &Dir,
    mut source: File,
) -> io::Result<Prepared<'a>> {
    let temporary = temporary(name)?;
    #[cfg(unix)]
    {
        let _ = (source_parent, &mut source);
        bin.symlink_contents(executable, &temporary)?;
    }
    #[cfg(not(unix))]
    {
        use super::{MemberKind, member_kind};
        match member_kind(&executable.to_string_lossy(), true) {
            MemberKind::JavaScript | MemberKind::CommandScript => {
                Prepared::bytes(bin, name, &wrapper(executable)?.ok_or_else(invalid)?)
            }
            MemberKind::Native => {
                let source_name = executable.file_name().ok_or_else(invalid)?;
                if source_parent
                    .hard_link(source_name, bin, &temporary)
                    .is_err()
                {
                    use std::io::Seek;
                    source.rewind()?;
                    let mut options = OpenOptions::new();
                    options
                        .write(true)
                        .create_new(true)
                        .follow(FollowSymlinks::No);
                    let mut file = bin.open_with(&temporary, &options)?;
                    let prepared = Prepared::created(bin, temporary, &file.metadata()?);
                    let expected = source.metadata()?.len();
                    if io::copy(&mut source.take(expected + 1), &mut file)? != expected {
                        return Err(invalid());
                    }
                    file.sync_all()?;
                    return prepared.seal();
                }
                let expected = source.metadata()?;
                let prepared = Prepared::created(bin, temporary, &expected);
                let mut options = OpenOptions::new();
                options.read(true).follow(FollowSymlinks::No).nonblock(true);
                let linked = bin.open_with(&prepared.name, &options)?.metadata()?;
                if expected.dev() != linked.dev() || expected.ino() != linked.ino() {
                    return Err(invalid());
                }
                prepared.seal()
            }
        }
    }
    #[cfg(unix)]
    {
        let metadata = bin.symlink_metadata(&temporary)?;
        if bin.read_link_contents(&temporary)? != executable {
            return Err(invalid());
        }
        Prepared::created(bin, temporary, &metadata).seal()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use std::fs;

    #[test]
    fn temporary_replacement_neither_commits_nor_deletes_a_substituted_entry() {
        let path = std::env::temp_dir().join(format!(
            "software-prepared-{}",
            super::super::records::unique().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let prepared = Prepared::bytes(&directory, ".tool.version", b"1.2.3").unwrap();
        let name = prepared.name.clone();
        fs::rename(path.join(&name), path.join("retained-first")).unwrap();
        fs::write(path.join(&name), b"foreign temporary entry").unwrap();
        drop(prepared);
        assert_eq!(
            fs::read(path.join(&name)).unwrap(),
            b"foreign temporary entry"
        );

        let prepared = Prepared::bytes(&directory, ".tool.version", b"1.2.3").unwrap();
        let name = prepared.name.clone();
        fs::rename(path.join(&name), path.join("retained-second")).unwrap();
        fs::write(path.join(&name), b"foreign replacement").unwrap();
        fs::write(path.join(".tool.version"), b"previous").unwrap();
        assert!(prepared.commit(".tool.version").is_err());
        assert_eq!(fs::read(path.join(&name)).unwrap(), b"foreign replacement");
        assert_eq!(fs::read(path.join(".tool.version")).unwrap(), b"previous");
        let prepared = Prepared::bytes(&directory, ".tool.version", b"1.2.3").unwrap();
        let name = prepared.name.clone();
        fs::write(path.join(&name), b"changed in place").unwrap();
        assert!(prepared.commit(".tool.version").is_err());
        assert_eq!(fs::read(path.join(&name)).unwrap(), b"changed in place");
        assert_eq!(fs::read(path.join(".tool.version")).unwrap(), b"previous");

        let prepared = Prepared::bytes(&directory, ".tool.version", b"1.2.3").unwrap();
        let name = prepared.name.clone();
        drop(prepared);
        assert!(!path.join(name).exists());
        Prepared::bytes(&directory, ".tool.version", b"1.2.3")
            .unwrap()
            .commit(".tool.version")
            .unwrap();
        assert_eq!(fs::read(path.join(".tool.version")).unwrap(), b"1.2.3");
        drop(directory);
        fs::remove_dir_all(path).unwrap();
    }
}
