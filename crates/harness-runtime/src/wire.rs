//! The provider commands, answered through the kernel and nothing else.
//!
//! Each handler is small on purpose. The decisions that matter — what a target
//! is, when a lock is held, what a journal means, which backup is the last one —
//! belong to [`setup_core`], and repeating any of them here would create a
//! second answer that could disagree with the first.
//!
//! # What this build applies, and what it refuses
//!
//! `backup`, `restore` and `remove` need no bundle: they read the target, a
//! backup slot, or the provider's own state. They are implemented.
//!
//! `install` and `replace` materialize bundle contents, which needs a reader for
//! the bundle format. Until that exists they refuse with a detail saying so.
//! They stay *declared* because the contract requires all five core operations
//! in `provider-info`, and a refusal that names the real limitation is honest in
//! a way that a narrowed declaration could not be.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use provider_v3::argv::{Bundle, Invocation, PlanRequest};
use provider_v3::plan::{PlanArtifact, PlanInputs};
use provider_v3::{Error, Operation, Result, WireReason};
use setup_core::backup::{BackupRef, Pool, SLOT_SCHEMA, SlotRecord};
use setup_core::journal::{JOURNAL_SCHEMA, Journal, Phase};
use setup_core::stamp::{DriftState, ProviderState, STATE_SCHEMA, StateReading};
use setup_core::target::Target;
use setup_core::{digest, lock};

use crate::catalog::Setup;
use crate::expiry;
use crate::facts::{self, Harness};

/// Answer one parsed invocation.
///
/// # Errors
///
/// Every failure is a typed refusal the caller prints as a reason plus a detail.
pub fn dispatch(harness: &Harness, invocation: Invocation) -> Result<serde_json::Value> {
    match invocation {
        Invocation::ProviderInfo => {
            let info = harness.provider_info()?;
            serde_json::to_value(info).map_err(|source| {
                Error::declaration(format!("provider-info cannot be encoded: {source}"))
            })
        }
        Invocation::Status { target } => status(harness, &target),
        Invocation::ValidateBundle { bundle, .. } => validate_bundle(harness, &bundle),
        Invocation::PlanOperation { target, request } => plan(harness, &target, &request),
        Invocation::ApplyOperation {
            target,
            plan_path,
            plan_digest,
            ..
        } => apply(harness, &target, &plan_path, &plan_digest),
        Invocation::RecoverOperation { target } => recover(harness, &target),
        Invocation::Launch { .. } => Err(Error::refuse(
            WireReason::UnsupportedOperation,
            "this provider owns the configuration only and does not start the product",
        )),
    }
}

fn open(harness: &Harness, target: &Path) -> Result<(Target, std::path::PathBuf, Pool)> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let control = resolved.ensure_control_directory()?;
    let pool = Pool::open(&control, facts::BACKUP_SLOTS)?;
    Ok((resolved, control, pool))
}

/// Report the target without changing it, including a schema this build cannot write.
fn status(harness: &Harness, target: &Path) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let journal = Journal::read(&control).ok().flatten();

    let state = match ProviderState::read(resolved.root(), harness.state_file)? {
        StateReading::Absent => serde_json::json!({ "present": false }),
        StateReading::ForeignSchema { found_schema } => serde_json::json!({
            "present": true,
            "readable": false,
            "found_schema": found_schema,
            "detail": "a schema this build does not write; status never migrates it",
        }),
        StateReading::Current(current) => {
            let drift = if current.target_identity_digest == identity {
                DriftState::Clean
            } else {
                DriftState::LocalDrift
            };
            serde_json::json!({
                "present": true,
                "readable": true,
                "setup_stable_id": current.setup_stable_id,
                "setup_version": current.setup_version,
                "operation_id": current.operation_id,
                "backup_ref": current.backup_ref,
                "recorded_identity": current.target_identity_digest,
                "drift_state": drift,
            })
        }
    };

    Ok(serde_json::json!({
        "state": "verified",
        "protocol_version": provider_v3::PROTOCOL_VERSION,
        "provider_id": harness.provider_id,
        "harness_id": harness.harness_id,
        "canonical_target": resolved.root().to_string_lossy(),
        "target_identity_digest": identity,
        "provider_state": state,
        "journal": journal.map(|entry| serde_json::json!({
            "phase": entry.phase.as_str(),
            "operation": entry.operation,
            "operation_id": entry.operation_id,
        })),
        "backups": pool.list()?.iter().map(|record| serde_json::json!({
            "backup_ref": record.backup_ref.as_str(),
            "operation": record.operation,
            "setup_id": record.setup_id,
        })).collect::<Vec<_>>(),
    }))
}

/// Check a bundle's identity against the bytes the caller pointed at.
fn validate_bundle(harness: &Harness, bundle: &Bundle) -> Result<serde_json::Value> {
    let binding = &bundle.binding;
    if binding.bundle_format != facts::BUNDLE_FORMAT {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::UnsupportedBundleFormat,
        ));
    }

    let Ok(metadata) = fs::symlink_metadata(&bundle.path) else {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::DigestMismatch,
        ));
    };
    if metadata.is_symlink() {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::LinkNotAllowed,
        ));
    }
    if !metadata.is_file() {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::SpecialFileNotAllowed,
        ));
    }
    if metadata.len() != binding.bundle_size {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::DigestMismatch,
        ));
    }
    if metadata.len() > harness.max_bytes {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::LimitExceeded,
        ));
    }
    if digest::of_file(&bundle.path)? != binding.artifact_digest {
        return Ok(provider_v3::plan::bundle_rejected(
            binding,
            WireReason::DigestMismatch,
        ));
    }
    Ok(provider_v3::plan::bundle_accepted(binding))
}

/// Produce a plan without touching the target.
fn plan(harness: &Harness, target: &Path, request: &PlanRequest) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    setup_core::journal::require_clean_for_planning(
        &control,
        &control.join("transaction"),
        &pool.partial_slots()?,
    )?;

    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let profile = harness.projection_profile()?;
    let build_digest = harness.build_digest()?;

    let (effects, backup_ref, restore_target_digest) = match request.operation {
        Operation::Backup => (
            vec![format!(
                "capture {} into a new backup slot",
                resolved.root().display()
            )],
            None,
            None,
        ),
        Operation::Restore => {
            let record = chosen_backup(&pool, request.backup_ref.as_deref())?;
            let payload = pool.payload_of(&record.backup_ref)?;
            (
                vec![
                    format!("capture the current target before restoring"),
                    format!("restore the target from {}", record.backup_ref.as_str()),
                ],
                Some(record.backup_ref.as_str().to_owned()),
                Some(digest::of_tree(&payload)?),
            )
        }
        Operation::Remove => (
            vec![
                "capture the current target before removing".to_owned(),
                "withdraw every file this provider owns".to_owned(),
            ],
            None,
            None,
        ),
        Operation::Install | Operation::Replace => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{} needs a reader for {}, which this build does not carry yet",
                    request.operation,
                    facts::BUNDLE_FORMAT
                ),
            ));
        }
        other => {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                format!("{other} is not declared by this provider"),
            ));
        }
    };

    PlanArtifact::new(PlanInputs {
        provider_id: harness.provider_id,
        provider_version: harness.version,
        provider_build_digest: &build_digest,
        provider_release_digest: &request.provider_release_digest,
        operation_id: &request.operation_id,
        operation: request.operation,
        canonical_target: &resolved.root().to_string_lossy(),
        expected_target_digest: &identity,
        projection_profile_digest: &profile.digest,
        bundle: request.bundle.as_ref().map(|bundle| bundle.binding.clone()),
        backup_ref,
        restore_target_digest,
        permission_profile: request.permission_profile.clone(),
        expires_at: &request.expires_at,
        effects,
    })?
    .into_response()
}

/// The backup a restore names, or the newest when it names none.
pub(crate) fn chosen_backup(pool: &Pool, requested: Option<&str>) -> Result<SlotRecord> {
    match requested {
        Some(text) => {
            let reference = BackupRef::parse(text)?;
            pool.list()?
                .into_iter()
                .find(|record| record.backup_ref == reference)
                .ok_or_else(|| {
                    Error::refuse(
                        WireReason::ProviderUnavailable,
                        format!("{text} is not a completed backup of this target"),
                    )
                })
        }
        None => pool.latest()?.ok_or_else(|| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                "this target has no backup to restore",
            )
        }),
    }
}

/// Apply one exact plan under the target lock.
/// What a mutation actually does to the target once the lock is held.
///
/// Every variant runs through the same sequence in [`perform`]. Adding one here
/// is the only way to add an effect, which is what keeps the human surface and
/// the wire surface from growing separate write paths that could disagree.
pub(crate) enum Effect<'a> {
    /// The capture is the whole effect; nothing else is written.
    Backup,
    /// Put a captured tree back over the namespaces this provider owns.
    Restore {
        /// The slot to read, or the newest when absent.
        backup_ref: Option<String>,
    },
    /// Withdraw everything this provider owns.
    Remove,
    /// Write a complete setup from the local catalog over those namespaces.
    Materialize {
        /// The setup to write.
        setup: &'a Setup,
    },
}

/// One authorized mutation, whatever surface asked for it.
pub(crate) struct Mutation<'a> {
    pub operation: Operation,
    pub operation_id: String,
    pub plan_digest: String,
    /// The identity the plan was made against. Re-checked once the lock is held.
    pub expected_target_digest: String,
    pub effect: Effect<'a>,
    /// The plan artifact, recorded into provider state as provenance.
    pub provenance: serde_json::Value,
    /// The setup identity this mutation leaves applied, when there is one.
    pub setup_id: Option<String>,
}

/// Apply one exact plan under the target lock.
fn apply(
    harness: &Harness,
    target: &Path,
    plan_path: &Path,
    plan_digest: &str,
) -> Result<serde_json::Value> {
    let artifact = load_plan(plan_path, plan_digest)?;
    let operation = operation_of(&artifact)?;
    let expires_at = string_field(&artifact, "expires_at")?;
    if expiry::has_expired(&expires_at, SystemTime::now()) {
        return Err(Error::refuse(
            WireReason::Stale,
            "this plan expired before it was applied; no effect was made",
        ));
    }

    let effect = match operation {
        Operation::Backup => Effect::Backup,
        Operation::Restore => Effect::Restore {
            backup_ref: artifact
                .get("backup_ref")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        },
        Operation::Remove => Effect::Remove,
        other => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{other} arrives over the wire as a bundle, and this build carries no \
                     reader for {}; the same operation from the local catalog is available \
                     through the human surface",
                    facts::BUNDLE_FORMAT
                ),
            ));
        }
    };

    perform(
        harness,
        target,
        &Mutation {
            operation,
            operation_id: string_field(&artifact, "operation_id")?,
            plan_digest: plan_digest.to_owned(),
            expected_target_digest: string_field(&artifact, "expected_target_digest")?,
            effect,
            setup_id: None,
            provenance: artifact,
        },
    )
}

/// The one write path. Lock, re-check, capture, journal, act, verify, commit.
///
/// Both surfaces come through here. A second sequence would be a second set of
/// guarantees, and the two would drift.
pub(crate) fn perform(
    harness: &Harness,
    target: &Path,
    mutation: &Mutation<'_>,
) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    let mut guard = setup_core::lock::TargetLock::acquire(&control)?;
    guard.annotate(&format!(
        "{} {}",
        harness.provider_id, mutation.operation_id
    ))?;

    // Re-check after the lock: everything observed before it could have moved.
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    if identity != mutation.expected_target_digest {
        return Err(Error::refuse(
            WireReason::Stale,
            "the target changed after the lock was taken; no effect was made",
        ));
    }
    setup_core::journal::require_clean_for_planning(
        &control,
        &control.join("transaction"),
        &pool.partial_slots()?,
    )?;

    let operation_id = mutation.operation_id.clone();
    let operation_name = mutation.operation.as_str().to_owned();
    let previous_setup = match ProviderState::read(resolved.root(), harness.state_file)? {
        StateReading::Current(current) => current.setup_stable_id,
        _ => None,
    };
    let captured = pool.capture(resolved.root(), &harness.never_captured(), |backup_ref| {
        SlotRecord {
            schema_version: SLOT_SCHEMA,
            backup_ref,
            operation: operation_name.clone(),
            operation_id: operation_id.clone(),
            target_identity_digest: identity.clone(),
            setup_id: previous_setup.clone(),
        }
    })?;

    let journal = Journal {
        schema_version: JOURNAL_SCHEMA,
        phase: Phase::Prepared,
        operation_id: mutation.operation_id.clone(),
        operation: mutation.operation.as_str().to_owned(),
        plan_digest: mutation.plan_digest.clone(),
        target_precondition_digest: identity.clone(),
        backup_ref: Some(captured.backup_ref.as_str().to_owned()),
    }
    .publish_prepared(&control)?;

    let outcome = match &mutation.effect {
        // The capture above *is* the effect. Nothing else is written.
        Effect::Backup => Ok(()),
        Effect::Restore { backup_ref } => {
            let record = chosen_backup(&pool, backup_ref.as_deref())?;
            let payload = pool.payload_of(&record.backup_ref)?;
            replace_managed_from(harness, &resolved, &payload)
        }
        Effect::Remove => remove_managed(harness, &resolved),
        Effect::Materialize { setup } => {
            setup.check_within(harness)?;
            replace_managed_from(harness, &resolved, &setup.payload)
        }
    };

    // On failure the journal stays in `prepared`, which is what makes the
    // interruption legible: recovery restores the captured pre-operation target.
    outcome?;

    let after = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    write_state(
        harness,
        &resolved,
        &mutation.provenance,
        &identity,
        &after,
        &captured,
        mutation.setup_id.as_deref(),
    )?;
    journal.promote_to_committed(&control)?;
    Journal::clear(&control)?;

    Ok(serde_json::json!({
        "state": "verified",
        "operation": mutation.operation.as_str(),
        "plan_digest": mutation.plan_digest,
        "expected_target_digest": identity,
        "target_identity_digest": after,
        "backup_ref": captured.backup_ref.as_str(),
        "setup_id": mutation.setup_id,
    }))
}

/// Resolve an interrupted operation from its journal.
fn recover(harness: &Harness, target: &Path) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    let _guard = setup_core::lock::TargetLock::acquire(&control)?;

    let Some(journal) = Journal::read(&control)? else {
        return Ok(serde_json::json!({
            "state": "verified",
            "recovered": false,
            "detail": "no journal is published; there is nothing to resolve",
        }));
    };

    match journal.phase {
        Phase::Prepared => {
            // The effect may be partial. Return the exact pre-operation target.
            let Some(reference) = journal.backup_ref.as_deref() else {
                return Err(Error::refuse(
                    WireReason::RecoveryRequired,
                    "the journal names no backup, so the pre-operation target cannot be restored",
                ));
            };
            let backup_ref = BackupRef::parse(reference)?;
            let payload = pool.payload_of(&backup_ref)?;
            replace_managed_from(harness, &resolved, &payload)?;
            Journal::clear(&control)?;
            Ok(serde_json::json!({
                "state": "verified",
                "recovered": true,
                "phase": Phase::Prepared.as_str(),
                "restored_from": reference,
                "target_identity_digest": resolved.identity_digest_excluding(&harness.not_our_identity())?,
            }))
        }
        Phase::Committed => {
            // The effect is complete. Verify and clear the tails only.
            Journal::clear(&control)?;
            Ok(serde_json::json!({
                "state": "verified",
                "recovered": true,
                "phase": Phase::Committed.as_str(),
                "target_identity_digest": resolved.identity_digest_excluding(&harness.not_our_identity())?,
            }))
        }
    }
}

/// Replace this provider's namespaces from a captured tree.
///
/// Only the namespaces this provider owns are removed and rewritten. A sibling
/// overlay the product or the owner put in the target survives, because a
/// restore that also reverted files this provider never wrote would be undoing
/// someone else's work.
fn replace_managed_from(harness: &Harness, target: &Target, payload: &Path) -> Result<()> {
    for namespace in harness.native_namespaces {
        let destination = target.root().join(namespace);
        remove_path(&destination)?;
        let source = payload.join(namespace);
        if !source.exists() {
            continue;
        }
        if source.is_dir() {
            setup_core::backup::copy_tree(&source, &destination, &[])?;
        } else {
            let bytes = fs::read(&source).map_err(|error| {
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot read {}", source.display()),
                )
                .with_source(error)
            })?;
            lock::atomic_write(&destination, &bytes)?;
        }
    }
    Ok(())
}

fn remove_managed(harness: &Harness, target: &Target) -> Result<()> {
    for namespace in harness.native_namespaces {
        remove_path(&target.root().join(namespace))?;
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    let outcome = if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    outcome.map_err(|error| {
        Error::from(
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!("cannot remove {}", path.display()),
            )
            .with_source(error),
        )
    })
}

fn write_state(
    harness: &Harness,
    target: &Target,
    artifact: &serde_json::Value,
    before: &str,
    after: &str,
    captured: &SlotRecord,
    setup_id: Option<&str>,
) -> Result<()> {
    let previous = match ProviderState::read(target.root(), harness.state_file)? {
        StateReading::Current(current) => Some(current.target_identity_digest),
        _ => None,
    };
    ProviderState {
        state_schema: STATE_SCHEMA,
        protocol_version: provider_v3::PROTOCOL_VERSION,
        provider_id: harness.provider_id.to_owned(),
        provider_version: harness.version.to_owned(),
        provider_build_digest: harness.build_digest()?,
        provider_release_digest: artifact
            .get("provider_release_digest")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        harness_id: harness.harness_id.to_owned(),
        canonical_target: target.root().to_string_lossy().into_owned(),
        target_identity_digest: after.to_owned(),
        setup_stable_id: setup_id.map(str::to_owned),
        setup_version: None,
        setup_version_passport_digest: None,
        setup_definition_digest: None,
        component_refs: Vec::new(),
        bundle_format: None,
        bundle_digest: None,
        artifact_digest: None,
        projection_profile_digest: Some(harness.projection_profile()?.digest),
        provider_plan_digest: artifact
            .get("plan_digest")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        operation_id: string_field(artifact, "operation_id")?,
        target_precondition_digest: before.to_owned(),
        native_ownership: harness
            .native_namespaces
            .iter()
            .map(|n| (*n).to_owned())
            .collect(),
        backup_ref: Some(captured.backup_ref.as_str().to_owned()),
        previous_verified_identity: previous,
        drift_state: DriftState::Clean,
    }
    .write(target.root(), harness.state_file)
    .map_err(Error::from)
}

fn load_plan(path: &Path, expected_digest: &str) -> Result<serde_json::Value> {
    let bytes = fs::read(path).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "the approved plan at {} cannot be read: {error}",
                path.display()
            ),
        )
    })?;
    let artifact: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("the approved plan is not JSON: {error}"),
        )
    })?;
    let actual = digest::of_domain_canonical_json(provider_v3::PLAN_DOMAIN, &artifact)?;
    if actual != expected_digest {
        return Err(Error::refuse(
            WireReason::DigestMismatch,
            "the approved plan artifact has another digest; no effect was made",
        ));
    }
    Ok(artifact)
}

fn operation_of(artifact: &serde_json::Value) -> Result<Operation> {
    let name = string_field(artifact, "operation")?;
    Operation::parse(&name).ok_or_else(|| {
        Error::refuse(
            WireReason::UnsupportedOperation,
            format!("{name:?} is not an operation this protocol defines"),
        )
    })
}

fn string_field(artifact: &serde_json::Value, name: &str) -> Result<String> {
    artifact
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("the plan artifact has no {name}"),
            )
        })
}

#[cfg(test)]
pub(crate) mod tests_support {
    //! A harness shaped like a real one, shared by the tests in this crate.
    //!
    //! Exercising the runtime through a fabricated harness rather than a real
    //! product's facts keeps these tests about the runtime. A change to what
    //! Claude Code or Codex owns should not move them.

    use provider_v3::{ComponentKind, ProjectionKind};

    use crate::facts::Harness;

    pub(crate) const TEST: Harness = Harness {
        harness_id: "test",
        provider_id: "test-setup-system",
        version: "0.1.0",
        product: "Test Product",
        vendor: "NDDev",
        documented_config_home: "~/.test",
        config_home_env: "TEST_CONFIG_DIR",
        control_directory: ".test-setup-system",
        state_file: "NDDEV-TEST-PROVIDER.json",
        profile_id: "test/native-files/1",
        native_namespaces: &["AGENTS.md", "settings.json", "skills"],
        never_touch: &[".credentials.json", "sessions"],
        permission_profiles: &["default"],
        component_kinds: &[
            ComponentKind::Instruction,
            ComponentKind::Skill,
            ComponentKind::Setting,
        ],
        projection_kinds: &[ProjectionKind::NativeFiles],
        max_files: 4096,
        max_bytes: 64 * 1024 * 1024,
        kit_identity: r#"{"aggregate_digest":"sha256:aa","protocol_version":3}"#,
    };
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::fs;
    use std::path::{Path, PathBuf};

    use provider_v3::argv;

    use super::*;

    use crate::wire::tests_support::TEST;

    const RELEASE: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    /// A per-test directory holding the target and anything written beside it.
    ///
    /// The plan artifact must live *outside* the target: inside, it would change
    /// the target's identity between plan and apply, and the apply would then
    /// correctly refuse its own plan as stale. It must also be unique per test,
    /// because these run in parallel.
    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("harness-runtime-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("target")).unwrap();
        fs::canonicalize(&base).unwrap()
    }

    fn seeded(name: &str) -> PathBuf {
        let target = scratch(name).join("target");
        fs::write(target.join("AGENTS.md"), "# first\n").unwrap();
        fs::write(target.join("settings.json"), "{\"model\":\"first\"}").unwrap();
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(target.join("skills").join("a.md"), "skill one").unwrap();
        // A sibling overlay this provider does not own.
        fs::write(target.join("unrelated.txt"), "keep me").unwrap();
        // Product-owned state this provider promises never to read or copy.
        fs::write(target.join(".credentials.json"), "SECRET").unwrap();
        target
    }

    fn args(command: &str, target: &Path, extra: &[&str]) -> Vec<String> {
        let mut tokens = vec![
            command.to_owned(),
            "--target".to_owned(),
            target.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ];
        tokens.extend(extra.iter().map(|s| (*s).to_owned()));
        tokens
    }

    fn run(tokens: Vec<String>) -> serde_json::Value {
        dispatch(&TEST, argv::parse(tokens).unwrap()).unwrap()
    }

    fn refuse(tokens: Vec<String>) -> provider_v3::Error {
        dispatch(&TEST, argv::parse(tokens).unwrap()).unwrap_err()
    }

    fn far_future() -> &'static str {
        "2099-01-01T00:00:00.000Z"
    }

    fn plan_then_apply(target: &Path, operation: &str, extra: &[&str]) -> serde_json::Value {
        let mut arguments = vec![
            "--operation",
            operation,
            "--provider-release-digest",
            RELEASE,
            "--operation-id",
            "operation_01TEST",
            "--expires-at",
            far_future(),
        ];
        arguments.extend_from_slice(extra);
        let planned = run(args("plan-operation", target, &arguments));
        assert_eq!(planned["state"], "planned", "plan refused: {planned}");

        let plan_path = target.join("..").join(format!("plan-{operation}.json"));
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();

        run(args(
            "apply-operation",
            target,
            &[
                "--plan",
                &plan_path.to_string_lossy(),
                "--plan-digest",
                planned["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ))
    }

    #[test]
    fn provider_info_answers_without_a_target() {
        let answer = dispatch(&TEST, argv::parse(["provider-info"]).unwrap()).unwrap();
        assert_eq!(answer["provider_id"], TEST.provider_id);
        assert_eq!(answer["protocol_version"], 3);
        assert!(
            answer["projection_profile"]["digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
    }

    #[test]
    fn status_reports_an_untouched_target_without_changing_it() {
        let target = seeded("status");
        let before = fs::read_to_string(target.join("AGENTS.md")).unwrap();
        let answer = run(args("status", &target, &[]));
        assert_eq!(answer["state"], "verified");
        assert_eq!(answer["provider_state"]["present"], false);
        assert!(answer["journal"].is_null());
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            before
        );
    }

    #[test]
    fn a_backup_captures_the_target_and_leaves_it_alone() {
        let target = seeded("backup");
        let applied = plan_then_apply(&target, "backup", &[]);
        assert_eq!(applied["state"], "verified");
        assert_eq!(
            applied["expected_target_digest"],
            applied["target_identity_digest"]
        );
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );

        let status = run(args("status", &target, &[]));
        assert_eq!(status["backups"].as_array().unwrap().len(), 1);
        assert_eq!(status["provider_state"]["drift_state"], "clean");
    }

    #[test]
    fn a_backup_never_copies_product_owned_credentials() {
        let target = seeded("no-secrets");
        plan_then_apply(&target, "backup", &[]);
        let slot = target
            .join(TEST.control_directory)
            .join("backups")
            .join("slot-000000000001")
            .join("payload");
        assert!(slot.join("AGENTS.md").exists());
        assert!(
            !slot.join(".credentials.json").exists(),
            "a backup slot captured a secret"
        );
    }

    #[test]
    fn restore_returns_the_captured_state_and_keeps_unowned_files() {
        let target = seeded("restore");
        plan_then_apply(&target, "backup", &[]);

        fs::write(target.join("AGENTS.md"), "# second\n").unwrap();
        fs::write(target.join("skills").join("b.md"), "skill two").unwrap();
        fs::write(target.join("unrelated.txt"), "still mine").unwrap();

        let applied = plan_then_apply(&target, "restore", &[]);
        assert_eq!(applied["state"], "verified");
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );
        assert!(!target.join("skills").join("b.md").exists());
        // Reverting someone else's work is not what restore means.
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "still mine"
        );
    }

    #[test]
    fn restore_can_name_an_older_backup_than_the_last_one() {
        let target = seeded("restore-chosen");
        plan_then_apply(&target, "backup", &[]);
        let first = run(args("status", &target, &[]))["backups"][0]["backup_ref"]
            .as_str()
            .unwrap()
            .to_owned();

        fs::write(target.join("AGENTS.md"), "# second\n").unwrap();
        plan_then_apply(&target, "backup", &[]);
        fs::write(target.join("AGENTS.md"), "# third\n").unwrap();

        // Without a reference this would restore the second capture.
        let applied = plan_then_apply(&target, "restore", &["--backup-ref", &first]);
        assert_eq!(applied["state"], "verified");
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );
    }

    #[test]
    fn remove_withdraws_only_what_this_provider_owns() {
        let target = seeded("remove");
        assert_eq!(plan_then_apply(&target, "remove", &[])["state"], "verified");
        assert!(!target.join("AGENTS.md").exists());
        assert!(!target.join("settings.json").exists());
        assert!(!target.join("skills").exists());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(
            fs::read_to_string(target.join(".credentials.json")).unwrap(),
            "SECRET"
        );
    }

    #[test]
    fn an_expired_plan_has_no_effect() {
        let target = seeded("expired");
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                "2000-01-01T00:00:00.000Z",
            ],
        ));
        let plan_path = target.join("..").join("expired.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();

        let error = refuse(args(
            "apply-operation",
            &target,
            &[
                "--plan",
                &plan_path.to_string_lossy(),
                "--plan-digest",
                planned["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::Stale));
        assert_eq!(
            run(args("status", &target, &[]))["backups"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn a_plan_digest_that_does_not_bind_the_artifact_has_no_effect() {
        let target = seeded("wrong-digest");
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
            ],
        ));
        let plan_path = target.join("..").join("mismatched.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();

        let error = refuse(args(
            "apply-operation",
            &target,
            &[
                "--plan",
                &plan_path.to_string_lossy(),
                "--plan-digest",
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "--provider-release-digest",
                RELEASE,
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        assert_eq!(
            run(args("status", &target, &[]))["backups"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn planning_is_refused_while_a_journal_is_published() {
        let target = seeded("journaled");
        let control = target.join(TEST.control_directory);
        fs::create_dir_all(&control).unwrap();
        Journal {
            schema_version: JOURNAL_SCHEMA,
            phase: Phase::Prepared,
            operation_id: "operation_01STUCK".to_owned(),
            operation: "backup".to_owned(),
            plan_digest: RELEASE.to_owned(),
            target_precondition_digest: RELEASE.to_owned(),
            backup_ref: None,
        }
        .publish_prepared(&control)
        .unwrap();

        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::RecoveryRequired));
    }

    #[test]
    fn recovery_from_prepared_returns_the_exact_pre_operation_target() {
        let target = seeded("recover");
        plan_then_apply(&target, "backup", &[]);
        let reference = run(args("status", &target, &[]))["backups"][0]["backup_ref"]
            .as_str()
            .unwrap()
            .to_owned();

        // An interruption after the capture and part-way through a write.
        let control = target.join(TEST.control_directory);
        fs::write(target.join("AGENTS.md"), "# half written\n").unwrap();
        Journal {
            schema_version: JOURNAL_SCHEMA,
            phase: Phase::Prepared,
            operation_id: "operation_01INTERRUPTED".to_owned(),
            operation: "restore".to_owned(),
            plan_digest: RELEASE.to_owned(),
            target_precondition_digest: RELEASE.to_owned(),
            backup_ref: Some(reference.clone()),
        }
        .publish_prepared(&control)
        .unwrap();

        let recovered = run(args("recover-operation", &target, &[]));
        assert_eq!(recovered["recovered"], true);
        assert_eq!(recovered["phase"], "prepared");
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );

        // With the journal cleared, planning works again.
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01AFTER",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(planned["state"], "planned");
    }

    #[test]
    fn recovery_with_no_journal_says_so_rather_than_inventing_work() {
        let target = seeded("recover-clean");
        assert_eq!(
            run(args("recover-operation", &target, &[]))["recovered"],
            false
        );
    }

    #[test]
    fn a_restore_plan_names_the_target_it_will_produce() {
        let target = seeded("restore-shape");
        plan_then_apply(&target, "backup", &[]);
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "restore",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
            ],
        ));
        assert!(
            planned["plan"]["restore_target_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(planned["plan"]["backup_ref"].is_string());
    }

    #[test]
    fn install_and_replace_are_declared_but_refuse_with_the_real_limitation() {
        let target = seeded("not-yet");
        for operation in ["install", "replace"] {
            let error = refuse(args(
                "plan-operation",
                &target,
                &[
                    "--operation",
                    operation,
                    "--provider-release-digest",
                    RELEASE,
                    "--operation-id",
                    "operation_01TEST",
                    "--expires-at",
                    far_future(),
                ],
            ));
            assert!(
                error.detail().contains("needs a reader for"),
                "{operation}: {error}"
            );
        }
        let info = TEST.provider_info().unwrap();
        assert!(info.declares(Operation::Install));
        assert!(info.declares(Operation::Replace));
    }

    #[test]
    fn launch_is_refused_because_this_runtime_does_not_start_a_product() {
        let target = seeded("launch");
        assert_eq!(
            refuse(args("launch", &target, &[])).reason(),
            Some(WireReason::UnsupportedOperation)
        );
    }

    #[test]
    fn a_restore_with_no_backup_to_read_refuses_rather_than_emptying_the_target() {
        let target = seeded("restore-empty");
        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "restore",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
            ],
        ));
        assert!(error.detail().contains("no backup"));
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );
    }
}
