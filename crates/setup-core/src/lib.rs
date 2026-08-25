//! The kernel every NDDev setup system writes through.
//!
//! One write path serves both surfaces a setup system exposes: the ai-stp
//! provider wire commands and the human commands typed in a terminal. A human
//! command that reached the target directly would bypass the guarantees the
//! wire surface owes its consumer, so it does not exist.
//!
//! The sequence a mutation follows is fixed:
//!
//! ```text
//! resolve target -> acquire lock -> re-check preconditions
//!   -> write journal(prepared) -> capture backup -> stage -> promote
//!   -> journal(committed) -> verify -> clear
//! ```
//!
//! Every step is durable before the next begins, so an interrupted mutation
//! leaves evidence rather than ambiguity. [`journal`] owns what that evidence
//! means and which command is allowed to resolve it.

pub mod archive;
pub mod backup;
pub mod canonical;
pub mod checksum;
pub mod digest;
pub mod error;
pub mod journal;
pub mod lock;
pub mod software;
pub mod stamp;
pub mod target;

pub use error::{Error, ReasonCode, Result};

/// This host's operating system and architecture, in the consumer's spellings.
///
/// `provider-v3` owns the canonical answer and depends on this crate, so it
/// cannot be asked from here. The two are bound by a test rather than by an
/// import.
#[must_use]
pub fn platform_of_this_host() -> (&'static str, &'static str) {
    (
        std::env::consts::OS,
        match std::env::consts::ARCH {
            "aarch64" => "arm64",
            other => other,
        },
    )
}
