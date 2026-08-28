//! The plan artifact, its digest, and the responses that carry it.
//!
//! A plan is the provider's immutable description of an effect it has not yet
//! applied. `plan-operation` is always pure: it reads the target, decides, and
//! returns. `apply-operation` then receives that exact artifact back, together
//! with its digest, and refuses anything else.
//!
//! # Why the digest is over the artifact, not the response
//!
//! The consumer recomputes `digest_canonical(PLAN_DOMAIN, artifact)` and
//! compares. So the artifact must serialize to the same bytes on both sides —
//! that is what RFC 8785 is for — and the digest must cover only the artifact.
//! A digest over the whole response would change when a response field the plan
//! does not own changes, and the consumer's recomputation would never match.
//!
//! # Redundant echoes are the point
//!
//! `plan_digest` and `expected_target_digest` appear both inside the artifact
//! and beside it. That is not duplication for convenience: the consumer checks
//! them against its own inputs *before* it will build an operation, so a
//! provider that planned against a different target or a different bundle is
//! caught by disagreement rather than by trust.

use serde::{Deserialize, Serialize};
use setup_core::digest;

use crate::error::{Error, Result};
use crate::platform;
use crate::reason::WireReason;
use crate::vocabulary::TargetScope;
use crate::vocabulary::{Operation, PLAN_DOMAIN, PLAN_FORMAT, PROTOCOL_VERSION};

/// One literal bundle artifact and its independent logical identity.
///
/// The path is deliberately absent: identity is the format, the two digests and
/// the size. Where the bytes happen to sit on this machine is not part of what
/// two parties agree on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BundleBinding {
    /// The bundle format tag.
    pub bundle_format: String,
    /// The logical bundle digest.
    pub bundle_digest: String,
    /// The digest of the raw artifact bytes.
    pub artifact_digest: String,
    /// The exact artifact size in bytes.
    pub bundle_size: u64,
}

/// The artifact a software operation needs, stated before any network is open.
///
/// This is the whole reason the contract gives software a download phase of its
/// own. Planning names the exact bytes -- one url, one length, one digest --
/// while the provider is offline, and applying re-checks them while it is
/// offline again. Whoever holds the network in between fetches what this names
/// and nothing else, so no part of *what* gets installed is decided at a moment
/// when the answer could come from the network.
/// The five fields agreed on `ai_stp#414` and recorded in
/// `docs/contracts/provider-protocol.md`, and no others.
///
/// Everything the fetching side needs and nothing it does not. Whether the
/// bytes are the program or enclose it is this provider's business, decided
/// from the table compiled into it, and putting it here would invite a consumer
/// to act on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareArtifact {
    /// The platform this artifact is for, as the consumer spells it.
    pub platform: String,
    /// Where the bytes come from.
    pub url: String,
    /// The `sha256:`-prefixed digest of those bytes.
    pub sha256: String,
    /// How many bytes to expect.
    pub byte_length: u64,
    /// The path, relative to `--prefix`, that will run the program.
    pub entry_point: String,
}

/// The provider's immutable description of one effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanArtifact {
    /// Always [`PLAN_FORMAT`].
    pub format: String,
    /// Always [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// The provider that planned this.
    pub provider_id: String,
    /// The provider build version.
    pub provider_version: String,
    /// A digest of the provider's own build manifest.
    pub provider_build_digest: String,
    /// The release digest the consumer verified and passed in.
    pub provider_release_digest: String,
    /// The stable identifier of this operation.
    pub operation_id: String,
    /// The operation to be performed.
    pub operation: String,
    /// The canonical target directory.
    pub canonical_target: String,
    /// The target identity this plan was made against.
    pub expected_target_digest: String,
    /// The projection profile the provider declared.
    pub projection_profile_digest: String,
    /// The bundle this plan applies, when there is one.
    pub bundle: Option<BundleBinding>,
    /// The backup this plan reads or writes, when there is one.
    pub backup_ref: Option<String>,
    /// The target identity a restore will produce. Restore only.
    pub restore_target_digest: Option<String>,
    /// The permission profile to apply, when one was requested.
    pub permission_profile: Option<String>,
    /// The scope the consumer resolved this target to be, when it said.
    ///
    /// Recorded so `apply` can act on it. `plan-operation` accepts
    /// `--target-scope`; `apply-operation` takes a plan and not a scope,
    /// because a scope on both would be a second statement of a settled fact
    /// and the two could disagree. So the plan is where it travels.
    ///
    /// Omitted entirely when the consumer did not say, which is every
    /// invocation today: nothing sends the flag until a consumer's release
    /// does, and a plan without the key is byte-identical to what this build
    /// produced before the key existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_scope: Option<String>,
    /// The operating system and architecture that planned this.
    pub platform: serde_json::Value,
    /// When this plan stops being applicable.
    pub expires_at: String,
    /// The artifacts a software operation will fetch and install.
    ///
    /// One element is one file. `apply` receives one `--software-artifact` per
    /// element in this order, so which file answers which entry never has to be
    /// inferred. Empty for every configuration operation, which reaches nothing,
    /// and for `software_remove`, which downloads nothing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub software_artifacts: Vec<SoftwareArtifact>,
    /// What applying it will do, in order. Never empty.
    pub effects: Vec<String>,
}

/// Everything a caller must supply to build a plan artifact.
#[derive(Debug, Clone)]
pub struct PlanInputs<'a> {
    /// The provider identity.
    pub provider_id: &'a str,
    /// The provider build version.
    pub provider_version: &'a str,
    /// A digest of the provider's own build manifest.
    pub provider_build_digest: &'a str,
    /// The release digest the consumer verified.
    pub provider_release_digest: &'a str,
    /// The stable operation identifier the consumer minted.
    pub operation_id: &'a str,
    /// The operation to plan.
    pub operation: Operation,
    /// The canonical target directory, already resolved.
    pub canonical_target: &'a str,
    /// The target identity observed while planning.
    pub expected_target_digest: &'a str,
    /// The declared projection profile digest.
    pub projection_profile_digest: &'a str,
    /// The bundle, when the operation carries one.
    pub bundle: Option<BundleBinding>,
    /// The backup, when the operation reads or writes one.
    pub backup_ref: Option<String>,
    /// The identity a restore will produce. Required for restore, refused otherwise.
    pub restore_target_digest: Option<String>,
    /// The permission profile, when one was requested.
    pub permission_profile: Option<String>,
    /// The scope the consumer resolved this target to be, when it said.
    pub target_scope: Option<TargetScope>,
    /// When the plan expires.
    pub expires_at: &'a str,
    /// The artifacts a software operation will fetch, in the order `apply`
    /// will be handed them.
    pub software_artifacts: Vec<SoftwareArtifact>,
    /// What applying it will do. Never empty.
    pub effects: Vec<String>,
}

impl PlanArtifact {
    /// Build a plan artifact, refusing a shape the consumer would reject.
    ///
    /// # Errors
    ///
    /// Refuses an empty effect list, a restore with no result digest, and a
    /// non-restore that names one. Each of those is checked by the consumer
    /// too; failing here means the provider never emits a plan it knows is bad.
    pub fn new(inputs: PlanInputs<'_>) -> Result<Self> {
        if inputs.effects.is_empty() || inputs.effects.iter().any(String::is_empty) {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                "a plan must enumerate at least one non-empty effect",
            ));
        }
        let restores = inputs.operation.requires_restore_target_digest();
        match (&inputs.restore_target_digest, restores) {
            (None, true) => {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    "a restore plan must name the exact target it will produce",
                ));
            }
            (Some(_), false) => {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    format!(
                        "a {} plan must not name a restored target digest",
                        inputs.operation
                    ),
                ));
            }
            _ => {}
        }

        Ok(Self {
            format: PLAN_FORMAT.to_owned(),
            protocol_version: PROTOCOL_VERSION,
            provider_id: inputs.provider_id.to_owned(),
            provider_version: inputs.provider_version.to_owned(),
            provider_build_digest: inputs.provider_build_digest.to_owned(),
            provider_release_digest: inputs.provider_release_digest.to_owned(),
            operation_id: inputs.operation_id.to_owned(),
            operation: inputs.operation.as_str().to_owned(),
            canonical_target: inputs.canonical_target.to_owned(),
            expected_target_digest: inputs.expected_target_digest.to_owned(),
            projection_profile_digest: inputs.projection_profile_digest.to_owned(),
            target_scope: inputs.target_scope.map(|scope| scope.as_str().to_owned()),
            bundle: inputs.bundle,
            backup_ref: inputs.backup_ref,
            restore_target_digest: inputs.restore_target_digest,
            permission_profile: inputs.permission_profile,
            platform: platform::echo(),
            expires_at: inputs.expires_at.to_owned(),
            software_artifacts: inputs.software_artifacts,
            effects: inputs.effects,
        })
    }

    /// The digest that binds this exact artifact inside the plan domain.
    ///
    /// # Errors
    ///
    /// Propagates a canonicalization refusal.
    pub fn digest(&self) -> Result<String> {
        let value = serde_json::to_value(self).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("the plan artifact cannot be encoded: {source}"),
            )
        })?;
        digest::of_domain_canonical_json(PLAN_DOMAIN, &value).map_err(Error::from)
    }

    /// The complete `plan-operation` response, with its redundant echoes.
    ///
    /// # Errors
    ///
    /// Propagates a digest failure.
    pub fn into_response(self) -> Result<serde_json::Value> {
        let plan_digest = self.digest()?;
        let mut response = serde_json::Map::new();
        response.insert("state".to_owned(), serde_json::json!("planned"));
        response.insert("plan_digest".to_owned(), serde_json::json!(plan_digest));
        response.insert(
            "effects".to_owned(),
            serde_json::json!(self.effects.clone()),
        );
        response.insert(
            "expected_target_digest".to_owned(),
            serde_json::json!(self.expected_target_digest.clone()),
        );
        if let Some(bundle) = self.bundle.clone() {
            insert_bundle_echo(&mut response, &bundle);
            response.insert("valid".to_owned(), serde_json::json!(true));
        }
        let artifact = serde_json::to_value(self).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("the plan artifact cannot be encoded: {source}"),
            )
        })?;
        response.insert("plan".to_owned(), artifact);
        Ok(serde_json::Value::Object(response))
    }
}

/// The `validate-bundle` answer for a bundle this provider accepts.
#[must_use]
pub fn bundle_accepted(bundle: &BundleBinding) -> serde_json::Value {
    let mut response = serde_json::Map::new();
    insert_bundle_echo(&mut response, bundle);
    response.insert("valid".to_owned(), serde_json::json!(true));
    serde_json::Value::Object(response)
}

/// The `validate-bundle` answer for a bundle this provider refuses.
///
/// The echoes are present on a refusal too. Without them the consumer cannot
/// tell whether the refusal concerns the bytes it sent or some other bundle.
#[must_use]
pub fn bundle_rejected(bundle: &BundleBinding, reason: WireReason) -> serde_json::Value {
    rejected_with_detail(bundle, reason, None)
}

/// The `validate-bundle` refusal, with the detail that explains it.
///
/// The consumer decides from `reason` alone, and the detail is for the person
/// reading afterwards. A refusal carrying only a code is correct and nearly
/// useless: it says a bundle was wrong without saying which part.
#[must_use]
pub fn rejected_with_detail(
    bundle: &BundleBinding,
    reason: WireReason,
    detail: Option<&str>,
) -> serde_json::Value {
    let mut response = serde_json::Map::new();
    insert_bundle_echo(&mut response, bundle);
    response.insert("rejected".to_owned(), serde_json::json!(true));
    response.insert("reason".to_owned(), serde_json::json!(reason.as_str()));
    if let Some(text) = detail {
        response.insert("detail".to_owned(), serde_json::json!(text));
    }
    serde_json::Value::Object(response)
}

fn insert_bundle_echo(
    response: &mut serde_json::Map<String, serde_json::Value>,
    b: &BundleBinding,
) {
    response.insert(
        "bundle_format".to_owned(),
        serde_json::json!(b.bundle_format),
    );
    response.insert(
        "bundle_digest".to_owned(),
        serde_json::json!(b.bundle_digest),
    );
    response.insert(
        "artifact_digest".to_owned(),
        serde_json::json!(b.artifact_digest),
    );
    response.insert("bundle_size".to_owned(), serde_json::json!(b.bundle_size));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    const DIGEST: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    fn binding() -> BundleBinding {
        BundleBinding {
            bundle_format: "ai-stp-bundle/1".to_owned(),
            bundle_digest: DIGEST.to_owned(),
            artifact_digest: DIGEST.to_owned(),
            bundle_size: 4096,
        }
    }

    fn inputs(operation: Operation) -> PlanInputs<'static> {
        PlanInputs {
            target_scope: None,
            software_artifacts: Vec::new(),
            provider_id: "claude-setup-system",
            provider_version: "0.1.0",
            provider_build_digest: DIGEST,
            provider_release_digest: DIGEST,
            operation_id: "operation_01TEST",
            operation,
            canonical_target: "/tmp/target",
            expected_target_digest: DIGEST,
            projection_profile_digest: DIGEST,
            bundle: Some(binding()),
            backup_ref: Some("slot-000000000001".to_owned()),
            restore_target_digest: None,
            permission_profile: Some("default".to_owned()),
            expires_at: "2026-08-23T15:00:00Z",
            effects: vec!["write settings.json".to_owned()],
        }
    }

    #[test]
    fn the_artifact_carries_exactly_the_members_the_consumer_compares() {
        let artifact = PlanArtifact::new(inputs(Operation::Install)).unwrap();
        let encoded = serde_json::to_value(&artifact).unwrap();
        let mut present: Vec<&str> = encoded
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        present.sort_unstable();
        assert_eq!(
            present,
            vec![
                "backup_ref",
                "bundle",
                "canonical_target",
                "effects",
                "expected_target_digest",
                "expires_at",
                "format",
                "operation",
                "operation_id",
                "permission_profile",
                "platform",
                "projection_profile_digest",
                "protocol_version",
                "provider_build_digest",
                "provider_id",
                "provider_release_digest",
                "provider_version",
                "restore_target_digest",
            ]
        );
    }

    #[test]
    fn the_digest_is_reproducible_and_domain_separated() {
        let artifact = PlanArtifact::new(inputs(Operation::Install)).unwrap();
        let once = artifact.digest().unwrap();
        assert_eq!(once, artifact.digest().unwrap());
        assert!(once.starts_with("sha256:"));

        let value = serde_json::to_value(&artifact).unwrap();
        assert_ne!(once, setup_core::digest::of_canonical_json(&value).unwrap());
    }

    #[test]
    fn changing_one_planned_field_changes_the_digest() {
        let base = PlanArtifact::new(inputs(Operation::Install))
            .unwrap()
            .digest()
            .unwrap();
        let mut other = inputs(Operation::Install);
        other.operation_id = "operation_01OTHER";
        let changed = PlanArtifact::new(other).unwrap().digest().unwrap();
        assert_ne!(base, changed);
    }

    #[test]
    fn a_restore_plan_must_name_the_target_it_will_produce() {
        let error = PlanArtifact::new(inputs(Operation::Restore)).unwrap_err();
        assert!(error.detail().contains("restore plan"));

        let mut good = inputs(Operation::Restore);
        good.restore_target_digest = Some(DIGEST.to_owned());
        assert!(PlanArtifact::new(good).is_ok());
    }

    #[test]
    fn a_non_restore_plan_must_not_name_one() {
        let mut wrong = inputs(Operation::Install);
        wrong.restore_target_digest = Some(DIGEST.to_owned());
        let error = PlanArtifact::new(wrong).unwrap_err();
        assert!(error.detail().contains("must not name"));
    }

    #[test]
    fn an_empty_effect_list_is_refused_before_it_reaches_the_consumer() {
        let mut none = inputs(Operation::Install);
        none.effects = Vec::new();
        assert!(PlanArtifact::new(none).is_err());

        let mut blank = inputs(Operation::Install);
        blank.effects = vec![String::new()];
        assert!(PlanArtifact::new(blank).is_err());
    }

    #[test]
    fn the_response_repeats_the_digest_target_and_bundle_beside_the_plan() {
        let artifact = PlanArtifact::new(inputs(Operation::Install)).unwrap();
        let expected_digest = artifact.digest().unwrap();
        let response = artifact.into_response().unwrap();

        assert_eq!(response["state"], "planned");
        assert_eq!(response["plan_digest"], expected_digest.as_str());
        assert_eq!(response["expected_target_digest"], DIGEST);
        assert_eq!(
            response["effects"],
            serde_json::json!(["write settings.json"])
        );
        assert_eq!(response["bundle_format"], "ai-stp-bundle/1");
        assert_eq!(response["bundle_size"], 4096);
        assert_eq!(response["valid"], true);

        // The digest must bind the nested artifact, not the response around it.
        let nested = response["plan"].clone();
        assert_eq!(
            setup_core::digest::of_domain_canonical_json(PLAN_DOMAIN, &nested).unwrap(),
            expected_digest
        );
    }

    #[test]
    fn a_plan_without_a_bundle_carries_no_bundle_echo_or_validity_claim() {
        let mut none = inputs(Operation::Backup);
        none.bundle = None;
        let response = PlanArtifact::new(none).unwrap().into_response().unwrap();
        let object = response.as_object().unwrap();
        assert!(!object.contains_key("bundle_format"));
        assert!(!object.contains_key("valid"));
        assert_eq!(response["plan"]["bundle"], serde_json::Value::Null);
    }

    #[test]
    fn a_refusal_still_echoes_the_bytes_it_refuses() {
        let response = bundle_rejected(&binding(), WireReason::PathEscapesTarget);
        assert_eq!(response["rejected"], true);
        assert_eq!(response["reason"], "path_escapes_target");
        assert_eq!(response["bundle_digest"], DIGEST);
        assert_eq!(response["bundle_size"], 4096);
        assert!(response.get("valid").is_none());
    }

    #[test]
    fn every_reason_spelling_fits_the_pattern_the_consumer_accepts() {
        // The consumer reads `reason` only when it matches [a-z0-9_]{1,64}.
        for reason in WireReason::ALL {
            let text = reason.as_str();
            assert!((1..=64).contains(&text.len()), "{text} is out of range");
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{text} has a character the consumer will not read"
            );
        }
    }

    #[test]
    fn acceptance_and_refusal_are_never_both_claimed() {
        let accepted = bundle_accepted(&binding());
        assert_eq!(accepted["valid"], true);
        assert!(accepted.get("rejected").is_none());
    }
}
