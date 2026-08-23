//! Two kinds of failure live at this boundary, and they are not the same.
//!
//! A **refusal** is an answer: the request was understood and declined, and the
//! consumer is told which contract reason applies. A **declaration error** is a
//! bug in this build — a `provider-info` that omits a core command, a projection
//! profile with no component kinds. It never reaches the wire as a reason,
//! because the consumer did nothing wrong and changing its request would not
//! help.

use std::fmt;

use crate::reason::WireReason;

/// A failure at the wire boundary.
#[derive(Debug)]
pub enum Error {
    /// The request was understood and declined for a contract reason.
    Refusal {
        /// The reason the consumer matches on.
        reason: WireReason,
        /// A human-readable detail. Never a second machine channel.
        detail: String,
    },
    /// This build declared something the contract does not permit.
    Declaration {
        /// What is wrong with the declaration.
        detail: String,
    },
}

impl Error {
    /// Decline a request for a contract reason.
    pub fn refuse(reason: WireReason, detail: impl Into<String>) -> Self {
        Self::Refusal {
            reason,
            detail: detail.into(),
        }
    }

    /// Report a defect in this build's own declaration.
    pub fn declaration(detail: impl Into<String>) -> Self {
        Self::Declaration {
            detail: detail.into(),
        }
    }

    /// The wire reason, when this failure is one the consumer can act on.
    ///
    /// A declaration error has none: it is this build's mistake, and naming a
    /// contract reason for it would send the consumer to fix its own request.
    #[must_use]
    pub const fn reason(&self) -> Option<WireReason> {
        match self {
            Self::Refusal { reason, .. } => Some(*reason),
            Self::Declaration { .. } => None,
        }
    }

    /// The human-readable detail.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Refusal { detail, .. } | Self::Declaration { detail } => detail,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refusal { reason, detail } => write!(f, "{reason}: {detail}"),
            Self::Declaration { detail } => write!(f, "provider declaration is invalid: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<setup_core::Error> for Error {
    fn from(error: setup_core::Error) -> Self {
        Self::refuse(WireReason::from(error.reason()), error.detail().to_owned())
    }
}

/// Result at the wire boundary.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_declaration_error_carries_no_wire_reason() {
        assert!(Error::declaration("no component kinds").reason().is_none());
    }

    #[test]
    fn a_refusal_carries_the_reason_the_consumer_matches_on() {
        let error = Error::refuse(WireReason::PathEscapesTarget, "../etc/passwd");
        assert_eq!(error.reason(), Some(WireReason::PathEscapesTarget));
        assert!(error.to_string().starts_with("path_escapes_target: "));
    }

    #[test]
    fn a_kernel_failure_becomes_a_refusal_with_its_translated_reason() {
        let kernel = setup_core::Error::new(setup_core::ReasonCode::Stale, "target moved");
        let error = Error::from(kernel);
        assert_eq!(error.reason(), Some(WireReason::Stale));
        assert_eq!(error.detail(), "target moved");
    }
}
