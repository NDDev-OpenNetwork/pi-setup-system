//! Validate launch payloads and publish one durable preparation.

use std::{
    io::{self, Read},
    path::{Component, Path},
    time::{Duration, Instant},
};

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, File, OpenOptions};
use sha2::{Digest, Sha256};

use super::{
    Manifest, launch,
    preparation::{Binding, Expected, Preparation},
    records,
};
use crate::{Result, digest};

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

pub(super) fn expose(
    executable: &Path,
    exposed: &Path,
    version: &str,
    command: &str,
    predecessor: &launch::Snapshot,
) -> Result<()> {
    let root_path = exposed
        .parent()
        .and_then(Path::parent)
        .ok_or_else(records::refuse)?;
    let root = records::open_root(root_path)?;
    let relative = executable
        .strip_prefix(root_path)
        .map_err(|_| records::refuse())?;
    let member = relative
        .strip_prefix(version)
        .map_err(|_| records::refuse())?
        .to_str()
        .ok_or_else(records::refuse)?;
    predecessor.validate(root_path, command, member)?;
    let (_source_parent, source, sha256) = records::io(payload(&root, relative))?;
    let manifest = Manifest {
        schema_version: 1,
        version: version.to_owned(),
        executable: relative.to_string_lossy().replace('\\', "/"),
        executable_sha256: sha256.clone(),
    };
    let body = serde_json::to_vec(&manifest).map_err(|_| records::refuse())?;
    let bin = records::io(root.open_dir_nofollow("bin"))?;
    let name = exposed
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(records::refuse)?;
    #[cfg(unix)]
    let entry = Expected::link(executable)?;
    #[cfg(not(unix))]
    let wrapper = records::io(wrapper(executable))?;
    #[cfg(not(unix))]
    let entry = wrapper.as_ref().map_or_else(
        || {
            Ok(Expected::file(
                records::io(source.metadata())?.len(),
                sha256,
            ))
        },
        |bytes| Ok(Expected::bytes(bytes)),
    )?;
    let binding = Binding {
        root_identity: records::Identity::of(&root)?,
        bin_identity: records::Identity::of(&bin)?,
        predecessor_digest: digest::of_bytes(
            &serde_json::to_vec(predecessor).map_err(|_| records::refuse())?,
        ),
        version: version.to_owned(),
        member: member.to_owned(),
        names: [
            name.to_owned(),
            format!(".{command}.version"),
            format!(".{command}.manifest.json"),
        ],
        expected: [
            entry,
            Expected::bytes(version.as_bytes()),
            Expected::bytes(&body),
        ],
    };
    if !super::store::exists(&root, &super::preparation::journal(command))? {
        // Completion can precede deletion of the parent staging/switch record.
        // An already exact result needs no new temporary entries or publication.
        let current = binding
            .names
            .iter()
            .map(|name| launch::stamp(&bin, name, 8 * 1024 * 1024 * 1024))
            .collect::<Result<Vec<_>>>()?;
        if current
            .iter()
            .zip(&binding.expected)
            .all(|(stamp, expected)| stamp.as_ref().is_some_and(|stamp| expected.matches(stamp)))
        {
            return Ok(());
        }
        // A first preparation must refuse changed destinations before it writes
        // its own intent or temporary files. Partial renames have a journal.
        predecessor.revalidate(root_path, command, member)?;
    }
    let mut preparation = Preparation::begin(root_path, &root, &bin, command, &binding)?;
    preparation.bytes(1, version.as_bytes())?;
    preparation.bytes(2, &body)?;
    #[cfg(unix)]
    {
        let _ = source;
        preparation.link(0, executable)?;
    }
    #[cfg(not(unix))]
    if let Some(bytes) = wrapper {
        preparation.bytes(0, &bytes)?;
    } else {
        // A held source is copied through the same bounded prefix verifier as
        // metadata. Its identity is recorded before any byte reaches bin.
        preparation.regular(0, &mut { source })?;
    }
    preparation.commit(predecessor)
}
