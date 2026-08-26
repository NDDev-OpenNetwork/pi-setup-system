//! The local setup catalog: complete harness states, on disk, beside the binary.
//!
//! A setup here is the whole thing — the system-prompt components and the
//! configuration together, as a verbatim tree. That is what makes selecting one
//! and restoring one mean the same kind of thing: both put a known complete
//! state into the target, rather than adjusting part of it and leaving the rest
//! wherever the last change left it.
//!
//! ```text
//! setups/
//!   <setup-id>/
//!     setup.json    identity and description
//!     home/         copied verbatim into the target
//! ```
//!
//! This is one of the three sources the design admits. The other two — a setup
//! compiled by ai-stp, and a set of ai-stp components — arrive as a
//! `HarnessBundle` over the wire. All three converge on the same immutable
//! definition before any plan is made, so nothing downstream needs to know which
//! one produced it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use setup_core::digest;

use crate::facts::Harness;
use provider_v3::{Error, Result, WireReason};

/// The catalog directory name beside the executable or repository root.
pub const CATALOG_DIRECTORY: &str = "setups";

/// The per-setup manifest file.
pub const SETUP_MANIFEST: &str = "setup.json";

/// The subdirectory copied verbatim into a target.
pub const SETUP_PAYLOAD: &str = "home";

/// The schema this build writes and is willing to read.
pub const SETUP_SCHEMA: u32 = 1;

/// What one setup says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupManifest {
    /// Schema of this manifest.
    pub schema_version: u32,
    /// The setup identity, matching its directory name.
    pub id: String,
    /// One line on what this setup is for.
    pub description: String,
}

/// One setup in the catalog, with the digest that identifies its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// What the setup says about itself.
    pub manifest: SetupManifest,
    /// Where its verbatim tree lives.
    pub payload: PathBuf,
    /// The digest of that tree.
    ///
    /// Two setups with the same bytes have the same definition digest, whatever
    /// they are called — identity is content, not a name.
    pub definition_digest: String,
    /// How many files the tree holds.
    pub file_count: u64,
    /// Keeps a materialized catalog alive for as long as `payload` names it.
    ///
    /// A `Setup` outlives the `Catalog` it came from — `get` returns one and the
    /// catalog is dropped — and for an embedded catalog the drop deletes the
    /// directory `payload` points into. `list` did not notice, because it reads
    /// everything before returning; `install` failed with *cannot list
    /// …/baseline/home*, having been handed a path to bytes that no longer
    /// existed.
    lifeline: Lifeline,
}

/// A handle that keeps a materialized catalog on disk, and is never identity.
///
/// Two setups with the same bytes are the same setup — that is the whole claim
/// `definition_digest` makes — so where those bytes were written cannot
/// participate in equality. Comparing always-equal is not a shortcut here; it is
/// the statement that provenance is not identity.
#[derive(Debug, Clone, Default)]
struct Lifeline(
    #[expect(
        dead_code,
        reason = "held for its Drop: this is the handle that keeps a materialized \
                  catalog on disk, and dead-code analysis does not count a \
                  destructor as a use"
    )]
    Option<Arc<Materialized>>,
);

impl PartialEq for Lifeline {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Lifeline {}

/// A compiled-in catalog written to a directory this process owns.
///
/// Removed when the last handle goes. A failure to remove it is deliberately
/// silent: the bytes are a copy of something already inside the binary, the
/// directory is inside the system's temporary space, and refusing a command
/// that has already succeeded because a cleanup did not would be the tail
/// wagging the dog.
#[derive(Debug)]
struct Materialized {
    root: PathBuf,
}

impl Materialized {
    /// Write every embedded file under a fresh directory.
    fn write(harness: &Harness) -> Option<Self> {
        // Unique without a dependency: the process cannot collide with itself,
        // and the counter separates two catalogs opened in one process — which
        // the tests do, in threads.
        //
        // The name is kept short on purpose, and the reason is Windows. A
        // classic `MAX_PATH` is 260, and the longest path here is
        // `<temp>/<this directory>/<deepest file in any setup>`. Measured
        // 2026-08-26: the deepest relative path any setup holds is 98 bytes
        // (cursor's `nddev-builder/home/plugins/…/installation-lifecycle.md`)
        // and a normal Windows temp root is around 42, which leaves this name
        // as the only part anyone here controls. `<provider_id>-<pid>-<n>` is
        // 46 for the longest provider id, for a worst case near 186 — enough
        // headroom for a long user name, which `…-setups-…` was eating for a
        // word that says nothing the provider id does not.
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            harness.provider_id,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A leftover from a crashed run of the same pid must not be read as
        // this one's catalog.
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).ok()?;

        let held = Self { root };
        for (relative, bytes) in harness.embedded_setups {
            let path = held.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok()?;
            }
            fs::write(&path, bytes).ok()?;
        }
        Some(held)
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Materialized {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The catalog this build ships.
#[derive(Debug, Clone)]
pub struct Catalog {
    root: PathBuf,
    /// Kept alive so a materialized catalog outlives every reader of it.
    ///
    /// Shared rather than owned because `Catalog` is cloned, and two owners of
    /// one temporary directory would delete it twice — the second time out from
    /// under whoever still held the first. Handed to every [`Setup`] this
    /// catalog produces, so the bytes outlive the catalog that found them.
    lifeline: Lifeline,
}

impl Catalog {
    /// Open the catalog at an explicit root.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lifeline: Lifeline::default(),
        }
    }

    /// Find the catalog this harness ships.
    ///
    /// An explicit `<PROVIDER_ID>_SETUP_CATALOG` wins, so a caller can point the
    /// binary at a catalog of their own without rebuilding it — the owner's own
    /// setups are as legitimate a source as the shipped ones.
    ///
    /// Otherwise `setups/` is looked for beside the executable and upward, then
    /// in the working directory. Each candidate is tried twice: once as the
    /// catalog itself, and once with the harness id beneath it. A published tree
    /// ships one harness and uses the first shape; the workspace that authors
    /// them all uses the second, and a developer should not have to know which
    /// one they are standing in.
    ///
    /// That second shape was a claim rather than a fact until the embedded
    /// catalog existed: it joins the *harness id*, and two harness ids are not
    /// their tool names — `claude-code` against `setups/claude`, `grok-build`
    /// against `setups/grok`. Two of the seven could not find their own catalog
    /// from the workspace root, and the comment above said they could.
    ///
    /// When nothing is found on disk, the catalog compiled into this binary is
    /// materialized and used. That is the case for every user who installed the
    /// documented way, because the release ships binaries and no `setups/`.
    #[must_use]
    pub fn discover(harness: &Harness) -> Option<Self> {
        Self::on_disk(harness).or_else(|| Self::embedded(harness))
    }

    /// The catalog someone put on a disk, if there is one.
    #[must_use]
    fn on_disk(harness: &Harness) -> Option<Self> {
        let variable = format!(
            "{}_SETUP_CATALOG",
            harness.provider_id.to_uppercase().replace('-', "_")
        );
        if let Ok(explicit) = std::env::var(&variable) {
            let path = PathBuf::from(explicit);
            return path.is_dir().then_some(Self::at(path));
        }

        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(executable) = std::env::current_exe()
            && let Some(directory) = executable.parent()
        {
            roots.push(directory.to_path_buf());
            let mut walk = directory;
            for _ in 0..3 {
                let Some(up) = walk.parent() else { break };
                roots.push(up.to_path_buf());
                walk = up;
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            roots.push(cwd);
        }

        roots
            .into_iter()
            .flat_map(|root| {
                let base = root.join(CATALOG_DIRECTORY);
                [base.join(harness.harness_id), base]
            })
            .find(|path| path.is_dir() && Self::holds_a_setup(path))
            .map(Self::at)
    }

    /// Write the compiled-in catalog somewhere real, and read it from there.
    ///
    /// Materializing rather than teaching every reader about a second kind of
    /// catalog is the whole point. A setup's identity is the digest of its
    /// tree, computed by walking a directory; a second in-memory implementation
    /// of that walk would be a second chance to disagree, and the two would
    /// disagree about exactly the thing that decides whether a target has
    /// drifted. Writing the bytes down means `list`, `get`, the digest and the
    /// copy are the same code they have always been, and the embedded catalog
    /// is provably the same setup as the on-disk one because it *is* one.
    ///
    /// The directory belongs to this process and is removed when the last
    /// handle to it goes.
    #[must_use]
    fn embedded(harness: &Harness) -> Option<Self> {
        if harness.embedded_setups.is_empty() {
            return None;
        }
        let root = Materialized::write(harness)?;
        let path = root.path().to_path_buf();
        Some(Self {
            root: path,
            lifeline: Lifeline(Some(Arc::new(root))),
        })
    }

    /// Whether a directory holds at least one readable setup manifest.
    ///
    /// Without this, the workspace's `setups/` — which holds one directory per
    /// harness and no manifests — would be chosen as an empty catalog and the
    /// harness-scoped directory beneath it never reached.
    fn holds_a_setup(path: &Path) -> bool {
        let Ok(read) = fs::read_dir(path) else {
            return false;
        };
        read.flatten()
            .any(|entry| entry.path().join(SETUP_MANIFEST).is_file())
    }

    /// The catalog root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every readable setup, by identity.
    ///
    /// A directory that does not parse is skipped rather than fatal: one broken
    /// setup should not make its neighbours unlistable.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] if the catalog cannot be
    /// listed at all.
    pub fn list(&self) -> Result<Vec<Setup>> {
        let read = match fs::read_dir(&self.root) {
            Ok(read) => read,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    format!(
                        "cannot list the setup catalog at {}: {source}",
                        self.root.display()
                    ),
                ));
            }
        };
        let mut setups = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Ok(setup) = read_setup(&path, &self.lifeline) {
                setups.push(setup);
            }
        }
        setups.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(setups)
    }

    /// One setup by identity.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] when no setup carries that
    /// identity, naming the ones that do.
    pub fn get(&self, id: &str) -> Result<Setup> {
        let available = self.list()?;
        available
            .iter()
            .find(|setup| setup.manifest.id == id)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = available
                    .iter()
                    .map(|setup| setup.manifest.id.as_str())
                    .collect();
                Error::refuse(
                    WireReason::ProviderUnavailable,
                    if names.is_empty() {
                        format!("{id:?} is not a setup; this build ships no catalog")
                    } else {
                        format!(
                            "{id:?} is not a setup; this build ships {}",
                            names.join(", ")
                        )
                    },
                )
            })
    }
}

/// Read one setup directory, or say why it is not one.
fn read_setup(directory: &Path, lifeline: &Lifeline) -> Result<Setup> {
    let manifest_path = directory.join(SETUP_MANIFEST);
    let bytes = fs::read(&manifest_path).map_err(|source| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("cannot read {}: {source}", manifest_path.display()),
        )
    })?;
    let manifest: SetupManifest = serde_json::from_slice(&bytes).map_err(|source| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} does not parse: {source}", manifest_path.display()),
        )
    })?;
    if manifest.schema_version != SETUP_SCHEMA {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is schema {} and this build writes {SETUP_SCHEMA}",
                manifest_path.display(),
                manifest.schema_version
            ),
        ));
    }
    // A setup whose directory and declared identity disagree would be
    // selectable by one name and recorded under another.
    let directory_name = directory.file_name().and_then(|name| name.to_str());
    if directory_name != Some(manifest.id.as_str()) {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} declares id {:?} but sits in {:?}",
                manifest_path.display(),
                manifest.id,
                directory_name.unwrap_or("?")
            ),
        ));
    }

    let payload = directory.join(SETUP_PAYLOAD);
    if !payload.is_dir() {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} has no {SETUP_PAYLOAD} tree", directory.display()),
        ));
    }
    Ok(Setup {
        definition_digest: digest::of_tree(&payload)?,
        file_count: count_files(&payload)?,
        manifest,
        payload,
        lifeline: lifeline.clone(),
    })
}

impl Setup {
    /// Every file this setup would write, as a path relative to the target.
    ///
    /// Files rather than top-level entries, because a harness may own a nested
    /// namespace and nothing else beside it: listing only the first component
    /// cannot tell `antigravity-cli/settings.json`, which is owned, from
    /// `antigravity-cli` as a whole, which is not.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] if the payload cannot be
    /// listed, or holds a name this provider cannot represent as a path.
    pub fn relative_paths(&self) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let mut stack = vec![self.payload.clone()];
        while let Some(current) = stack.pop() {
            let read = fs::read_dir(&current).map_err(|source| {
                Error::refuse(
                    WireReason::ProviderUnavailable,
                    format!("cannot list {}: {source}", current.display()),
                )
            })?;
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path.strip_prefix(&self.payload).map_err(|_| {
                    Error::refuse(
                        WireReason::ProviderUnavailable,
                        format!("{} escaped the setup payload", path.display()),
                    )
                })?;
                let Some(text) = relative.to_str() else {
                    return Err(Error::refuse(
                        WireReason::ProviderUnavailable,
                        format!("{} is not representable as a path", relative.display()),
                    ));
                };
                found.push(text.replace('\\', "/"));
            }
        }
        found.sort();
        Ok(found)
    }

    /// Check that every entry this setup writes is one the harness owns.
    ///
    /// A setup that wrote outside the declared namespaces would put files into a
    /// target that `remove` would then leave behind, and that `status` would not
    /// account for. Refusing here keeps ownership and effect the same set.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::UnsupportedNativeSurface`] naming the first entry
    /// outside the harness's declared surface.
    pub fn check_within(&self, harness: &Harness) -> Result<()> {
        for path in self.relative_paths()? {
            if !harness.owns(&path) {
                return Err(Error::refuse(
                    WireReason::UnsupportedNativeSurface,
                    format!(
                        "setup {:?} writes {path:?}, which is outside the surface {} owns",
                        self.manifest.id, harness.provider_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn count_files(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read = fs::read_dir(&current).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("cannot list {}: {source}", current.display()),
            )
        })?;
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                total = total.saturating_add(1);
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("harness-catalog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn write_setup(root: &Path, id: &str, files: &[(&str, &str)]) {
        let directory = root.join(id);
        fs::create_dir_all(directory.join(SETUP_PAYLOAD)).unwrap();
        fs::write(
            directory.join(SETUP_MANIFEST),
            serde_json::to_vec_pretty(&SetupManifest {
                schema_version: SETUP_SCHEMA,
                id: id.to_owned(),
                description: format!("the {id} setup"),
            })
            .unwrap(),
        )
        .unwrap();
        for (relative, content) in files {
            let path = directory.join(SETUP_PAYLOAD).join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn an_absent_catalog_lists_nothing_rather_than_failing() {
        let catalog = Catalog::at(scratch("absent").join("nowhere"));
        assert!(catalog.list().unwrap().is_empty());
    }

    #[test]
    fn setups_are_listed_by_identity_with_a_content_digest() {
        let root = scratch("list");
        write_setup(&root, "safe", &[("AGENTS.md", "# safe\n")]);
        write_setup(&root, "full-auto", &[("AGENTS.md", "# full\n")]);

        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|s| s.manifest.id.as_str())
                .collect::<Vec<_>>(),
            vec!["full-auto", "safe"]
        );
        assert!(listed[0].definition_digest.starts_with("sha256:"));
        assert_ne!(listed[0].definition_digest, listed[1].definition_digest);
        assert_eq!(listed[0].file_count, 1);
    }

    #[test]
    fn identity_is_content_so_two_names_over_the_same_bytes_agree() {
        let root = scratch("same-bytes");
        write_setup(&root, "one", &[("AGENTS.md", "identical\n")]);
        write_setup(&root, "two", &[("AGENTS.md", "identical\n")]);
        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(listed[0].definition_digest, listed[1].definition_digest);
    }

    #[test]
    fn a_setup_whose_directory_and_declared_id_disagree_is_not_listed() {
        // It would be selectable by one name and recorded under another.
        let root = scratch("mismatch");
        write_setup(&root, "safe", &[("AGENTS.md", "x")]);
        fs::rename(root.join("safe"), root.join("renamed")).unwrap();
        assert!(Catalog::at(&root).list().unwrap().is_empty());
    }

    #[test]
    fn one_broken_setup_does_not_make_the_others_unlistable() {
        let root = scratch("partly-broken");
        write_setup(&root, "good", &[("AGENTS.md", "x")]);
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken").join(SETUP_MANIFEST), "{ not json").unwrap();

        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].manifest.id, "good");
    }

    #[test]
    fn a_setup_with_no_payload_tree_is_refused() {
        let root = scratch("no-payload");
        write_setup(&root, "empty", &[]);
        fs::remove_dir_all(root.join("empty").join(SETUP_PAYLOAD)).unwrap();
        assert!(Catalog::at(&root).list().unwrap().is_empty());
    }

    #[test]
    fn asking_for_an_unknown_setup_names_the_ones_that_exist() {
        let root = scratch("unknown");
        write_setup(&root, "safe", &[("AGENTS.md", "x")]);
        let error = Catalog::at(&root).get("nope").unwrap_err();
        assert!(error.detail().contains("safe"), "{error}");
    }

    #[test]
    fn a_setup_writing_outside_the_declared_surface_is_refused() {
        let root = scratch("outside");
        write_setup(
            &root,
            "sneaky",
            &[("AGENTS.md", "x"), ("elsewhere.txt", "y")],
        );
        let setup = Catalog::at(&root).get("sneaky").unwrap();
        let harness = crate::wire::tests_support::TEST;
        let error = setup.check_within(&harness).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        assert!(error.detail().contains("elsewhere.txt"));
    }

    #[test]
    fn a_setup_inside_the_declared_surface_is_accepted() {
        let root = scratch("inside");
        write_setup(&root, "fine", &[("AGENTS.md", "x"), ("skills/a.md", "y")]);
        let setup = Catalog::at(&root).get("fine").unwrap();
        assert!(
            setup
                .check_within(&crate::wire::tests_support::TEST)
                .is_ok()
        );
        assert_eq!(setup.file_count, 2);
    }

    /// The same two files a real setup holds, as the build script would embed
    /// them: relative slash paths and bytes, nothing else.
    const EMBEDDED: &[(&str, &[u8])] = &[
        (
            "baseline/setup.json",
            br#"{"schema_version":1,"id":"baseline","description":"the baseline setup"}"#,
        ),
        ("baseline/home/AGENTS.md", b"# instructions\n"),
        ("baseline/home/skills/a.md", b"a skill\n"),
    ];

    fn harness_carrying_the_embedded_catalog() -> Harness {
        let mut harness = crate::wire::tests_support::TEST;
        harness.embedded_setups = EMBEDDED;
        harness
    }

    /// The defect this exists for: `get` returns a `Setup` and the `Catalog`
    /// that produced it is dropped on the next line. For an embedded catalog
    /// that drop deletes the directory `payload` points into, so the caller is
    /// handed a path to bytes that no longer exist.
    ///
    /// `list` never noticed, because it reads everything before returning. The
    /// first thing that did was a real `install` from a binary with no `setups/`
    /// beside it, which refused with *cannot list …/baseline/home*.
    #[test]
    fn a_setup_outlives_the_embedded_catalog_it_came_from() {
        let harness = harness_carrying_the_embedded_catalog();
        let setup = Catalog::embedded(&harness)
            .unwrap()
            .get("baseline")
            .unwrap();

        // Everything that found the setup is gone; only the setup is held.
        assert_eq!(
            fs::read_to_string(setup.payload.join("AGENTS.md")).unwrap(),
            "# instructions\n",
            "the bytes were deleted while a caller still held the path to them"
        );
        assert_eq!(setup.file_count, 2);
    }

    /// A setup is its content, so the same bytes must have the same identity
    /// whether they were shipped inside the binary or found on a disk. If these
    /// two ever disagree, one target configured from the release and another
    /// from a checkout would report different identities for the same setup, and
    /// every drift, restore and `setup_definition_digest` downstream inherits
    /// the disagreement.
    #[test]
    fn the_embedded_catalog_and_the_same_bytes_on_disk_are_one_setup() {
        let root = scratch("embedded-equals-disk");
        write_setup(
            &root,
            "baseline",
            &[
                ("AGENTS.md", "# instructions\n"),
                ("skills/a.md", "a skill\n"),
            ],
        );
        let on_disk = Catalog::at(&root).get("baseline").unwrap();

        let harness = harness_carrying_the_embedded_catalog();
        let embedded = Catalog::embedded(&harness)
            .unwrap()
            .get("baseline")
            .unwrap();

        assert_eq!(
            embedded.definition_digest, on_disk.definition_digest,
            "the binary and the tree disagree about what the baseline setup is"
        );
        assert_eq!(embedded.file_count, on_disk.file_count);
    }

    /// A harness that ships no catalog says so by finding nothing, rather than
    /// by producing an empty directory that reads as a catalog with no setups.
    /// The two are different answers: one is "this build has none", the other is
    /// "this build has a catalog and it is empty".
    #[test]
    fn a_build_carrying_no_embedded_catalog_finds_none() {
        assert!(Catalog::embedded(&crate::wire::tests_support::TEST).is_none());
    }

    /// The temporary directory belongs to the process, and two catalogs opened
    /// in one process must not be handed the same one — the second would delete
    /// the first's bytes when it cleared a stale directory before writing.
    #[test]
    fn two_embedded_catalogs_in_one_process_do_not_share_a_directory() {
        let harness = harness_carrying_the_embedded_catalog();
        let first = Catalog::embedded(&harness).unwrap();
        let second = Catalog::embedded(&harness).unwrap();
        assert_ne!(first.root(), second.root());
        assert!(first.get("baseline").is_ok());
        assert!(second.get("baseline").is_ok());
    }
}
