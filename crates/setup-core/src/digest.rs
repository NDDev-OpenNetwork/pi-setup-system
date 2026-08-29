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
        // Nothing there is nothing to hash; see the note above on why this does
        // not leave a marker behind. Through the same retry as every other stat
        // in this walk, so a namespace root being replaced is not a refusal for
        // the reason an entry inside it is not.
        if let Some(metadata) = stat_if_present(&path)?
            && describe(root, &path, &metadata, &mut entries)?
            && metadata.is_dir()
        {
            collect(root, &path, excluded, &mut entries)?;
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

fn describe(
    root: &Path,
    path: &Path,
    metadata: &fs::Metadata,
    out: &mut Vec<(String, String, String)>,
) -> Result<bool> {
    let relative = relative_slash_path(root, path)?;
    if metadata.is_symlink() {
        // Retried like the open and the stat beside it. A link being replaced
        // races the same way, and leaving one of three sites interrogating the
        // path is how the second one was found -- by shipping it.
        let mut read = None;
        for attempt in 0..ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
            match fs::read_link(path) {
                Ok(destination) => {
                    read = Some(destination);
                    break;
                }
                Err(source) => {
                    if source.kind() == std::io::ErrorKind::NotFound {
                        return Ok(false);
                    }
                    if attempt + 1 == ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
                        return Err(Error::new(
                            ReasonCode::StateUnavailable,
                            format!("cannot read link {}", path.display()),
                        )
                        .with_source(source));
                    }
                    if attempt == 0 {
                        std::thread::yield_now();
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                }
            }
        }
        let Some(destination) = read else {
            return Ok(false);
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

/// How many times an open may be retried before the failure is believed.
///
/// Small on purpose. A genuine `PermissionDenied` pays this once per unreadable
/// file and then refuses; a file caught mid-replace is readable again well
/// inside it.
const ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL: u8 = 8;

/// [`of_file`], answering `None` for a file that is no longer there.
///
/// See [`vanished_under_us`]: the error kind alone is the Unix half of the
/// question. **Asking the path is the other half, and it has its own race.**
///
/// The fifth defect of this shape, and the first where the *check* was wrong
/// rather than the condition. A writer that deletes and immediately recreates a
/// file — which is what replacing one looks like — leaves the walk holding an
/// error about one instant and a `symlink_metadata` about a later one. The open
/// fails with `PermissionDenied` while the delete is pending, the path is asked
/// after the recreate, the answer is *"it is there"*, and a walk that was racing
/// a replacement refuses as though it had been denied. Measured, not reasoned:
/// `cannot open …\skills\0.md` on `windows-latest`, from the race test written
/// for the previous two, on one of seven identical trees — because it needs the
/// race to be lost on exactly that file.
///
/// So the failure is retried rather than interrogated. The distinction the
/// previous fix protects is kept and is now enforced by time instead of by a
/// second syscall: *"it is gone"* answers `None` immediately, and *"I am not
/// allowed"* survives every attempt and is still a refusal. A file being
/// replaced stops being either within a few attempts.
///
/// Retrying rather than widening the tolerated error kind is the point. Treating
/// `PermissionDenied` as *gone* would hash a target as smaller than it is on any
/// Stat an entry, answering `None` for one that is no longer there.
///
/// The `stat` half of what [`of_file_if_present`] does for `open`, and it exists
/// because it was missing. The retry landed on the open path and this site kept
/// asking `vanished_under_us`, which interrogates the path and therefore answers
/// about a later instant than the failure -- a writer that deletes and
/// immediately recreates leaves the stat failing while the path is present a
/// moment later.
///
/// Measured, not reasoned, twice. The open site was found by
/// `cannot open ...\skills\0.md` on `windows-latest`; this one by
/// `cannot stat ...\skills\31.md` on the same job two releases later, from the
/// same race test. **The first fix was applied to one of two call sites**, which
/// is the shape this estate keeps meeting -- and the reason both now share one
/// function rather than one rule written twice.
fn stat_if_present(path: &Path) -> Result<Option<fs::Metadata>> {
    for attempt in 0..ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
        match fs::symlink_metadata(path) {
            Ok(metadata) => return Ok(Some(metadata)),
            Err(source) => {
                if source.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                if attempt + 1 == ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
                    return Err(Error::new(
                        ReasonCode::StateUnavailable,
                        format!("cannot stat {}", path.display()),
                    )
                    .with_source(source));
                }
                if attempt == 0 {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }
    Err(Error::new(
        ReasonCode::StateUnavailable,
        format!("cannot stat {}", path.display()),
    ))
}

/// machine with a locked file in it, which is the failure this whole family
/// exists to prevent.
fn of_file_if_present(path: &Path) -> Result<Option<String>> {
    for attempt in 0..ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
        match of_file(path) {
            Ok(digest) => return Ok(Some(digest)),
            Err(error) => {
                if error.is_missing_path() || fs::symlink_metadata(path).is_err() {
                    return Ok(None);
                }
                if attempt + 1 == ATTEMPTS_BEFORE_BELIEVING_A_REFUSAL {
                    return Err(error);
                }
                // Yield first: a delete-pending window closes when the other
                // handle does, which is usually the next scheduling slot.
                if attempt == 0 {
                    std::thread::yield_now();
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
    }
    // The loop returns on its last attempt; this is unreachable and is written
    // as a refusal rather than a panic because a digest that cannot be taken is
    // never a reason to abort the process.
    Err(Error::new(
        ReasonCode::StateUnavailable,
        format!("cannot open {}", path.display()),
    ))
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
        let Some(metadata) = stat_if_present(&path)? else {
            continue;
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

    /// Gone is skipped and denied is refused, asserted on the live path.
    ///
    /// This replaces a test of `vanished_under_us`, the check that asked the
    /// path whether it was still there. Three call sites retry instead now, so
    /// that function is gone and the mechanism it tested with it -- but the two
    /// guarantees it protected are the point and are asserted here:
    ///
    /// * a path that is not there is skipped, not refused;
    /// * a path that is there and unreadable is refused, and stays refused
    ///   after every attempt, because *"it is gone"* and *"I am not allowed"*
    ///   are different facts and a walk that conflated them would hash a target
    ///   as smaller than it is.
    ///
    /// The third property the old test asserted -- that a dangling symbolic
    /// link is an entry that is there -- needs no assertion now. It existed
    /// because the check called `symlink_metadata` rather than `exists()`; the
    /// retry calls neither, and `read_link` on a dangling link succeeds.
    #[test]
    fn gone_is_skipped_and_denied_is_refused() {
        let root = scratch("gone-or-denied");
        let absent = root.join("never-was.md");
        assert!(
            matches!(stat_if_present(&absent), Ok(None)),
            "a path that is not there must be skipped"
        );

        let present = root.join("still-here.md");
        fs::write(&present, b"x").unwrap();
        assert!(
            matches!(stat_if_present(&present), Ok(Some(_))),
            "a path that is there must be stated"
        );
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

    /// A file this process may not read is still a refusal, after the retry.
    ///
    /// The control for `of_file_if_present`. Retrying an open makes a walk
    /// tolerate a file being replaced; it must not make one tolerate a file it
    /// is genuinely denied, because a walk that skipped those would hash a
    /// target as smaller than it is and call the result a match.
    ///
    /// Unix only, and it establishes that this caller *can* be denied rather
    /// than assuming it: root reads a mode-000 file regardless, and the
    /// assertion would then be measuring the runner instead of the code. Asked
    /// by trying the read, which is the same question the walk asks.
    #[cfg(unix)]
    #[test]
    fn a_file_this_process_may_not_read_is_still_a_refusal() {
        use std::os::unix::fs::PermissionsExt;

        let root = scratch("denied-not-vanished");
        fs::create_dir_all(root.join("skills")).unwrap();
        let denied = root.join("skills").join("locked.md");
        fs::write(&denied, b"secret").unwrap();
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o000)).unwrap();

        if fs::File::open(&denied).is_ok() {
            // This caller cannot be denied, so there is no refusal to observe.
            fs::set_permissions(&denied, fs::Permissions::from_mode(0o644)).unwrap();
            return;
        }

        let walked = of_owned(&root, &["skills"], &[]);

        // Restored before asserting, so a failure does not leave an unreadable
        // file in the scratch tree for the next run to trip over.
        fs::set_permissions(&denied, fs::Permissions::from_mode(0o644)).unwrap();

        match walked {
            Ok(digest) => panic!(
                "a file that cannot be read was skipped rather than refused, \
                 and the walk reported a digest for a target it did not read: {digest}"
            ),
            Err(error) => assert!(
                error.to_string().contains("cannot open"),
                "the refusal should name the open that failed: {error}"
            ),
        }
    }
}
