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
use crate::vocabulary::{
    Command, ComponentKind, Operation, PROJECTION_DOMAIN, PROTOCOL_VERSION, ProjectionKind,
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
        let input = serde_json::json!({
            "profile_id": profile_id,
            "component_kinds": components,
            "projection_kinds": projections,
            "native_namespaces": namespaces,
            "bundle_formats": formats,
            "max_files": max_files,
            "max_bytes": max_bytes,
        });
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
        assert_eq!(present, required);
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
}
