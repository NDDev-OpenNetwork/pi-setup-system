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

/// Render bytes as lowercase hexadecimal.
///
/// Written here rather than borrowed from a `LowerHex` implementation on the
/// hasher's output type: that type changed between `sha2` 0.10 and 0.11, and a
/// digest's spelling is part of this program's contract. It should not move
/// because a dependency reorganized its traits.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            // Writing into a String cannot fail; the Result exists for the trait.
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Hash bytes and return the prefixed hexadecimal digest.
#[must_use]
pub fn of_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{PREFIX}{}", hex(&hasher.finalize()))
}

/// Hash a JSON value through its RFC 8785 canonicalization.
///
/// # Errors
///
/// Propagates the refusal from [`canonical::to_canonical_bytes`].
pub fn of_canonical_json(value: &serde_json::Value) -> Result<String> {
    Ok(of_bytes(&canonical::to_canonical_bytes(value)?))
}

/// Hash bytes inside a named domain: `sha256(domain || 0x00 || payload)`.
///
/// Domain separation is what stops two object classes with identical bytes from
/// producing an interchangeable identifier. A plan and a projection profile that
/// happened to serialize the same way would otherwise share a digest, and a
/// consumer checking one against the other would find them equal.
#[must_use]
pub fn of_domain_bytes(domain: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{PREFIX}{}", hex(&hasher.finalize()))
}

/// Hash a JSON value inside a named domain through its RFC 8785 form.
///
/// # Errors
///
/// Propagates the refusal from [`canonical::to_canonical_bytes`].
pub fn of_domain_canonical_json(domain: &str, value: &serde_json::Value) -> Result<String> {
    Ok(of_domain_bytes(
        domain,
        &canonical::to_canonical_bytes(value)?,
    ))
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
    Ok(format!("{PREFIX}{}", hex(&hasher.finalize())))
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

/// Hash only the paths a provider declares it owns, and say when one is absent.
///
/// This is what a target's *identity* is, and for a long time it was not.
/// Identity used to be [`of_tree_excluding`] over a short denylist — everything
/// in the directory except this provider's bookkeeping and the product's
/// credentials — while backup, restore, remove and materialization were all
/// correctly scoped to the declared namespaces. One caller disagreeing with the
/// declaration is one caller too many, and it disagreed in two ways:
///
/// - it read bytes the provider does not own. Antigravity is a guest inside
///   `~/.gemini`; on a real Windows machine that is ~124,065 files and 20.2 GB,
///   `status` did not return within 150 seconds against a 120-second boundary,
///   and planning failed before an operation id existed;
/// - it let a neighbour strand a plan. A change under a sibling that no effect
///   of ours would ever have touched moved the identity, so a plan made against
///   the target before it went stale for a reason that had nothing to do with
///   the operation.
///
/// The second is the one that is wrong at any size, on any platform.
///
/// **An absent namespace contributes nothing, and that is deliberate.** The
/// consumer's report asks for an explicit absence marker; this does not emit
/// one, for two reasons found by writing it the other way first.
///
/// A marker is not needed to tell absence from emptiness — those already differ,
/// because an empty directory emits a `dir` entry and a missing one emits
/// nothing at all. And a marker breaks the strongest statement this design
/// makes: *a target holding nothing but one setup is that setup, byte for
/// byte*. A setup's definition digest is a whole-tree digest of its payload, and
/// `setup_definition_digest` sits beside `target_identity_digest` in provider
/// state precisely so the two can be compared. Markers for namespaces the setup
/// does not fill would make them differ for every setup that does not use every
/// namespace, which is most of them — a test caught exactly that.
///
/// It also means adding a namespace to a future declaration does not invalidate
/// targets that never used it.
///
/// Namespaces are slash-separated and may be nested (`config/skills`), which is
/// how a harness owns part of a directory another product also writes into.
///
/// # Errors
///
/// Returns [`ReasonCode::StateUnavailable`] if an owned path cannot be walked,
/// and [`ReasonCode::InvalidTarget`] if `root` is not a directory.
pub fn of_owned(root: &Path, namespaces: &[&str], excluded: &[&str]) -> Result<String> {
    if !root.is_dir() {
        return Err(Error::new(
            ReasonCode::InvalidTarget,
            format!("{} is not a directory", root.display()),
        ));
    }

    let mut owned: Vec<&str> = namespaces
        .iter()
        .copied()
        .filter(|name| !excluded.contains(name))
        .collect();
    owned.sort_unstable();
    owned.dedup();

    // Reduce the declaration to a cover before walking it. A namespace inside
    // another adds nothing to hash and would be hashed twice, which is why this
    // was once a refusal — but refusing it conflated two different lists that
    // happen to have the same name. `native_namespaces` is a *declaration*: a
    // consumer validates a compiler's route against it by exact membership, so
    // a provider may have to name both a directory and a place inside it during
    // the window where either could arrive. What this function needs is a
    // *cover* — the smallest set of roots whose walk visits each file once.
    //
    // Reducing is strictly better than refusing: identity no longer depends on
    // how the declaration is phrased, so naming a path already covered cannot
    // move a digest and cannot strand an installed target. Checked against the
    // whole set rather than the previous entry, because sorting does not put an
    // ancestor immediately before its descendants (`plugins` < `plugins-extra`
    // < `plugins/local`, and the ancestor of the third is the first).
    let cover: Vec<&str> = owned
        .iter()
        .copied()
        .filter(|name| {
            !owned.iter().any(|other| {
                other != name
                    && name
                        .strip_prefix(*other)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        })
        .collect();
    let owned = cover;

    let mut entries = Vec::new();
    for namespace in owned {
        let path = namespace
            .split('/')
            .fold(root.to_path_buf(), |at, part| at.join(part));
        match fs::symlink_metadata(&path) {
            // Nothing there is nothing to hash; see the note above on why this
            // does not leave a marker behind.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot stat {}", path.display()),
                )
                .with_source(source));
            }
            Ok(metadata) => {
                if describe(root, &path, &metadata, &mut entries)? && metadata.is_dir() {
                    collect(root, &path, excluded, &mut entries)?;
                }
            }
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(fold(entries))
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
    Ok(fold(entries))
}

/// Hash sorted entries into the digest both readings produce.
///
/// Shared so that a tree digest and an owned-projection digest of the same
/// files agree byte for byte -- the two differ in *which* entries they collect
/// and in nothing else.
fn fold(entries: Vec<(String, String, String)>) -> String {
    let mut hasher = Sha256::new();
    for (relative, kind, payload) in entries {
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(kind.as_bytes());
        hasher.update([0]);
        hasher.update(payload.as_bytes());
        hasher.update([0]);
    }
    format!("{PREFIX}{}", hex(&hasher.finalize()))
}

/// Describe one entry as the digest records it: path, kind, and content.
/// Describe one entry, or report that it went away while being described.
///
/// `false` means the entry vanished between being listed and being read. Every
/// step of a walk is a separate syscall against a tree somebody else may be
/// writing, and the gap this covers is not one gap but three: the listing to
/// the stat, the stat to the open, and the stat to the link read. Closing only
/// the first moved the refusal from `cannot stat` to `cannot open`, which is
/// how the second and third came to be measured -- by a race test that found
/// them rather than by reading for them.
///
/// Only `NotFound` is tolerated. A file this process may not read is a real
/// refusal and stays one: "it is gone" and "I am not allowed" are different
/// facts and a walk that conflated them would hash a target as smaller than it
/// is.
/// Whether a failed read was a file going away underneath the walk.
///
/// **`NotFound` is the Unix answer and only half of it.** On Windows a delete
/// is not immediate: the file enters a *delete-pending* state while any handle
/// remains open, and an attempt to open it in that window returns
/// `ERROR_ACCESS_DENIED`, which Rust maps to `PermissionDenied`. So the first
/// version of this tolerance passed on Linux and macOS and failed on Windows,
/// with `cannot open …\skills\3.md` — the fourth defect this estate has shipped
/// of one shape: two correct halves and a bad joint at the platform.
///
/// So the question is asked of the path rather than of the error kind. If it is
/// no longer there, the failure was a race. If it is still there, this process
/// genuinely cannot read it and that is a refusal — *"it is gone"* and *"I am
/// not allowed"* stay different facts, and a walk that conflated them would
/// hash a target as smaller than it is.
///
/// The second check is safe where it is used: `read_dir` on the parent has
/// already succeeded, so the entries of a directory this walk is inside can be
/// stated. A path that cannot be stated from there is one that is not there.
///
/// `symlink_metadata` rather than `exists()`, because the question is *is there
/// an entry here* and not *does it resolve*. A dangling symbolic link is an
/// entry — `exists()` follows it and answers false, which would have this
/// report a link that is really there as one that vanished.
fn vanished_under_us(path: &Path, source: &std::io::Error) -> bool {
    source.kind() == std::io::ErrorKind::NotFound || fs::symlink_metadata(path).is_err()
}

fn describe(
    root: &Path,
    path: &Path,
    metadata: &fs::Metadata,
    out: &mut Vec<(String, String, String)>,
) -> Result<bool> {
    let relative = relative_slash_path(root, path)?;
    if metadata.is_symlink() {
        let destination = match fs::read_link(path) {
            Ok(destination) => destination,
            Err(source) if vanished_under_us(path, &source) => return Ok(false),
            Err(source) => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot read link {}", path.display()),
                )
                .with_source(source));
            }
        };
        out.push((
            relative,
            "link".to_owned(),
            destination.to_string_lossy().into_owned(),
        ));
    } else if metadata.is_dir() {
        out.push((relative, "dir".to_owned(), String::new()));
    } else {
        let Some(content) = of_file_if_present(path)? else {
            return Ok(false);
        };
        let executable = if is_executable(metadata) { "x" } else { "-" };
        out.push((relative, format!("file:{executable}"), content));
    }
    Ok(true)
}

/// [`of_file`], answering `None` for a file that is no longer there.
///
/// See [`vanished_under_us`]: the error kind alone is the Unix half of the
/// question, and this one is asked on every platform.
fn of_file_if_present(path: &Path) -> Result<Option<String>> {
    match of_file(path) {
        Ok(digest) => Ok(Some(digest)),
        Err(error) if error.is_missing_path() || fs::symlink_metadata(path).is_err() => Ok(None),
        Err(error) => Err(error),
    }
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
        // An entry named by `read_dir` and gone by the time it is stated is a
        // *race*, not a failure of the target. Two processes on one target is
        // an ordinary state -- the second is refused by the lock -- and its
        // identity walk runs before that lock is taken. It read as
        // `provider_unavailable: cannot stat <path>`, so a concurrent install
        // was correctly refused with a reason that named a filesystem instead
        // of naming the lock, once in roughly forty runs.
        //
        // Skipped rather than refused, and the digest is then a snapshot of a
        // target that was moving. That is the honest answer: taken before the
        // lock it is advisory and a plan built on it goes stale, which is
        // exactly what should happen. Under the lock nothing else is writing,
        // so nothing here can vanish and the digest is exact.
        //
        // The same arm already exists a hundred lines up, for a namespace root
        // that is not there. This is the entry-level half of the same fact.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) if vanished_under_us(&path, &source) => continue,
            Err(source) => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot stat {}", path.display()),
                )
                .with_source(source));
            }
        };

        if !describe(root, &path, &metadata, out)? {
            continue;
        }
        if metadata.is_dir() && !metadata.is_symlink() {
            collect(root, &path, excluded_top_level, out)?;
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
        let base =
            std::env::temp_dir().join(format!("setup-core-digest-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn naming_a_path_already_covered_cannot_move_the_identity() {
        // The consumer validates a compiler's route against `native_namespaces`
        // by exact membership, so moving a route from a directory to a place
        // inside it needs one release declaring both -- otherwise whichever
        // side moves first refuses every install against the side that has not.
        // That window is only safe if the extra name is inert here.
        let base = scratch("cover");
        fs::create_dir_all(base.join("plugins/local/floor")).unwrap();
        fs::write(base.join("plugins/local/floor/plugin.json"), b"{}").unwrap();
        fs::create_dir_all(base.join("plugins-extra")).unwrap();
        fs::write(base.join("plugins-extra/note"), b"beside, not inside").unwrap();

        let parent_only = of_owned(&base, &["cli-config.json", "plugins"], &[]).unwrap();
        let both = of_owned(&base, &["cli-config.json", "plugins", "plugins/local"], &[]).unwrap();
        let child_first =
            of_owned(&base, &["plugins/local", "plugins", "cli-config.json"], &[]).unwrap();
        assert_eq!(parent_only, both);
        assert_eq!(parent_only, child_first);

        // A sibling whose name merely starts with the same letters is not
        // covered, and sorting does not place it after its would-be ancestor.
        let with_sibling = of_owned(
            &base,
            &[
                "cli-config.json",
                "plugins",
                "plugins-extra",
                "plugins/local",
            ],
            &[],
        )
        .unwrap();
        assert_ne!(parent_only, with_sibling);
    }

    #[test]
    fn bytes_are_tagged_with_the_algorithm() {
        assert!(of_bytes(b"x").starts_with(PREFIX));
    }

    #[test]
    fn a_domain_changes_the_digest_of_identical_bytes() {
        let one = of_domain_bytes("ai-stp:provider-plan:v3", b"payload");
        let two = of_domain_bytes("ai-stp:provider-projection:v3", b"payload");
        assert_ne!(one, two);
        assert_ne!(one, of_bytes(b"payload"));
    }

    #[test]
    fn the_domain_separator_is_a_nul_byte_not_a_concatenation() {
        // Without the separator, ("ab", "c") and ("a", "bc") would collide.
        assert_ne!(of_domain_bytes("ab", b"c"), of_domain_bytes("a", b"bc"));
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

    /// The joint between "gone" and "not allowed", asked on every platform.
    ///
    /// The first version of this tolerance matched `ErrorKind::NotFound` and
    /// nothing else. That is the Unix answer: on Windows a delete is not
    /// immediate, the file enters *delete-pending* while any handle is open,
    /// and opening it there returns `ERROR_ACCESS_DENIED` -- `PermissionDenied`
    /// in Rust. Every one of the seven published trees failed
    /// `rust / test (windows-latest)` with `cannot open …\skills\3.md` while
    /// Linux and macOS were green, which is the fourth defect this estate has
    /// shipped of one shape: two correct halves and a bad joint at the platform.
    ///
    /// The race test next door cannot catch it here -- it only races on the
    /// platform it runs on. This one states the rule directly, so both answers
    /// are asserted from either system.
    #[test]
    fn a_read_that_failed_is_a_race_only_when_the_path_is_gone() {
        let root = scratch("vanished-joint");
        let present = root.join("still-here.md");
        fs::write(&present, b"x").unwrap();
        let absent = root.join("never-was.md");

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);

        // Gone, whatever the kernel called it. This is the Windows case.
        assert!(vanished_under_us(&absent, &denied));
        assert!(vanished_under_us(&absent, &missing));

        // Still there and unreadable is a refusal, and stays one: "it is gone"
        // and "I am not allowed" are different facts, and a walk that treated
        // the second as the first would hash a target as smaller than it is.
        assert!(!vanished_under_us(&present, &denied));

        // A `NotFound` about a path that is there is still a race -- the entry
        // could have been recreated between the failure and this check.
        assert!(vanished_under_us(&present, &missing));

        // **A dangling symbolic link is an entry that is there.** This is the
        // whole reason the check asks `symlink_metadata` rather than
        // `exists()`: the second follows the link, answers false, and would
        // have `describe` treat a link whose `read_link` failed as vanished --
        // silently dropping it from the digest of a target that contains it.
        //
        // Creating one is privileged on Windows, so the assertion runs on Unix
        // and the branch it protects runs on both. That asymmetry is in the
        // test and not in the code: `vanished_under_us` takes the path as an
        // argument and has no `cfg!` in it, which is what lets the three
        // assertions above be checked from either system.
        #[cfg(unix)]
        {
            let dangling = root.join("points-nowhere");
            std::os::unix::fs::symlink(root.join("no-such-target"), &dangling).unwrap();
            assert!(!dangling.exists(), "the link must not resolve");
            assert!(
                !vanished_under_us(&dangling, &denied),
                "a link that is there read as vanished because it does not resolve"
            );
        }
    }

    /// An entry that vanishes between the listing and the stat is a race.
    ///
    /// Two processes on one target is an ordinary state: the second is refused
    /// by the lock, and its identity walk runs *before* that lock is taken. So
    /// the walk can list a name the other process is deleting, and it refused
    /// with `provider_unavailable: cannot stat <path>` -- a concurrent install
    /// correctly refused with a reason that named a filesystem rather than the
    /// lock. Seen in the cursor lifecycle probe, roughly once in forty runs.
    ///
    /// Raced rather than mocked, because the defect is a race and a mock of one
    /// proves the mock. The writer churns while the reader walks; before the
    /// fix this reached `cannot stat` within a few hundred passes, and it is
    /// asserted here that *no* pass refuses. A pass that finds nothing to skip
    /// is not a failure of the test -- the assertion is that the walk never
    /// refuses, which is true whether or not the race is hit on a given run.
    #[test]
    fn a_walk_racing_a_writer_skips_what_vanishes_rather_than_refusing() {
        let root = scratch("racing-writer");
        fs::create_dir_all(root.join("skills")).unwrap();
        for index in 0..64 {
            fs::write(root.join("skills").join(format!("{index}.md")), b"x").unwrap();
        }

        let churn = root.join("skills");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                for index in 0..64 {
                    let path = churn.join(format!("{index}.md"));
                    let _ = fs::remove_file(&path);
                    let _ = fs::write(&path, b"x");
                }
            }
        });

        let mut refusals = Vec::new();
        for _ in 0..200 {
            if let Err(error) = of_owned(&root, &["skills"], &[]) {
                refusals.push(error.to_string());
                break;
            }
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        writer.join().unwrap();

        assert!(
            refusals.is_empty(),
            "a walk racing a writer refused instead of skipping: {refusals:?}"
        );
    }
}
