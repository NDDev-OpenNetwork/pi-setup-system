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
//! it only when it carries an artifact table -- so six of the seven do, and pi
//! does not, because npm resolves its dependency closure at install time and
//! there is no single artifact whose digest can be fixed in advance. `launch`
//! is declared by none. Declaring an optional operation this runtime cannot
//! perform would let a consumer call something that cannot be honoured, which
//! is worse than not offering it.

pub(crate) mod adopt;
pub mod catalog;
pub mod expiry;
pub mod facts;
pub mod human;
pub(crate) mod software;
pub mod wire;

pub use catalog::{Catalog, Setup};
// The software types belong to the kernel, but a setup system declares its
// artifact table and depends only on this crate. Re-exported so that stays
// true rather than widening seven dependency lists to reach past it.
pub use facts::{BACKUP_SLOTS, BUNDLE_FORMAT, Harness};
pub use setup_core::software::{Artifact, Delivery, Shape, Software};
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
    println!("something to return to. Over the wire, install and replace arrive as");
    println!("a bundle and refuse -- this build reads setups from its own catalog.");
}
