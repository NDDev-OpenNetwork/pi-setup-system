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
    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
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

    // What state records about this target, and what the consumer reads back to
    // decide the installation it approved is the one that is here.
    //
    // Both shapes, deliberately. The nested object is what every existing
    // reader parses and it does not change. The flat fields are what
    // `require_verified_status` looks at first, and publishing them is the
    // whole of `antigravity-setup-system#22`: every one of these twenty-five
    // fields was already written to disk after an apply, and `status` returned
    // six of them. Nothing here is computed -- this is a projection that was
    // never written, and without it the consumer refuses to record a successful
    // installation it just performed.
    let mut flat = serde_json::Map::new();
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
            // Only for a target that still is what state says it is. A drifted
            // target's recorded provenance describes bytes that are no longer
            // there, and publishing it flat -- where it reads as a statement
            // about the target rather than about a record -- would invite a
            // caller to treat a changed target as the approved one. It stays in
            // the nested object, beside the drift that qualifies it.
            if drift == DriftState::Clean {
                flat = provenance_of(&current, drift);
            }
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

    // One builder, not two. The flat provenance goes in first so that the
    // fields below always win: `state`, the identities and the digest are what
    // this build observes right now, while everything in `flat` is what a file
    // in the target claims. A record that disagreed about the provider it
    // belongs to would be describing someone else, and echoing it would launder
    // that disagreement into an answer the consumer trusts.
    let mut answer = serde_json::Map::new();
    answer.extend(flat);
    for (key, value) in [
        ("state", serde_json::json!(state)),
        ("target_digest", serde_json::json!(identity)),
        (
            "protocol_version",
            serde_json::json!(provider_v3::PROTOCOL_VERSION),
        ),
        ("provider_id", serde_json::json!(harness.provider_id)),
        ("harness_id", serde_json::json!(harness.harness_id)),
        (
            "canonical_target",
            serde_json::json!(resolved.root().to_string_lossy()),
        ),
        ("target_identity_digest", serde_json::json!(identity)),
        ("provider_state", provider_state),
        (
            "journal",
            match journal {
                Some(entry) => serde_json::json!({
                    "phase": entry.phase.as_str(),
                    "operation": entry.operation,
                    "operation_id": entry.operation_id,
                }),
                None => serde_json::Value::Null,
            },
        ),
        (
            "backups",
            serde_json::json!(
                pool.list()?
                    .iter()
                    .map(|record| serde_json::json!({
                        "backup_ref": record.backup_ref.as_str(),
                        "operation": record.operation,
                        "setup_id": record.setup_id,
                    }))
                    .collect::<Vec<_>>()
            ),
        ),
    ] {
        answer.insert(key.to_owned(), value);
    }
    Ok(serde_json::Value::Object(answer))
}

/// Everything a clean managed target's own state says about how it got here.
///
/// Serialized from the record rather than listed again here. The contract's
/// `PROVENANCE_FIELDS` is already bound to the vendored kit by a test, and the
/// record already carries exactly those fields, so taking whatever it holds
/// means a field added to the state cannot be silently dropped by this
/// function. Naming them a second time would be the defect this project already
/// has a rule about: a value written twice eventually disagrees with itself.
fn provenance_of(
    current: &setup_core::stamp::ProviderState,
    drift: DriftState,
) -> serde_json::Map<String, serde_json::Value> {
    let mut flat = match serde_json::to_value(current) {
        Ok(serde_json::Value::Object(map)) => map,
        // A record that will not serialize is not a reason to fail a read that
        // has already succeeded. The nested object still carries the identity
        // and the drift, which is what decides what a caller may do next.
        _ => serde_json::Map::new(),
    };
    flat.insert("drift_state".to_owned(), serde_json::json!(drift));
    flat
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

    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
    let profile = harness.projection_profile()?;
    let build_digest = harness.build_digest()?;

    // Every operation that touches the target captures a backup first, so a
    // path a capture could not take is a plan that cannot be applied. Saying so
    // here means a caller learns before approving rather than halfway through
    // the apply, which is where it used to arrive.
    //
    // The software operations are exempt because they capture nothing: they
    // write under `--prefix` and leave the target's own namespaces alone.
    if !matches!(
        request.operation,
        Operation::SoftwareInstall | Operation::SoftwareUpdate | Operation::SoftwareRemove
    ) {
        refuse_uncapturable(harness, &resolved)?;
    }

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
    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
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
    // Before the slot exists, not while it is being filled. A capture that met
    // an entry it could not take used to stop halfway, leaving a partial
    // operation and control artifacts for a target shape that was knowable for
    // free -- reported from a Windows target whose owned `config/skills` held
    // four Junctions.
    refuse_uncapturable(harness, &resolved)?;
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

    let after =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
    write_state(
        harness, &resolved, mutation, &identity, &after, &captured, &applied,
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
                "target_identity_digest": resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?,
            }))
        }
        Phase::Committed => {
            // The effect is complete. Verify and clear the tails only.
            Journal::clear(&control)?;
            Ok(serde_json::json!({
                "state": "verified",
                "recovered": true,
                "phase": Phase::Committed.as_str(),
                "target_identity_digest": resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?,
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

/// Refuse the exact owned paths a backup could not capture, before any of it.
///
/// Named rather than counted, and all of them rather than the first: a caller
/// fixing them one refusal at a time is the same defect as an argv that
/// surfaces one missing flag at a time.
fn refuse_uncapturable(harness: &Harness, resolved: &Target) -> Result<()> {
    let refused = setup_core::backup::uncapturable(resolved.root(), harness.native_namespaces)?;
    if refused.is_empty() {
        return Ok(());
    }
    Err(Error::refuse(
        WireReason::UnsupportedNativeSurface,
        format!(
            "a backup captures content, and these owned paths are links rather than \
             content: {}. Nothing has been changed. Replace them with what they point \
             at, or move them out of the namespaces this provider owns.",
            refused.join(", ")
        ),
    ))
}

/// Record what this operation leaves behind, as the contract asks it to.
///
/// Takes the whole [`Mutation`] rather than the two fields it needs from it.
/// Passing them separately grew this past what the workspace allows a signature
/// to carry, and the honest reading is that they were never two arguments: the
/// plan's bytes and the digest taken over them are one authorization, and
/// splitting them is what let them disagree in the first place.
fn write_state(
    harness: &Harness,
    target: &Target,
    mutation: &Mutation<'_>,
    before: &str,
    after: &str,
    captured: &SlotRecord,
    applied: &Applied,
) -> Result<()> {
    let artifact = &mutation.provenance;
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
        // The digest of the plan this operation was authorized by -- taken
        // from the value the caller passed as `--plan-digest`, which is what
        // was checked against the plan's bytes before anything ran.
        //
        // It used to be read out of the plan object itself, which never carries
        // it: the digest is taken *over* the plan and travels beside it in the
        // planner's envelope. So this was `None` after every operation, of
        // every kind -- reported as an empty-setup defect and never about
        // emptiness at all.
        //
        // Only visible once `status` started publishing what it persists. While
        // the field was absent from the answer the consumer skipped it; a
        // published null is a value it compares and refuses.
        provider_plan_digest: Some(mutation.plan_digest.clone()),
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

    /// `antigravity-setup-system#22`: the consumer refuses to record an
    /// installation it just performed, because `status` returned six of the
    /// twenty-five provenance fields it had already written to disk.
    ///
    /// Every name checked here comes from the contract's `PROVENANCE_FIELDS`,
    /// which is bound to the vendored kit by its own test. This asserts they
    /// reach the *answer*, which is the step that was missing.
    #[test]
    fn a_clean_managed_target_publishes_the_provenance_it_persisted() {
        let target = seeded("status-provenance");
        plan_then_apply(&target, "backup", &[]);
        let status = run(args("status", &target, &[]));

        for field in setup_core::stamp::PROVENANCE_FIELDS {
            assert!(
                status.get(*field).is_some(),
                "status omits {field}, which state records and the consumer reads"
            );
        }

        // The ones the consumer compares against what it approved, rather than
        // merely reads. A null here is the same defect as an absence.
        for field in [
            "provider_version",
            "provider_build_digest",
            "projection_profile_digest",
            "operation_id",
            "target_identity_digest",
        ] {
            assert!(
                !status[field].is_null(),
                "status publishes {field} as null on a target it calls managed"
            );
        }
        assert_eq!(status["drift_state"], "clean");
        assert_eq!(status["provider_id"], "test-setup-system");
    }

    /// Fail-closed, and this is the half worth having.
    ///
    /// A drifted target's recorded provenance describes bytes that are no
    /// longer there. Flat, those fields read as statements about the target;
    /// nested, they read as what a record claims. Publishing them flat on a
    /// changed target would invite a consumer to treat it as the approved
    /// installation, which is exactly the confusion `#22` exists to end.
    #[test]
    fn a_drifted_target_publishes_no_flat_provenance() {
        let target = seeded("status-drifted");
        plan_then_apply(&target, "backup", &[]);
        assert!(run(args("status", &target, &[]))["provider_build_digest"].is_string());

        fs::write(target.join("AGENTS.md"), "someone edited this\n").unwrap();
        let status = run(args("status", &target, &[]));

        assert_eq!(status["state"], "managed");
        assert_eq!(status["provider_state"]["drift_state"], "local_drift");
        for field in [
            "provider_build_digest",
            "provider_plan_digest",
            "setup_definition_digest",
            "operation_id",
            "drift_state",
        ] {
            assert!(
                status.get(field).is_none(),
                "{field} is published flat for a target that no longer matches it"
            );
        }
        // The record is still readable, and still says what it said. Nothing is
        // hidden -- it is qualified by the drift beside it.
        assert!(status["provider_state"]["recorded_identity"].is_string());
    }

    /// A target this provider does not manage has no provenance to publish, and
    /// must not borrow the shape of one.
    #[test]
    fn an_unmanaged_target_publishes_no_flat_provenance() {
        let target = seeded("status-unmanaged-flat");
        let status = run(args("status", &target, &[]));
        assert_eq!(status["state"], "unmanaged");
        assert!(status.get("provider_build_digest").is_none());
        assert!(status.get("operation_id").is_none());
        assert_eq!(status["provider_state"]["present"], false);
    }

    /// `ai_stp#417`, and the half that is wrong at any size.
    ///
    /// Antigravity is a guest inside `~/.gemini` and the report came from a
    /// Windows target of ~124,065 files where `status` could not answer inside
    /// the consumer's 120-second boundary. But the timeout is the symptom. The
    /// defect is that a file this provider would never touch moved the identity
    /// a plan was made against, so an operation went stale because a *different
    /// product* wrote to its own directory.
    #[test]
    fn a_neighbours_file_is_not_part_of_this_targets_identity() {
        let target = seeded("identity-overlay");
        let before = run(args("status", &target, &[]))["target_identity_digest"].clone();

        // Three things that are emphatically not ours: a sibling file, a whole
        // sibling tree, and the product's own credentials.
        fs::write(target.join("unrelated.txt"), "the neighbour edited this").unwrap();
        fs::create_dir_all(target.join("browser-profile/Default/Cache")).unwrap();
        fs::write(
            target.join("browser-profile/Default/Cache/blob"),
            "20 GB, morally",
        )
        .unwrap();
        fs::write(target.join(".credentials.json"), "ROTATED").unwrap();

        let after = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_eq!(
            before, after,
            "a change outside every declared namespace moved this target's identity"
        );
    }

    /// The other direction, which is what stops the fix above from being a way
    /// to stop noticing things.
    #[test]
    fn a_change_inside_an_owned_namespace_moves_the_identity() {
        let target = seeded("identity-owned");
        let before = run(args("status", &target, &[]))["target_identity_digest"].clone();

        fs::write(target.join("skills").join("a.md"), "edited").unwrap();
        let edited = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_ne!(
            before, edited,
            "an edit inside skills left the identity alone"
        );

        fs::remove_dir_all(target.join("skills")).unwrap();
        let removed = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_ne!(edited, removed, "deleting skills left the identity alone");

        // Absence and emptiness are different states, and stay different
        // without an explicit marker: an empty directory is an entry, a missing
        // one is not.
        fs::create_dir_all(target.join("skills")).unwrap();
        let empty = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_ne!(
            removed, empty,
            "a deleted namespace and an empty one hash the same"
        );
    }

    /// Drift is still drift. The narrower reading must not turn a real change
    /// into a clean target.
    #[test]
    fn drift_inside_an_owned_namespace_is_still_reported() {
        let target = seeded("identity-drift");
        plan_then_apply(&target, "backup", &[]);
        assert_eq!(
            run(args("status", &target, &[]))["provider_state"]["drift_state"],
            "clean"
        );

        fs::write(target.join("unrelated.txt"), "neighbour").unwrap();
        assert_eq!(
            run(args("status", &target, &[]))["provider_state"]["drift_state"],
            "clean",
            "a neighbour's write was reported as this provider's drift"
        );

        fs::write(target.join("AGENTS.md"), "# edited\n").unwrap();
        assert_eq!(
            run(args("status", &target, &[]))["provider_state"]["drift_state"],
            "local_drift"
        );
    }

    /// `ai_stp#418`, measured rather than assumed: an explicit empty
    /// `SetupVersion` is a real setup with a zero-file projection, and it must
    /// record everything a populated one does.
    ///
    /// The report was against provider `0.0.1`, and the path it describes was
    /// rewritten since. Writing a fix for a defect that closed two releases ago
    /// is how a test that has never been red gets into a repository -- so this
    /// asserts the current behaviour and either closes the issue with evidence
    /// or names the field still missing.
    #[test]
    fn an_empty_setup_version_records_everything_a_populated_one_does() {
        let target = seeded("empty-bundle");
        let (bytes, bundle_digest, artifact) = bundle_bytes(&[]);
        let artifact_path = target.parent().unwrap().join("empty.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        // The bundle is named to both phases: the plan authorizes an identity
        // and the apply re-verifies the bytes behind it.
        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01EMPTY".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", &target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");

        let plan_path = target.join("..").join("empty-plan.json");
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
        assert_eq!(applied["state"], "verified", "{applied}");

        let status = run(args("status", &target, &[]));
        assert_eq!(status["state"], "managed");
        assert_eq!(status["drift_state"], "clean");

        // The four the report named as null, and the two identities beside
        // them. An empty projection changes none of these: they describe the
        // operation and the bundle, not how many files it carried.
        for field in [
            "bundle_format",
            "bundle_digest",
            "artifact_digest",
            "provider_plan_digest",
            "setup_stable_id",
            "setup_version",
            "projection_profile_digest",
            "operation_id",
            "provider_build_digest",
        ] {
            assert!(
                status[field].is_string(),
                "an empty setup left {field} as {}",
                status[field]
            );
        }
        assert_eq!(status["bundle_digest"], bundle_digest);
        assert_eq!(status["artifact_digest"], artifact);
        assert_eq!(status["setup_version"], "3.1.0");

        // The three the consumer compares against each other. `recorded_identity`
        // is the nested spelling of what the record holds; flat, the same name
        // carries what was observed just now, and their agreeing is the whole
        // claim that this target is still the installation that was approved.
        assert_eq!(status["target_digest"], status["target_identity_digest"]);
        assert_eq!(
            status["target_digest"],
            status["provider_state"]["recorded_identity"]
        );
        assert!(status["backup_ref"].is_string());

        // `component_refs` is empty because the bundle named none, which is the
        // truthful answer for a zero-file projection rather than a missing one.
        assert_eq!(status["component_refs"], serde_json::json!([]));
    }

    /// `ai_stp#422`: a restore must come back verified after the product has
    /// written to its own runtime files.
    ///
    /// The report is against `0.0.4`, where a target's identity was the whole
    /// directory, so a session log or a cache write moved it and a restore
    /// could not match the digest the slot recorded. Since identity became the
    /// owned projection that cause is gone -- this asserts it, because the
    /// consumer is carrying a workaround that relaxes a fail-closed check for a
    /// problem that may no longer exist.
    #[test]
    fn a_restore_is_exact_after_the_product_writes_its_own_runtime_files() {
        let target = seeded("restore-overlay");
        let before = run(args("status", &target, &[]))["target_identity_digest"].clone();
        plan_then_apply(&target, "backup", &[]);

        // A change inside what this provider owns, so the restore has work.
        fs::write(target.join("AGENTS.md"), "# edited\n").unwrap();
        assert_ne!(
            run(args("status", &target, &[]))["target_identity_digest"],
            before
        );

        // And what a running product does to its own state between the two:
        // sessions, logs, caches. None of it is ours and none of it is in the
        // slot, so a restore cannot and must not put it back.
        fs::create_dir_all(target.join("sessions/2026-08-26")).unwrap();
        fs::write(target.join("sessions/2026-08-26/log.jsonl"), "{}\n").unwrap();
        fs::create_dir_all(target.join("cache/blobs")).unwrap();
        fs::write(target.join("cache/blobs/a"), vec![7_u8; 4096]).unwrap();
        fs::write(target.join("unrelated.txt"), "the neighbour moved too").unwrap();

        let restored = plan_then_apply(&target, "restore", &[]);
        assert_eq!(restored["state"], "verified");

        let after = run(args("status", &target, &[]));
        assert_eq!(
            after["target_identity_digest"], before,
            "a restore did not reach the identity the slot recorded, because \
             something outside the owned namespaces moved"
        );
        assert_eq!(after["drift_state"], "clean");

        // The runtime files are still there: a restore returns what this
        // provider owns and leaves everything else exactly as it found it.
        assert!(target.join("sessions/2026-08-26/log.jsonl").is_file());
        assert!(target.join("cache/blobs/a").is_file());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "the neighbour moved too"
        );
    }

    /// `ai_stp#419`: a link inside an owned namespace is refused before
    /// anything moves, and every one of them is named at once.
    ///
    /// The capture always refused a link -- a slot is a statement about content
    /// and a link is a pointer -- but it refused *while copying*: the slot was
    /// created, files were written into it, and the walk then stopped. The
    /// operation became partial and left control artifacts behind, for a shape
    /// that was knowable for free. Reported from a real Windows target whose
    /// owned `config/skills` held four Junctions; `is_symlink` reports a
    /// Junction and a symbolic link alike, so one reading covers both systems.
    #[test]
    #[cfg(unix)]
    fn links_inside_an_owned_namespace_are_refused_before_anything_moves() {
        let target = seeded("junction-preflight");
        let elsewhere = target.parent().unwrap().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("real.md"), "somewhere else").unwrap();

        // Two of them, in the shape the report describes: entries inside a
        // namespace this provider owns, pointing outside the target.
        std::os::unix::fs::symlink(elsewhere.join("real.md"), target.join("skills/one.md"))
            .unwrap();
        std::os::unix::fs::symlink(&elsewhere, target.join("skills/two")).unwrap();

        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01LINK",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        // Both, and by the path a caller can act on -- not a count, and not
        // the first one alphabetically.
        assert!(
            error.detail().contains("skills/one.md"),
            "{}",
            error.detail()
        );
        assert!(error.detail().contains("skills/two"), "{}", error.detail());

        // And it is a refusal to *start*: no slot, no journal, no control state
        // that a recovery would have to resolve.
        let control = target.join(TEST.control_directory);
        let slots = fs::read_dir(control.join("backups")).map_or(0, Iterator::count);
        assert_eq!(slots, 0, "a refused plan left a backup slot behind");
        assert!(
            !control.join("journal.json").exists(),
            "a refused plan left a journal behind"
        );
        assert_eq!(run(args("status", &target, &[]))["state"], "unmanaged");

        // A link outside every owned namespace is none of our business, and the
        // same operation goes through.
        fs::remove_file(target.join("skills/one.md")).unwrap();
        fs::remove_file(target.join("skills/two")).unwrap();
        std::os::unix::fs::symlink(elsewhere.join("real.md"), target.join("their-link")).unwrap();
        assert_eq!(plan_then_apply(&target, "backup", &[])["state"], "verified");
    }

    /// A configuration operation is bound to the target it planned against; a
    /// software operation is not, and that difference is load-bearing.
    ///
    /// `perform` re-checks `expected_target_digest` under the lock and refuses
    /// `Stale` when the target moved. Software operations do not go through it
    /// at all, so a configuration edit between plan and apply does **not**
    /// strand a program install — which is correct: a program lives under
    /// `--prefix` and has nothing to do with the configuration in the target,
    /// and binding them would let someone editing their own instructions
    /// invalidate a download of a hundred and sixty megabytes.
    ///
    /// It was correct, deliberate, undocumented and held by nothing. The
    /// consumer has been told they may rely on it, so it is asserted from both
    /// sides here: routing software through `perform` "for uniformity" is a
    /// reasonable-looking cleanup that would break them silently, with every
    /// existing test still green.
    #[test]
    fn a_configuration_edit_strands_a_configuration_plan_and_not_a_program_one() {
        // The software side: plan, edit the target, apply, and it still lands.
        let target = seeded("precondition-software");
        let file = downloaded(&target, TEST_PAYLOAD);
        let planned = software_plan(&target, "software_install");
        let plan_path = target.join("..").join("precondition-plan.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();

        fs::write(
            target.join("AGENTS.md"),
            "# edited between plan and apply\n",
        )
        .unwrap();

        let prefix = target.join("..").join("precondition-prefix");
        let applied = run(args(
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
                &prefix.to_string_lossy(),
                "--software-artifact",
                &file.to_string_lossy(),
            ],
        ));
        assert_eq!(
            applied["state"], "verified",
            "a configuration edit stranded a program install"
        );

        // The configuration side, same edit, and it must refuse: a plan that
        // authorized one target state cannot be applied to another.
        let other = seeded("precondition-configuration");
        let config_plan = run(args(
            "plan-operation",
            &other,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01PRECOND",
                "--expires-at",
                far_future(),
            ],
        ));
        let config_path = other.join("..").join("precondition-config-plan.json");
        fs::write(
            &config_path,
            setup_core::canonical::to_canonical_bytes(&config_plan["plan"]).unwrap(),
        )
        .unwrap();

        fs::write(other.join("AGENTS.md"), "# edited between plan and apply\n").unwrap();

        let error = refuse(args(
            "apply-operation",
            &other,
            &[
                "--plan",
                &config_path.to_string_lossy(),
                "--plan-digest",
                config_plan["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ));
        assert_eq!(
            error.reason(),
            Some(WireReason::Stale),
            "a configuration plan survived the target changing under it: {}",
            error.detail()
        );
    }

    /// Holds a file unreadable for as long as it lives, and puts it back.
    ///
    /// Two mechanisms, one condition. On Windows a file another process holds
    /// open with no sharing cannot be opened at all; on Unix the same effect
    /// comes from permissions. A guard rather than a bare call so the condition
    /// cannot outlive the test that wanted it.
    struct Unreadable {
        #[cfg(windows)]
        _handle: fs::File,
        #[cfg(unix)]
        path: PathBuf,
    }

    impl Unreadable {
        /// `None` when this process can still read the file afterwards, which
        /// is what running as root looks like. A test that cannot create its
        /// condition must say so rather than pass.
        fn of(path: &Path) -> Option<Self> {
            #[cfg(windows)]
            {
                use std::os::windows::fs::OpenOptionsExt;
                // Deny every kind of sharing: what a running product does to a
                // database or a log it owns.
                let handle = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(path)
                    .ok()?;
                fs::File::open(path)
                    .is_err()
                    .then_some(Self { _handle: handle })
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o000)).ok()?;
                if fs::File::open(path).is_ok() {
                    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o644));
                    return None;
                }
                Some(Self {
                    path: path.to_path_buf(),
                })
            }
        }
    }

    impl Drop for Unreadable {
        fn drop(&mut self) {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(0o644));
            }
        }
    }

    /// A file inside an owned namespace that cannot be read stops the operation
    /// before it starts, names the file, and leaves nothing behind.
    ///
    /// `G12` left this open and `run_once_it_is_not_busy` was never it: that is
    /// a Linux `ETXTBSY` fork race the test harness creates by writing and
    /// exec'ing in one multi-threaded process, and its own comment says so.
    ///
    /// The real condition is a product holding its own file open — a database,
    /// a log — inside a namespace this provider owns. It is reachable on both
    /// systems and it is the same condition: on Windows through a share mode
    /// that denies everything, on Unix through permissions.
    ///
    /// What must be true is not that the operation succeeds. A slot that
    /// silently skipped a file it could not read would not restore the target,
    /// which is worse than refusing. What must be true is that the refusal
    /// **names the file** and that nothing partial survives it: identity is
    /// computed before any capture, so a target that cannot be read is a target
    /// nothing has been done to.
    #[test]
    fn a_file_this_process_cannot_read_stops_the_operation_and_leaves_nothing() {
        let target = seeded("unreadable");
        plan_then_apply(&target, "backup", &[]);
        let control = target.join(TEST.control_directory);
        let before = fs::read_dir(control.join("backups")).map_or(0, Iterator::count);

        let locked = target.join("skills").join("held-open.md");
        fs::write(&locked, "a product owns this").unwrap();
        let Some(held) = Unreadable::of(&locked) else {
            // Cannot create the condition here — running as root, or the
            // platform declined. Saying so beats a green that proves nothing.
            panic!(
                "this process can still read a file it made unreadable, so this test \
                 would prove nothing; it needs to run as a user permissions apply to"
            );
        };

        let error = refuse(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01LOCKED",
                "--expires-at",
                far_future(),
            ],
        ));
        assert!(
            error.detail().contains("held-open.md"),
            "the refusal does not name the file it could not read: {}",
            error.detail()
        );

        // Nothing started. No new slot, no journal, no transaction to recover.
        assert_eq!(
            fs::read_dir(control.join("backups")).map_or(0, Iterator::count),
            before,
            "a refused operation left a backup slot behind"
        );
        assert!(!control.join("journal.json").exists());
        assert!(!control.join("transaction").exists());

        // And it recovers by itself once the file is readable again: nothing
        // was recorded that has to be undone.
        drop(held);
        assert_eq!(
            plan_then_apply(&target, "backup", &[])["state"],
            "verified",
            "the target did not become usable again once the file could be read"
        );
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
        // The plan this was authorized by. Reported against empty setups and
        // never about emptiness: it was null after every operation of every
        // kind, because it was read out of the plan object, which never carries
        // the digest taken over it.
        assert_eq!(
            state["provider_plan_digest"], planned["plan_digest"],
            "the state does not name the plan it was authorized by"
        );
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
        // Refused at the argv layer now rather than in dispatch: `--prefix` is
        // one of the three arguments `launch` is defined by, so the parser
        // names it before a target is ever opened. `launch --help` lists the
        // same three, from the same table.
        let target = seeded("launch-noprefix");
        let error = argv::parse(args("launch", &target, &[])).unwrap_err();
        assert!(error.detail().contains("--prefix"), "{}", error.detail());
        assert!(error.detail().contains("--help"), "{}", error.detail());
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
