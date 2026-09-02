//! The `provider-info` answer, and the projection profile inside it.
//!
//! This is the only command that describes the provider rather than a target,
//! and it is what the consumer reads before deciding whether to call anything
//! else. Two properties make it usable:
//!
//! - **The profile digest binds the declaration, not the answer.** It is
//!   computed from the profile's own fields inside the projection domain, so a
//!   provider cannot quietly widen what it accepts while still reporting the
//!   digest a compiler built against.
//! - **The release digest is absent.** A provider reporting the digest of its
//!   own executable would be attesting to itself; the consumer verifies the
//!   release before invoking, and passes that digest *in*.

use serde::Serialize;
use setup_core::digest;

use crate::error::{Error, Result};
use crate::plan::EndState;
use crate::vocabulary::{
    Command, ComponentKind, Operation, PROJECTION_DOMAIN, PROTOCOL_VERSION, ProjectionKind,
    TargetScope,
};

/// What a provider accepts, and the digest that binds the declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionProfile {
    /// The profile identity a compiler builds against.
    pub profile_id: String,
    /// The digest of this declaration inside the projection domain.
    pub digest: String,
    /// The component kinds this provider projects.
    pub component_kinds: Vec<String>,
    /// The projection kinds this provider performs.
    pub projection_kinds: Vec<String>,
    /// The native identifier namespaces this provider owns.
    pub native_namespaces: Vec<String>,
    /// The bundle formats this provider reads.
    pub bundle_formats: Vec<String>,
    /// The largest file count a bundle may carry.
    pub max_files: u64,
    /// The largest byte count a bundle may carry.
    pub max_bytes: u64,
    /// The target this profile owns, when it is not the product's own home.
    ///
    /// Absent on the global profile and **serialized as absent**, so every
    /// declaration published before a provider owned a second scope stays byte
    /// for byte what it was and its digest does not move. A field added inside
    /// `projection_profile` would have given `projection_profile_mismatch` to
    /// every bundle compiled against the old one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_scope: Option<String>,
}

impl ProjectionProfile {
    /// Build a profile and compute the digest that binds it.
    ///
    /// # Errors
    ///
    /// Returns an error if the declaration is empty where the schema requires at
    /// least one member, or if the canonical form cannot be produced.
    pub fn new(
        profile_id: impl Into<String>,
        component_kinds: &[ComponentKind],
        projection_kinds: &[ProjectionKind],
        native_namespaces: &[&str],
        bundle_formats: &[&str],
        max_files: u64,
        max_bytes: u64,
    ) -> Result<Self> {
        Self::build(
            profile_id,
            component_kinds,
            projection_kinds,
            native_namespaces,
            bundle_formats,
            max_files,
            max_bytes,
            None,
        )
    }

    /// Build a profile that owns a target other than the product's own home.
    ///
    /// The scope is folded into the digest input, so two profiles differing
    /// only in which target they own cannot share an identity — the property
    /// that lets a plan record which profile it was made against.
    ///
    /// # Errors
    ///
    /// As [`ProjectionProfile::new`].
    #[allow(clippy::too_many_arguments)]
    pub fn scoped(
        profile_id: impl Into<String>,
        component_kinds: &[ComponentKind],
        projection_kinds: &[ProjectionKind],
        native_namespaces: &[&str],
        bundle_formats: &[&str],
        max_files: u64,
        max_bytes: u64,
        target_scope: TargetScope,
    ) -> Result<Self> {
        Self::build(
            profile_id,
            component_kinds,
            projection_kinds,
            native_namespaces,
            bundle_formats,
            max_files,
            max_bytes,
            Some(target_scope),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        profile_id: impl Into<String>,
        component_kinds: &[ComponentKind],
        projection_kinds: &[ProjectionKind],
        native_namespaces: &[&str],
        bundle_formats: &[&str],
        max_files: u64,
        max_bytes: u64,
        target_scope: Option<TargetScope>,
    ) -> Result<Self> {
        let profile_id = profile_id.into();
        let components: Vec<String> = component_kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect();
        let projections: Vec<String> = projection_kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect();
        let namespaces: Vec<String> = native_namespaces
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let formats: Vec<String> = bundle_formats
            .iter()
            .map(|name| (*name).to_owned())
            .collect();

        for (label, length) in [
            ("component_kinds", components.len()),
            ("projection_kinds", projections.len()),
            ("native_namespaces", namespaces.len()),
            ("bundle_formats", formats.len()),
        ] {
            if length == 0 {
                return Err(Error::declaration(format!(
                    "a projection profile must declare at least one {label}"
                )));
            }
        }
        if profile_id.is_empty() {
            return Err(Error::declaration(
                "a projection profile must have an identity",
            ));
        }
        if max_files == 0 || max_bytes == 0 {
            return Err(Error::declaration(
                "a projection profile limit cannot be zero",
            ));
        }

        // The digest input excludes `digest` itself: a value cannot bind a
        // declaration it is part of.
        let mut input = serde_json::json!({
            "profile_id": profile_id,
            "component_kinds": components,
            "projection_kinds": projections,
            "native_namespaces": namespaces,
            "bundle_formats": formats,
            "max_files": max_files,
            "max_bytes": max_bytes,
        });
        if let Some(scope) = target_scope {
            // Only when there is one. A global profile whose input gained a
            // null would hash differently from every declaration already
            // published, which is the one thing this field must not do.
            input["target_scope"] = serde_json::Value::String(scope.as_str().to_owned());
        }
        let digest = digest::of_domain_canonical_json(PROJECTION_DOMAIN, &input)
            .map_err(|source| Error::declaration(source.detail()))?;

        Ok(Self {
            profile_id,
            digest,
            component_kinds: components,
            projection_kinds: projections,
            native_namespaces: namespaces,
            bundle_formats: formats,
            max_files,
            max_bytes,
            target_scope: target_scope.map(|scope| scope.as_str().to_owned()),
        })
    }
}

/// The complete `provider-info` answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderInfo {
    /// Always [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// The provider identity.
    pub provider_id: String,
    /// The harness this provider configures.
    pub harness_id: String,
    /// The provider build version.
    pub provider_version: String,
    /// A digest of the provider's own build manifest.
    pub provider_build_digest: String,
    /// The commands this build implements.
    pub supported_commands: Vec<String>,
    /// The operations this build declares.
    pub supported_operations: Vec<String>,
    /// The operating systems this build supports.
    pub supported_os: Vec<String>,
    /// The architectures this build supports.
    pub supported_arch: Vec<String>,
    /// The permission profiles this provider can apply.
    pub permission_profiles: Vec<String>,
    /// What this provider accepts, and the digest binding it.
    pub projection_profile: ProjectionProfile,
    /// What it accepts for targets that are not the product's own home.
    ///
    /// Omitted entirely when empty, so a build owning one scope publishes
    /// exactly what it published before this field existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scoped_projection_profiles: Vec<ProjectionProfile>,
    /// The arguments `status` accepts beside its target, once the kit names
    /// the member.
    ///
    /// Empty and therefore absent until kit `0.2.9` publishes the member and
    /// a released consumer accepts it -- the same two gates every
    /// `provider-info` field has to pass, because the field set is compared
    /// for exact equality and an unknown member refuses the whole document.
    /// The runtime already honours `status --target-scope` (0.0.55); this is
    /// the sentence that lets a consumer send it. Agreed with the consumer on
    /// 2026-09-02 after their project-scope branch found that a workspace
    /// nobody has installed into has no record `status` could read a scope
    /// from, while the plan it is bound to is made under one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub status_request_fields: Vec<String>,
    /// The request-side arguments this release accepts.
    ///
    /// **A provider says what it will tolerate, so a consumer can send it.** A
    /// request field runs the opposite way to a response field: a consumer that
    /// starts sending an unknown flag makes every older provider refuse the
    /// invocation outright, so tolerance has to be published before anything
    /// sends. Measured before the acceptance existed:
    /// `--target-scope is not an argument of this command`.
    ///
    /// The consumer sends `--target-scope` only where this list names it, so an
    /// older release that does not publish the field is never sent the flag.
    /// Omitted entirely when empty, which is what a build declaring no
    /// request-side arguments publishes -- byte-identical to what it published
    /// before this field existed.
    ///
    /// Not "which scopes this build knows". That is a different statement, and
    /// argv makes it: an unknown scope is refused by name rather than served a
    /// `global` plan.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan_request_fields: Vec<String>,
}

/// What a build declares about itself, by name rather than by position.
///
/// Ten positional arguments are ten chances to swap two strings that both
/// compile. Naming them moves that mistake to the call site, where it reads
/// wrong instead of running wrong.
#[derive(Debug, Clone)]
pub struct Declaration<'a> {
    /// The provider identity.
    pub provider_id: &'a str,
    /// The harness this provider configures.
    pub harness_id: &'a str,
    /// The provider build version.
    pub provider_version: &'a str,
    /// A digest of the provider's own build manifest.
    pub provider_build_digest: &'a str,
    /// The commands this build implements.
    pub commands: &'a [Command],
    /// The operations this build declares.
    pub operations: &'a [Operation],
    /// The operating systems this build supports.
    pub supported_os: &'a [&'a str],
    /// The architectures this build supports.
    pub supported_arch: &'a [&'a str],
    /// The permission profiles this provider can apply.
    pub permission_profiles: &'a [&'a str],
    /// What this provider accepts, and the digest binding it.
    pub projection_profile: ProjectionProfile,
    /// What it accepts for targets that are not the product's own home.
    pub scoped_projection_profiles: Vec<ProjectionProfile>,
}

impl ProviderInfo {
    /// Assemble an answer and check it against the contract's own bounds.
    ///
    /// # Errors
    ///
    /// Returns an error when a declaration would be refused by the published
    /// schema — a missing core command, fewer than the five core operations, or
    /// an empty platform matrix. Failing here rather than at the consumer keeps
    /// the mistake inside the build that made it.
    pub fn declare(declaration: Declaration<'_>) -> Result<Self> {
        for required in Command::CORE {
            if !declaration.commands.contains(required) {
                return Err(Error::declaration(format!(
                    "provider-info omits the core command {required}"
                )));
            }
        }
        for required in Operation::CORE {
            if !declaration.operations.contains(required) {
                return Err(Error::declaration(format!(
                    "provider-info omits the core operation {required}"
                )));
            }
        }
        if declaration.supported_os.is_empty() || declaration.supported_arch.is_empty() {
            return Err(Error::declaration(
                "provider-info must name at least one operating system and architecture",
            ));
        }

        // A scope may be claimed once. Two entries owning one target would make
        // "which profile was this plan made against" unanswerable, and the
        // consumer refuses the whole declaration for it -- which takes `fetch`,
        // `plan`, `apply` and `status` down with `provider-info`, so the mistake
        // is worth catching in the build that made it.
        let mut claimed: Vec<&str> = Vec::new();
        for profile in &declaration.scoped_projection_profiles {
            let Some(scope) = profile.target_scope.as_deref() else {
                return Err(Error::declaration(
                    "a scoped projection profile must name the target it owns; the \
                     global scope is declared by projection_profile",
                ));
            };
            if claimed.contains(&scope) {
                return Err(Error::declaration(format!(
                    "two projection profiles claim the {scope} scope"
                )));
            }
            claimed.push(scope);
        }

        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            provider_id: declaration.provider_id.to_owned(),
            harness_id: declaration.harness_id.to_owned(),
            provider_version: declaration.provider_version.to_owned(),
            provider_build_digest: declaration.provider_build_digest.to_owned(),
            supported_commands: declaration
                .commands
                .iter()
                .map(|c| c.as_str().to_owned())
                .collect(),
            supported_operations: declaration
                .operations
                .iter()
                .map(|o| o.as_str().to_owned())
                .collect(),
            supported_os: declaration
                .supported_os
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            supported_arch: declaration
                .supported_arch
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            permission_profiles: declaration
                .permission_profiles
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            projection_profile: declaration.projection_profile,
            scoped_projection_profiles: declaration.scoped_projection_profiles,
            // **Declared since 2026-08-29**, and it was held for a month
            // before that because the kit blessed the field and the released
            // runner did not know it. With it declared, `ai-stp-cli 0.0.7`
            // refused **all seven** with `provider-info fields differ from the
            // closed v3 schema`; trading one conformance failure for seven was
            // not a trade, so the constant and the type sat here and the
            // declaration stayed empty.
            //
            // `0.0.8` ships `PLAN_REQUEST_FIELDS = {"target_scope"}`, read out
            // of the installed package rather than off a release note, and all
            // seven conform against it. The rule the wait produced is worth
            // more than the field: **a kit blessing a field is permission to
            // *build* against it; a released runner accepting it is permission
            // to *emit* it.** A state key needs only the first -- that is why
            // `written_paths` shipped the day the kit named it -- and a
            // `provider-info` field needs both, because the field set is
            // compared for exact equality.
            //
            // `end_state` followed the same order on 2026-09-02: kit `0.2.8`
            // opened the enum to it, `ai-stp-cli 0.0.14` on PyPI carries
            // `PLAN_REQUEST_FIELDS = {"target_scope", "end_state"}` (read out of
            // the installed wheel), and only then is it declared here. Every
            // build declares it because the runtime that honours it is shared:
            // a remove may carry a bundle of surviving bytes on every harness.
            plan_request_fields: vec![
                TargetScope::REQUEST_FIELD.to_owned(),
                EndState::REQUEST_FIELD.to_owned(),
            ],
            // Held until the kit names it; see the field's own note. The test
            // beside `plan_request_fields`' flips this the day it does.
            status_request_fields: Vec::new(),
        })
    }

    /// Whether this build declares an operation.
    #[must_use]
    pub fn declares(&self, operation: Operation) -> bool {
        self.supported_operations
            .iter()
            .any(|name| name == operation.as_str())
    }

    /// Whether this build runs on the host it is executing on.
    #[must_use]
    pub fn supports_this_host(&self) -> bool {
        self.supported_os
            .iter()
            .any(|name| name == crate::platform::os())
            && self
                .supported_arch
                .iter()
                .any(|name| name == crate::platform::arch())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn profile() -> ProjectionProfile {
        ProjectionProfile::new(
            "claude/native-files/1",
            ComponentKind::ALL,
            &[ProjectionKind::NativeFiles, ProjectionKind::Marketplace],
            &["settings.json", "skills", "agents", "commands"],
            &["ai-stp-bundle/1"],
            4096,
            64 * 1024 * 1024,
        )
        .unwrap()
    }

    const ZERO_DIGEST: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn declaration<'a>(
        commands: &'a [Command],
        operations: &'a [Operation],
        supported_os: &'a [&'a str],
    ) -> Declaration<'a> {
        Declaration {
            provider_id: "claude-setup-system",
            harness_id: "claude",
            provider_version: "0.1.0",
            provider_build_digest: ZERO_DIGEST,
            commands,
            operations,
            supported_os,
            supported_arch: &["x86_64", "arm64"],
            permission_profiles: &["default"],
            projection_profile: profile(),
            scoped_projection_profiles: Vec::new(),
        }
    }

    fn info(commands: &[Command], operations: &[Operation]) -> Result<ProviderInfo> {
        ProviderInfo::declare(declaration(
            commands,
            operations,
            &["linux", "macos", "windows"],
        ))
    }

    #[test]
    fn the_profile_digest_is_reproducible_from_the_same_declaration() {
        assert_eq!(profile().digest, profile().digest);
        assert!(profile().digest.starts_with("sha256:"));
    }

    #[test]
    fn changing_any_declared_field_changes_the_digest_that_binds_it() {
        let base = profile().digest;
        let narrower = ProjectionProfile::new(
            "claude/native-files/1",
            &[ComponentKind::Instruction],
            &[ProjectionKind::NativeFiles, ProjectionKind::Marketplace],
            &["settings.json", "skills", "agents", "commands"],
            &["ai-stp-bundle/1"],
            4096,
            64 * 1024 * 1024,
        )
        .unwrap();
        assert_ne!(narrower.digest, base);

        let looser = ProjectionProfile::new(
            "claude/native-files/1",
            ComponentKind::ALL,
            &[ProjectionKind::NativeFiles, ProjectionKind::Marketplace],
            &["settings.json", "skills", "agents", "commands"],
            &["ai-stp-bundle/1"],
            4097,
            64 * 1024 * 1024,
        )
        .unwrap();
        assert_ne!(looser.digest, base);
    }

    #[test]
    fn the_digest_is_not_part_of_its_own_input() {
        // If it were, no value could satisfy the computation.
        let one = profile();
        let two = profile();
        assert_eq!(one, two);
    }

    #[test]
    fn an_empty_declaration_is_refused_where_the_schema_requires_a_member() {
        assert!(
            ProjectionProfile::new(
                "id",
                &[],
                &[ProjectionKind::NativeFiles],
                &["x"],
                &["ai-stp-bundle/1"],
                1,
                1
            )
            .is_err()
        );
        assert!(
            ProjectionProfile::new("id", ComponentKind::ALL, &[], &["x"], &["y"], 1, 1).is_err()
        );
        assert!(
            ProjectionProfile::new(
                "",
                ComponentKind::ALL,
                ProjectionKind::ALL,
                &["x"],
                &["y"],
                1,
                1
            )
            .is_err()
        );
        assert!(
            ProjectionProfile::new(
                "id",
                ComponentKind::ALL,
                ProjectionKind::ALL,
                &["x"],
                &["y"],
                0,
                1
            )
            .is_err()
        );
    }

    #[test]
    fn a_build_that_omits_a_core_command_is_refused_in_its_own_build() {
        let missing: Vec<Command> = Command::CORE
            .iter()
            .copied()
            .filter(|c| *c != Command::Status)
            .collect();
        assert!(info(&missing, Operation::CORE).is_err());
    }

    #[test]
    fn a_build_that_omits_a_core_operation_is_refused_in_its_own_build() {
        let missing: Vec<Operation> = Operation::CORE
            .iter()
            .copied()
            .filter(|o| *o != Operation::Restore)
            .collect();
        assert!(info(Command::CORE, &missing).is_err());
    }

    #[test]
    fn a_core_only_build_satisfies_the_published_bounds() {
        let answer = info(Command::CORE, Operation::CORE).unwrap();
        assert_eq!(answer.protocol_version, 3);
        assert_eq!(answer.supported_commands.len(), 6);
        assert_eq!(answer.supported_operations.len(), 5);
        assert!(!answer.declares(Operation::Launch));
        assert!(!answer.declares(Operation::SoftwareInstall));
    }

    #[test]
    fn a_build_declaring_launch_stays_inside_the_seven_command_maximum() {
        let answer = info(Command::ALL, Operation::ALL).unwrap();
        assert_eq!(answer.supported_commands.len(), 7);
        assert!(answer.declares(Operation::Launch));
    }

    #[test]
    fn the_answer_serializes_with_exactly_the_members_the_schema_requires() {
        let answer = info(Command::CORE, Operation::CORE).unwrap();
        let encoded = serde_json::to_value(&answer).unwrap();
        let mut present: Vec<&str> = encoded
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        present.sort_unstable();

        let schema = crate::vocabulary::kit::json("provider-info.schema.json");
        let mut required: Vec<String> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        required.sort();

        // Two questions, and this used to ask one of them by asking for
        // equality: *are the required members all present* and *is every member
        // present one the schema allows*. Equality answered both only while
        // this build declared nothing optional, and it stopped being able to on
        // the day `plan_request_fields` was declared -- a member the schema
        // carries in `properties` and deliberately not in `required`.
        //
        // Asked separately now, and the pair is **stricter** than the equality
        // was: under `additionalProperties: false` the consumer compares the
        // field set exactly, so a member outside `properties` is refused by the
        // runner. The old assertion could not have caught that; it would have
        // failed on any optional member, allowed or not, which is a different
        // complaint that happens to fire at the same time.
        for member in &required {
            assert!(
                present.contains(&member.as_str()),
                "the schema requires {member} and this answer omits it"
            );
        }
        let allowed = schema["properties"].as_object().unwrap();
        assert_eq!(
            schema["additionalProperties"],
            serde_json::Value::Bool(false),
            "this test's second half only means something while the schema is closed"
        );
        for member in &present {
            assert!(
                allowed.contains_key(*member),
                "this answer publishes {member}, which the closed schema does not allow"
            );
        }
    }

    #[test]
    fn the_profile_serializes_with_exactly_the_members_the_schema_requires() {
        let encoded = serde_json::to_value(profile()).unwrap();
        let mut present: Vec<&str> = encoded
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        present.sort_unstable();

        let schema = crate::vocabulary::kit::json("provider-info.schema.json");
        let mut required: Vec<String> = schema["properties"]["projection_profile"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        required.sort();
        assert_eq!(present, required);
    }

    #[test]
    fn a_host_outside_the_declared_matrix_is_reported_as_unsupported() {
        let elsewhere =
            ProviderInfo::declare(declaration(Command::CORE, Operation::CORE, &["plan9"])).unwrap();
        assert!(!elsewhere.supports_this_host());
        assert!(
            info(Command::CORE, Operation::CORE)
                .unwrap()
                .supports_this_host()
        );
    }
    /// The scope is part of what the digest binds.
    ///
    /// Two profiles identical in every other field must not share an identity:
    /// a plan records which profile it was made against, and if a workspace
    /// profile and a home profile hashed alike, a plan made for one would
    /// verify against the other.
    #[test]
    fn a_scope_changes_the_identity_of_an_otherwise_identical_profile() {
        let fields = || {
            (
                &[ComponentKind::Skill][..],
                &[ProjectionKind::NativeFiles][..],
                &["skills"][..],
                &["ai-stp-bundle/1"][..],
            )
        };
        let (kinds, projections, namespaces, formats) = fields();
        let global =
            ProjectionProfile::new("same/1", kinds, projections, namespaces, formats, 8, 8)
                .unwrap();
        let scoped = ProjectionProfile::scoped(
            "same/1",
            kinds,
            projections,
            namespaces,
            formats,
            8,
            8,
            TargetScope::Project,
        )
        .unwrap();
        assert_ne!(global.digest, scoped.digest);
        assert_eq!(scoped.target_scope.as_deref(), Some("project"));
        assert_eq!(global.target_scope, None);
    }

    /// A declaration with no second scope is byte for byte what it always was.
    ///
    /// This is the property that lets a provider add the field without moving
    /// `projection_profile`'s digest, and therefore without giving
    /// `projection_profile_mismatch` to every bundle compiled against the old
    /// declaration. Worth a test rather than a serde attribute nobody reads
    /// twice.
    #[test]
    fn a_single_scope_declaration_publishes_no_trace_of_the_field() {
        let encoded = serde_json::to_value(info(Command::ALL, Operation::CORE).unwrap()).unwrap();
        let object = encoded.as_object().unwrap();

        // Asked of the structure rather than of the rendered string, and the
        // reason is a false positive this test produced the day
        // `plan_request_fields` was declared: its value **is** the string
        // `target_scope`, so a substring search found the field this test is
        // about in a field it is not about. The property has not changed --
        // a declaration with no second scope still carries neither key -- but
        // the question has to be asked of keys to mean that.
        assert!(
            !object.contains_key("scoped_projection_profiles"),
            "{encoded}"
        );
        assert!(
            !object["projection_profile"]
                .as_object()
                .unwrap()
                .contains_key("target_scope"),
            "{encoded}"
        );
    }

    /// The kit publishes the closed enum; a name declared outside it refuses
    /// the whole document on the consumer's side, and a name inside it that
    /// this build does not declare is a capability withheld -- which is a
    /// decision, so it is written down here rather than inferred from a diff.
    #[test]
    fn every_declared_request_field_is_one_the_kit_names_and_none_is_withheld() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../../provider-kit/v3/provider-info.schema.json"
        ))
        .unwrap();
        let mut published: Vec<&str> = schema["properties"]["plan_request_fields"]["items"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        published.sort_unstable();
        let mut declared = info(Command::ALL, Operation::CORE)
            .unwrap()
            .plan_request_fields;
        declared.sort_unstable();
        // Held back on purpose, with the reason beside the name. Empty since
        // 2026-09-02, when `end_state` shipped with the released runner that
        // reads it; a field goes here while the kit names it and no released
        // consumer accepts it, and comes out the day one does.
        let withheld: &[(&str, &str)] = &[];
        for (name, _reason) in withheld {
            assert!(
                published.contains(name),
                "{name} is withheld but the kit does not name it"
            );
        }
        let expected: Vec<&str> = published
            .iter()
            .copied()
            .filter(|name| !withheld.iter().any(|(held, _)| held == name))
            .collect();
        assert_eq!(
            declared, expected,
            "declared {declared:?}, kit {published:?}"
        );

        // The same rule for `status_request_fields`: while the kit's schema
        // has no such property, declaring anything would refuse the whole
        // document; the day it appears, this asserts the declaration follows.
        let status_declared = info(Command::ALL, Operation::CORE)
            .unwrap()
            .status_request_fields;
        match schema["properties"].get("status_request_fields") {
            None => assert!(
                status_declared.is_empty(),
                "the kit does not name status_request_fields and this build declares {status_declared:?}"
            ),
            Some(property) => {
                let mut named: Vec<&str> = property["items"]["enum"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                named.sort_unstable();
                let mut declared = status_declared;
                declared.sort_unstable();
                assert_eq!(declared, named, "the kit names {named:?} for status");
            }
        }
    }

    /// One target, one owner.
    #[test]
    fn two_profiles_cannot_claim_the_same_scope() {
        let scoped = |id: &str| {
            ProjectionProfile::scoped(
                id,
                &[ComponentKind::Skill],
                &[ProjectionKind::NativeFiles],
                &["skills"],
                &["ai-stp-bundle/1"],
                8,
                8,
                TargetScope::Project,
            )
            .unwrap()
        };
        let mut both = declaration(Command::ALL, Operation::CORE, &["linux"]);
        both.scoped_projection_profiles = vec![scoped("a/1"), scoped("b/1")];
        assert!(
            ProviderInfo::declare(both).is_err(),
            "two owners of one scope were accepted"
        );
    }
}
