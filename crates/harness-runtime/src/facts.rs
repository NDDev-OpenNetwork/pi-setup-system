//! What one harness is, stated once as data.
//!
//! Every setup system runs the same commands over the same kernel; what differs
//! is which directory it owns, which files inside it are its own, and which
//! files belong to the product and must be left alone. Those differences are
//! *facts about a product*, verified against its official documentation and
//! recorded in a baseline — not behaviour, and not code.
//!
//! Holding them as data rather than as five copies of a dispatcher means a
//! change to the shared logic lands in one place, and a change to a product's
//! surface lands in exactly one struct with a test binding it to that product's
//! baseline.

use provider_v3::{
    Command, ComponentKind, Declaration, Operation, ProjectionKind, ProjectionProfile, ProviderInfo,
};
use setup_core::digest;
use setup_core::software::{Delivery, Software};

/// One harness, as the runtime needs to know it.
#[derive(Debug, Clone, Copy)]
pub struct Harness {
    /// The harness identity on the wire.
    pub harness_id: &'static str,
    /// The provider identity on the wire, matching the crate name.
    pub provider_id: &'static str,
    /// The build version.
    pub version: &'static str,
    /// The product being configured.
    pub product: &'static str,
    /// Who publishes the product.
    pub vendor: &'static str,
    /// The documented configuration home. Documentation, never a fallback.
    pub documented_config_home: &'static str,
    /// The environment variable a product documents for its configuration home.
    ///
    /// Empty when the product documents none. That is a real state -- not every
    /// product offers an override -- and it is worth saying rather than
    /// inventing a plausible variable name that nothing reads.
    ///
    /// Documentation either way: nothing here resolves a path from it, because
    /// every command takes an explicit target.
    pub config_home_env: &'static str,
    /// The provider-owned control directory inside a target.
    pub control_directory: &'static str,
    /// The provider-owned state file inside a target.
    pub state_file: &'static str,
    /// The projection profile identity a compiler builds against.
    pub profile_id: &'static str,
    /// The top-level entries this provider owns inside a target.
    ///
    /// Everything else is a sibling overlay preserved verbatim.
    pub native_namespaces: &'static [&'static str],
    /// Product-owned paths this provider never reads and never writes.
    ///
    /// Excluded from backups so a slot never holds credentials, and excluded
    /// from target identity so the product's own traffic cannot strand a plan.
    pub never_touch: &'static [&'static str],
    /// The permission profiles this provider can apply.
    pub permission_profiles: &'static [&'static str],
    /// The component kinds this provider projects.
    pub component_kinds: &'static [ComponentKind],
    /// The projection kinds this provider performs.
    pub projection_kinds: &'static [ProjectionKind],
    /// The largest file count a bundle may carry.
    pub max_files: u64,
    /// The largest byte count a bundle may carry.
    pub max_bytes: u64,
    /// The exact provider-kit revision this build was compiled against.
    pub kit_identity: &'static str,
    /// How the product's own software is installed, when this build can do it.
    ///
    /// `None` means the software lifecycle is not offered at all. So does a
    /// [`Delivery::Manager`], which is a different statement -- the product is
    /// installable, but not by fetching bytes whose digest was fixed in advance
    /// -- and the refusal says which.
    pub software: Option<Software>,
}

/// How many backup slots a target keeps.
pub const BACKUP_SLOTS: usize = 10;

/// The bundle format every setup system reads.
pub const BUNDLE_FORMAT: &str = "ai-stp-bundle/1";

impl Harness {
    /// Whether one relative path falls inside a namespace this harness claims.
    ///
    /// A namespace is not always a single path component. Codex routes skills
    /// to `.agents/skills` while owning nothing else under `.agents`, and
    /// Antigravity is a guest inside `~/.gemini` where every namespace is
    /// nested. Comparing only the first component reads those as `.agents` and
    /// `config` -- directories holding another product's files -- and refuses
    /// every write to the deeper path this harness genuinely owns.
    ///
    /// A path is owned when it *is* a namespace or lies beneath one. The
    /// trailing separator matters: without it `skills-experimental` would match
    /// the namespace `skills`, and a neighbour would be swallowed by a prefix.
    ///
    /// This lives here, not beside either caller, because the wire surface and
    /// the local catalog must answer this question identically. They did not
    /// once, and the catalog refused every setup the wire would have accepted.
    #[must_use]
    pub fn owns(&self, path: &str) -> bool {
        self.native_namespaces.iter().any(|namespace| {
            path == *namespace
                || path
                    .strip_prefix(namespace)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// Top-level entries that are not part of this target's identity.
    ///
    /// The control directory and the state file are this provider's own
    /// bookkeeping; counting them would make an applied operation leave the
    /// target different from the identity it just recorded. The never-touch
    /// paths are the product's own — it rewrites credentials and session
    /// history constantly, and letting that traffic move the identity would
    /// strand a plan for a change no effect of ours would have overwritten.
    #[must_use]
    pub fn not_our_identity(&self) -> Vec<&'static str> {
        let mut names = vec![self.state_file];
        names.extend_from_slice(self.never_touch);
        names
    }

    /// Top-level entries a backup never captures.
    ///
    /// Credentials belong to the product. A slot that held them would put them
    /// on disk in a second place, which is a worse outcome than an incomplete
    /// restore of files this provider never wrote anyway.
    #[must_use]
    pub fn never_captured(&self) -> Vec<&'static str> {
        let mut names = vec![self.control_directory];
        names.extend_from_slice(self.never_touch);
        names
    }

    /// A digest of this build's own manifest.
    ///
    /// The contract is explicit that the release digest must not come from
    /// `provider-info` — an artifact hashing itself proves nothing. This is a
    /// different value: an independent statement of what this build *is*, which
    /// the consumer records beside the release digest it verified separately.
    ///
    /// # Errors
    ///
    /// Returns a declaration error if the vendored kit identity is unreadable.
    pub fn build_digest(&self) -> provider_v3::Result<String> {
        let kit: serde_json::Value = serde_json::from_str(self.kit_identity).map_err(|source| {
            provider_v3::Error::declaration(format!(
                "the vendored kit identity is unreadable: {source}"
            ))
        })?;
        let manifest = serde_json::json!({
            "provider_id": self.provider_id,
            "provider_version": self.version,
            "protocol_version": provider_v3::PROTOCOL_VERSION,
            "harness_id": self.harness_id,
            "kit_aggregate_digest": kit["aggregate_digest"],
        });
        digest::of_canonical_json(&manifest)
            .map_err(|source| provider_v3::Error::declaration(source.detail()))
    }

    /// The projection profile this build declares.
    ///
    /// # Errors
    ///
    /// Propagates a declaration refusal.
    pub fn projection_profile(&self) -> provider_v3::Result<ProjectionProfile> {
        ProjectionProfile::new(
            self.profile_id,
            self.component_kinds,
            self.projection_kinds,
            self.native_namespaces,
            &[BUNDLE_FORMAT],
            self.max_files,
            self.max_bytes,
        )
    }

    /// The complete `provider-info` answer for this build.
    ///
    /// Only the five core operations are declared. The software lifecycle and
    /// `launch` are optional in the contract, and this runtime does not
    /// implement them — declaring one would let a consumer call an operation
    /// that cannot be honoured, which is worse than not offering it.
    ///
    /// The operations this build actually performs.
    ///
    /// The software lifecycle is optional in the contract, and declaring an
    /// operation a build cannot perform lets a consumer ask for something that
    /// cannot be honoured. So it appears here only when this harness carries an
    /// artifact table -- never when the product is delivered by a package
    /// manager this provider does not run.
    #[must_use]
    pub fn operations(&self) -> &'static [Operation] {
        match self.software {
            Some(Software {
                delivery: Delivery::Artifacts(_),
                ..
            }) => Operation::CORE_AND_SOFTWARE,
            _ => Operation::CORE,
        }
    }

    /// # Errors
    ///
    /// Propagates a declaration refusal.
    pub fn provider_info(&self) -> provider_v3::Result<ProviderInfo> {
        let build_digest = self.build_digest()?;
        ProviderInfo::declare(Declaration {
            provider_id: self.provider_id,
            harness_id: self.harness_id,
            provider_version: self.version,
            provider_build_digest: &build_digest,
            commands: Command::CORE,
            operations: self.operations(),
            supported_os: &["linux", "macos", "windows"],
            supported_arch: &["x86_64", "arm64"],
            permission_profiles: self.permission_profiles,
            projection_profile: self.projection_profile()?,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    /// A harness that offers no software lifecycle, which is most of what the
    /// declaration tests are about.
    pub(crate) const SAMPLE: Harness = Harness {
        software: None,
        harness_id: "sample",
        provider_id: "sample-setup-system",
        version: "0.1.0",
        product: "Sample",
        vendor: "NDDev",
        documented_config_home: "~/.sample",
        config_home_env: "SAMPLE_CONFIG_DIR",
        control_directory: ".sample-setup-system",
        state_file: "NDDEV-SAMPLE-PROVIDER.json",
        profile_id: "sample/native-files/1",
        native_namespaces: &["AGENTS.md", "settings.json", "skills"],
        never_touch: &[".credentials.json", "sessions"],
        permission_profiles: &["default"],
        component_kinds: &[ComponentKind::Instruction, ComponentKind::Skill],
        projection_kinds: &[ProjectionKind::NativeFiles],
        max_files: 4096,
        max_bytes: 1024,
        kit_identity: r#"{"aggregate_digest":"sha256:aa","protocol_version":3}"#,
    };

    #[test]
    fn identity_excludes_provider_bookkeeping_and_product_traffic() {
        let excluded = SAMPLE.not_our_identity();
        assert!(excluded.contains(&"NDDEV-SAMPLE-PROVIDER.json"));
        assert!(excluded.contains(&".credentials.json"));
        assert!(excluded.contains(&"sessions"));
    }

    #[test]
    fn a_backup_never_captures_credentials_or_the_control_directory() {
        let excluded = SAMPLE.never_captured();
        assert!(excluded.contains(&".sample-setup-system"));
        assert!(excluded.contains(&".credentials.json"));
    }

    #[test]
    fn nothing_is_both_owned_and_never_touched() {
        // Claiming a path the product owns would make an effect of ours
        // overwrite state we promised not to read.
        for name in SAMPLE.never_touch {
            assert!(
                !SAMPLE.native_namespaces.contains(name),
                "{name} is claimed and disclaimed"
            );
        }
    }

    #[test]
    fn the_declaration_offers_only_operations_this_runtime_performs() {
        let info = SAMPLE.provider_info().unwrap();
        for operation in Operation::CORE {
            assert!(info.declares(*operation));
        }
        for optional in [
            Operation::Launch,
            Operation::SoftwareInstall,
            Operation::SoftwareUpdate,
            Operation::SoftwareRemove,
        ] {
            assert!(
                !info.declares(optional),
                "{optional} is declared but not performed"
            );
        }
    }

    #[test]
    fn the_build_digest_is_reproducible_and_binds_the_kit() {
        let once = SAMPLE.build_digest().unwrap();
        assert_eq!(once, SAMPLE.build_digest().unwrap());
        assert!(once.starts_with("sha256:"));

        let other = Harness {
            version: "0.2.0",
            ..SAMPLE
        };
        assert_ne!(other.build_digest().unwrap(), once);
    }

    #[test]
    fn two_harnesses_differing_only_in_surface_get_different_profile_digests() {
        let narrower = Harness {
            native_namespaces: &["AGENTS.md"],
            ..SAMPLE
        };
        assert_ne!(
            narrower.projection_profile().unwrap().digest,
            SAMPLE.projection_profile().unwrap().digest
        );
    }
}
