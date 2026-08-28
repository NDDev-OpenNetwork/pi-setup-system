//! The whole documented round trip, driven against a built binary.
//!
//! The three-OS matrix has always proved that this code compiles on ubuntu,
//! macos and windows and that its unit tests pass there. It has never proved
//! that a *target* survives a round trip on those systems — every lifecycle run
//! against a real directory happened on Linux, and `docs/PLAN.md` said so under
//! open risks for four releases.
//!
//! The difference is not academic. Both Windows defects this project has
//! shipped lived in the joint between two correct halves and were invisible to
//! a unit test: `expose` answering "no version is installed" on a system with
//! no symbolic links, and a fixture that was absolute on two systems out of
//! three. A test that runs the binary and then *looks at the directory* is the
//! only shape that catches those.
//!
//! So this drives the real executable through `list`, `install`, `status`,
//! `select`, `backups`, `hold`, `restore`, `restore --backup`, `release`,
//! `remove` and `recover-operation`, and reads the target after each one. It
//! takes the executable as an argument, so each setup system runs it against
//! its own binary and the same text runs seven times on three systems.
//!
//! Two rules shape every line here.
//!
//! **Nothing spells a platform detail.** Paths come from
//! [`std::env::temp_dir`] and are joined rather than written, output is
//! compared after normalising line endings, and no assertion mentions a
//! separator — a test that writes down what it believes a platform does is
//! testing its belief.
//!
//! **Nothing panics.** This module is compiled into the shipped binary, so an
//! environment that will not cooperate is reported as a disagreement like any
//! other rather than unwound through a `panic!` the workspace lints forbid.

// The second of the two places that may spawn. This module drives *this
// program's own executable* from a test; no argv routes to it, and it goes
// behind a Cargo feature in a later release so a released artifact does not
// carry test scaffolding at all. That gating is for having no user, not for the
// network claim: `launch` puts `execvp` in the import table either way.
#![allow(
    clippy::disallowed_types,
    reason = "the probe runs this binary; it is the second named spawn site"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A file found in a target, by slash-separated relative path.
type Tree = BTreeMap<String, Vec<u8>>;

/// A file this provider does not own, present in the target throughout.
const OVERLAY: &str = "a-file-this-provider-does-not-own.txt";
/// What that file holds, checked byte for byte after every command.
const OVERLAY_BYTES: &[u8] = b"kept verbatim\n";

/// One run of the binary.
struct Ran {
    ok: bool,
    out: String,
}

/// Run the executable and collect what it said, whichever stream it used.
///
/// A binary that cannot be started at all answers as a failed run carrying the
/// reason, so the caller reports it in the same list as everything else.
fn run(exe: &Path, arguments: &[&OsStr]) -> Ran {
    match Command::new(exe).args(arguments).output() {
        Ok(output) => {
            let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
            out.push_str(&String::from_utf8_lossy(&output.stderr));
            Ran {
                ok: output.status.success(),
                // A Windows console ends its lines differently, and nothing
                // asked here is a line ending.
                out: out.replace("\r\n", "\n"),
            }
        }
        Err(error) => Ran {
            ok: false,
            out: format!("{} could not be run: {error}", exe.display()),
        },
    }
}

/// Run the executable against a target.
fn at(exe: &Path, target: &Path, arguments: &[&str]) -> Ran {
    let mut all: Vec<&OsStr> = arguments.iter().map(OsStr::new).collect();
    all.push(OsStr::new("--target"));
    all.push(target.as_os_str());
    run(exe, &all)
}

/// Every regular file under a root, as slash-separated relative paths.
fn files_under(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let parts: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            found.push((parts.join("/"), path));
        }
    }
    found
}

/// Every file in a target except the provider's own control state.
///
/// The control directory holds the lock, the journal and the backup pool, and
/// the state file records the last operation; all change on every command by
/// design. What a round trip is *about* is the rest.
///
/// `own` is discovered rather than passed in — see [`control_state`] — so this
/// carries no second copy of a fact that already lives in a `Harness`.
fn tree(target: &Path, own: &[String]) -> Tree {
    let mut found = Tree::new();
    for (key, path) in files_under(target) {
        let mine = own
            .iter()
            .any(|name| key == *name || key.starts_with(&format!("{name}/")));
        if !mine {
            found.insert(key, std::fs::read(&path).unwrap_or_default());
        }
    }
    found
}

/// The relative paths a target holds, in a stable order.
fn names(target: &Path, own: &[String]) -> Vec<String> {
    tree(target, own).into_keys().collect()
}

/// A scratch directory nothing else in this process will collide with.
fn scratch(label: &str) -> Result<PathBuf, String> {
    // No randomness: the process id and the label are unique within one test
    // binary, and a deterministic name is easier to find after a failure.
    let path =
        std::env::temp_dir().join(format!("setup-system-probe-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path)
        .map(|()| path.clone())
        .map_err(|error| format!("no scratch directory at {}: {error}", path.display()))
}

/// The setup ids this binary carries, read from its own `list`.
fn catalog(exe: &Path) -> Result<Vec<String>, String> {
    let listed = run(exe, &[OsStr::new("list")]);
    if !listed.ok {
        return Err(format!("list failed:\n{}", listed.out));
    }
    Ok(listed
        .out
        .lines()
        .filter_map(|line| {
            // `list` indents each id by two spaces and describes it by six.
            let body = line.trim_end().strip_prefix("  ")?;
            if body.starts_with(' ') || body.is_empty() {
                return None;
            }
            Some(body.to_owned())
        })
        .collect())
}

/// What this provider leaves in a target that is its own, learned by watching.
///
/// Install into an empty directory and remove again: whatever survives is the
/// provider's control state, because `remove` takes away everything it owns of
/// the *product's*.
///
/// Nothing is assumed about the names. An earlier draft took
/// `control_directory` and `state_file` as arguments, which would have been a
/// second copy of two facts that already live in a `Harness` — and a copy that
/// could disagree with it.
///
/// Only surviving *files* count, reduced to their top-level component. Reading
/// the directory entries instead counted an empty directory `remove` had left
/// behind — antigravity's `antigravity-cli` — as control state, and then hid
/// every file its setups write beneath it. Observed: the probe reported a setup
/// that installs nothing.
fn control_state(exe: &Path, setup: &str) -> Result<Vec<String>, String> {
    let target = scratch("control-state")?;
    let install = at(exe, &target, &["install", setup]);
    if !install.ok {
        return Err(format!(
            "install into an empty target failed:\n{}",
            install.out
        ));
    }
    let removed = at(exe, &target, &["remove"]);
    if !removed.ok {
        return Err(format!("remove failed:\n{}", removed.out));
    }

    let mut own: Vec<String> = Vec::new();
    for (key, _) in files_under(&target) {
        let Some(first) = key.split('/').next() else {
            continue;
        };
        if !own.iter().any(|held| held == first) {
            own.push(first.to_owned());
        }
    }
    own.sort();
    let _ = std::fs::remove_dir_all(&target);
    Ok(own)
}

/// Whether the sibling overlay is still exactly what was written.
fn overlay_survives(target: &Path, after: &str, found: &mut Vec<String>) {
    match std::fs::read(target.join(OVERLAY)) {
        Ok(bytes) if bytes == OVERLAY_BYTES => {}
        Ok(_) => found.push(format!("the sibling overlay was rewritten by {after}")),
        Err(_) => found.push(format!("the sibling overlay was removed by {after}")),
    }
}

/// Install, observe and select. `None` means it did not get far enough to go on.
fn through_install(
    exe: &Path,
    target: &Path,
    own: &[String],
    setups: (&str, &str),
    found: &mut Vec<String>,
) -> Option<Tree> {
    let (first, second) = setups;
    let empty = at(exe, target, &["status"]);
    if !empty.ok {
        found.push(format!(
            "status on an untouched target failed:\n{}",
            empty.out
        ));
    }
    if !empty.out.contains("none applied") {
        found.push(format!(
            "status on an untouched target does not say nothing is applied:\n{}",
            empty.out
        ));
    }

    let installed = at(exe, target, &["install", first]);
    if !installed.ok {
        found.push(format!("install {first} failed:\n{}", installed.out));
        return None;
    }
    let after_install = tree(target, own);
    if !after_install.contains_key(OVERLAY) {
        found.push("install removed the sibling overlay".to_owned());
    }
    if after_install.len() < 2 {
        found.push(format!(
            "install {first} left {} file(s) beside the overlay",
            after_install.len().saturating_sub(1)
        ));
    }
    overlay_survives(target, "install", found);

    let applied = at(exe, target, &["status"]);
    if !applied.out.contains(first) {
        found.push(format!(
            "status does not name the applied setup:\n{}",
            applied.out
        ));
    }
    if !applied.out.contains("Drift    none") {
        found.push(format!(
            "status reports drift on a fresh install:\n{}",
            applied.out
        ));
    }

    let selected = at(exe, target, &["select", second]);
    if !selected.ok {
        found.push(format!("select {second} failed:\n{}", selected.out));
        return None;
    }
    if tree(target, own) == after_install {
        found.push(format!(
            "select {second} left the target byte-identical to {first}"
        ));
    }
    overlay_survives(target, "select", found);
    Some(after_install)
}

/// Back up, hold, restore twice, release.
fn through_restore(
    exe: &Path,
    target: &Path,
    own: &[String],
    first: &str,
    after_install: &Tree,
    found: &mut Vec<String>,
) {
    let listed = at(exe, target, &["backups"]);
    if !listed.out.contains("slot-000000000001") || !listed.out.contains("slot-000000000002") {
        found.push(format!(
            "backups does not list both captures:\n{}",
            listed.out
        ));
    }

    let held = at(
        exe,
        target,
        &[
            "hold",
            "--backup",
            "slot-000000000001",
            "--reason",
            "the probe",
        ],
    );
    if !held.ok {
        found.push(format!("hold failed:\n{}", held.out));
    }

    let restored = at(exe, target, &["restore"]);
    if !restored.ok {
        found.push(format!("restore failed:\n{}", restored.out));
        return;
    }
    let after_restore = tree(target, own);
    if after_restore != *after_install {
        found.push(format!(
            "restore did not return the target to what {first} left: {} file(s) then, {} now",
            after_install.len(),
            after_restore.len()
        ));
    }
    overlay_survives(target, "restore", found);

    // The first slot holds the state before anything was installed, which is
    // the overlay and nothing else.
    let to_first = at(exe, target, &["restore", "--backup", "slot-000000000001"]);
    if !to_first.ok {
        found.push(format!("restore --backup failed:\n{}", to_first.out));
    }
    let at_origin = names(target, own);
    if at_origin != vec![OVERLAY.to_owned()] {
        found.push(format!(
            "a restore to the first slot left {at_origin:?} instead of the overlay alone"
        ));
    }
    overlay_survives(target, "restore --backup", found);

    let released = at(exe, target, &["release", "--backup", "slot-000000000001"]);
    if !released.ok {
        found.push(format!("release failed:\n{}", released.out));
    }
}

/// Reinstall, remove, and ask a settled target to recover nothing.
fn through_remove(exe: &Path, target: &Path, own: &[String], first: &str, found: &mut Vec<String>) {
    let reinstalled = at(exe, target, &["install", first]);
    if !reinstalled.ok {
        found.push(format!(
            "a second install of {first} failed:\n{}",
            reinstalled.out
        ));
    }
    let removed = at(exe, target, &["remove"]);
    if !removed.ok {
        found.push(format!("remove failed:\n{}", removed.out));
    }
    let left = names(target, own);
    if left != vec![OVERLAY.to_owned()] {
        found.push(format!("remove left {left:?} instead of the overlay alone"));
    }
    overlay_survives(target, "remove", found);

    // A target with no interrupted mutation has nothing to recover, and saying
    // so is a different answer from failing.
    let recovered = at(exe, target, &["recover-operation", "--json"]);
    if !recovered.ok {
        found.push(format!(
            "recover-operation on a settled target failed:\n{}",
            recovered.out
        ));
    }
}

/// Drive one setup system through everything it documents, and report what
/// disagreed.
///
/// Empty is the only passing answer. A failure of the environment — an
/// executable that will not start, a temporary directory that cannot be made —
/// is reported in the same list rather than raised, because this module is
/// compiled into the shipped binary.
#[must_use]
pub fn round_trip(exe: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let setups = match catalog(exe) {
        Ok(setups) => setups,
        Err(problem) => {
            found.push(problem);
            return found;
        }
    };
    if setups.len() < 2 {
        found.push(format!(
            "this binary carries {} setup(s); a round trip needs two to select between",
            setups.len()
        ));
        return found;
    }
    let (first, second) = (setups[0].clone(), setups[1].clone());

    let own = match control_state(exe, &first) {
        Ok(own) if own.is_empty() => {
            found.push(
                "an install followed by a remove left nothing at all, so this provider \
                 records no state of its own"
                    .to_owned(),
            );
            return found;
        }
        Ok(own) => own,
        Err(problem) => {
            found.push(problem);
            return found;
        }
    };

    let target = match scratch("target") {
        Ok(target) => target,
        Err(problem) => {
            found.push(problem);
            return found;
        }
    };
    if let Err(error) = std::fs::write(target.join(OVERLAY), OVERLAY_BYTES) {
        found.push(format!("the sibling overlay could not be written: {error}"));
        return found;
    }

    if let Some(after_install) = through_install(exe, &target, &own, (&first, &second), &mut found)
    {
        through_restore(exe, &target, &own, &first, &after_install, &mut found);
        through_remove(exe, &target, &own, &first, &mut found);
    }

    let _ = std::fs::remove_dir_all(&target);
    found
}

/// Prove two processes cannot write one target at once, on this system.
///
/// The lock is two layers: an in-process claim set, and `File::try_lock` under
/// it. The first is unit-tested; the second is an operating-system primitive --
/// `flock` on Unix, `LockFileEx` on Windows -- and a unit test in one process
/// cannot reach it. So this drives real processes, which is what this module is
/// for, and it does so on all three systems.
///
/// What it asserts is deliberately narrow: at most one of several concurrent
/// installs applies, every refusal names the lock, and the target afterwards
/// reports a setup with no drift. It does not assert that exactly one wins --
/// a machine slow enough to serialise them would apply them one after another,
/// which is correct behaviour and not what this is about.
#[must_use]
pub fn one_writer_at_a_time(exe: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let Ok(target) = scratch("one-writer") else {
        found.push("no scratch directory for the concurrency check".to_owned());
        return found;
    };
    let Ok(setups) = catalog(exe) else {
        found.push("the catalog could not be read for the concurrency check".to_owned());
        return found;
    };
    let Some(setup) = setups.first() else {
        found.push("no setup to install concurrently".to_owned());
        return found;
    };

    let mut children = Vec::new();
    for _ in 0..4 {
        match Command::new(exe)
            .args([
                OsStr::new("install"),
                OsStr::new(setup),
                OsStr::new("--target"),
                target.as_os_str(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => children.push(child),
            Err(error) => found.push(format!("a concurrent install could not start: {error}")),
        }
    }

    let mut applied = 0_usize;
    let mut refused_without_naming_the_lock = Vec::new();
    for child in children {
        match child.wait_with_output() {
            Ok(output) => {
                if output.status.success() {
                    applied += 1;
                } else {
                    let said = String::from_utf8_lossy(&output.stderr).into_owned()
                        + &String::from_utf8_lossy(&output.stdout);
                    if !said.contains("target.lock") {
                        refused_without_naming_the_lock.push(said.replace("\r\n", "\n"));
                    }
                }
            }
            Err(error) => found.push(format!(
                "a concurrent install could not be waited on: {error}"
            )),
        }
    }

    if applied == 0 {
        found.push("no concurrent install applied, so the target was never written".to_owned());
    }
    for said in refused_without_naming_the_lock {
        found.push(format!(
            "a concurrent install was refused without naming the lock:\n{said}"
        ));
    }

    // Whatever the ordering, the target must be coherent afterwards.
    let after = run(
        exe,
        &[
            OsStr::new("diff"),
            OsStr::new("--target"),
            target.as_os_str(),
        ],
    );
    if !after.ok || !after.out.contains("matches the setup recorded in it") {
        found.push(format!(
            "after concurrent installs the target does not match its own record:\n{}",
            after.out
        ));
    }

    let _ = std::fs::remove_dir_all(&target);
    found
}

/// Prove a target this provider was never pointed at is refused, not guessed.
#[must_use]
pub fn refuses_a_target_it_should(exe: &Path) -> Vec<String> {
    let mut found = Vec::new();

    // A relative target. Every command takes an absolute one, and on Windows
    // "absolute" means a drive or a UNC prefix rather than a leading separator
    // -- so this asks with a name that is relative on all three systems.
    let relative = run(
        exe,
        &[
            OsStr::new("status"),
            OsStr::new("--target"),
            OsStr::new("a-relative-name"),
        ],
    );
    if relative.ok {
        found.push("a relative --target was accepted".to_owned());
    }

    let Ok(holder) = scratch("not-a-target") else {
        found.push("no scratch directory for the refusal checks".to_owned());
        return found;
    };

    // A target that does not exist.
    let absent = holder.join("never-created");
    let missing = run(
        exe,
        &[
            OsStr::new("status"),
            OsStr::new("--target"),
            absent.as_os_str(),
        ],
    );
    if missing.ok {
        found.push("a --target that does not exist was accepted".to_owned());
    }

    // A target that is a file rather than a directory.
    let file = holder.join("a-file");
    if std::fs::write(&file, b"not a directory\n").is_err() {
        found.push("the not-a-directory check could not write its file".to_owned());
    } else {
        let not_a_directory = run(
            exe,
            &[
                OsStr::new("status"),
                OsStr::new("--target"),
                file.as_os_str(),
            ],
        );
        if not_a_directory.ok {
            found.push("a --target that is a file was accepted".to_owned());
        }
    }

    let _ = std::fs::remove_dir_all(&holder);
    found
}
