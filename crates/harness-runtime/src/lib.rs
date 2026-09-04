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
        // That file is a release ledger whose older entries can name checks that
        // this clone does not carry. A generated ledger cannot promise a missing
        // guard the way a hand-written document can, which is why every other
        // document here is still read.
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
}
