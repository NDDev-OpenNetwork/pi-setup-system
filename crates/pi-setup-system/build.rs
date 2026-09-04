//! Compile this harness's setup catalog into the binary.
//!
//! Until this existed, the catalog was only ever found on disk — beside the
//! executable, or up from it, or in the working directory. That is fine in a
//! checkout and useless everywhere else: the release ships six binaries and a
//! `SHA256SUMS`, `install.sh` places one file, and `setups/` lives only in the
//! git tree. So the first two commands the published README gives — `list` and
//! `install <setup> --target <dir>` — refused for every user who installed the
//! documented way, on all three operating systems, for four releases.
//!
//! Embedding is the floor, not the ceiling. `<PROVIDER>_SETUP_CATALOG` and the
//! on-disk search still win when they find something, because the owner's own
//! setups are as legitimate a source as the ones we ship.
//!
//! This file is identical in all seven crates and is rendered into the public
//! trees verbatim; the only thing that varies is the directory it finds, and it
//! works out which by looking.

// `unwrap_used`, `expect_used` and `panic` are denied across this workspace, and
// they are denied for the right reason: a *provider* that panics has abandoned a
// half-written target instead of refusing with a reason someone can act on.
//
// A build script is not that program. It runs on a developer's machine before
// any target exists, and stopping the build with a message naming the offending
// file is precisely the refusal this file is for -- there is no caller to return
// a `Result` to, and a build script that swallowed an error would emit a binary
// silently missing the catalog it is named after. That is the defect this whole
// file exists to prevent, so it must not be reachable by ignoring an error here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "a build script's panic is a compile-time refusal, not a runtime abort; \
              see the note above"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let catalog = locate(&manifest);

    println!("cargo:rerun-if-changed={}", catalog.display());
    println!("cargo:rerun-if-changed=build.rs");

    // Only the setup directories, which is exactly what the reader lists: a
    // directory holding a `setup.json`. A rendered public tree also carries a
    // `setups/README.md` at the catalog root, and embedding it would put a file
    // in the binary that no setup contains and that the workspace build does
    // not have — two builds of one program carrying different tables, for a
    // file neither of them reads.
    let mut files = BTreeMap::new();
    let mut roots: Vec<PathBuf> = fs::read_dir(&catalog)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", catalog.display()))
        .map(|entry| entry.expect("cannot read a catalog entry").path())
        .filter(|path| path.join("setup.json").is_file())
        .collect();
    roots.sort();
    for setup in &roots {
        collect(&catalog, setup, &mut files);
    }
    assert!(
        !files.is_empty(),
        "{} holds no setup: every directory in it lacks a setup.json, so this \
         binary would ship unable to answer `list`",
        catalog.display()
    );

    let mut source = String::from("&[\n");
    for (relative, absolute) in &files {
        // The path is absolute and produced here, so it resolves the same way
        // from `OUT_DIR` as it does from the crate root.
        writeln!(
            source,
            "    ({:?}, include_bytes!({:?})),",
            relative,
            absolute.display().to_string()
        )
        .expect("writing to a String cannot fail");
    }
    source.push(']');

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo always sets OUT_DIR"));
    fs::write(out.join("embedded_setups.rs"), source).expect("cannot write the embedded catalog");
}

/// Find the catalog, in whichever of the two layouts this crate is standing in.
///
/// A workspace with one directory per harness uses `setups/<tool>/`;
/// a tree that ships exactly one harness holds it flat (`setups/`).
/// Both sit two levels above the crate, so the only question is
/// whether the harness-scoped directory exists.
fn locate(manifest: &Path) -> PathBuf {
    let tool = env!("CARGO_PKG_NAME")
        .strip_suffix("-setup-system")
        .expect("every crate using this script is named <tool>-setup-system");
    let root = manifest.join("..").join("..").join("setups");

    let scoped = root.join(tool);
    let chosen = if scoped.is_dir() { scoped } else { root };

    // A missing catalog is not a warning to be scrolled past. Every one of
    // these binaries is named after the setups it carries; one built without
    // them would refuse the first command it is documented with, which is the
    // exact defect this file exists to close.
    assert!(
        chosen.is_dir(),
        "no setup catalog at {} — this binary would ship unable to answer `list`",
        chosen.display()
    );
    chosen
}

/// Walk the catalog into relative-slash-path → absolute-path pairs.
///
/// Refusals here are deliberate and are the reason this is a walk rather than a
/// glob. The digest that identifies a setup records a link's destination and a
/// file's executable bit, and neither survives being embedded as bytes — so a
/// catalog holding either would give one identity on disk and a different one
/// in the binary, for the same setup. Rather than let the two disagree
/// silently, the build stops and names the file. Today no setup holds one; the
/// day one does, this is what says so.
fn collect(root: &Path, current: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(current)
        .unwrap_or_else(|error| panic!("cannot list {}: {error}", current.display()))
        .map(|entry| entry.expect("cannot read a catalog entry").path())
        .collect();
    entries.sort();

    for path in entries {
        let metadata = fs::symlink_metadata(&path)
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));

        assert!(
            !metadata.is_symlink(),
            "{} is a symbolic link; a setup's digest records where a link points, \
             and an embedded copy cannot carry that. Replace it with its content.",
            path.display()
        );

        if metadata.is_dir() {
            collect(root, &path, out);
            continue;
        }

        assert!(
            !is_executable(&metadata),
            "{} is executable; a setup's digest records the executable bit on Unix, \
             and an embedded copy cannot carry it. Remove the bit, or teach this \
             script and the materializer to round-trip it together.",
            path.display()
        );

        let relative = path
            .strip_prefix(root)
            .expect("collect only ever descends into root")
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        out.insert(relative, path);
    }
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    // Windows has no executable bit, so there is nothing here that could
    // disagree with an embedded copy. The Unix arm is where the risk lives.
    false
}
