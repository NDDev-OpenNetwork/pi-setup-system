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

pub mod backup;
pub mod canonical;
pub mod digest;
pub mod error;
pub mod journal;
pub mod lock;
pub mod stamp;
pub mod target;

pub use error::{Error, ReasonCode, Result};
