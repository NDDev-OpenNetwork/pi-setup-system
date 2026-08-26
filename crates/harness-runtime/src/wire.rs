//! The provider commands, answered through the kernel and nothing else.
//!
//! Each handler is small on purpose. The decisions that matter — what a target
//! is, when a lock is held, what a journal means, which backup is the last one —
//! belong to [`setup_core`], and repeating any of them here would create a
//! second answer that could disagree with the first.
//!
//! # What each operation does
//!
//! `backup`, `restore` and `remove` need no bundle: they read the target, a
//! backup slot, or the provider's own state.
//!
//! `install` and `replace` materialize an `ai-stp-bundle/1`. It is read and
//! checked in full — raw digest, canonical archive shape, manifest identity,
//! every file's digest and mode, every path — *before* the lock is taken, and
//! again before the effect runs: the plan authorized an identity, not a file
//! that might have changed on disk since.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use provider_v3::argv::{Bundle as ArgvBundle, Invocation, PlanRequest};
use provider_v3::bundle::{Bundle, Claim};
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
use crate::software;

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
        Invocation::ValidateBundle { bundle, .. } => Ok(validate_bundle(harness, &bundle)),
        Invocation::PlanOperation { target, request } => plan(harness, &target, &request),
        Invocation::ApplyOperation {
            target,
            plan_path,
            plan_digest,
            bundle,
            prefix,
            software_artifacts,
            ..
        } => apply(
            harness,
            &target,
            &plan_path,
            &plan_digest,
            bundle.as_ref(),
            prefix.as_deref(),
            &software_artifacts,
        ),
        Invocation::RecoverOperation { target } => recover(harness, &target),
        Invocation::Launch {
            target,
            prefix,
            arguments,
        } => software::launch(harness, &target, prefix.as_deref(), &arguments),
    }
}

/// Read the bytes a caller pointed at and check them against its exact claim.
///
/// Every refusal here happens before the lock is taken, let alone before
/// anything is written: a bundle is either wholly acceptable or not applied.
fn verified_bundle(harness: &Harness, bundle: &ArgvBundle) -> Result<Bundle> {
    let bytes = fs::read(&bundle.path).map_err(|source| {
        Error::refuse(
            WireReason::DigestMismatch,
            format!(
                "cannot read the bundle at {}: {source}",
                bundle.path.display()
            ),
        )
    })?;
    let verified = Bundle::read(
        &bytes,
        Claim {
            bundle_format: &bundle.binding.bundle_format,
            bundle_digest: &bundle.binding.bundle_digest,
            artifact_digest: &bundle.binding.artifact_digest,
            bundle_size: bundle.binding.bundle_size,
            harness_id: harness.harness_id,
        },
    )?;
    check_within_surface(harness, verified.files.keys())?;
    check_declared_kinds(harness, &verified)?;
    Ok(verified)
}

/// Every component kind a bundle names must be one this harness implements.
///
/// The kind is not in the manifest and not in the setup passport -- the passport
/// carries component references without kinds. It is stated once, in the
/// conversion report, which is why a provider that never reads that report
/// cannot tell it has been handed a kind it does not implement. It would simply
/// write the files and report success for a component it does not understand.
fn check_declared_kinds(harness: &Harness, bundle: &Bundle) -> Result<()> {
    for entry in &bundle.manifest.conversion_report.entries {
        if entry.component_type.is_empty() {
            continue;
        }
        let known = harness
            .component_kinds
            .iter()
            .any(|kind| kind.as_str() == entry.component_type);
        if !known {
            return Err(Error::refuse(
                WireReason::UnsupportedComponentKind,
                format!(
                    "the bundle declares component {:?} as kind {:?}, which {} does not implement",
                    entry.stable_id, entry.component_type, harness.provider_id
                ),
            ));
        }
    }
    Ok(())
}

/// Every path a bundle writes must be one this harness owns.
///
/// A file outside the declared surface would be installed here and then left
/// behind by `remove`, and unaccounted for by `status`. Ownership and effect are
/// the same set or they are nothing.
fn check_within_surface<'a>(
    harness: &Harness,
    paths: impl Iterator<Item = &'a String>,
) -> Result<()> {
    for path in paths {
        if !harness.owns(path) {
            return Err(Error::refuse(
                WireReason::UnsupportedNativeSurface,
                format!(
                    "the bundle writes {path:?}, which is outside the surface {} owns",
                    harness.provider_id
                ),
            ));
        }
    }
    Ok(())
}

fn open(harness: &Harness, target: &Path) -> Result<(Target, std::path::PathBuf, Pool)> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let control = resolved.ensure_control_directory()?;
    let pool = Pool::open(&control, facts::BACKUP_SLOTS)?;
    Ok((resolved, control, pool))
}

/// Open a target for reading, creating nothing inside it.
///
/// [`open`] makes the control directory and the backup pool, which is right
/// when an effect is about to be written and wrong when a command is only
/// reporting. `status` used [`open`], so observing a fresh target left a
/// directory behind -- and a consumer that had just been told the target was
/// empty found it no longer was, because asking had changed the answer.
fn observe(harness: &Harness, target: &Path) -> Result<(Target, std::path::PathBuf, Pool)> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let control = resolved.control_directory();
    let pool = Pool::observe(&control, facts::BACKUP_SLOTS)?;
    Ok((resolved, control, pool))
}

/// Report the target without changing it, including a schema this build cannot write.
///
/// The shape is the consumer's, not ours. `ai_stp` reads exactly two fields to
/// decide what it is looking at — `state`, one of `missing`, `unmanaged` or
/// `managed`, and `target_digest` — and it calls this twice, requiring the two
/// answers to be *identical*. So nothing here may vary between calls: no clock,
/// no counter, no ordering that depends on a directory walk.
fn status(harness: &Harness, target: &Path) -> Result<serde_json::Value> {
    let (resolved, control, pool) = observe(harness, target)?;
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let journal = Journal::read(&control).ok().flatten();

    let reading = ProviderState::read(resolved.root(), harness.state_file)?;
    // `managed` carries our state; `unmanaged` holds content that is not ours;
    // `missing` means there is nothing here at all.
    //
    // That last one used to be looser -- it asked whether this provider owned
    // anything, so a directory full of another product's files reported
    // `missing`. A consumer reads this to decide what it is looking at, and
    // being told a populated directory is missing invites it to treat the
    // place as free. Emptiness is about the directory, not about us.
    let is_empty =
        fs::read_dir(resolved.root()).map_or(true, |mut entries| entries.next().is_none());
    let state = match &reading {
        StateReading::Current(_) => "managed",
        _ if is_empty => "missing",
        _ => "unmanaged",
    };

    let provider_state = match reading {
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
        "state": state,
        "target_digest": identity,
        "protocol_version": provider_v3::PROTOCOL_VERSION,
        "provider_id": harness.provider_id,
        "harness_id": harness.harness_id,
        "canonical_target": resolved.root().to_string_lossy(),
        "target_identity_digest": identity,
        "provider_state": provider_state,
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

/// Check a bundle against the exact claim that named it.
///
/// Every answer carries the four echoes, refusal included. That is the point of
/// them: without the echoes a consumer cannot tell whether a refusal concerns
/// the bytes it sent or some other bundle entirely, and a refusal it cannot
/// attribute is a refusal it cannot act on.
///
/// So a failure here is *not* propagated as an error. It is turned into an
/// answer — `rejected: true`, a stable reason, and the echoes — because that is
/// what `validate-bundle` means.
fn validate_bundle(harness: &Harness, bundle: &ArgvBundle) -> serde_json::Value {
    // There is no error path out of here, and the signature says so.
    match verified_bundle(harness, bundle) {
        Ok(_) => provider_v3::plan::bundle_accepted(&bundle.binding),
        Err(error) => {
            // A refusal always names a reason. A declaration defect in this
            // build is not one, and reporting it as a bundle problem would
            // blame the caller's bytes for our own; it is reported under the
            // format reason with the detail saying what actually happened.
            let reason = error
                .reason()
                .unwrap_or(WireReason::UnsupportedBundleFormat);
            provider_v3::plan::rejected_with_detail(&bundle.binding, reason, Some(error.detail()))
        }
    }
}

/// Refuse a request this build could not honour as asked, before promising it.
///
/// Both checks belong before any effect is described, because both describe a
/// plan that could never be applied -- and a refusal deferred to apply time
/// arrives after the consumer has stored the plan, scheduled it, and come back.
fn honourable(harness: &Harness, request: &PlanRequest) -> Result<()> {
    // A flag that means nothing to this operation is refused rather than
    // dropped: silently ignoring it would report success for a request that was
    // only partly understood, which is the rule the argv parser already keeps.
    if !Operation::SOFTWARE.contains(&request.operation) {
        if let Some(named) = request.prefix.as_deref() {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                format!(
                    "{} configures a target and installs no program, so --prefix {} means \
                     nothing to it",
                    request.operation,
                    named.display()
                ),
            ));
        }
        if let Some(asked) = request.software_version.as_deref() {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                format!(
                    "{} installs no program, so --software-version {asked:?} means nothing to it",
                    request.operation
                ),
            ));
        }
    }

    // A profile this build never advertised cannot be honoured, and recording
    // it in a plan would be worse than refusing: the apply would run under the
    // only posture this build has while the artifact claimed another.
    //
    // This used to answer `projection_profile_mismatch`, documented here as a
    // compromise: the closed set carried no permission-profile refusal, and that
    // was the nearest thing to "a profile you named is not one I have". Kit
    // 0.2.1 added `unsupported_permission_profile`, so the compromise is over
    // and the reason says what actually happened.
    if let Some(profile) = request.permission_profile.as_deref()
        && !harness.permission_profiles.contains(&profile)
    {
        return Err(Error::refuse(
            WireReason::UnsupportedPermissionProfile,
            format!(
                "{profile:?} is not a permission profile {} declares; it offers {:?}",
                harness.provider_id, harness.permission_profiles
            ),
        ));
    }

    // Read the deadline before promising anything. A plan carrying an expiry
    // this build cannot parse is a plan that can never authorize an apply, and
    // handing one back is a refusal deferred to the moment it costs most.
    if expiry::parse_utc_seconds(&request.expires_at).is_none() {
        return Err(Error::refuse(
            WireReason::Stale,
            format!(
                "the expiry {:?} is not the exact shape YYYY-MM-DDTHH:MM:SS.mmmZ, \
                 so no plan made from it could ever be applied",
                request.expires_at
            ),
        ));
    }

    Ok(())
}

/// Produce a plan without touching the target.
/// What writing a bundle over the target will do, enumerated for the plan.
///
/// The bundle is read and verified here rather than at apply time only, so a
/// plan is never issued for bytes that would be refused when it was applied.
fn bundle_effects(harness: &Harness, request: &PlanRequest) -> Result<Vec<String>> {
    let Some(named) = request.bundle.as_ref() else {
        return Err(Error::refuse(
            WireReason::UnsupportedBundleFormat,
            format!(
                "{} arrives as a bundle, and none was named",
                request.operation
            ),
        ));
    };
    let verified = verified_bundle(harness, named)?;
    let mut effects = vec![
        "capture the current target into a new backup slot".to_owned(),
        format!(
            "write the {} declared files over the entries this provider owns",
            verified.files.len()
        ),
    ];
    effects.extend(
        verified
            .files
            .keys()
            .take(16)
            .map(|path| format!("write {path}")),
    );
    Ok(effects)
}

fn plan(harness: &Harness, target: &Path, request: &PlanRequest) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    setup_core::journal::require_clean_for_planning(
        &control,
        &control.join("transaction"),
        &pool.partial_slots()?,
    )?;

    honourable(harness, request)?;

    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let profile = harness.projection_profile()?;
    let build_digest = harness.build_digest()?;

    let mut software_artifacts = Vec::new();
    let (effects, backup_ref, restore_target_digest) = match request.operation {
        Operation::SoftwareInstall | Operation::SoftwareUpdate | Operation::SoftwareRemove => {
            let (planned, effects) = software::plan(
                harness,
                request.prefix.as_deref(),
                request.operation,
                request.software_version.as_deref(),
            )?;
            software_artifacts = planned;
            (effects, None, None)
        }
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
                    "capture the current target before restoring".to_owned(),
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
        Operation::Install | Operation::Replace => (bundle_effects(harness, request)?, None, None),
        other @ Operation::Launch => {
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
        software_artifacts,
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
    /// Take over a target the frozen estate's program still claims.
    ///
    /// Writes none of the product's files: they are already what the old stamp
    /// recorded, and this only changes who owns them. The stamp itself is moved
    /// into this provider's control directory rather than deleted — the old
    /// program stops recognising it there, and the pre-adoption state is one
    /// `mv` away from being back.
    Adopt {
        /// The stamp file to move aside.
        stamp: std::path::PathBuf,
    },
    /// Write a verified `HarnessBundle` over those namespaces.
    ///
    /// The bundle is read and checked *before* this effect exists, so reaching
    /// it means every digest, path and limit already held.
    MaterializeBundle {
        /// Each declared file's bytes and mode, by target-relative path.
        files: &'a BTreeMap<String, (Vec<u8>, u32)>,
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
    /// What provider state should record about whatever this leaves applied.
    pub applied: Applied,
}

/// The identity of what a mutation leaves in the target, as state records it.
///
/// The contract asks a provider to say *what* is applied, not only that
/// something is. Before this existed only the setup's name was carried, and a
/// restore carried nothing at all -- so a target holding a known setup byte for
/// byte reported itself as unnamed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// The setup in effect, when one is named.
    pub setup_id: Option<String>,
    /// The digest of that setup's definition, which is its real identity.
    pub setup_definition_digest: Option<String>,
    /// The setup version, when the thing that arrived stated one.
    ///
    /// A bundle's setup passport does; a setup in the local catalog carries an
    /// id and a description and no version, and inventing one there would be
    /// worse than the null.
    pub setup_version: Option<String>,
    /// The bundle format, when a bundle put it there.
    pub bundle_format: Option<String>,
    /// The bundle's own digest.
    pub bundle_digest: Option<String>,
    /// The digest of the artifact the bundle arrived in.
    pub artifact_digest: Option<String>,
    /// The components the projection carried, in the order it declared them.
    ///
    /// The contract asks a provider to record which components a target holds,
    /// and the bundle's conversion report is the only place their stable
    /// identities appear — the manifest carries paths and the setup passport
    /// carries references without kinds. This was written as an empty list
    /// whatever arrived, so a target configured from a bundle reported that it
    /// held no components while holding every one the bundle named.
    ///
    /// Empty is still correct for the local catalog: a setup there is a tree of
    /// files with an id and a description, and it has no component identities
    /// to record. Inventing some would be worse than the empty list.
    pub component_refs: Vec<String>,
}

/// Apply one exact plan under the target lock.
fn apply(
    harness: &Harness,
    target: &Path,
    plan_path: &Path,
    plan_digest: &str,
    bundle: Option<&ArgvBundle>,
    prefix: Option<&Path>,
    // `downloaded`, not `artifacts`: the plan artifact is read into a local of
    // a similar name a few lines below, and one of the two had to give way.
    downloaded: &[std::path::PathBuf],
) -> Result<serde_json::Value> {
    let verified: Option<Bundle>;
    let artifact = load_plan(plan_path, plan_digest)?;
    let operation = operation_of(&artifact)?;
    let expires_at = string_field(&artifact, "expires_at")?;
    // Both refusals are `stale` and both are fail-closed, but they are not the
    // same problem, and saying "expired" to someone whose timestamp simply did
    // not parse sends them to look at a clock instead of at a format. Costing
    // an hour of that is how the distinction earned its two lines.
    match expiry::parse_utc_seconds(&expires_at) {
        None => {
            return Err(Error::refuse(
                WireReason::Stale,
                format!(
                    "the plan's expiry {expires_at:?} is not the exact shape YYYY-MM-DDTHH:MM:SS.mmmZ, so no authorization could be read from it; no effect was made"
                ),
            ));
        }
        Some(_) if expiry::has_expired(&expires_at, SystemTime::now()) => {
            return Err(Error::refuse(
                WireReason::Stale,
                "this plan expired before it was applied; no effect was made",
            ));
        }
        Some(_) => {}
    }

    // The software lifecycle writes under the control directory and never
    // touches the namespaces the effect machinery below exists to mutate, so it
    // parts company here rather than pretending to be one of those effects.
    if Operation::SOFTWARE.contains(&operation) {
        return software::apply(harness, prefix, operation, downloaded);
    }
    if let Some(named) = prefix {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "{operation} configures a target and installs no program, so --prefix {} means \
                 nothing to it",
                named.display()
            ),
        ));
    }

    // A bundle names itself: the contract asks provider state to record which
    // bundle put the bytes there, and it arrives bound to exact identities.
    let mut applied = Applied::default();
    let effect = match operation {
        Operation::Backup => Effect::Backup,
        Operation::Restore => Effect::Restore {
            backup_ref: artifact
                .get("backup_ref")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        },
        Operation::Remove => Effect::Remove,
        Operation::Install | Operation::Replace => {
            let Some(named) = bundle.as_ref() else {
                return Err(Error::refuse(
                    WireReason::UnsupportedBundleFormat,
                    format!("{operation} arrives as a bundle, and none was named"),
                ));
            };
            // Re-read and re-verify: the plan authorized an identity, not a file
            // that might have changed on disk since.
            verified = Some(verified_bundle(harness, named)?);
            let Some(ready) = verified.as_ref() else {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    "bundle vanished",
                ));
            };
            applied.bundle_format = Some(named.binding.bundle_format.clone());
            applied.bundle_digest = Some(named.binding.bundle_digest.clone());
            applied.artifact_digest = Some(named.binding.artifact_digest.clone());
            // Two provenance fields the contract names and the passport
            // states. They were null for every bundle install, because the
            // passport was a required member that nothing read.
            if !ready.passport.stable_id.is_empty() {
                applied.setup_id = Some(ready.passport.stable_id.clone());
            }
            if !ready.passport.version.is_empty() {
                applied.setup_version = Some(ready.passport.version.clone());
            }
            applied.component_refs = ready
                .manifest
                .conversion_report
                .entries
                .iter()
                .map(|entry| entry.stable_id.clone())
                .filter(|stable_id| !stable_id.is_empty())
                .collect();
            Effect::MaterializeBundle {
                files: &ready.files,
            }
        }
        other => {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                format!("{other} is not declared by this provider"),
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
            applied,
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
    let (previous_setup, previous_definition) =
        match ProviderState::read(resolved.root(), harness.state_file)? {
            StateReading::Current(current) => {
                (current.setup_stable_id, current.setup_definition_digest)
            }
            _ => (None, None),
        };
    let captured = pool.capture(resolved.root(), harness.native_namespaces, |backup_ref| {
        SlotRecord {
            schema_version: SLOT_SCHEMA,
            backup_ref,
            operation: operation_name.clone(),
            operation_id: operation_id.clone(),
            target_identity_digest: identity.clone(),
            setup_id: previous_setup.clone(),
            setup_definition_digest: previous_definition.clone(),
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

    // What the state will say is applied. A restore learns it from the slot it
    // restores; every other effect was told at plan time.
    let mut applied = mutation.applied.clone();
    let outcome = match &mutation.effect {
        // The capture above *is* the effect. Nothing else is written.
        Effect::Backup => Ok(()),
        Effect::Restore { backup_ref } => {
            let record = chosen_backup(&pool, backup_ref.as_deref())?;
            let payload = pool.payload_of(&record.backup_ref)?;
            // The slot wrote down which setup was in effect when it was taken.
            // Returning its bytes without its name would report a target that
            // is a known setup byte for byte as one nobody can name.
            applied.setup_id.clone_from(&record.setup_id);
            applied
                .setup_definition_digest
                .clone_from(&record.setup_definition_digest);
            replace_managed_from(harness, &resolved, &payload)
        }
        Effect::Remove => remove_managed(harness, &resolved),
        Effect::Materialize { setup } => {
            setup.check_within(harness)?;
            replace_managed_from(harness, &resolved, &setup.payload)
        }
        Effect::MaterializeBundle { files } => write_bundle_files(harness, &resolved, files),
        Effect::Adopt { stamp } => {
            crate::adopt::keep_aside(&control, stamp, harness.predecessor_state_file).map(|_| ())
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
        &applied,
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
        "setup_id": applied.setup_id,
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

/// Write a verified bundle over the namespaces this provider owns.
///
/// The owned entries are cleared first, for the same reason selecting a setup
/// clears them: the result is the bundle's complete state, not the bundle merged
/// into whatever happened to be there.
fn write_bundle_files(
    harness: &Harness,
    target: &Target,
    files: &BTreeMap<String, (Vec<u8>, u32)>,
) -> Result<()> {
    remove_managed(harness, target)?;
    for (relative, (bytes, mode)) in files {
        // `atomic_write` creates the parent; this used to do it here, and the
        // catalog path next door did not, which is how they came apart.
        let destination = target.root().join(relative);
        lock::atomic_write(&destination, bytes)?;
        set_mode(&destination, *mode)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
        Error::from(
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!("cannot set the mode of {}", path.display()),
            )
            .with_source(error),
        )
    })
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    // Windows has no Unix mode to set. Claiming one was applied would be a
    // claim about permissions this platform does not express that way.
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
    applied: &Applied,
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
        setup_stable_id: applied.setup_id.clone(),
        // A local setup carries an id and a description and no version, so
        // there is no version to record and inventing one would be worse than
        // the null. A bundle states one in its passport, and that is where this
        // comes from.
        setup_version: applied.setup_version.clone(),
        // Still null, deliberately. The passport does not carry its own digest
        // and the contract does not define how one is taken, so a value
        // computed here would be this program's opinion of the passport's
        // identity rather than the passport's.
        setup_version_passport_digest: None,
        setup_definition_digest: applied.setup_definition_digest.clone(),
        component_refs: applied.component_refs.clone(),
        bundle_format: applied.bundle_format.clone(),
        bundle_digest: applied.bundle_digest.clone(),
        artifact_digest: applied.artifact_digest.clone(),
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

    /// The bytes `TEST_SOFTWARE` installs. A shell script rather than a real
    /// binary, so the test can run what it installed and read the answer.
    pub(crate) const TEST_PAYLOAD: &[u8] = b"#!/bin/sh\nexec echo test-harness 1.2.3\n";

    /// One artifact, published for every platform, so the test does not depend
    /// on which machine runs it. `Raw` because raw bytes have one digest on
    /// every system, where a compressor's output is only as stable as its
    /// version.
    pub(crate) const TEST_ARTIFACTS: &[setup_core::software::Artifact] = &[
        test_artifact("linux/x86_64"),
        test_artifact("linux/arm64"),
        test_artifact("macos/x86_64"),
        test_artifact("macos/arm64"),
        test_artifact("windows/x86_64"),
        test_artifact("windows/arm64"),
    ];

    const fn test_artifact(platform: &'static str) -> setup_core::software::Artifact {
        setup_core::software::Artifact {
            platform,
            url: "https://example.invalid/test-harness",
            bytes: 39,
            sha256: "sha256:0c7c47cc1bc9116feb15bd468d039e954093ccfca8d6246b32ea94d1ab2213ad",
            shape: setup_core::software::Shape::Raw,
            member: "",
        }
    }

    pub(crate) const TEST_SOFTWARE: setup_core::software::Software =
        setup_core::software::Software {
            version: "1.2.3",
            command: "test-harness",
            delivery: setup_core::software::Delivery::Artifacts(TEST_ARTIFACTS),
            unsupported: &[],
        };

    pub(crate) const TEST: Harness = Harness {
        software: Some(TEST_SOFTWARE),
        predecessor_state_file: "NDDEV-TEST-SETUP.json",
        embedded_setups: &[],
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

    use crate::wire::tests_support::{TEST, TEST_PAYLOAD};

    const RELEASE: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    /// A per-test directory holding the target and anything written beside it.
    ///
    /// The plan artifact must live *outside* the target: inside, it would change
    /// the target's identity between plan and apply, and the apply would then
    /// correctly refuse its own plan as stale. It must also be unique per test,
    /// because these run in parallel.
    fn scratch(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("harness-runtime-{name}-{}", std::process::id()));
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
    fn a_component_kind_this_build_does_not_implement_is_refused() {
        // The kind lives only in the conversion report: the manifest carries
        // none and the passport carries references without them. A provider
        // that does not read that report writes the files and reports success
        // for a component it does not understand.
        let target = seeded("kind");
        let (bytes, digest, artifact) =
            bundle_bytes_declaring(&[("AGENTS.md", "x", 0o644)], Some("quantum-manifest"));
        let path = target.join("..").join("kind.zip");
        fs::write(&path, &bytes).unwrap();
        let flags = bundle_flags(&path, &digest, &artifact, bytes.len());
        let answer = run(args(
            "validate-bundle",
            &target,
            &flags.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
        assert_eq!(answer["rejected"], true, "{answer}");
        assert_eq!(answer["reason"], "unsupported_component_kind");

        // A kind this harness does implement still passes.
        let (bytes, digest, artifact) =
            bundle_bytes_declaring(&[("AGENTS.md", "x", 0o644)], Some("instruction"));
        fs::write(&path, &bytes).unwrap();
        let flags = bundle_flags(&path, &digest, &artifact, bytes.len());
        let ok = run(args(
            "validate-bundle",
            &target,
            &flags.iter().map(String::as_str).collect::<Vec<_>>(),
        ));
        assert_eq!(ok["valid"], true, "{ok}");
    }

    #[test]
    fn a_nested_namespace_is_owned_all_the_way_down() {
        // Codex declares `.agents/skills` and nothing else under `.agents`.
        // Matching on the first path component alone read that as `.agents`,
        // which is not declared, so every write to the route the compiler
        // actually uses was refused. This fails on that reading.
        const NESTED: Harness = Harness {
            native_namespaces: &[".agents/skills", "AGENTS.md"],
            ..TEST
        };
        assert!(NESTED.owns(".agents/skills"));
        assert!(NESTED.owns(".agents/skills/review/SKILL.md"));
        assert!(NESTED.owns("AGENTS.md"));
    }

    #[test]
    fn a_nested_namespace_does_not_claim_its_parent() {
        // The other half: declaring `.agents/skills` must not hand this
        // provider `.agents/hooks.json`, which belongs to the product.
        const NESTED: Harness = Harness {
            native_namespaces: &[".agents/skills"],
            ..TEST
        };
        assert!(!NESTED.owns(".agents"));
        assert!(!NESTED.owns(".agents/hooks.json"));
    }

    #[test]
    fn a_namespace_does_not_swallow_a_neighbour_that_starts_with_it() {
        // `skills-experimental` is a different directory from `skills`, and a
        // prefix comparison without the separator would take it.
        const NEIGHBOURS: Harness = Harness {
            native_namespaces: &["skills"],
            ..TEST
        };
        assert!(NEIGHBOURS.owns("skills/a/SKILL.md"));
        assert!(!NEIGHBOURS.owns("skills-experimental/a/SKILL.md"));
        assert!(!NEIGHBOURS.owns("skillsdata"));
    }

    #[test]
    fn status_reports_an_untouched_target_without_changing_it() {
        let target = seeded("status");
        let before = fs::read_to_string(target.join("AGENTS.md")).unwrap();
        let answer = run(args("status", &target, &[]));
        // The seeded target holds files this provider owns but no state of ours.
        assert_eq!(answer["state"], "unmanaged");
        assert_eq!(answer["provider_state"]["present"], false);
        assert!(answer["journal"].is_null());
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            before
        );
    }

    #[test]
    fn a_permission_profile_this_build_never_advertised_is_refused() {
        // `provider-info` publishes a closed list. Accepting anything else and
        // writing it into the plan would record a posture the apply would not
        // use -- the artifact would claim one thing and the effect be another.
        let target = seeded("permission-profile");
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
                "--permission-profile",
                "not-declared-anywhere",
            ],
        ));
        // This assertion used to be `is_some()`, because the closed set carried
        // no permission-profile refusal and the reason on the wire was a
        // documented compromise. Kit 0.2.1 added one, so the test names it.
        assert_eq!(
            error.reason(),
            Some(WireReason::UnsupportedPermissionProfile)
        );
        assert!(
            error.detail().contains("not-declared-anywhere"),
            "{}",
            error.detail()
        );

        // The declared one still plans.
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
                "--permission-profile",
                "default",
            ],
        ));
        assert_eq!(planned["state"], "planned");
    }

    #[test]
    fn an_unreadable_expiry_says_so_instead_of_claiming_the_plan_expired() {
        // Both refusals are `stale` and both are right to refuse. What differs
        // is where the reader is sent to look: a clock, or a format. Being told
        // "expired" about a deadline half an hour in the future costs an hour.
        let target = seeded("expiry-shape");
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
                // Valid RFC 3339, and still not the shape the consumer emits.
                "--expires-at",
                "2026-08-23T22:21:39Z",
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::Stale));
        assert!(
            error.detail().contains("not the exact shape"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn observing_a_target_leaves_nothing_behind() {
        // `status` used to create the control directory, so asking a fresh
        // target what it held made it hold something. The consumer's own
        // conformance caught it from the outside: it hands over a directory the
        // operator named and then finds it is no longer empty.
        let empty = scratch("status-creates-nothing").join("target");
        fs::create_dir_all(&empty).unwrap();

        let answer = run(args("status", &empty, &[]));
        assert_eq!(answer["state"], "missing");

        let left = fs::read_dir(&empty).unwrap().count();
        assert_eq!(
            left, 0,
            "status wrote into a target it was only reporting on"
        );
    }

    #[test]
    fn status_speaks_the_three_states_the_consumer_reads() {
        // `ai_stp` decides what it is looking at from `state` and `target_digest`
        // alone, and its set is closed. Anything else reads as a broken provider.
        let empty = scratch("status-missing").join("target");
        let answer = run(args("status", &empty, &[]));
        assert_eq!(answer["state"], "missing");
        assert!(
            answer["target_digest"]
                .as_str()
                .unwrap()
                .starts_with("sha256:")
        );

        let seeded_target = seeded("status-unmanaged");
        assert_eq!(
            run(args("status", &seeded_target, &[]))["state"],
            "unmanaged"
        );

        // Content that is not ours still makes a target populated: reporting
        // `missing` would tell a consumer the place is free while another
        // product's files sit in it.
        let foreign = scratch("status-foreign").join("target");
        fs::create_dir_all(foreign.join("someone-elses")).unwrap();
        fs::write(foreign.join("someone-elses/notes.md"), "not ours").unwrap();
        assert_eq!(run(args("status", &foreign, &[]))["state"], "unmanaged");

        plan_then_apply(&seeded_target, "backup", &[]);
        assert_eq!(run(args("status", &seeded_target, &[]))["state"], "managed");
    }

    #[test]
    fn two_status_calls_return_the_same_bytes() {
        // The consumer calls it twice and requires the answers to be identical,
        // so nothing in here may vary: no clock, no counter, no walk order.
        let target = seeded("status-repeatable");
        plan_then_apply(&target, "backup", &[]);
        let first = run(args("status", &target, &[]));
        let second = run(args("status", &target, &[]));
        assert_eq!(first, second);
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

    /// Build a canonical bundle whose three identities are internally consistent.
    fn bundle_bytes(files: &[(&str, &str, u32)]) -> (Vec<u8>, String, String) {
        bundle_bytes_declaring(files, None)
    }

    /// The same, with a component kind stated in the conversion report.
    fn bundle_bytes_declaring(
        files: &[(&str, &str, u32)],
        kind: Option<&str>,
    ) -> (Vec<u8>, String, String) {
        use provider_v3::bundle::{BUNDLE_DOMAIN, FILES_PREFIX, MANIFEST_MEMBER, REQUIRED_MEMBERS};
        use provider_v3::zip::build::{Entry, write};

        let records: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, body, mode)| {
                serde_json::json!({
                    "schema_version": 1,
                    "path": path,
                    "digest": setup_core::digest::of_bytes(body.as_bytes()),
                    "byte_length": body.len(),
                    "mode": mode,
                    "owner": "",
                })
            })
            .collect();
        let mut manifest = serde_json::json!({
            "schema_version": 1,
            "bundle_format": "ai-stp-bundle/1",
            "protocol_version": provider_v3::bundle::BUNDLE_PROTOCOL_VERSION,
            "harness_id": TEST.harness_id,
            "builder_version": "0.1.0",
            "input_digest": "sha256:".to_owned() + &"3".repeat(64),
            "managed_paths": files.iter().map(|(path, _, _)| *path).collect::<Vec<_>>(),
            "files": records,
            "limits": {
                "max_files": 2000,
                "max_file_bytes": 4 * 1024 * 1024,
                "max_bundle_bytes": 64 * 1024 * 1024,
            },
        });
        if let Some(kind) = kind {
            manifest["conversion_report"] = serde_json::json!({
                "complete": true,
                "entries": [{
                    "stable_id": "component_00000000000000000000000000",
                    "component_type": kind,
                    "native_surface": files.first().map_or("", |(path, _, _)| path),
                    "state": "complete",
                    "losses": [],
                }],
            });
        }
        let bundle_digest =
            setup_core::digest::of_domain_canonical_json(BUNDLE_DOMAIN, &manifest).unwrap();
        manifest["bundle_digest"] = serde_json::json!(bundle_digest);

        let mut entries = vec![Entry {
            name: MANIFEST_MEMBER.to_owned(),
            data: setup_core::canonical::to_canonical_bytes(&manifest).unwrap(),
            mode: 0o644,
        }];
        for name in REQUIRED_MEMBERS.iter().skip(1) {
            // The passport carries the setup identity a provider must record.
            // An empty object here would have made every bundle test agree with
            // a provider that read nothing, which is how the field stayed null.
            let data = if *name == "setup-passport.json" {
                serde_json::to_vec(&serde_json::json!({
                    "stable_id": "setup_00000000000000000000000000",
                    "version": "3.1.0",
                    "harness_id": TEST.harness_id,
                }))
                .unwrap()
            } else {
                b"{}".to_vec()
            };
            entries.push(Entry {
                name: (*name).to_owned(),
                data,
                mode: 0o644,
            });
        }
        for (path, body, mode) in files {
            entries.push(Entry {
                name: format!("{FILES_PREFIX}{path}"),
                data: body.as_bytes().to_vec(),
                mode: *mode,
            });
        }
        let bytes = write(&entries);
        let artifact = setup_core::digest::of_bytes(&bytes);
        (bytes, bundle_digest, artifact)
    }

    fn bundle_flags(path: &Path, bundle_digest: &str, artifact: &str, size: usize) -> Vec<String> {
        vec![
            "--bundle".to_owned(),
            path.to_string_lossy().into_owned(),
            "--bundle-format".to_owned(),
            "ai-stp-bundle/1".to_owned(),
            "--bundle-digest".to_owned(),
            bundle_digest.to_owned(),
            "--artifact-digest".to_owned(),
            artifact.to_owned(),
            "--bundle-size".to_owned(),
            size.to_string(),
        ]
    }

    #[test]
    fn what_a_bundle_states_about_itself_is_recorded_in_provider_state() {
        // `component_refs` is a provenance field the contract asks for, and the
        // conversion report is the only place a component's stable identity
        // appears. It was written as an empty list whatever arrived, so a
        // target configured from a bundle reported holding no components while
        // holding every one the bundle named.
        let target = seeded("bundle-components");
        let (bytes, bundle_digest, artifact) =
            bundle_bytes_declaring(&[("AGENTS.md", "# named\n", 0o644)], Some("instruction"));
        let artifact_path = target.join("..").join("components.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01COMPONENT".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", &target, &borrowed));
        let plan_path = target.join("..").join("components-plan.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();
        let mut apply_args = vec![
            "--plan".to_owned(),
            plan_path.to_string_lossy().into_owned(),
            "--plan-digest".to_owned(),
            planned["plan_digest"].as_str().unwrap().to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
        ];
        apply_args.extend(flags);
        let borrowed: Vec<&str> = apply_args.iter().map(String::as_str).collect();
        assert_eq!(
            run(args("apply-operation", &target, &borrowed))["state"],
            "verified"
        );

        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target.join(TEST.state_file)).unwrap())
                .unwrap();
        assert_eq!(
            state["component_refs"],
            serde_json::json!(["component_00000000000000000000000000"])
        );
        // The passport is a required member and was required and discarded, so
        // a target configured from a bundle recorded no setup identity and no
        // setup version at all.
        assert_eq!(state["setup_stable_id"], "setup_00000000000000000000000000");
        assert_eq!(state["setup_version"], "3.1.0");
        // Still null, and deliberately: the passport does not state its own
        // digest and the contract does not define how one is taken.
        assert!(state["setup_version_passport_digest"].is_null());
    }

    #[test]
    fn a_setup_from_the_local_catalog_records_no_components_because_it_has_none() {
        // A catalog setup is a tree of files with an id and a description. It
        // carries no component identities, and inventing some would be worse
        // than the empty list the contract allows.
        let target = seeded("catalog-components");
        plan_then_apply(&target, "backup", &[]);
        let state: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(target.join(TEST.state_file)).unwrap())
                .unwrap();
        assert_eq!(state["component_refs"], serde_json::json!([]));
    }

    #[test]
    fn a_bundle_installs_over_the_wire_and_leaves_unowned_files_alone() {
        let target = seeded("bundle-install");
        let (bytes, bundle_digest, artifact) = bundle_bytes(&[
            ("AGENTS.md", "# from a bundle\n", 0o644),
            ("skills/b.md", "two", 0o644),
        ]);
        let artifact_path = target.join("..").join("bundle.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01BUNDLE".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", &target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert_eq!(
            planned["valid"], true,
            "a plan carrying a bundle echoes its validity"
        );
        assert_eq!(planned["bundle_digest"], bundle_digest.as_str());

        let plan_path = target.join("..").join("bundle-plan.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();
        let mut apply_args = vec![
            "--plan".to_owned(),
            plan_path.to_string_lossy().into_owned(),
            "--plan-digest".to_owned(),
            planned["plan_digest"].as_str().unwrap().to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
        ];
        apply_args.extend(flags);
        let borrowed: Vec<&str> = apply_args.iter().map(String::as_str).collect();
        let applied = run(args("apply-operation", &target, &borrowed));

        assert_eq!(applied["state"], "verified");
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# from a bundle\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("skills").join("b.md")).unwrap(),
            "two"
        );
        // The seeded settings.json was not in the bundle, so the complete state
        // it describes does not include it.
        assert!(!target.join("settings.json").exists());
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
    fn a_bundle_writing_outside_the_declared_surface_never_reaches_the_target() {
        let target = seeded("bundle-outside");
        let (bytes, bundle_digest, artifact) =
            bundle_bytes(&[("AGENTS.md", "x", 0o644), ("elsewhere.txt", "y", 0o644)]);
        let artifact_path = target.join("..").join("hostile.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01HOSTILE".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags);
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let error = refuse(args("plan-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        assert!(!target.join("elsewhere.txt").exists());
    }

    #[test]
    fn install_without_a_bundle_says_a_bundle_is_what_it_takes() {
        let target = seeded("bundle-missing");
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
            assert_eq!(
                error.reason(),
                Some(WireReason::UnsupportedBundleFormat),
                "{operation}"
            );
            assert!(
                error.detail().contains("none was named"),
                "{operation}: {error}"
            );
        }
        let info = TEST.provider_info().unwrap();
        assert!(info.declares(Operation::Install));
        assert!(info.declares(Operation::Replace));
    }

    #[test]
    fn launch_with_nothing_installed_says_to_install_first() {
        // The failure this command exists to avoid is starting a name found on
        // PATH. So an empty prefix is a refusal that names the path it looked
        // at, not a fallback to whatever else answers to `test-harness`.
        let target = seeded("launch-empty");
        let prefix = ready_prefix(&target);
        let error = refuse(args("launch", &target, &["--prefix", &prefix]));
        assert_eq!(error.reason(), Some(WireReason::ProviderUnavailable));
        assert!(
            error.detail().contains("software_install"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn launch_without_a_prefix_says_where_a_program_lives() {
        let target = seeded("launch-noprefix");
        let error = refuse(args("launch", &target, &[]));
        assert!(error.detail().contains("--prefix"), "{}", error.detail());
    }

    #[test]
    fn a_build_that_cannot_point_the_product_at_a_target_does_not_declare_launch() {
        // Every command here takes a `--target`. A product documenting no
        // environment variable for its configuration home cannot be pointed at
        // one, so a launch would be answering a different question.
        let mut mute = TEST;
        mute.config_home_env = "";
        assert!(!mute.can_launch());
        let info = mute.provider_info().unwrap();
        assert!(!info.declares(Operation::Launch));

        let error = software::launch(&mute, Path::new("/nowhere"), None, &[]).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
        assert!(
            error.detail().contains("configuration home"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn a_build_that_installs_nothing_does_not_declare_launch_either() {
        let mut bare = TEST;
        bare.software = None;
        assert!(!bare.can_launch());
        assert!(!bare.provider_info().unwrap().declares(Operation::Launch));
        let error = software::launch(&bare, Path::new("/nowhere"), None, &[]).unwrap_err();
        assert!(error.detail().contains("PATH"), "{}", error.detail());
    }

    #[test]
    fn a_build_that_installs_and_can_be_pointed_declares_launch() {
        assert!(TEST.can_launch());
        let info = TEST.provider_info().unwrap();
        assert!(info.declares(Operation::Launch));
        assert!(info.supported_commands.iter().any(|c| c == "launch"));
    }

    #[test]
    fn what_launch_starts_is_the_file_that_was_installed() {
        // Proven without replacing this process: install, then check that the
        // path launch resolves is the exact executable the install exposed,
        // and that it runs and reports the version the plan named.
        let target = seeded("launch-installed");
        let file = downloaded(&target, TEST_PAYLOAD);
        let applied = plan_then_install(&target, "software_install", Some(&file));
        let exposed = std::path::PathBuf::from(applied["executable"].as_str().unwrap());

        let prefix = ready_prefix(&target);
        assert_eq!(
            exposed,
            std::path::Path::new(&prefix)
                .join("bin")
                .join("test-harness")
        );
        assert!(exposed.symlink_metadata().is_ok());

        #[cfg(unix)]
        {
            let output = run_once_it_is_not_busy(
                std::process::Command::new(&exposed).env(TEST.config_home_env, &target),
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout).trim(),
                "test-harness 1.2.3"
            );
        }
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

    // ── the software lifecycle ───────────────────────────────────────────────

    /// The program directory: a sibling of the target, never inside it.
    fn prefix_for(target: &Path) -> std::path::PathBuf {
        target.join("..").join("program")
    }

    /// Write the bytes a consumer would have fetched between the two phases.
    fn downloaded(target: &Path, bytes: &[u8]) -> std::path::PathBuf {
        let at = target.join("..").join("downloaded-artifact");
        fs::write(&at, bytes).unwrap();
        at
    }

    /// The argv every software plan in these tests shares.
    fn software_plan_args<'a>(operation: &'a str, prefix: &'a str) -> Vec<&'a str> {
        vec![
            "--operation",
            operation,
            "--provider-release-digest",
            RELEASE,
            "--operation-id",
            "operation_01SOFT",
            "--expires-at",
            far_future(),
            "--prefix",
            prefix,
        ]
    }

    /// An absolute program directory beside the target, created and canonical.
    fn ready_prefix(target: &Path) -> String {
        let prefix = prefix_for(target);
        fs::create_dir_all(&prefix).unwrap();
        fs::canonicalize(&prefix)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    fn plan_then_install(
        target: &Path,
        operation: &str,
        artifact: Option<&Path>,
    ) -> serde_json::Value {
        let prefix = ready_prefix(target);
        let planned = run(args(
            "plan-operation",
            target,
            &software_plan_args(operation, &prefix),
        ));
        assert_eq!(planned["state"], "planned", "plan refused: {planned}");
        let plan_path = target.join("..").join(format!("plan-{operation}.json"));
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();

        let digest = planned["plan_digest"].as_str().unwrap().to_owned();
        let path = plan_path.to_string_lossy().into_owned();
        let mut extra = vec![
            "--plan",
            &path,
            "--plan-digest",
            &digest,
            "--provider-release-digest",
            RELEASE,
            "--prefix",
            &prefix,
        ];
        let held;
        if let Some(file) = artifact {
            held = file.to_string_lossy().into_owned();
            extra.push("--software-artifact");
            extra.push(&held);
        }
        run(args("apply-operation", target, &extra))
    }

    /// Plan a software operation and return the whole response.
    fn software_plan(target: &Path, operation: &str) -> serde_json::Value {
        let prefix = ready_prefix(target);
        run(args(
            "plan-operation",
            target,
            &software_plan_args(operation, &prefix),
        ))
    }

    #[test]
    fn a_software_plan_names_the_exact_bytes_before_any_network_is_open() {
        let target = seeded("software-plan");
        let planned = software_plan(&target, "software_install");

        // The array, and the five fields agreed on ai_stp#414. One element is
        // one file, and apply receives one --software-artifact per element.
        let artifacts = planned["plan"]["software_artifacts"].as_array().unwrap();
        assert_eq!(artifacts.len(), 1);
        let only = &artifacts[0];
        let mut fields: Vec<&str> = only
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        assert_eq!(
            fields,
            vec!["byte_length", "entry_point", "platform", "sha256", "url"],
            "the plan carries the agreed fields and no others"
        );
        assert_eq!(only["byte_length"], 39);
        assert_eq!(
            only["sha256"],
            "sha256:0c7c47cc1bc9116feb15bd468d039e954093ccfca8d6246b32ea94d1ab2213ad"
        );
        assert_eq!(only["entry_point"], "bin/test-harness");

        // The plan says the download is somebody else's phase, which is why
        // this provider opens no socket in any of the three.
        let effects = planned["effects"].as_array().unwrap();
        assert!(
            effects[0].as_str().unwrap().contains("download phase"),
            "{effects:?}"
        );
    }

    #[test]
    fn a_software_operation_without_a_prefix_says_where_a_program_lives() {
        let target = seeded("software-noprefix");
        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "software_install",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01SOFT",
                "--expires-at",
                far_future(),
            ],
        ));
        assert!(error.detail().contains("--prefix"), "{}", error.detail());
    }

    #[test]
    fn a_relative_prefix_is_refused_because_a_plan_cannot_be_bound_to_one() {
        // Refused by the parser, before dispatch sees it: a path that resolves
        // against whatever directory the caller happened to be in is not
        // something a plan can be bound to, and that is true of every command.
        let target = seeded("software-relprefix");
        let error = argv::parse(args(
            "plan-operation",
            &target,
            &software_plan_args("software_install", "program"),
        ))
        .unwrap_err();
        assert!(error.detail().contains("absolute"), "{}", error.detail());
    }

    #[test]
    fn a_prefix_on_an_operation_that_installs_nothing_is_refused_not_ignored() {
        let target = seeded("software-strayprefix");
        // Not a literal `/tmp`: on Windows that is rooted but not absolute, so
        // the parser refuses it one step earlier and this test never reaches
        // its assertion. The three-OS matrix caught exactly that.
        let elsewhere = std::env::temp_dir();
        let elsewhere = elsewhere.to_string_lossy();
        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01SOFT",
                "--expires-at",
                far_future(),
                "--prefix",
                &elsewhere,
            ],
        ));
        assert!(
            error.detail().contains("means nothing"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn a_version_this_build_does_not_pin_is_refused_rather_than_neighboured() {
        let target = seeded("software-version");
        let prefix = ready_prefix(&target);
        let mut arguments = software_plan_args("software_install", &prefix);
        arguments.extend_from_slice(&["--software-version", "9.9.9"]);
        let error = refuse(args("plan-operation", &target, &arguments));
        assert!(error.detail().contains("1.2.3"), "{}", error.detail());

        // The pinned one is accepted when named explicitly.
        let mut exact = software_plan_args("software_install", &prefix);
        exact.extend_from_slice(&["--software-version", "1.2.3"]);
        let planned = run(args("plan-operation", &target, &exact));
        assert_eq!(planned["state"], "planned");
    }

    #[test]
    fn installing_places_a_command_and_leaves_the_configuration_alone() {
        let target = seeded("software-install");
        let before = run(args("status", &target, &[]))["target_identity_digest"].clone();

        let file = downloaded(&target, TEST_PAYLOAD);
        let applied = plan_then_install(&target, "software_install", Some(&file));
        assert_eq!(applied["state"], "verified");
        assert_eq!(applied["version"], "1.2.3");

        let exposed = Path::new(&ready_prefix(&target))
            .to_path_buf()
            .join("bin")
            .join("test-harness");
        assert!(exposed.symlink_metadata().is_ok(), "no command was exposed");
        assert_eq!(fs::read(&exposed).unwrap(), TEST_PAYLOAD);

        // The bytes live in a directory named for their version, so a second
        // version can arrive without disturbing this one.
        assert!(
            Path::new(&ready_prefix(&target))
                .to_path_buf()
                .join("1.2.3")
                .join("test-harness")
                .is_file()
        );

        // The whole claim of this path: a program was installed and not one
        // byte of the configuration this provider owns moved.
        let after = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_eq!(
            before, after,
            "installing software moved the target identity"
        );
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# first\n"
        );
    }

    /// Run something that was just written, tolerating a fork that has not
    /// finished exec'ing yet.
    ///
    /// Linux refuses to exec a file any process holds open for writing, with
    /// `ETXTBSY`. The test harness runs these in threads, and
    /// `Command::output` forks: between the fork and the child's exec, the
    /// child holds a copy of every descriptor its parent had, including a write
    /// handle another thread is about to close. A thread exec'ing that file in
    /// exactly that window is told it is busy.
    ///
    /// The window is microseconds and closes on its own, which is why this
    /// failed on one CI runner out of seven and passes locally every time. It
    /// is a property of writing and exec'ing in one multi-threaded process, and
    /// this program does neither: `launch` is its own invocation, reading a
    /// file some earlier invocation wrote. So the retry belongs here, in the
    /// test that creates the condition, and not in the code under test.
    #[cfg(unix)]
    fn run_once_it_is_not_busy(command: &mut std::process::Command) -> std::process::Output {
        for _ in 0..50 {
            match command.output() {
                Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                other => return other.unwrap(),
            }
        }
        panic!("it stayed busy for a second, which is longer than the fork race lasts");
    }

    #[test]
    #[cfg(unix)]
    fn what_was_installed_actually_runs() {
        let target = seeded("software-runs");
        let file = downloaded(&target, TEST_PAYLOAD);
        let applied = plan_then_install(&target, "software_install", Some(&file));

        let output = run_once_it_is_not_busy(&mut std::process::Command::new(
            applied["executable"].as_str().unwrap(),
        ));
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "test-harness 1.2.3"
        );
    }

    #[test]
    fn an_update_of_nothing_is_refused_rather_than_quietly_installing() {
        // Two names for one act is what this had been: install and update
        // produced byte-identical plans. An update of nothing is a request that
        // cannot be honoured as asked, and installing instead would be doing
        // something else and calling it done.
        let target = seeded("software-update-empty");
        let prefix = ready_prefix(&target);
        let error = refuse(args(
            "plan-operation",
            &target,
            &software_plan_args("software_update", &prefix),
        ));
        assert!(
            error.detail().contains("software_install is the operation"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn a_plan_says_what_is_already_under_the_prefix() {
        let target = seeded("software-plan-present");
        let file = downloaded(&target, TEST_PAYLOAD);
        plan_then_install(&target, "software_install", Some(&file));

        // Installing the pinned version again says so, rather than reading
        // exactly like the first install did.
        let planned = software_plan(&target, "software_install");
        let effects = planned["effects"].as_array().unwrap();
        assert!(
            effects[0].as_str().unwrap().contains("already installed"),
            "{effects:?}"
        );

        // And an update is now a different plan from an install, because there
        // is something to update.
        let updating = software_plan(&target, "software_update");
        assert_eq!(updating["state"], "planned");
    }

    #[test]
    fn a_remove_names_the_versions_it_leaves_behind() {
        // This build pins one version and cannot know whether an older tree is
        // still wanted. Leaving it is right; leaving it silently is not.
        let target = seeded("software-remove-others");
        let file = downloaded(&target, TEST_PAYLOAD);
        plan_then_install(&target, "software_install", Some(&file));
        let prefix = ready_prefix(&target);
        fs::create_dir_all(Path::new(&prefix).join("0.9.0")).unwrap();

        let planned = software_plan(&target, "software_remove");
        let effects: Vec<&str> = planned["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect.as_str().unwrap())
            .collect();
        assert!(
            effects.iter().any(|effect| effect.contains("leave 0.9.0")),
            "{effects:?}"
        );
    }

    #[test]
    fn installing_software_spends_no_backup_slot() {
        // Ten slots exist and they hold configuration. If a software install
        // captured one, installing ten times would evict every backup a person
        // took of the thing this provider actually owns.
        let target = seeded("software-slots");
        let file = downloaded(&target, TEST_PAYLOAD);
        plan_then_install(&target, "software_install", Some(&file));

        let slots = target.join(TEST.control_directory).join("backups");
        let taken = fs::read_dir(&slots).map_or(0, Iterator::count);
        assert_eq!(
            taken, 0,
            "a software install captured a configuration backup"
        );
    }

    #[test]
    fn bytes_that_are_not_the_ones_the_plan_named_are_refused() {
        let target = seeded("software-digest");
        let mut tampered = TEST_PAYLOAD.to_vec();
        tampered[0] = b'X';
        let file = downloaded(&target, &tampered);

        let prefix = ready_prefix(&target);
        let planned = software_plan(&target, "software_install");
        let plan_path = target.join("..").join("plan-tampered.json");
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
                "--prefix",
                &prefix,
                "--software-artifact",
                &file.to_string_lossy(),
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        assert!(
            !Path::new(&ready_prefix(&target))
                .to_path_buf()
                .join("bin")
                .exists()
        );
    }

    #[test]
    fn an_install_with_no_artifact_says_what_is_missing() {
        let target = seeded("software-missing");
        let prefix = ready_prefix(&target);
        let planned = software_plan(&target, "software_install");
        let plan_path = target.join("..").join("plan-missing.json");
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
                "--prefix",
                &prefix,
            ],
        ));
        assert!(
            error.detail().contains("--software-artifact"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn removing_takes_the_program_back_down() {
        let target = seeded("software-remove");
        let file = downloaded(&target, TEST_PAYLOAD);
        plan_then_install(&target, "software_install", Some(&file));
        assert!(
            Path::new(&ready_prefix(&target))
                .to_path_buf()
                .join("bin/test-harness")
                .symlink_metadata()
                .is_ok()
        );

        let removed = plan_then_install(&target, "software_remove", None);
        assert_eq!(removed["removed"], true);
        assert!(
            !Path::new(&ready_prefix(&target))
                .to_path_buf()
                .join("1.2.3")
                .exists()
        );
        assert!(
            Path::new(&ready_prefix(&target))
                .to_path_buf()
                .join("bin/test-harness")
                .symlink_metadata()
                .is_err()
        );
    }

    #[test]
    fn a_build_that_installs_software_declares_all_three_operations() {
        let info = TEST.provider_info().unwrap();
        assert!(info.declares(Operation::SoftwareInstall));
        assert!(info.declares(Operation::SoftwareUpdate));
        assert!(info.declares(Operation::SoftwareRemove));
    }

    #[test]
    fn a_build_that_installs_no_software_declares_none_of_them() {
        // Declaring an operation a build cannot perform lets a consumer ask for
        // something that cannot be honoured, which is worse than not offering.
        let mut bare = TEST;
        bare.software = None;
        let info = bare.provider_info().unwrap();
        assert!(!info.declares(Operation::SoftwareInstall));
        assert!(!info.declares(Operation::SoftwareUpdate));
        assert!(!info.declares(Operation::SoftwareRemove));

        let error = software::plan(
            &bare,
            Some(Path::new("/nowhere")),
            Operation::SoftwareInstall,
            None,
        )
        .unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
    }
}
