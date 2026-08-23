//! The provider state written into the target, and what it is allowed to claim.
//!
//! The field set is the `provenance_fields` list the provider kit manifest
//! owns. It is reproduced here as a Rust type because a program must name its
//! fields to read them — but the manifest remains the authority, and a change
//! there is a change here, verified by [`PROVENANCE_FIELDS`] against the
//! vendored kit.
//!
//! Two rules travel with this record:
//!
//! - **No secret values.** The state describes identity and shape, never
//!   credentials, tokens or file contents.
//! - **Reading never migrates.** `status` reports what it found, including a
//!   schema it does not write. A mutation is what may replace the record, and
//!   only after it has captured a backup — so a migration that goes wrong is
//!   still recoverable.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, ReasonCode, Result};
use crate::lock;

/// The schema this kernel writes.
pub const STATE_SCHEMA: u32 = 3;

/// The exact provenance field names, in the manifest's order.
///
/// A test binds this list to the vendored `provider-kit/v3/manifest.json`, so
/// the type and the contract cannot drift apart silently.
pub const PROVENANCE_FIELDS: &[&str] = &[
    "state_schema",
    "protocol_version",
    "provider_id",
    "provider_version",
    "provider_build_digest",
    "provider_release_digest",
    "harness_id",
    "canonical_target",
    "target_identity_digest",
    "setup_stable_id",
    "setup_version",
    "setup_version_passport_digest",
    "setup_definition_digest",
    "component_refs",
    "bundle_format",
    "bundle_digest",
    "artifact_digest",
    "projection_profile_digest",
    "provider_plan_digest",
    "operation_id",
    "target_precondition_digest",
    "native_ownership",
    "backup_ref",
    "previous_verified_identity",
    "drift_state",
];

/// Whether the target still matches the state that describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftState {
    /// The target matches its recorded identity.
    Clean,
    /// The target changed outside this provider.
    LocalDrift,
    /// No verified identity has been recorded yet.
    Unknown,
}

/// The provider-owned state record inside a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderState {
    /// Schema of this record.
    pub state_schema: u32,
    /// The wire protocol this state was written under.
    pub protocol_version: u32,
    /// The provider that owns this state.
    pub provider_id: String,
    /// The provider build version.
    pub provider_version: String,
    /// A digest of the provider's own build manifest, reported independently.
    pub provider_build_digest: String,
    /// The release digest the consumer verified before invoking the provider.
    pub provider_release_digest: Option<String>,
    /// The harness this target belongs to.
    pub harness_id: String,
    /// The canonical target directory.
    pub canonical_target: String,
    /// The target identity digest after the verified operation.
    pub target_identity_digest: String,
    /// The stable identity of the applied setup.
    pub setup_stable_id: Option<String>,
    /// The applied setup version.
    pub setup_version: Option<String>,
    /// The digest of the setup version passport.
    pub setup_version_passport_digest: Option<String>,
    /// The digest of the immutable setup definition.
    pub setup_definition_digest: Option<String>,
    /// The ordered exact components the setup projected.
    pub component_refs: Vec<String>,
    /// The bundle format the projection arrived in.
    pub bundle_format: Option<String>,
    /// The logical bundle digest.
    pub bundle_digest: Option<String>,
    /// The raw artifact digest.
    pub artifact_digest: Option<String>,
    /// The projection profile the compiler built for.
    pub projection_profile_digest: Option<String>,
    /// The provider plan the operation was authorized by.
    pub provider_plan_digest: Option<String>,
    /// The operation that produced this state.
    pub operation_id: String,
    /// The target identity digest observed before the operation.
    pub target_precondition_digest: String,
    /// The native files and surfaces this provider owns in the target.
    pub native_ownership: Vec<String>,
    /// The backup captured before the operation.
    pub backup_ref: Option<String>,
    /// The identity verified before this one.
    pub previous_verified_identity: Option<String>,
    /// Whether the target still matches this record.
    pub drift_state: DriftState,
}

/// What reading a target's state found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateReading {
    /// No state record is present.
    Absent,
    /// A state record this build writes and understands.
    Current(Box<ProviderState>),
    /// A record in a schema this build does not write.
    ///
    /// Reported, never rewritten: a read command that migrated state would
    /// mutate a target the caller asked only to inspect.
    ForeignSchema {
        /// The schema found in the record.
        found_schema: u64,
    },
}

impl ProviderState {
    /// The state path inside a target.
    #[must_use]
    pub fn path(target_root: &Path, state_file_name: &str) -> PathBuf {
        target_root.join(state_file_name)
    }

    /// Read the state without migrating it.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if a record exists but cannot be
    /// read or does not parse as JSON.
    pub fn read(target_root: &Path, state_file_name: &str) -> Result<StateReading> {
        let path = Self::path(target_root, state_file_name);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StateReading::Absent);
            }
            Err(source) => {
                return Err(Error::new(
                    ReasonCode::StateUnavailable,
                    format!("cannot read {}", path.display()),
                )
                .with_source(source));
            }
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("{} does not parse as JSON", path.display()),
            )
            .with_source(source)
        })?;
        let found = value
            .get("state_schema")
            .and_then(serde_json::Value::as_u64);
        match found {
            Some(schema) if schema == u64::from(STATE_SCHEMA) => {
                let state: Self = serde_json::from_value(value).map_err(|source| {
                    Error::new(
                        ReasonCode::StateUnavailable,
                        format!(
                            "{} is schema {STATE_SCHEMA} but does not match it",
                            path.display()
                        ),
                    )
                    .with_source(source)
                })?;
                Ok(StateReading::Current(Box::new(state)))
            }
            Some(schema) => Ok(StateReading::ForeignSchema {
                found_schema: schema,
            }),
            None => Err(Error::new(
                ReasonCode::StateUnavailable,
                format!("{} declares no state_schema", path.display()),
            )),
        }
    }

    /// Write the state atomically through its canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ReasonCode::StateUnavailable`] if the record cannot be encoded
    /// or written.
    pub fn write(&self, target_root: &Path, state_file_name: &str) -> Result<()> {
        let value = serde_json::to_value(self).map_err(|source| {
            Error::new(
                ReasonCode::StateUnavailable,
                "cannot encode the provider state",
            )
            .with_source(source)
        })?;
        let bytes = crate::canonical::to_canonical_bytes(&value)?;
        lock::atomic_write(&Self::path(target_root, state_file_name), &bytes)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("setup-core-stamp-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn sample() -> ProviderState {
        ProviderState {
            state_schema: STATE_SCHEMA,
            protocol_version: 3,
            provider_id: "claude-setup-system".to_owned(),
            provider_version: "0.1.0".to_owned(),
            provider_build_digest: "sha256:build".to_owned(),
            provider_release_digest: None,
            harness_id: "claude".to_owned(),
            canonical_target: "/tmp/target".to_owned(),
            target_identity_digest: "sha256:after".to_owned(),
            setup_stable_id: Some("full-auto".to_owned()),
            setup_version: Some("1".to_owned()),
            setup_version_passport_digest: None,
            setup_definition_digest: Some("sha256:definition".to_owned()),
            component_refs: vec!["instruction:AGENTS.md".to_owned()],
            bundle_format: Some("ai-stp-bundle/1".to_owned()),
            bundle_digest: Some("sha256:bundle".to_owned()),
            artifact_digest: Some("sha256:artifact".to_owned()),
            projection_profile_digest: Some("sha256:profile".to_owned()),
            provider_plan_digest: Some("sha256:plan".to_owned()),
            operation_id: "op_test".to_owned(),
            target_precondition_digest: "sha256:before".to_owned(),
            native_ownership: vec!["settings.json".to_owned()],
            backup_ref: Some("slot-000000000001".to_owned()),
            previous_verified_identity: None,
            drift_state: DriftState::Clean,
        }
    }

    #[test]
    fn the_type_carries_exactly_the_manifest_provenance_fields() {
        let manifest_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../provider-kit/v3/manifest.json");
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        let declared: Vec<String> = manifest["provenance_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        assert_eq!(declared, PROVENANCE_FIELDS);

        let encoded = serde_json::to_value(sample()).unwrap();
        let object = encoded.as_object().unwrap();
        let mut present: Vec<&str> = object.keys().map(String::as_str).collect();
        present.sort_unstable();
        let mut expected: Vec<&str> = PROVENANCE_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(present, expected);
    }

    #[test]
    fn an_absent_record_reads_as_absent() {
        let root = scratch("absent");
        assert_eq!(
            ProviderState::read(&root, "STATE.json").unwrap(),
            StateReading::Absent
        );
    }

    #[test]
    fn a_written_record_round_trips() {
        let root = scratch("roundtrip");
        let state = sample();
        state.write(&root, "STATE.json").unwrap();
        match ProviderState::read(&root, "STATE.json").unwrap() {
            StateReading::Current(read) => assert_eq!(*read, state),
            other => panic!("expected a current record, got {other:?}"),
        }
    }

    #[test]
    fn a_foreign_schema_is_reported_and_left_exactly_as_found() {
        let root = scratch("foreign");
        let path = root.join("STATE.json");
        let original = br#"{"state_schema":99,"anything":"kept"}"#;
        fs::write(&path, original).unwrap();

        assert_eq!(
            ProviderState::read(&root, "STATE.json").unwrap(),
            StateReading::ForeignSchema { found_schema: 99 }
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn a_record_without_a_schema_is_refused_rather_than_assumed_current() {
        let root = scratch("schemaless");
        fs::write(root.join("STATE.json"), br#"{"provider_id":"x"}"#).unwrap();
        let error = ProviderState::read(&root, "STATE.json").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::StateUnavailable);
    }
}
