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
//! The software lifecycle and `launch` are optional in the contract and are not
//! declared at all. Declaring an optional operation this runtime cannot perform
//! would let a consumer call something that cannot be honoured, which is worse
//! than not offering it.

pub mod catalog;
pub mod expiry;
pub mod facts;
pub mod human;
pub mod wire;

pub use catalog::{Catalog, Setup};
pub use facts::{BACKUP_SLOTS, BUNDLE_FORMAT, Harness};
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
    println!();
    println!();
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
    println!();
    println!("Every one takes an explicit --target. There is no default: a change");
    println!("aimed at a guessed path is a change aimed at someone else's state.");
    println!();
    println!("A backup is captured before every change, so `restore` always has");
    println!("something to return to. Over the wire, install and replace arrive as");
    println!("a bundle and refuse -- this build reads setups from its own catalog.");
}
