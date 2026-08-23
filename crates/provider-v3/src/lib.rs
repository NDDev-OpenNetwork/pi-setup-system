//! The ai-stp provider protocol v3 wire boundary.
//!
//! This crate is what a consumer talks to. It owns the argv contract, the closed
//! vocabulary, the exact echoes a response must repeat, and the refusal reasons
//! a consumer matches on. It owns no target: every effect belongs to
//! [`setup_core`], and this crate is the translation between one program's
//! arguments and that kernel's operations.
//!
//! # The one authority
//!
//! The vocabulary is owned by `provider-kit/v3/manifest.json`, vendored beside
//! this crate and verified against its own `SHA256SUMS`. The enums in
//! [`vocabulary`] exist only because a program must name its variants to match
//! on them; tests bind every set back to the manifest, so a kit that gains a
//! command and a build that does not is a test failure, not a silent divergence.
//!
//! Nothing in this crate restates a list the kit defines. Where a value must
//! appear in Rust, a test proves it is the same value.
//!
//! # How a call is shaped
//!
//! ```text
//! <executable> provider-info
//! <executable> <command> --target <absolute-resolved-dir> --json [command arguments]
//! ```
//!
//! `provider-info` is the exception: it describes the provider, not a target, so
//! it receives neither `--target` nor `--json`. Capabilities that depended on a
//! target would be useless for choosing one.
//!
//! # What a refusal means
//!
//! A refusal is an answer, not a crash. It names a reason from [`reason`], and
//! the consumer decides what to do from that reason alone — never by reading the
//! detail text. A failure in this build's own declaration is a different thing
//! entirely and never becomes a wire reason; see [`error::Error`].

pub mod error;
pub mod info;
pub mod plan;
pub mod platform;
pub mod reason;
pub mod vocabulary;

pub use error::{Error, Result};
pub use info::{Declaration, ProjectionProfile, ProviderInfo};
pub use plan::{BundleBinding, PlanArtifact, PlanInputs};
pub use reason::WireReason;
pub use vocabulary::{
    Command, ComponentKind, Operation, PLAN_DOMAIN, PLAN_FORMAT, PROJECTION_DOMAIN,
    PROTOCOL_VERSION, ProjectionKind,
};
