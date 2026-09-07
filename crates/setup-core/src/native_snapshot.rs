//! Complete local native configuration snapshots, separate from write ownership.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::{Error, ReasonCode, Result};

/// Filesystem base of a declared preservation cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeBase {
    /// Every covered path is relative to the provider target.
    #[default]
    Target,
    /// A closed cover includes a documented companion beside the target.
    Parent,
}

impl NativeBase {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip callback takes a reference"
    )]
    fn is_target(&self) -> bool {
        *self == Self::Target
    }
}

/// The complete configuration surface and its measured members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeSnapshot {
    /// Base for the declared relative roots, included in the snapshot digest.
    #[serde(default, skip_serializing_if = "NativeBase::is_target")]
    pub base_root: NativeBase,
    /// Target-relative native namespace roots, reduced to a non-overlapping cover.
    pub roots: Vec<String>,
    /// Product-owned paths excluded from capture and replacement.
    pub excluded: Vec<String>,
    /// Exact relative member paths and metadata; no raw configuration values.
    pub entries: BTreeMap<String, Member>,
}

/// A regular file or directory captured without following links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// File content digest; absent for a directory.
    pub digest: Option<String>,
    /// Unix permission bits, or the read-only flag on other platforms.
    pub permissions: u32,
}

fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::new(
        ReasonCode::StateUnavailable,
        format!("cannot access native snapshot member {}", path.display()),
    )
    .with_source(source)
}

fn within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(Error::new(
            ReasonCode::IntegrityMismatch,
            "invalid native snapshot relative path",
        ));
    }
    Ok(())
}

fn permissions(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

fn set_permissions(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    let value = {
        use std::os::unix::fs::PermissionsExt;
        fs::Permissions::from_mode(mode)
    };
    #[cfg(not(unix))]
    let value = {
        let mut value = fs::metadata(path)
            .map_err(|error| io_error(path, error))?
            .permissions();
        value.set_readonly(mode != 0);
        value
    };
    fs::set_permissions(path, value).map_err(|error| io_error(path, error))
}

impl NativeSnapshot {
    /// Measure every member of a native surface without following filesystem links.
    ///
    /// # Errors
    /// Refuses unsupported entries, unreadable members and invalid relative paths.
    pub fn inspect(root: &Path, roots: &[&str], excluded: &[&str]) -> Result<Self> {
        let mut roots: Vec<String> = roots.iter().map(|path| (*path).to_owned()).collect();
        let mut excluded: Vec<String> = excluded.iter().map(|path| (*path).to_owned()).collect();
        for path in roots.iter().chain(&excluded) {
            validate_path(path)?;
        }
        roots.sort();
        roots.dedup();
        excluded.sort();
        excluded.dedup();
        let cover = roots
            .iter()
            .filter(|path| {
                !roots
                    .iter()
                    .any(|ancestor| ancestor != *path && within(path, ancestor))
                    && !excluded.iter().any(|skip| within(path, skip))
            })
            .cloned()
            .collect();
        let mut snapshot = Self {
            base_root: NativeBase::Target,
            roots: cover,
            excluded,
            entries: BTreeMap::new(),
        };
        for relative in snapshot.roots.clone() {
            // A nested declaration must not cross a symlink in a transport parent.
            let mut parent = root.to_path_buf();
            for component in Path::new(&relative).components() {
                parent.push(component);
                match fs::symlink_metadata(&parent) {
                    Ok(meta) if meta.is_symlink() => {
                        return Err(Error::new(
                            ReasonCode::IntegrityMismatch,
                            "native snapshot refuses symbolic links",
                        ));
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => return Err(io_error(&parent, error)),
                }
            }
            snapshot.walk(root, &relative)?;
        }
        Ok(snapshot)
    }

    fn walk(&mut self, root: &Path, relative: &str) -> Result<()> {
        if self.excluded.iter().any(|skip| within(relative, skip)) {
            return Ok(());
        }
        let path = root.join(relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(io_error(&path, error)),
        };
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!("native snapshot refuses unsupported entry {relative}"),
            ));
        }
        // Transport directories of excluded runtime state cannot be removed or
        // have their mode rewound. Their covered children still belong to the
        // snapshot, but creation of excluded state must not create native drift.
        if !metadata.is_dir() || !self.excluded.iter().any(|skip| within(skip, relative)) {
            self.entries.insert(
                relative.to_owned(),
                Member {
                    digest: if metadata.is_file() {
                        Some(crate::digest::of_file(&path)?)
                    } else {
                        None
                    },
                    permissions: permissions(&metadata),
                },
            );
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|error| io_error(&path, error))? {
                let entry = entry.map_err(|error| io_error(&path, error))?;
                let name = entry.file_name().into_string().map_err(|_| {
                    Error::new(
                        ReasonCode::IntegrityMismatch,
                        "native snapshot member name is not UTF-8",
                    )
                })?;
                validate_path(&name)?;
                self.walk(root, &format!("{relative}/{name}"))?;
            }
        }
        Ok(())
    }

    /// A digest of coverage, member bytes and permission metadata.
    ///
    /// # Errors
    /// Propagates canonical serialization failures.
    pub fn digest(&self) -> Result<String> {
        let value = serde_json::to_value(self).map_err(|error| {
            Error::new(
                ReasonCode::IntegrityMismatch,
                "cannot encode native snapshot",
            )
            .with_source(error)
        })?;
        crate::digest::of_domain_canonical_json("nddev.native-snapshot.v1", &value)
    }

    /// Remeasure the whole surface and require an exact match.
    ///
    /// # Errors
    /// Refuses missing, extra, changed, unreadable or unsupported members.
    pub fn verify(&self, root: &Path) -> Result<()> {
        let mut measured = Self::inspect(
            root,
            &self.roots.iter().map(String::as_str).collect::<Vec<_>>(),
            &self.excluded.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        measured.base_root = self.base_root;
        if measured != *self {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                "native snapshot inventory does not match",
            ));
        }
        Ok(())
    }

    /// Copy measured members and preserve supported file and directory permissions.
    ///
    /// # Errors
    /// Refuses changed source state and failed copies or readback verification.
    pub fn copy_to(&self, source: &Path, destination: &Path) -> Result<()> {
        self.verify(source)?;
        for (relative, member) in &self.entries {
            let from = source.join(relative);
            let to = destination.join(relative);
            if member.digest.is_none() {
                fs::create_dir_all(&to).map_err(|error| io_error(&to, error))?;
            } else {
                if let Some(parent) = to.parent() {
                    fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
                }
                fs::copy(&from, &to).map_err(|error| io_error(&to, error))?;
                set_permissions(&to, member.permissions)?;
            }
        }
        // Restrictive directory modes apply after their children are copied.
        for (relative, member) in self.entries.iter().rev() {
            if member.digest.is_none() {
                set_permissions(&destination.join(relative), member.permissions)?;
            }
        }
        self.verify(source)?;
        self.verify(destination)
    }

    /// Replace the covered surface after a caller has preserved its current state.
    ///
    /// # Errors
    /// Refuses invalid recovery bytes before deletion and verifies the final state.
    pub fn restore(&self, payload: &Path, target: &Path) -> Result<()> {
        self.verify(payload)?;
        let current = Self::inspect(
            target,
            &self.roots.iter().map(String::as_str).collect::<Vec<_>>(),
            &self.excluded.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        for (relative, member) in current.entries.iter().rev() {
            let path = target.join(relative);
            if member.digest.is_none() {
                match fs::remove_dir(&path) {
                    Ok(()) => (),
                    Err(error)
                        if error.kind() == std::io::ErrorKind::DirectoryNotEmpty
                            && self.excluded.iter().any(|skip| within(skip, relative)) => {}
                    Err(error) => return Err(io_error(&path, error)),
                }
            } else {
                fs::remove_file(&path).map_err(|error| io_error(&path, error))?;
            }
        }
        self.copy_to(payload, target)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("native-snapshot-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn restore_returns_complete_state_and_preserves_the_state_it_replaces() {
        let root = scratch("return");
        let target = root.join("target");
        let original = root.join("original");
        let edited = root.join("edited");
        fs::create_dir_all(target.join("skills/empty")).unwrap();
        fs::write(target.join("skills/run.py"), b"print('original')\n").unwrap();
        fs::write(target.join("config.json"), b"{\"x\":1}\n").unwrap();
        fs::write(target.join("project.txt"), b"outside").unwrap();
        let baseline = NativeSnapshot::inspect(&target, &["skills", "config.json"], &[]).unwrap();
        fs::create_dir_all(&original).unwrap();
        baseline.copy_to(&target, &original).unwrap();
        fs::write(target.join("config.json"), b"changed").unwrap();
        fs::write(target.join("skills/user-added.sh"), b"echo user\n").unwrap();
        fs::remove_dir(target.join("skills/empty")).unwrap();
        let current = NativeSnapshot::inspect(&target, &["skills", "config.json"], &[]).unwrap();
        fs::create_dir_all(&edited).unwrap();
        current.copy_to(&target, &edited).unwrap();
        baseline.restore(&original, &target).unwrap();
        baseline.verify(&target).unwrap();
        assert!(target.join("skills/empty").is_dir());
        assert!(!target.join("skills/user-added.sh").exists());
        assert_eq!(fs::read(target.join("project.txt")).unwrap(), b"outside");
        current.restore(&edited, &target).unwrap();
        current.verify(&target).unwrap();
        fs::write(target.join("skills/unexpected"), b"extra").unwrap();
        assert!(current.verify(&target).is_err());
    }

    #[test]
    fn runtime_creation_does_not_change_an_empty_native_surface() {
        let root = scratch("runtime-parent");
        let target = root.join("target");
        let payload = root.join("payload");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&payload).unwrap();
        let empty = NativeSnapshot::inspect(&target, &["plugins"], &["plugins/data"]).unwrap();
        empty.copy_to(&target, &payload).unwrap();
        fs::create_dir_all(target.join("plugins/data")).unwrap();
        fs::write(target.join("plugins/data/session"), b"current runtime").unwrap();
        empty.verify(&target).unwrap();
        fs::create_dir_all(target.join("plugins/cache/new")).unwrap();
        fs::write(target.join("plugins/cache/new/code.js"), b"new config").unwrap();
        assert!(empty.verify(&target).is_err());
        empty.restore(&payload, &target).unwrap();
        empty.verify(&target).unwrap();
        assert_eq!(
            fs::read(target.join("plugins/data/session")).unwrap(),
            b"current runtime"
        );
        assert!(!target.join("plugins/cache").exists());
    }

    #[test]
    fn empty_original_removes_later_native_configuration() {
        let root = scratch("empty");
        let target = root.join("target");
        let payload = root.join("payload");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&payload).unwrap();
        let original = NativeSnapshot::inspect(&target, &["skills", "config.json"], &[]).unwrap();
        original.copy_to(&target, &payload).unwrap();
        fs::create_dir_all(target.join("skills/new")).unwrap();
        fs::write(target.join("config.json"), b"new").unwrap();
        original.restore(&payload, &target).unwrap();
        original.verify(&target).unwrap();
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    #[test]
    fn corrupt_payload_is_refused_before_target_changes() {
        let root = scratch("corrupt");
        let target = root.join("target");
        let payload = root.join("payload");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&payload).unwrap();
        fs::write(target.join("config.json"), b"original").unwrap();
        let snapshot = NativeSnapshot::inspect(&target, &["config.json"], &[]).unwrap();
        snapshot.copy_to(&target, &payload).unwrap();
        fs::write(payload.join("config.json"), b"corrupt").unwrap();
        fs::write(target.join("config.json"), b"user edit").unwrap();
        assert!(snapshot.restore(&payload, &target).is_err());
        assert_eq!(fs::read(target.join("config.json")).unwrap(), b"user edit");
    }

    #[test]
    fn never_touch_files_are_neither_copied_nor_replaced() {
        let root = scratch("excluded");
        let target = root.join("target");
        let payload = root.join("payload");
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::create_dir_all(&payload).unwrap();
        fs::write(target.join("skills/session.json"), b"product-owned").unwrap();
        fs::write(target.join("skills/a.md"), b"component").unwrap();
        let snapshot =
            NativeSnapshot::inspect(&target, &["skills"], &["skills/session.json"]).unwrap();
        snapshot.copy_to(&target, &payload).unwrap();
        assert!(!payload.join("skills/session.json").exists());
        fs::write(target.join("skills/session.json"), b"updated by product").unwrap();
        fs::write(target.join("skills/new.md"), b"new").unwrap();
        snapshot.restore(&payload, &target).unwrap();
        assert_eq!(
            fs::read(target.join("skills/session.json")).unwrap(),
            b"updated by product"
        );
        assert!(!target.join("skills/new.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_restored_and_part_of_preconditions() {
        let root = scratch("permissions");
        let target = root.join("target");
        let payload = root.join("payload");
        fs::create_dir_all(target.join("skills/empty")).unwrap();
        fs::create_dir_all(&payload).unwrap();
        fs::write(target.join("skills/tool"), b"tool").unwrap();
        set_permissions(&target.join("skills/tool"), 0o750).unwrap();
        set_permissions(&target.join("skills/empty"), 0o700).unwrap();
        let snapshot = NativeSnapshot::inspect(&target, &["skills"], &[]).unwrap();
        snapshot.copy_to(&target, &payload).unwrap();
        set_permissions(&target.join("skills/tool"), 0o700).unwrap();
        assert!(snapshot.verify(&target).is_err());
        snapshot.restore(&payload, &target).unwrap();
        snapshot.verify(&target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_and_linked_transport_parents_are_refused() {
        let root = scratch("links");
        fs::create_dir_all(root.join("outside")).unwrap();
        fs::write(root.join("outside/tool"), b"foreign").unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("skills")).unwrap();
        assert!(NativeSnapshot::inspect(&root, &["skills"], &[]).is_err());
        assert!(NativeSnapshot::inspect(&root, &["skills/tool"], &[]).is_err());
        assert!(NativeSnapshot::inspect(&root, &["../outside"], &[]).is_err());
    }
}
