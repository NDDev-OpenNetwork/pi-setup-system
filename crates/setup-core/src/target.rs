//! The canonical target directory, and the control directory inside it.
//!
//! A target is never inferred. The caller names an absolute directory and this
//! module decides whether it is usable; it never falls back to a home directory
//! or the current working directory, because a mutation aimed at a guessed path
//! is a mutation aimed at someone else's state.

use std::fs;
use std::path::{Path, PathBuf};

use crate::digest;
use crate::error::{Error, ReasonCode, Result};

/// A validated, canonicalized target directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    root: PathBuf,
    control_directory_name: String,
}

impl Target {
    /// Validate and canonicalize `path` as a target root.
    ///
    /// The path must be absolute, must exist, must be a directory, and its
    /// final component must not be a symbolic link. The last rule matters:
    /// following a link at the final component would let a link swapped between
    /// validation and write redirect every managed file elsewhere.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::InvalidTarget`] when any of those conditions fails.
    pub fn resolve(path: &Path, control_directory_name: &str) -> Result<Self> {
        if !path.is_absolute() {
            return Err(Error::new(
                ReasonCode::InvalidTarget,
                format!("target must be absolute, got {}", path.display()),
            ));
        }
        let metadata = fs::symlink_metadata(path).map_err(|source| {
            Error::new(
                ReasonCode::InvalidTarget,
                format!("target {} cannot be inspected", path.display()),
            )
            .with_source(source)
        })?;
        if metadata.is_symlink() {
            return Err(Error::new(
                ReasonCode::InvalidTarget,
                format!("target {} is a symbolic link", path.display()),
            ));
        }
        if !metadata.is_dir() {
            return Err(Error::new(
                ReasonCode::InvalidTarget,
                format!("target {} is not a directory", path.display()),
            ));
        }
        let root = fs::canonicalize(path).map_err(|source| {
            Error::new(
                ReasonCode::InvalidTarget,
                format!("target {} cannot be canonicalized", path.display()),
            )
            .with_source(source)
        })?;
        Ok(Self {
            root,
            control_directory_name: control_directory_name.to_owned(),
        })
    }

    /// The canonical root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The provider-owned control directory inside the target.
    #[must_use]
    pub fn control_directory(&self) -> PathBuf {
        self.root.join(&self.control_directory_name)
    }

    /// Create the control directory if it is absent.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the directory cannot be
    /// created, and [`ReasonCode::InvalidTarget`] if the path exists as
    /// something other than a directory.
    pub fn ensure_control_directory(&self) -> Result<PathBuf> {
        let control = self.control_directory();
        match fs::symlink_metadata(&control) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => Ok(control),
            Ok(_) => Err(Error::new(
                ReasonCode::InvalidTarget,
                format!("{} exists but is not a plain directory", control.display()),
            )),
            Err(_) => {
                fs::create_dir_all(&control).map_err(|source| {
                    Error::new(
                        ReasonCode::StateUnavailable,
                        format!("cannot create {}", control.display()),
                    )
                    .with_source(source)
                })?;
                restrict_to_owner(&control)?;
                Ok(control)
            }
        }
    }

    /// The digest of everything under the target except the control directory.
    ///
    /// Provider-owned bookkeeping must not appear in the identity of the state
    /// it describes, or writing the journal would itself change the digest the
    /// journal was taken against.
    ///
    /// # Errors
    ///
    /// Propagates the refusal from the tree walk.
    pub fn identity_digest(&self) -> Result<String> {
        self.identity_digest_excluding(&[])
    }

    /// The identity digest, also skipping the named top-level entries.
    ///
    /// Two kinds of entry belong here beyond the control directory.
    ///
    /// The provider's own **state file** sits in the target root, so counting it
    /// would make every applied operation leave the target different from the
    /// identity that operation just recorded — the target would read as drifted
    /// the instant it became clean.
    ///
    /// Paths the provider **never touches** belong here too. A product rewrites
    /// its own credentials and session history constantly; letting that traffic
    /// move the identity would make a plan go stale between planning and
    /// applying for a change no effect of ours would have overwritten.
    ///
    /// # Errors
    ///
    /// Propagates the refusal from the tree walk.
    pub fn identity_digest_excluding(&self, also_excluded: &[&str]) -> Result<String> {
        let mut excluded = vec![self.control_directory_name.as_str()];
        excluded.extend_from_slice(also_excluded);
        digest::of_tree_excluding(&self.root, &excluded)
    }

    /// The identity of this target: the digest of what the provider owns.
    ///
    /// The declaration is the authority. Backup, restore, remove and
    /// materialization were always scoped to `native_namespaces`; identity was
    /// the one reading that was not, and it disagreed loudly enough to time out
    /// a real Windows target and quietly enough to strand a plan on a
    /// neighbour's file everywhere else. See [`digest::of_owned`].
    ///
    /// # Errors
    ///
    /// Propagates the refusal from the owned-projection walk.
    pub fn identity_of_owned(&self, namespaces: &[&str], excluded: &[&str]) -> Result<String> {
        let mut not_ours = vec![self.control_directory_name.as_str()];
        not_ours.extend_from_slice(excluded);
        digest::of_owned(&self.root, namespaces, &not_ours)
    }
}

/// Tighten a directory to owner-only access where the platform supports it.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if the permissions cannot be set.
pub fn restrict_to_owner(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("cannot restrict {}", path.display()),
            )
            .with_source(source)
        })?;
    }
    #[cfg(not(unix))]
    {
        // Windows access is governed by ACLs, which this kernel does not
        // rewrite. The directory inherits the parent's ACL, and claiming an
        // owner-only mode here would report a restriction that was never made.
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("setup-core-target-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::canonicalize(&base).unwrap()
    }

    #[test]
    fn a_relative_target_is_refused() {
        let error = Target::resolve(Path::new("relative/path"), ".ctl").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::InvalidTarget);
    }

    #[test]
    fn a_missing_target_is_refused() {
        let error = Target::resolve(Path::new("/definitely/not/here"), ".ctl").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::InvalidTarget);
    }

    #[test]
    fn a_file_is_not_a_target() {
        let base = scratch("file");
        let file = base.join("f");
        fs::write(&file, "x").unwrap();
        let error = Target::resolve(&file, ".ctl").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::InvalidTarget);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_final_component_is_refused_before_it_can_be_swapped() {
        let base = scratch("link");
        let real = base.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let error = Target::resolve(&link, ".ctl").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::InvalidTarget);
    }

    #[test]
    fn the_control_directory_is_created_inside_the_root() {
        let base = scratch("control");
        let target = Target::resolve(&base, ".ctl").unwrap();
        let control = target.ensure_control_directory().unwrap();
        assert_eq!(control, base.join(".ctl"));
        assert!(control.is_dir());
    }

    #[test]
    fn identity_can_also_ignore_the_state_file_it_describes() {
        // Counting it would make an applied operation leave the target different
        // from the identity it just recorded.
        let base = scratch("identity-state");
        fs::write(base.join("kept.txt"), "value").unwrap();
        let target = Target::resolve(&base, ".ctl").unwrap();
        let before = target.identity_digest_excluding(&["STATE.json"]).unwrap();

        fs::write(base.join("STATE.json"), "{\"state_schema\":3}").unwrap();
        assert_eq!(
            target.identity_digest_excluding(&["STATE.json"]).unwrap(),
            before
        );
        assert_ne!(target.identity_digest().unwrap(), before);
    }

    #[test]
    fn identity_ignores_the_control_directory_it_is_measured_beside() {
        let base = scratch("identity");
        fs::write(base.join("kept.txt"), "value").unwrap();
        let target = Target::resolve(&base, ".ctl").unwrap();
        let before = target.identity_digest().unwrap();

        target.ensure_control_directory().unwrap();
        fs::write(base.join(".ctl").join("journal.json"), "{}").unwrap();
        assert_eq!(target.identity_digest().unwrap(), before);
    }
}
