//! The closed set of refusal reasons the wire may carry.
//!
//! A consumer matches on these strings, so a spelling is part of the contract
//! and never changes to suit a message. Three sources define the set and all
//! three are bound by test to the vendored kit:
//!
//! - `conformance-cases.json:bundle_rejections` — what a bundle can be wrong about
//! - `conformance-cases.json:capability_rejections` — what a provider cannot do
//! - `manifest.json:unsupported_reasons` — the capability half, stated again
//!
//! Two more come from the protocol prose rather than a list: `recovery_required`
//! and `stale`. Both are terminal answers a plan may give before any effect.

use std::fmt;

use setup_core::ReasonCode;

/// A refusal the provider is allowed to put on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WireReason {
    /// The bundle bytes do not hash to the digest that named them.
    DigestMismatch,
    /// The bundle exceeds a declared file-count or byte limit.
    LimitExceeded,
    /// The bundle contains a link, which is never materialized.
    LinkNotAllowed,
    /// The bundle names one path twice.
    PathDuplicate,
    /// A bundle path resolves outside the target.
    PathEscapesTarget,
    /// A bundle path is absolute or otherwise not relative.
    PathNotRelative,
    /// The bundle contains a device, socket or other special file.
    SpecialFileNotAllowed,
    /// The bundle format is not one this provider reads.
    UnsupportedBundleFormat,
    /// A component kind outside the declared vocabulary.
    UnsupportedComponentKind,
    /// A native surface this provider does not own.
    UnsupportedNativeSurface,
    /// A protocol version this provider does not implement.
    UnsupportedProtocolVersion,
    /// The operation is not declared by this provider.
    UnsupportedOperation,
    /// The bundle was compiled for a different projection profile.
    ProjectionProfileMismatch,
    /// A permission profile this provider does not offer.
    UnsupportedPermissionProfile,
    /// The running operating system is outside the declared matrix.
    UnsupportedPlatform,
    /// The running architecture is outside the declared matrix.
    UnsupportedArchitecture,
    /// Unresolved mutation state is present; only recovery may clear it.
    RecoveryRequired,
    /// The target moved after the lock was taken; no effect was applied.
    Stale,
    /// The provider could not complete the request for a local reason.
    ///
    /// This is the honest answer when a refusal is real but does not belong to
    /// any contract category — a lock held elsewhere, unreadable state, a target
    /// that is not a usable directory. Reporting one of those as a contract
    /// reason would tell the consumer something false about its own request.
    ProviderUnavailable,
}

impl WireReason {
    /// Every reason, in wire order.
    pub const ALL: &'static [Self] = &[
        Self::DigestMismatch,
        Self::LimitExceeded,
        Self::LinkNotAllowed,
        Self::PathDuplicate,
        Self::PathEscapesTarget,
        Self::PathNotRelative,
        Self::SpecialFileNotAllowed,
        Self::UnsupportedBundleFormat,
        Self::UnsupportedComponentKind,
        Self::UnsupportedNativeSurface,
        Self::UnsupportedProtocolVersion,
        Self::UnsupportedOperation,
        Self::ProjectionProfileMismatch,
        Self::UnsupportedPermissionProfile,
        Self::UnsupportedPlatform,
        Self::UnsupportedArchitecture,
        Self::RecoveryRequired,
        Self::Stale,
        Self::ProviderUnavailable,
    ];

    /// The exact string the wire carries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DigestMismatch => "digest_mismatch",
            Self::LimitExceeded => "limit_exceeded",
            Self::LinkNotAllowed => "link_not_allowed",
            Self::PathDuplicate => "path_duplicate",
            Self::PathEscapesTarget => "path_escapes_target",
            Self::PathNotRelative => "path_not_relative",
            Self::SpecialFileNotAllowed => "special_file_not_allowed",
            Self::UnsupportedBundleFormat => "unsupported_bundle_format",
            Self::UnsupportedComponentKind => "unsupported_component_kind",
            Self::UnsupportedNativeSurface => "unsupported_native_surface",
            Self::UnsupportedProtocolVersion => "unsupported_protocol_version",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::UnsupportedPermissionProfile => "unsupported_permission_profile",
            Self::ProjectionProfileMismatch => "projection_profile_mismatch",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::UnsupportedArchitecture => "unsupported_architecture",
            Self::RecoveryRequired => "recovery_required",
            Self::Stale => "stale",
            Self::ProviderUnavailable => "provider_unavailable",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|reason| reason.as_str() == text)
    }
}

impl fmt::Display for WireReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<ReasonCode> for WireReason {
    /// Translate a kernel refusal into the reason the consumer understands.
    ///
    /// The match is exhaustive on purpose: a new kernel reason will not compile
    /// until someone decides how it appears on the wire. Deciding that at the
    /// boundary is the point — the kernel's vocabulary is larger than the
    /// contract's, and collapsing the extra ones silently would report a
    /// contract category for a local failure.
    fn from(reason: ReasonCode) -> Self {
        match reason {
            ReasonCode::UnsupportedOperation => Self::UnsupportedOperation,
            ReasonCode::UnsupportedComponentKind => Self::UnsupportedComponentKind,
            ReasonCode::UnsupportedNativeSurface => Self::UnsupportedNativeSurface,
            ReasonCode::UnsupportedBundleFormat => Self::UnsupportedBundleFormat,
            ReasonCode::UnsupportedProtocolVersion => Self::UnsupportedProtocolVersion,
            ReasonCode::ProjectionProfileMismatch => Self::ProjectionProfileMismatch,
            ReasonCode::UnsupportedPermissionProfile => Self::UnsupportedPermissionProfile,
            ReasonCode::UnsupportedPlatform => Self::UnsupportedPlatform,
            ReasonCode::UnsupportedArchitecture => Self::UnsupportedArchitecture,
            ReasonCode::RecoveryRequired => Self::RecoveryRequired,
            ReasonCode::Stale => Self::Stale,
            ReasonCode::IntegrityMismatch => Self::DigestMismatch,
            // A lock held elsewhere, unreadable state, or a target that is not a
            // usable directory are all real refusals with no contract category.
            ReasonCode::InvalidTarget
            | ReasonCode::LockUnavailable
            | ReasonCode::StateUnavailable => Self::ProviderUnavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::vocabulary::kit::{json, strings};

    fn expected_reasons(group: &str) -> Vec<String> {
        json("conformance-cases.json")[group]
            .as_array()
            .unwrap_or_else(|| panic!("{group} is not an array"))
            .iter()
            .map(|case| case["expected_reason"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn every_bundle_rejection_the_kit_names_has_a_wire_reason() {
        for reason in expected_reasons("bundle_rejections") {
            assert!(
                WireReason::parse(&reason).is_some(),
                "no wire reason for {reason}"
            );
        }
    }

    #[test]
    fn every_capability_rejection_the_kit_names_has_a_wire_reason() {
        for reason in expected_reasons("capability_rejections") {
            assert!(
                WireReason::parse(&reason).is_some(),
                "no wire reason for {reason}"
            );
        }
    }

    #[test]
    fn every_unsupported_reason_in_the_manifest_has_a_wire_reason() {
        for reason in strings(&json("manifest.json"), "unsupported_reasons") {
            assert!(
                WireReason::parse(&reason).is_some(),
                "no wire reason for {reason}"
            );
        }
    }

    #[test]
    fn the_two_prose_reasons_are_present_and_not_confused_with_a_list_one() {
        assert_eq!(
            WireReason::parse("recovery_required"),
            Some(WireReason::RecoveryRequired)
        );
        assert_eq!(WireReason::parse("stale"), Some(WireReason::Stale));
    }

    #[test]
    fn spellings_are_unique_so_two_reasons_never_read_as_one() {
        let mut seen: Vec<&str> = WireReason::ALL.iter().map(|r| r.as_str()).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total);
    }

    #[test]
    fn a_kernel_lock_failure_is_not_dressed_up_as_a_contract_refusal() {
        // Telling the consumer "unsupported_operation" because a lock was busy
        // would make it change a request that was never wrong.
        assert_eq!(
            WireReason::from(ReasonCode::LockUnavailable),
            WireReason::ProviderUnavailable
        );
        assert_eq!(
            WireReason::from(ReasonCode::InvalidTarget),
            WireReason::ProviderUnavailable
        );
        assert_eq!(
            WireReason::from(ReasonCode::StateUnavailable),
            WireReason::ProviderUnavailable
        );
    }

    #[test]
    fn the_kernel_reasons_that_are_contract_reasons_keep_their_spelling() {
        assert_eq!(
            WireReason::from(ReasonCode::RecoveryRequired).as_str(),
            "recovery_required"
        );
        assert_eq!(WireReason::from(ReasonCode::Stale).as_str(), "stale");
        assert_eq!(
            WireReason::from(ReasonCode::UnsupportedPlatform).as_str(),
            "unsupported_platform"
        );
    }
}
