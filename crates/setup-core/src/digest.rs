//! SHA-256 over bytes, files and whole trees.
//!
//! Every digest this kernel produces is prefixed `sha256:` on the wire, so a
//! bare hexadecimal string never travels where an algorithm-tagged one is
//! expected.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::canonical;
use crate::error::{Error, ReasonCode, Result};

/// The `sha256:` prefix every wire digest carries.
pub const PREFIX: &str = "sha256:";

/// Hash bytes and return the prefixed hexadecimal digest.
#[must_use]
pub fn of_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{PREFIX}{:x}", hasher.finalize())
}

/// Hash a JSON value through its RFC 8785 canonicalization.
///
/// # Errors
///
/// Propagates the refusal from [`canonical::to_canonical_bytes`].
pub fn of_canonical_json(value: &serde_json::Value) -> Result<String> {
    Ok(of_bytes(&canonical::to_canonical_bytes(value)?))
}

/// Hash a regular file without loading it whole.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if the file cannot be read.
pub fn of_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot open {}", path.display()),
        )
        .with_source(source)
    })?;
    let mut hasher = Sha256::new();
    // Heap-allocated: a 64 KiB stack frame is a poor trade for a hot loop that
    // runs once per file in a tree walk.
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = read_chunk(&mut file, &mut buffer).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot read {}", path.display()),
            )
            .with_source(source)
        })?;
        if read == 0 {
            break;
        }
        match buffer.get(..read) {
            Some(chunk) => hasher.update(chunk),
            None => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    "read reported more bytes than the buffer holds",
                ));
            }
        }
    }
    Ok(format!("{PREFIX}{:x}", hasher.finalize()))
}

fn read_chunk(file: &mut fs::File, buffer: &mut [u8]) -> io::Result<usize> {
    file.read(buffer)
}

/// Hash a directory tree by relative path, executable bit and content.
///
/// Modification times are excluded deliberately: a checkout that only differs
/// in `mtime` is the same tree, and treating it as different would make every
/// fresh clone read as drift. The executable bit is included just as
/// deliberately — mode drift is a real difference that a content-only digest
/// cannot see, and the frozen estate learned that the expensive way.
///
/// Symbolic links are recorded as their link target rather than followed, so a
/// link that points outside the tree cannot smuggle foreign bytes into the
/// digest.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if the tree cannot be walked, and
/// [`ReasonCode::InvalidTarget`] if `root` is not a directory.
pub fn of_tree(root: &Path) -> Result<String> {
    of_tree_excluding(root, &[])
}

/// Hash a directory tree, skipping the named top-level entries.
///
/// Provider-owned bookkeeping is excluded this way rather than deleted, so the
/// digest of a target never depends on the journal that describes it.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if the tree cannot be walked, and
/// [`ReasonCode::InvalidTarget`] if `root` is not a directory.
pub fn of_tree_excluding(root: &Path, excluded_top_level: &[&str]) -> Result<String> {
    if !root.is_dir() {
        return Err(Error::new(
            ReasonCode::InvalidTarget,
            format!("{} is not a directory", root.display()),
        ));
    }
    let mut entries = Vec::new();
    collect(root, root, excluded_top_level, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative, kind, payload) in entries {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(kind.as_bytes());
        hasher.update([0]);
        hasher.update(payload.as_bytes());
        hasher.update([0]);
    }
    Ok(format!("{PREFIX}{:x}", hasher.finalize()))
}

fn collect(
    root: &Path,
    current: &Path,
    excluded_top_level: &[&str],
    out: &mut Vec<(String, String, String)>,
) -> Result<()> {
    let read = fs::read_dir(current).map_err(|source| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("cannot list {}", current.display()),
        )
        .with_source(source)
    })?;
    for entry in read {
        let entry = entry.map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot read an entry of {}", current.display()),
            )
            .with_source(source)
        })?;
        let path = entry.path();
        let relative = relative_slash_path(root, &path)?;
        if current == root && excluded_top_level.contains(&relative.as_str()) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot stat {}", path.display()),
            )
            .with_source(source)
        })?;

        if metadata.is_symlink() {
            let destination = fs::read_link(&path).map_err(|source| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot read link {}", path.display()),
                )
                .with_source(source)
            })?;
            out.push((
                relative,
                "link".to_owned(),
                destination.to_string_lossy().into_owned(),
            ));
        } else if metadata.is_dir() {
            out.push((relative, "dir".to_owned(), String::new()));
            collect(root, &path, excluded_top_level, out)?;
        } else {
            let content = of_file(&path)?;
            let executable = if is_executable(&metadata) { "x" } else { "-" };
            out.push((relative, format!("file:{executable}"), content));
        }
    }
    Ok(())
}

fn relative_slash_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path.strip_prefix(root).map_err(|source| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("{} is not inside {}", path.display(), root.display()),
        )
        .with_source(source)
    })?;
    let joined: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    Ok(joined.join("/"))
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    // Windows has no executable bit on a file. Reporting a constant keeps the
    // digest stable per platform; it never claims a mode the platform cannot
    // represent.
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let base = std::env::temp_dir().join(format!("setup-core-digest-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn bytes_are_tagged_with_the_algorithm() {
        assert!(of_bytes(b"x").starts_with(PREFIX));
    }

    #[test]
    fn a_tree_digest_ignores_mtime_but_notices_bytes() {
        let root = scratch("mtime");
        fs::write(root.join("a.txt"), "one").unwrap();
        let first = of_tree(&root).unwrap();

        // Rewriting identical bytes moves mtime and must not move the digest.
        fs::write(root.join("a.txt"), "one").unwrap();
        assert_eq!(of_tree(&root).unwrap(), first);

        fs::write(root.join("a.txt"), "two").unwrap();
        assert_ne!(of_tree(&root).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn a_tree_digest_notices_mode_drift_that_content_cannot_show() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("mode");
        let file = root.join("run.sh");
        fs::write(&file, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        let plain = of_tree(&root).unwrap();

        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        assert_ne!(of_tree(&root).unwrap(), plain);
    }

    #[test]
    fn a_missing_root_is_an_invalid_target_not_an_io_error() {
        let error = of_tree(std::path::Path::new("/definitely/not/here")).unwrap_err();
        assert_eq!(error.reason(), ReasonCode::InvalidTarget);
    }
}
