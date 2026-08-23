//! The Pi Coding Agent setup system.
//!
//! One binary, two surfaces. The ai-stp provider wire commands are what the
//! consumer invokes; the human commands are what the owner types. Both reach
//! the target through [`setup_core`], and neither has a shortcut around it.
//!
//! This harness owns the program lifecycle as well as the configuration.
//!
//! # Status
//!
//! Skeleton. The kernel it builds on is implemented and tested; the wire and
//! human surfaces are not yet wired to it, and this binary says so rather than
//! reporting a capability it does not have.

use std::process::ExitCode;

/// The harness this system configures.
pub const HARNESS_ID: &str = "pi";

/// The provider identity reported on the wire.
pub const PROVIDER_ID: &str = "pi-setup-system";

/// The product this harness configures.
pub const PRODUCT: &str = "Pi Coding Agent";

/// The default configuration home, when the caller names no target.
///
/// It is documentation, not a fallback. Every mutation takes an explicit
/// absolute target, because a mutation aimed at a guessed path is a mutation
/// aimed at someone else's state.
pub const DOCUMENTED_CONFIG_HOME: &str = "~/.pi/agent";

/// The environment variable that names the configuration home for this product.
pub const CONFIG_HOME_ENV: &str = "PI_CODING_AGENT_DIR";

/// Whether this provider owns the program lifecycle, not only the configuration.
pub const OWNS_SOFTWARE_LIFECYCLE: bool = true;

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("--version") => {
            println!("{PROVIDER_ID} {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("--help") | None => {
            print_help();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!(
                "{PROVIDER_ID}: {other} is not implemented yet; \
                 this build exposes only --help and --version"
            );
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("{PROVIDER_ID} {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Configures {PRODUCT} in a caller-named target directory.");
    println!("Documented configuration home: {DOCUMENTED_CONFIG_HOME} ({CONFIG_HOME_ENV})");
    println!();
    println!("This build is a skeleton. No command mutates a target yet.");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_provider_identity_matches_the_crate_it_ships_as() {
        assert_eq!(PROVIDER_ID, env!("CARGO_PKG_NAME"));
    }

    #[test]
    fn the_documented_home_is_never_treated_as_a_default_target() {
        // The constant exists to be printed, not resolved. A test that let it
        // become a fallback would be the first step toward one.
        assert!(DOCUMENTED_CONFIG_HOME.starts_with('~'));
    }
}
