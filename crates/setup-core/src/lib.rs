//! The kernel every NDDev setup system writes through.
//!
//! One write path serves both surfaces a setup system exposes: the ai-stp
//! provider wire commands and the human commands typed in a terminal. A human
//! command that reached the target directly would bypass the guarantees the
//! wire surface owes its consumer, so it does not exist.
//!
//! Configuration has two explicit recovery paths. The compatibility lifecycle
//! captures a slot for restore; [`forward`] retains only desired bytes and
//! resumes the same install/replace request. [`software`] manages program
//! publication and removal without configuration capture. All writers share
//! their target or prefix lock and validate durable preconditions.

pub mod archive;
pub mod backup;
pub mod canonical;
pub mod checksum;
pub mod digest;
pub mod error;
pub mod forward;
pub mod journal;
pub mod lock;
pub mod native_snapshot;
pub mod software;
pub mod software_prefix;
pub(crate) mod sqlite;
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
