//! The provider command runtime every NDDev setup system shares.
//!
//! Five products, one set of commands. What differs between them is not
//! behaviour but *facts*: which directory a product configures, which files
//! inside it this provider owns, and which files belong to the product and must
//! be left alone. [`Harness`] holds those facts; [`wire::dispatch`] performs the
//! commands over them.
//!
//! Written this way, a change to the shared logic lands once instead of five
//! times, and a change to one product's surface lands in exactly one struct that
//! a test binds to that product's verified baseline.
//!
//! # What this runtime does and does not do
//!
//! It performs all five core operations. `backup`, `restore` and `remove` read
//! the target, a backup slot, or the provider's own state. `install` and
//! `replace` materialize an `ai-stp-bundle/1` the consumer sends, or a complete
//! setup from the local catalog when the owner asks for one by name.
//!
//! The software lifecycle is optional in the contract, and a harness declares
//! it exactly when it carries an artifact table -- `Harness::installs_a_program`
//! is the whole rule, and `launch` adds one condition on top of it, that the
//! product documents an environment variable this build can point at a target.
//! Declaring an optional operation this runtime cannot perform would let a
//! consumer call something that cannot be honoured, which is worse than not
//! offering it.
//!
//! **The rule is stated here and the tally is not, deliberately.** This
//! paragraph used to carry one -- *"six of the seven do, and pi does not"*,
//! plus *"`launch` is declared by none"* -- and both were false by the time
//! anyone read them: pi gained an artifact table in `7180648`, and every
//! harness but antigravity declares `launch`. A count in prose has nothing
//! holding it. Ask the predicate.

pub(crate) mod adopt;
pub mod catalog;
pub mod expiry;
pub mod facts;
pub mod human;
pub mod probe;
pub(crate) mod software;
pub mod surfaces;
pub mod wire;

pub use catalog::{Catalog, Setup};
// The software types belong to the kernel, but a setup system declares its
// artifact table and depends only on this crate. Re-exported so that stays
// true rather than widening seven dependency lists to reach past it.
pub use facts::{BACKUP_SLOTS, BUNDLE_FORMAT, Foreign, Harness, LaunchBinding, Scoped, Shadow};
pub use setup_core::software::{Artifact, Delivery, Previous, Shape, Software};

/// The kernel's content digest, re-exported for the seven binaries.
///
/// They depend on this crate and on `provider-v3`, not on `setup-core`, and a
/// harness test that pins the bytes it ships needs the same hash the rest of
/// the estate uses. Re-exporting is cheaper than a dependency and keeps one
/// implementation.
#[must_use]
pub fn digest_of_bytes(bytes: &str) -> String {
    setup_core::digest::of_bytes(bytes.as_bytes())
}
pub use wire::dispatch;

use std::process::ExitCode;

/// Run one setup system end to end, from process arguments to an exit code.
///
/// Every binary in the estate is this function plus one [`Harness`]. Keeping the
/// entry point here means the exit-code contract below is stated once.
///
/// # Reading the exit code
///
/// `0` — the command answered. For a wire command the answer is one JSON object
/// on stdout, including a refusal that names a reason: a refusal *is* an answer,
/// and the consumer parses it rather than reading a message.
///
/// `1` — the invocation itself was not usable, or this build declared something
/// the contract does not permit. Nothing was written.
#[must_use]
pub fn run(harness: &Harness, arguments: Vec<String>) -> ExitCode {
    match arguments.first().map(String::as_str) {
        Some("--version") => {
            println!("{} {}", harness.provider_id, harness.version);
            return ExitCode::SUCCESS;
        }
        Some("--help") | None => {
            print_help(harness);
            return ExitCode::SUCCESS;
        }
        _ => {}
    }

    // `status` is the one name both surfaces answer to. The human form takes a
    // bare `--target`; the wire form also takes `--json`, and that is what tells
    // them apart. Nothing else is ambiguous.
    let first = arguments.first().map(String::as_str).unwrap_or_default();
    let human_status = first == "status" && !arguments.iter().any(|a| a == "--json");
    if human::is_human_command(first) || human_status {
        return match human::parse(arguments).and_then(|command| human::run(harness, command)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{}: {error}", harness.provider_id);
                ExitCode::FAILURE
            }
        };
    }

    // `<command> --help` used to answer `--help has no value`, because the flag
    // parser reads every `--flag` as taking one. A caller could not ask what a
    // command takes, and each missing argument surfaced singly -- seven
    // invocations to learn `plan-operation`, measured by a peer who already
    // knew the shape.
    //
    // Read before parsing, and only outside a passthrough: everything after a
    // bare `--` belongs to the product `launch` starts, where `--help` means
    // something else entirely and is not ours to intercept.
    if let Some(command) = provider_v3::vocabulary::Command::parse(first) {
        let mine = arguments
            .iter()
            .take_while(|token| *token != "--")
            .any(|token| token == "--help");
        if mine {
            print!("{}", provider_v3::argv::render_usage(command));
            return ExitCode::SUCCESS;
        }
    }

    let invocation = match provider_v3::argv::parse(arguments) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("{}: {error}", harness.provider_id);
            return ExitCode::FAILURE;
        }
    };

    match wire::dispatch(harness, invocation) {
        Ok(answer) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            if let Some(reason) = error.reason() {
                println!(
                    "{}",
                    serde_json::json!({
                        "state": "refused",
                        "rejected": true,
                        "reason": reason.as_str(),
                        "detail": error.detail(),
                    })
                );
                ExitCode::SUCCESS
            } else {
                // A defect in this build's own declaration is not the consumer's
                // to act on, so it is reported as a failure rather than dressed
                // up as a contract reason.
                eprintln!("{}: {error}", harness.provider_id);
                ExitCode::FAILURE
            }
        }
    }
}

fn print_help(harness: &Harness) {
    println!("{} {}", harness.provider_id, harness.version);
    println!();
    println!(
        "Configures {} ({}) in a caller-named target directory.",
        harness.product, harness.vendor
    );
    if harness.config_home_env.is_empty() {
        println!(
            "Documented configuration home: {} (no environment override documented)",
            harness.documented_config_home
        );
    } else {
        println!(
            "Documented configuration home: {} ({})",
            harness.documented_config_home, harness.config_home_env
        );
    }
    // Printed here rather than left in a baseline nobody runs: for one of the
    // seven the line above is conditionally false, and this is the surface a
    // person actually reads before choosing a `--target`.
    //
    // Labelled, like every other line in this help. The first version printed
    // `"  {}"` -- a bare continuation whose meaning depended entirely on the
    // line above it, which is not how anything else here reads.
    if !harness.config_home_note.is_empty() {
        println!("Also honoured: {}", harness.config_home_note);
    }
    println!();
    println!("Provider commands (ai-stp protocol v3):");
    println!("  provider-info");
    println!("  status            --target <dir> --json");
    println!("  validate-bundle   --target <dir> --json --bundle <path> ...");
    println!("  plan-operation    --target <dir> --json --operation <op> ...");
    println!("  apply-operation   --target <dir> --json --plan <path> --plan-digest <d> ...");
    println!("  recover-operation --target <dir> --json");
    if harness.can_launch() {
        println!("  launch            --target <dir> --prefix <dir> --json [-- <args>]");
    }
    println!();
    if harness
        .operations()
        .contains(&provider_v3::Operation::SoftwareInstall)
    {
        println!(
            "This build also installs {} itself. Those operations take a",
            harness.product
        );
        println!("`--prefix` for the program, distinct from the `--target` that holds");
        println!("its configuration, and the bytes are fetched between planning and");
        println!("applying by whoever holds the network:");
        println!();
        println!(
            "  plan-operation  --operation software_install --target <dir> --prefix <dir> ..."
        );
        println!("  apply-operation --prefix <dir> --software-artifact <file> ...");
        println!();
    }
    println!("Your commands:");
    println!("  list");
    println!("  status    --target <dir>");
    println!("  install   <setup> --target <dir>");
    println!("  select    <setup> --target <dir>");
    println!("  reinstall --target <dir>");
    println!("  diff      --target <dir>");
    println!("  backups   --target <dir>");
    println!("  restore   [--backup <ref>] --target <dir>");
    println!("  hold      --backup <ref> [--reason <why>] --target <dir>");
    println!("  release   --backup <ref> --target <dir>");
    println!("  remove    --target <dir>");
    if !harness.predecessor_state_file.is_empty() {
        println!("  adopt     --target <dir>");
    }
    if harness.installs_a_program() {
        println!("  software  --prefix <dir>");
        println!("  rollback  --to <version> --prefix <dir>");
    }
    println!();
    if harness.installs_a_program() {
        println!("`software` reads the program directory and `rollback` points the");
        println!("command back at a version already in it -- installing a new one leaves");
        println!("the old tree in place and moves only the command. Both need no network,");
        println!("which is why they are commands you type; install and update need bytes");
        println!("fetched between planning and applying, so they stay on the wire above.");
        println!();
    }
    if !harness.predecessor_state_file.is_empty() {
        println!(
            "`adopt` takes over a target still carrying {},",
            harness.predecessor_state_file
        );
        println!("written by the estate that came before this one. It is a command you");
        println!("type, never something install does behind you, and it deletes nothing.");
        println!();
    }
    println!("Every one takes an explicit --target, and the two program commands an");
    println!("explicit --prefix. There is no default: a change aimed at a guessed");
    println!("path is a change aimed at someone else's state. `rollback` names its");
    println!("version for the same reason -- there is no record of which was");
    println!("previous, only what is on disk.");
    println!();
    println!("A backup is captured before every change, so `restore` always has");
    println!("something to return to. The pool rolls, so a long series of changes");
    println!("eventually evicts the oldest: `hold` keeps one until `release` lets");
    println!("it go, which is how a baseline survives more captures than the pool.");
    println!("Over the wire, install and replace arrive as a bundle and refuse --");
    println!("this build reads setups from its own catalog.");
}

#[cfg(test)]
mod tests {
    // A test may spawn: this one drives a real executable, which is the only
    // way to prove what the shipped binary does rather than what this source
    // believes. The lint's subject is the program, not its tests.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::disallowed_types,
        reason = "tests drive real executables to check the shipped behaviour"
    )]

    /// The entry point uses the toolchain this tree pins, even where a
    /// different one comes first on `PATH`.
    ///
    /// This was closed once by *running* it: with `~/.local/bin` first,
    /// `cargo test --doc` failed with `E0514` and `scripts/gate.sh` reported
    /// the pinned version anyway. A run is not a test. The next
    /// `rust-toolchain.toml` bump is when a silently wrong entry point costs
    /// something, and until now nothing would have been watching.
    ///
    /// The shim is the shadow the trap describes: a real executable named
    /// `cargo`, earlier on `PATH`, reporting a different release. The test
    /// proves it *would* have won, and then that the entry point selects past
    /// it — otherwise a passing assertion could mean the shim was never
    /// consulted at all.
    #[test]
    #[cfg(unix)]
    fn the_entry_point_selects_the_pinned_toolchain_past_a_shadow() {
        use std::os::unix::fs::PermissionsExt;
        use std::process::Command;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

        // This crate is vendored into seven published trees, and the entry
        // point it tests belongs to exactly one repository: the workspace where
        // the code is written and the render is proved. A published tree ships
        // no `scripts/` at all, because contributing there means something
        // else -- so the test travelled somewhere its subject does not exist,
        // and failed on ubuntu and macos in all seven at once.
        //
        // Absence is asserted rather than skipped. A rendered tree is
        // identifiable: it carries no renderer either. If both are gone this is
        // a published tree and there is nothing here to test; if the script is
        // gone and the renderer is not, someone deleted the entry point in the
        // workspace and that is a failure, not a skip.
        if !root.join("scripts/gate.sh").is_file() {
            assert!(
                !root.join("tools/render_public_trees.py").is_file(),
                "the toolchain entry point is missing from a workspace that still \
                 renders the public trees; scripts/gate.sh was deleted rather than \
                 never present"
            );
            return;
        }

        let pinned = std::fs::read_to_string(root.join("rust-toolchain.toml"))
            .unwrap()
            .lines()
            .find_map(|line| {
                line.strip_prefix("channel = \"")
                    .and_then(|rest| rest.strip_suffix('"'))
                    .map(str::to_owned)
            })
            .expect("rust-toolchain.toml pins a channel");

        let shadow = std::env::temp_dir().join(format!("gate-shadow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&shadow);
        std::fs::create_dir_all(&shadow).unwrap();
        let shim = shadow.join("cargo");
        std::fs::write(&shim, "#!/bin/sh\necho 'cargo 1.0.0 (shadow)'\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();

        let shadowed = format!(
            "{}:{}",
            shadow.display(),
            std::env::var("PATH").unwrap_or_default()
        );

        // The shim would have won. Without this the assertion below could pass
        // because nothing ever put it in the way.
        let shadowing = Command::new("sh")
            .arg("-c")
            .arg("cargo --version")
            .env("PATH", &shadowed)
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&shadowing.stdout).contains("1.0.0 (shadow)"),
            "the shim did not shadow cargo, so this test proves nothing"
        );

        let asked = Command::new("bash")
            .arg("scripts/gate.sh")
            .arg("--toolchain")
            .current_dir(&root)
            .env("PATH", &shadowed)
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&asked.stdout).into_owned();
        let _ = std::fs::remove_dir_all(&shadow);

        assert!(asked.status.success(), "the entry point failed: {said}");
        assert!(
            said.contains(&format!("cargo reports {pinned}")),
            "the entry point used the shadow rather than the pinned {pinned}: {said}"
        );
    }

    /// The entry point asks the render question the ref it is on can answer.
    ///
    /// There are two, and `check_render.sh` says in its own header that they
    /// are not the same one: strict compares the published trees against this
    /// source and is a property of `main`; `--deterministic` asks whether the
    /// renderer agrees with itself and is what a branch can answer. The gate
    /// asked strict unconditionally while telling you, in *its* header, to run
    /// it before opening a pull request — so the documented pre-PR command was
    /// red on every branch that changed anything, for a reason belonging to no
    /// branch. The ways out of that are to ignore the gate or to misread its
    /// exit status, and both happened.
    ///
    /// Tested through `--render-mode <ref>` rather than by rendering: the
    /// decision is the thing that was wrong, and it is one function. Driving it
    /// with three refs also proves it *distinguishes* them — an implementation
    /// that answered `deterministic` to everything would satisfy a branch-only
    /// assertion and quietly stop proving the trees are what this source says.
    #[test]
    #[cfg(unix)]
    fn the_entry_point_asks_the_render_question_this_ref_can_answer() {
        use std::process::Command;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !root.join("scripts/gate.sh").is_file() {
            assert!(
                !root.join("tools/render_public_trees.py").is_file(),
                "the gate entry point is missing from a workspace that still \
                 renders the public trees; scripts/gate.sh was deleted rather \
                 than never present"
            );
            return;
        }

        let asked = |git_ref: &str| {
            let out = Command::new("bash")
                .arg("scripts/gate.sh")
                .arg("--render-mode")
                .arg(git_ref)
                .current_dir(&root)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "--render-mode {git_ref} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_owned()
        };

        assert_eq!(asked("main"), "strict", "main publishes, so it is compared");
        assert_eq!(
            asked("fix/something"),
            "deterministic",
            "a branch has published nothing, so strict is red by construction"
        );
        // CI checks out a detached HEAD, where `git branch --show-current` is
        // empty. Empty is not `main`, and the question it can answer is the
        // second one.
        assert_eq!(asked(""), "deterministic", "a detached HEAD is not main");
    }

    /// Every tool a document names exists.
    ///
    /// A sibling project found a contract file asserting, in the present tense,
    /// that `scripts/validate_harness_docs.py` fails when three lists disagree
    /// — and that script had never been written. The drift it promised to
    /// prevent had happened: the document listed seventeen identities where the
    /// code had seven. That is worse than a check examining nothing, because a
    /// reader has no way to tell a described guard from a running one, and the
    /// description sat in the very file it claimed to protect.
    ///
    /// This repository has the rule in the other direction already — nothing
    /// shipped may name a document it does not carry. This is the same question
    /// asked of executables: a prose promise that something runs is checkable
    /// for free, and the failure it prevents is a reader trusting a guard that
    /// does not exist.
    ///
    /// Deliberately only `scripts/` and `tools/`: those are the paths this
    /// estate's prose promises *behaviour* of. A missing source file is a
    /// compile error and needs no test.
    #[test]
    fn every_tool_a_document_names_exists() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

        // `CHANGELOG.md` is excluded, and the exclusion is a measurement rather
        // than a convenience — the first version of this test failed on all
        // seven published trees and the failure was real information.
        //
        // That file is not written here. It is a ledger kept in the source
        // repository and rendered into each published tree, and its entries
        // describe work done *there*: three of them name
        // `scripts/check_render.sh`, `scripts/check_citations.sh` and
        // `tools/build_nddev_builder.py`, none of which a published tree ships.
        // The entries are correct about the repository they describe. The
        // reader is the one who cannot tell, so the rendered changelog now says
        // in its own header which repository those paths belong to.
        //
        // What this test is for is the other thing: **a hand-written document
        // promising a guard that does not exist.** A sibling project had a
        // contract file asserting in the present tense that a validation script
        // fails when three lists disagree, and that script had never been
        // written. A generated ledger cannot make that mistake; a person can,
        // which is why every other document here is still read.
        let generated = "CHANGELOG.md";

        let mut prose = String::new();
        let mut read_all = |from: &std::path::Path| {
            if let Ok(entries) = std::fs::read_dir(from) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "md")
                        && path.file_name().is_some_and(|name| name != generated)
                        && let Ok(text) = std::fs::read_to_string(&path)
                    {
                        prose.push_str(&text);
                        prose.push('\n');
                    }
                }
            }
        };
        read_all(&root.join("docs"));
        read_all(&root);

        let mut missing = Vec::new();
        for word in prose.split(|c: char| !(c.is_alphanumeric() || "._/-".contains(c))) {
            let named = word.trim_matches(|c| c == '.' || c == '-');
            if !(named.starts_with("scripts/") || named.starts_with("tools/")) {
                continue;
            }
            // Asked through `extension` rather than by comparing the tail of
            // the string, which is the same correction `catalog::unsourced`
            // carries: a case-sensitive comparison decides differently on the
            // three systems this runs on, and the answer must not.
            let runnable = std::path::Path::new(named)
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("py") || extension.eq_ignore_ascii_case("sh")
                });
            if !runnable {
                continue;
            }
            if !root.join(named).is_file() && !missing.contains(&named.to_owned()) {
                missing.push(named.to_owned());
            }
        }
        assert!(
            missing.is_empty(),
            "these are named in this repository's prose and do not exist: {}",
            missing.join(", ")
        );
    }

    /// A workflow reading a count out of prose reads it out of a marker now,
    /// and this is what stops the marker being renamed back.
    ///
    /// Two sweeps report into an issue rather than failing, because both reach
    /// a vendor's server and a repository check that depends on someone else's
    /// uptime stops being read. Their counts reached the workflow through a
    /// regular expression over the human sentence, defaulting to zero when the
    /// match came back empty — so renaming one word in that sentence would have
    /// turned every failure into none, opened no issue, and reported the sweep
    /// as clean. Nothing anywhere said the prose was load-bearing.
    ///
    /// The tools print a `RESULT` line for a machine and a sentence for a
    /// person. This binds the first: the tool must print it, the workflow must
    /// read it, and the workflow must refuse when it is absent — because an
    /// absent measurement and a measurement of nothing are different states and
    /// only one of them is good news.
    #[test]
    fn the_reported_counts_come_from_a_marker_and_not_from_prose() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        if !root.join("tools").is_dir() {
            return; // A published tree ships neither tool nor workflow.
        }

        // The list is the source of the count below, rather than a number
        // typed beside it. A third sweep was added to this lane and the guard
        // failed with `left: 3, right: 2` -- correct, and one character from
        // being "fixed" by bumping the 2. A hand-written tally in a check is
        // the same defect the check exists to catch, one level up.
        let sweeps = [
            ("tools/validate_setup_schemas.py", "failed="),
            ("tools/conformance_report.py", "refused="),
            ("tools/check_vendored_kit.py", "behind="),
            ("tools/check_authored_keys.py", "unsourced="),
        ];

        for (tool, key) in sweeps {
            let source = std::fs::read_to_string(root.join(tool))
                .unwrap_or_else(|_| panic!("{tool} is missing"));
            assert!(
                source.contains("\"RESULT ") || source.contains("f\"RESULT "),
                "{tool} no longer prints a RESULT line, and the workflow reads one"
            );
            assert!(
                source.contains(key),
                "{tool}'s RESULT line no longer carries {key}"
            );
        }

        let workflow = std::fs::read_to_string(root.join(".github/workflows/conformance.yml"))
            .expect("the conformance workflow is missing");
        assert_eq!(
            workflow.matches("^RESULT").count(),
            sweeps.len(),
            "every sweep must read its count from the marker, anchored at the \
             start of the line"
        );
        assert_eq!(
            workflow.matches("so its verdict is unknown").count(),
            sweeps.len(),
            "every sweep must refuse a missing marker rather than assume zero"
        );
        assert!(
            !workflow.contains(":-0}"),
            "a count defaulting to zero is an absent measurement reported as a \
             clean one, which is the defect this test exists for"
        );
    }
}
