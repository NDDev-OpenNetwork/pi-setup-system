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
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use provider_v3::argv::{Bundle as ArgvBundle, Invocation, PlanRequest};
use provider_v3::bundle::{Bundle, Claim, FILES_PREFIX};
use provider_v3::plan::{EndState, PlanArtifact, PlanInputs};
use provider_v3::{Error, Operation, Result, WireReason};
use setup_core::backup::{BackupRef, Pool, SLOT_SCHEMA, SlotRecord};
use setup_core::journal::{JOURNAL_SCHEMA, Journal, Phase};
use setup_core::stamp::{DriftState, ProviderState, STATE_SCHEMA, StateReading};
use setup_core::target::Target;
use setup_core::{digest, lock};

use crate::catalog::Setup;
use crate::expiry;
use crate::facts::{self, Foreign, Harness};
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
        Invocation::Status {
            target,
            target_scope,
        } => status(harness, &target, target_scope),
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
fn verified_bundle(harness: &Harness, bundle: &ArgvBundle, surface: Surface) -> Result<Bundle> {
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
    if verified.manifest.bundle_format == provider_v3::bundle::BUNDLE_FORMAT {
        let bound_scope = verified
            .manifest
            .projection_profile
            .as_ref()
            .map(|profile| profile.target_scope.as_str())
            .ok_or_else(|| {
                Error::refuse(
                    WireReason::AdaptationBindingMissing,
                    "bundle v2 has no projection_profile",
                )
            })?;
        let bound_scope = match bound_scope {
            "global" => None,
            value => Some(provider_v3::TargetScope::parse(value).ok_or_else(|| {
                Error::refuse(
                    WireReason::ProjectionProfileMismatch,
                    format!("bundle v2 names unknown target scope {value:?}"),
                )
            })?),
        };
        if let Surface::At(requested) = surface
            && requested != bound_scope
        {
            return Err(Error::refuse(
                WireReason::ProjectionProfileMismatch,
                "bundle v2's bound scope differs from the requested target scope",
            ));
        }
        let profile = harness.projection_profile_for(bound_scope)?;
        verified.require_projection_profile(&profile)?;
    }
    check_within_surface(harness, verified.files.keys(), surface)?;
    check_declared_kinds(harness, &verified, surface)?;
    Ok(verified)
}

/// Every component kind a bundle names must be one this harness implements.
///
/// The kind is not in the manifest and not in the setup passport -- the passport
/// carries component references without kinds. It is stated once, in the
/// conversion report, which is why a provider that never reads that report
/// cannot tell it has been handed a kind it does not implement. It would simply
/// write the files and report success for a component it does not understand.
///
/// Asked of the same surface the paths are checked against. This read the
/// global list whatever the surface, so a kind declared only by a scoped
/// profile -- codex's `skill`, which lives under `~/.agents` -- was refused by
/// `validate-bundle` and by every scoped plan, while the four providers that
/// also declare the kind globally passed. The consumer's `user_root` slice
/// found it on 2026-09-02; codex had never passed. `AnyDeclared` now means
/// any profile, and a scope means that scope's profile, exactly as for paths.
fn check_declared_kinds(harness: &Harness, bundle: &Bundle, surface: Surface) -> Result<()> {
    for entry in &bundle.manifest.conversion_report.entries {
        if entry.component_type.is_empty() {
            continue;
        }
        let known = match surface {
            Surface::AnyDeclared => harness.implements_anywhere(&entry.component_type),
            Surface::At(scope) => harness
                .kinds_at(scope)
                .iter()
                .any(|kind| kind.as_str() == entry.component_type),
        };
        if !known {
            return Err(Error::refuse(
                WireReason::UnsupportedComponentKind,
                format!(
                    "the bundle declares component {:?} as kind {:?}, which {} does not implement{}",
                    entry.stable_id,
                    entry.component_type,
                    harness.provider_id,
                    match surface {
                        Surface::AnyDeclared => String::new(),
                        Surface::At(scope) => format!(
                            " at {}",
                            scope.map_or("the global profile", provider_v3::TargetScope::as_str)
                        ),
                    }
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
/// Which surface a bundle is being checked against.
///
/// `validate-bundle` is handed a bundle and a target and no scope, so the
/// question it can answer is whether *any* target this provider declares could
/// hold the bundle. Plan and apply are told the scope and ask about that one.
/// Answering the second question with the first is how a scope a provider
/// declares became a scope nothing could be installed into.
#[derive(Debug, Clone, Copy)]
enum Surface {
    /// Any target this provider declares. `validate-bundle`, which has no scope.
    AnyDeclared,
    /// Exactly the target this scope names.
    At(Option<provider_v3::TargetScope>),
}

fn check_within_surface<'a>(
    harness: &Harness,
    paths: impl Iterator<Item = &'a String>,
    surface: Surface,
) -> Result<()> {
    for path in paths {
        let owned = match surface {
            Surface::AnyDeclared => harness.owns_anywhere(path),
            Surface::At(scope) => harness.owns_at(path, scope),
        };
        if !owned {
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

/// What an unsettled operation owes, in the vocabulary the consumer reads.
///
/// `state` answers *what is in this directory* and has three values that
/// say nothing about the last operation. The journal has always carried
/// that, and `status` has always published it -- under `journal`, a key the
/// consumer never reads. Measured 2026-08-31 against `ai-stp-cli 0.0.10`:
/// both of its recovery paths gate on `state == "recovery_required"` or on
/// `cleanup_state`, and a target of ours holding a `prepared` journal
/// answers `managed` with neither. So the fact was in the answer, under a
/// name the reader does not know, and the recovery it exists to trigger
/// could not fire against any of the seven.
///
/// A separate key rather than a fourth `state`: `state` is read by
/// everything and means the directory, and overloading it would make every
/// existing reader wrong about a target that is merely mid-operation.
fn cleanup_owed(journal: Option<&Journal>) -> &'static str {
    match journal.map(|entry| entry.phase) {
        // The effect may be partial and `recover-operation` restores the
        // pre-operation target. Something is owed before this target is read
        // as anything.
        Some(Phase::Prepared) => "required",
        // The effect is complete; recovery clears the tail only.
        Some(Phase::Committed) => "pending",
        // Said rather than omitted. An absent key is what a provider that does
        // not speak this looks like, and that is the state this one was in.
        None => "none",
    }
}

/// Names the product reads that this provider does not own, and that are here.
///
/// `state: "managed"` and a clean `target_digest` are statements about the
/// bytes this provider wrote. Neither is a statement about what the product
/// obeys, and for `opencode` those differ: an `opencode.jsonc` beside our
/// `opencode.json` is the one the product keeps, and the digest cannot see it
/// because it is not ours to cover. The answer was true about what it examined
/// and silent about what decides.
///
/// Reported, not refused. Refusing would decide for the person whose file it
/// is, and this provider does not know whether they meant it.
fn shadowed_here(harness: &Harness, root: &Path) -> Vec<serde_json::Value> {
    harness
        .shadowing_names
        .iter()
        .filter(|shadow| root.join(shadow.name).exists())
        .map(|shadow| {
            serde_json::json!({
                "name": shadow.name,
                "over": shadow.over,
                "effect": shadow.effect,
            })
        })
        .collect()
}

/// Report the target without changing it, including a schema this build cannot write.
///
/// The shape is the consumer's, not ours. `ai_stp` reads exactly two fields to
/// decide what it is looking at — `state`, one of `missing`, `unmanaged` or
/// `managed`, and `target_digest` — and it calls this twice, requiring the two
/// answers to be *identical*. So nothing here may vary between calls: no clock,
/// no counter, no ordering that depends on a directory walk.
fn status(
    harness: &Harness,
    target: &Path,
    asked: Option<provider_v3::TargetScope>,
) -> Result<serde_json::Value> {
    let (resolved, control, pool) = observe(harness, target)?;
    let scope = scope_to_measure(harness, &resolved, asked)?;
    let owned = owned_here(harness, &resolved, scope)?;
    let identity = resolved.identity_of_owned(&as_paths(&owned), &harness.not_our_identity())?;
    let journal = Journal::read(&control).ok().flatten();
    status_of(harness, &resolved, &pool, &identity, journal)
}

/// Which scope `status` measures a target under.
fn scope_to_measure(
    harness: &Harness,
    resolved: &Target,
    asked: Option<provider_v3::TargetScope>,
) -> Result<Option<provider_v3::TargetScope>> {
    // A scope this provider never published cannot be asked about: the
    // answer would be an inventory this build has made no statement on.
    if let Some(named) = asked
        && harness.scoped_for(Some(named)).is_none()
    {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "--target-scope {named} names a target this provider publishes no \
                 projection profile for"
            ),
        ));
    }
    // The scope the caller asked about wins; without one, it comes from the
    // target's own record (`scope_recorded_at`). The record covers every
    // managed target. What it cannot cover is a workspace nobody has installed
    // into: no record, so the *global* namespaces are measured at its root --
    // and a repository is free to carry a top-level `skills/` or `rules/` of
    // its own that happens to spell one of them. The plan the consumer binds
    // to this answer is made under the scope it is about to install, where an
    // unrecorded target is exactly nothing of ours. Two inventories, one
    // comparison; asked, `status` measures the one the plan will. Agreed with
    // the consumer on 2026-09-02, out of their project-scope branch.
    Ok(asked.or_else(|| scope_recorded_at(harness, resolved)))
}

/// The status answer, once the inventory has been measured.
fn status_of(
    harness: &Harness,
    resolved: &Target,
    pool: &Pool,
    identity: &str,
    journal: Option<Journal>,
) -> Result<serde_json::Value> {
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
    let shadowed = shadowed_here(harness, resolved.root());

    let cleanup_state = cleanup_owed(journal.as_ref());

    let mut answer = serde_json::Map::new();
    answer.extend(flat);
    for (key, value) in [
        ("shadowed_by", serde_json::json!(shadowed)),
        ("cleanup_state", serde_json::json!(cleanup_state)),
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
        ("backups", {
            // A hold is the difference between a reference a plan can rely
            // on and one retention may take out from under it. The pool has
            // known which slots are held since 0.0.6; `status` did not say,
            // so a consumer could only find out by watching a baseline
            // disappear after fifty captures -- which is the failure the
            // hold exists to prevent, discovered the same way.
            //
            // Read here rather than in the map below because `held` walks
            // the pool once; asking per slot would be one walk per slot.
            let held = pool.held()?;
            pool.list()?
                .iter()
                .map(|record| {
                    let holder = held
                        .iter()
                        .find(|(reference, _)| *reference == record.backup_ref);
                    serde_json::json!({
                        "backup_ref": record.backup_ref.as_str(),
                        "operation": record.operation,
                        "setup_id": record.setup_id,
                        "held": holder.is_some(),
                        // The reason, not only the fact. A caller deciding
                        // whether it may release one needs to know whose
                        // baseline it would be taking, which is exactly what
                        // the refusal on `hold` already says.
                        "hold_reason": holder.map(|(_, reason)| reason.clone()),
                    })
                })
                .collect::<Vec<_>>()
                .into()
        }),
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
    match verified_bundle(harness, bundle, Surface::AnyDeclared) {
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
    // A scope this provider never published is refused before anything is
    // planned against it. `provider-info` carries one profile per declared
    // scope and nothing else, so a request naming another one is asking for a
    // target this provider has made no statement about -- and the old
    // behaviour was worse than accepting it: the runtime keyed its scoped
    // handling off the *request*, so a provider declaring no scope at all still
    // behaved as though it had one for `user_root` and as though it had none
    // for every other. The declaration decides, here as everywhere.
    if let Some(named) = request.target_scope
        && harness.scoped_for(Some(named)).is_none()
    {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "--target-scope {named} names a target this provider publishes no \
                 projection profile for; it declares {}",
                if harness.scoped_projections.is_empty() {
                    "only the global one".to_owned()
                } else {
                    harness
                        .scoped_projections
                        .iter()
                        .map(|scoped| scoped.target_scope.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ),
        ));
    }
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

    // The same rule for a bundle, and this one was worse than silently
    // dropped: only `install` and `replace` read one, but the plan **bound**
    // the five names into its artifact for every operation, so a remove plan
    // echoed a `bundle_digest` its apply would never read. Measured on the
    // released 0.0.50 while the consumer designed remove's `end_state`
    // extension (their ADR-0129): plan `planned, valid: true`, apply removed
    // everything, dummy bundle untouched -- accept and ignore, twice, exit 0.
    // Their rollout story assumed the loud refusal existed; now it does.
    //
    // `remove` learned to read one on 2026-09-02 (kit 0.2.8, `end_state`):
    // the bundle carries the bytes a path keeps once the setup is gone. So
    // this narrowed rather than lifted -- backup, restore and the software
    // operations still read none and still refuse.
    if request.bundle.is_some()
        && !matches!(
            request.operation,
            Operation::Install | Operation::Replace | Operation::Remove
        )
    {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "{} reads no bundle, so one named for it would be echoed into \
                 the plan and never read; install, replace and remove are the \
                 operations that take one",
                request.operation
            ),
        ));
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
/// What a mutation **takes away** before it writes, in the words the result
/// line already uses.
///
/// Default composition withdraws recorded files, not declared namespaces.
pub(crate) fn taken_before_writing(
    _harness: &Harness,
    _scope: Option<provider_v3::TargetScope>,
) -> Vec<String> {
    // Composition owns receipts, not declared namespaces. The global profile
    // used to empty every native namespace, including files this provider never
    // wrote (Antigravity's unused `config/rules`, extra skills, host files).
    // Scoped operations already used `written_paths`. Both profiles now do.
    vec![
        "only the files this provider recorded writing go; anything else under \
         the same root is left alone"
            .to_owned(),
    ]
}

fn taken_before_reset(harness: &Harness) -> Vec<String> {
    vec![
        format!(
            "these entries go whole, not file by file: {}",
            harness.native_namespaces.join(", ")
        ),
        "whatever else is in them goes too -- your own keys in a file it names, \
         your own files in a directory it names -- and the backup slot holds it"
            .to_owned(),
    ]
}

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
    let verified = verified_bundle(harness, named, Surface::At(request.target_scope))?;
    let mut effects = vec!["capture the current target into a new backup slot".to_owned()];
    effects.extend(taken_before_writing(harness, request.target_scope));
    effects.push(format!(
        "write the {} declared files over the entries this provider owns",
        verified.files.len()
    ));
    effects.extend(
        verified
            .files
            .keys()
            .take(16)
            .map(|path| format!("write {path}")),
    );
    Ok(effects)
}

#[allow(clippy::too_many_lines)]
fn plan(harness: &Harness, target: &Path, request: &PlanRequest) -> Result<serde_json::Value> {
    let (resolved, control, pool) = open(harness, target)?;
    setup_core::journal::require_clean_for_planning(
        &control,
        &control.join("transaction"),
        &pool.partial_slots()?,
    )?;

    honourable(harness, request)?;
    refuse_another_scopes_record(harness, &resolved, request.target_scope)?;

    let owned = owned_here(harness, &resolved, request.target_scope)?;
    let identity_paths = snapshot_if_unmanaged_backup(
        harness,
        &resolved,
        request.target_scope,
        &owned,
        request.operation,
    )?;
    let identity =
        resolved.identity_of_owned(&as_paths(&identity_paths), &harness.not_our_identity())?;
    // The profile at *this* target. `projection_profile()` answers with the
    // global block whatever scope it is asked about, and its digest went into
    // every scoped plan -- so a consumer that compiled against the scoped
    // profile `provider-info` publishes was handed a plan naming another one.
    let profile = harness.projection_profile_for(request.target_scope)?;
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
        refuse_a_neighbours_home(harness, &resolved, request.target_scope)?;
        let capture = match request.operation {
            Operation::Reset => harness
                .owned_projection(request.target_scope)
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            Operation::Backup
                if owned.is_empty()
                    && matches!(
                        ProviderState::read(resolved.root(), harness.state_file)?,
                        StateReading::Absent
                    ) =>
            {
                existing_under_projection(harness, &resolved, request.target_scope)?
            }
            _ => owned.clone(),
        };
        refuse_uncapturable(&resolved, &capture)?;
    }

    let mut software_artifacts = Vec::new();
    let mut software_prefix_held: Option<String> = None;
    let mut software_version_held: Option<String> = None;
    let mut end_state = Vec::new();
    let (effects, backup_ref, restore_target_digest) = match request.operation {
        Operation::SoftwareInstall | Operation::SoftwareUpdate | Operation::SoftwareRemove => {
            let (planned, effects, version) = software::plan(
                harness,
                request.prefix.as_deref(),
                request.operation,
                request.software_version.as_deref(),
            )?;
            software_artifacts = planned;
            software_prefix_held = request
                .prefix
                .as_ref()
                .map(|prefix| prefix.to_string_lossy().into_owned());
            software_version_held = Some(version.to_owned());
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
                Some(restore_target_identity(
                    harness,
                    &payload,
                    request.target_scope,
                )?),
            )
        }
        Operation::Remove => {
            if let Removal::WouldTakeUnrecorded(present) =
                classify_removal(harness, &resolved, request.target_scope)?
            {
                return Err(unrecorded_removal_refusal(harness, &resolved, &present));
            }
            let (lines, states) = removal_effects(harness, &resolved, request)?;
            end_state = states;
            (lines, None, None)
        }
        Operation::Reset => {
            match ProviderState::read(resolved.root(), harness.state_file)? {
                StateReading::Current(_) => {}
                StateReading::Absent | StateReading::ForeignSchema { .. } => {
                    return Err(Error::refuse(
                        WireReason::UnsupportedOperation,
                        format!(
                            "reset empties declared namespaces whole, and {} holds no \
                             record of this provider writing here; refused rather than \
                             guessing whose files those are",
                            resolved.root().display()
                        ),
                    ));
                }
            }
            (taken_before_reset(harness), None, None)
        }
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
        target_scope: request.target_scope,
        projection_profile_digest: &profile.digest,
        bundle: request.bundle.as_ref().map(|bundle| bundle.binding.clone()),
        backup_ref,
        restore_target_digest,
        permission_profile: request.permission_profile.clone(),
        expires_at: &request.expires_at,
        software_artifacts,
        software_prefix: software_prefix_held.as_deref(),
        software_version: software_version_held.as_deref(),
        end_state,
        effects,
    })?
    .into_response()
}

/// What a removal would do at a target this build has no record of writing.
///
/// Three answers, agreed with the consumer on 2026-09-02 after the human
/// surface was measured taking a person's own `config.toml`, `AGENTS.md` and
/// `prompts/` from a target it had never written to:
///
/// * a record exists -- the removal is the one it always was;
/// * no record and nothing of ours on disk -- silence, because "already
///   removed" must stay a no-op or a repeat becomes an error where nothing
///   happened;
/// * no record and declared entries present -- refused, naming them, because
///   taking a namespace whole there is guessing whose files those are.
///
/// The scoped profile already refuses on the same ground inside
/// `remove_managed`; this is the global profile's half.
pub(crate) enum Removal {
    /// A record exists, or the scope's own rule applies.
    Recorded,
    /// Nothing this provider declares is on disk.
    NothingHere,
    /// Declared entries are here and no record says this build wrote them.
    #[allow(dead_code)]
    WouldTakeUnrecorded(Vec<String>),
}

pub(crate) fn classify_removal(
    harness: &Harness,
    target: &Target,
    _scope: Option<provider_v3::TargetScope>,
) -> Result<Removal> {
    // Composition removes recorded files, not declared namespaces. Without a
    // record there is nothing of ours to take, even when native namespaces
    // already hold someone else's files. Guessing ownership from the namespace
    // list is the defect this used to protect against by refusing; the refusal
    // is no longer needed because the removal no longer takes those files.
    match ProviderState::read(target.root(), harness.state_file)? {
        StateReading::Current(_) => Ok(Removal::Recorded),
        StateReading::Absent | StateReading::ForeignSchema { .. } => Ok(Removal::NothingHere),
    }
}

/// The same question under the lock, because a record can vanish between a
/// plan and its apply.
///
/// The state file is deliberately outside the target's identity -- counting it
/// would make an applied operation leave the target different from the identity
/// it just recorded -- so deleting it does not move the digest that authorized
/// the plan, and a removal authorized while managed could arrive unrecorded.
fn refuse_an_unrecorded_removal(
    harness: &Harness,
    resolved: &Target,
    mutation: &Mutation<'_>,
) -> Result<()> {
    if !matches!(
        mutation.effect,
        Effect::Remove | Effect::RemoveKeeping { .. }
    ) {
        return Ok(());
    }
    if let Removal::WouldTakeUnrecorded(present) =
        classify_removal(harness, resolved, mutation.target_scope)?
    {
        return Err(unrecorded_removal_refusal(harness, resolved, &present));
    }
    Ok(())
}

/// The refusal `classify_removal` calls for, in the words both surfaces use.
pub(crate) fn unrecorded_removal_refusal(
    harness: &Harness,
    target: &Target,
    present: &[String],
) -> Error {
    Error::refuse(
        WireReason::UnsupportedOperation,
        format!(
            "{} has applied no setup at {} -- no state file, or one written before \
             this build recorded what it wrote. Removing would take {} whole, and \
             nothing here says this provider put them there. Install a setup first \
             if you want one removed, or take what you put there yourself.",
            harness.provider_id,
            target.root().display(),
            present.join(", ")
        ),
    )
}

/// A target managed under a scope is planned under that scope, or refused by
/// name.
///
/// A plan under another scope measures another inventory -- at a workspace
/// managed under `project`, the global namespaces are simply absent, so the
/// plan's identity is the empty tree's -- and its apply would rewrite the
/// record with the wrong ownership. The consumer met the first half as a bare
/// `expected_target_digest` mismatch on 2026-09-02 (their remove plan carried
/// no scope); this turns it into the sentence that says what to send.
///
/// One direction only. A home managed under the global profile may still be
/// asked about a scope: the global record is not an inventory of that scope's
/// files, and the scoped operations read what they need from it or refuse on
/// their own terms. The dangerous direction is the one a record describes.
fn refuse_another_scopes_record(
    harness: &Harness,
    target: &Target,
    asked: Option<provider_v3::TargetScope>,
) -> Result<()> {
    let Some(recorded) = scope_recorded_at(harness, target) else {
        return Ok(());
    };
    if asked == Some(recorded) {
        return Ok(());
    }
    Err(Error::refuse(
        WireReason::UnsupportedOperation,
        format!(
            "{} is managed under target_scope {}, and this plan names {}; a plan \
             under another scope would measure another inventory, so name the \
             scope the target is managed under",
            target.root().display(),
            recorded.as_str(),
            asked.map_or("the global profile", provider_v3::TargetScope::as_str)
        ),
    ))
}

/// What a removal will do, and what each path becomes when a bundle rides.
///
/// A bundle on a remove names what stays: the consumer rebuilt a host file
/// without the key this setup put there, and the file outlives the setup at
/// exactly those bytes. Read and verified here, as install's is, so the plan
/// is never issued for bytes the apply would refuse -- same reader, same
/// limits, same `validate-bundle` semantics, by the consumer's request.
fn removal_effects(
    harness: &Harness,
    resolved: &Target,
    request: &PlanRequest,
) -> Result<(Vec<String>, Vec<EndState>)> {
    let mut lines = vec!["capture the current target before removing".to_owned()];
    lines.extend(taken_before_writing(harness, request.target_scope));
    let Some(named) = request.bundle.as_ref() else {
        return Ok((lines, Vec::new()));
    };
    let verified = verified_bundle(harness, named, Surface::At(request.target_scope))?;
    let states = end_states_of(harness, resolved, request.target_scope, &verified)?;
    lines.push(format!(
        "leave {} declared files behind at the bytes the bundle carries",
        verified.files.len()
    ));
    lines.extend(
        verified
            .files
            .keys()
            .take(16)
            .map(|path| format!("leave {path}")),
    );
    Ok((lines, states))
}

/// What each path a removal touches looks like afterwards, when a bundle of
/// surviving bytes rides along.
///
/// Two lists, and the second wins where they meet. What *goes* is what
/// `remove_managed` will take: the files this provider recorded writing.
/// What *stays* is every file the bundle declares, at the member, digest and
/// length its own manifest binds -- so a consumer approving the plan can see,
/// per path, that the bytes it packed are the bytes that will be there. A path
/// in both lists is stated once, as a survivor: "gone, then present" is the
/// mechanism, not the end state.
fn end_states_of(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
    verified: &Bundle,
) -> Result<Vec<EndState>> {
    let taken = owned_here(harness, target, scope)?;
    let mut entries: Vec<EndState> = taken
        .iter()
        .filter(|path| !verified.files.contains_key(*path))
        .map(|path| EndState::removed(path))
        .collect();
    for path in verified.files.keys() {
        let Some(record) = verified
            .manifest
            .files
            .iter()
            .find(|file| &file.path == path)
        else {
            // `Bundle::read` refuses a member the manifest never declared, so
            // this is unreachable by construction; refusing keeps it so.
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!("the bundle carries {path:?} and its manifest does not declare it"),
            ));
        };
        entries.push(EndState::final_bytes(
            path,
            &format!("{FILES_PREFIX}{path}"),
            &record.digest,
            record.byte_length,
        ));
    }
    Ok(entries)
}

/// The end states a remove plan recorded, or none.
fn end_states_in(artifact: &serde_json::Value) -> Result<Vec<EndState>> {
    match artifact.get("end_state") {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("the approved plan's end_state member cannot be read: {source}"),
            )
        }),
    }
}

/// What a remove plan authorizes: everything gone, or everything gone and the
/// bundle's files left behind -- and only the bundle the plan described.
fn removal_effect<'a>(
    harness: &Harness,
    artifact: &serde_json::Value,
    bundle: Option<&ArgvBundle>,
    verified: &'a mut Option<Bundle>,
    applied: &mut Applied,
) -> Result<Effect<'a>> {
    let planned = end_states_in(artifact)?;
    if !planned.iter().any(EndState::survives) {
        // The plan the consumer approved leaves nothing behind, so a bundle
        // at apply is an authorization the plan never gave.
        if bundle.is_some() {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                "this remove plan leaves no bytes behind, so a bundle named at apply \
                 was never authorized; plan the removal with the bundle",
            ));
        }
        return Ok(Effect::Remove);
    }
    let Some(named) = bundle else {
        return Err(Error::refuse(
            WireReason::UnsupportedBundleFormat,
            "this remove was planned with surviving bytes, and no bundle was \
             named to carry them",
        ));
    };
    let ready = verified.insert(verified_bundle(
        harness,
        named,
        Surface::At(scope_of(artifact)),
    )?);
    check_survivors(&planned, ready)?;
    // Which bundle put the surviving bytes there is provenance worth keeping;
    // the passport's setup identity is not, because the setup is the thing
    // that just ended.
    applied.bundle_format = Some(named.binding.bundle_format.clone());
    applied.bundle_digest = Some(named.binding.bundle_digest.clone());
    applied.artifact_digest = Some(named.binding.artifact_digest.clone());
    Ok(Effect::RemoveKeeping {
        files: &ready.files,
    })
}

/// The bundle handed to `apply` must be the one the plan described, member by
/// member: the plan digest the consumer approved binds these entries, and the
/// bundle's own manifest binds its files, so the two are compared here rather
/// than trusted to agree.
fn check_survivors(planned: &[EndState], ready: &Bundle) -> Result<()> {
    for entry in planned.iter().filter(|entry| entry.survives()) {
        let record = ready
            .manifest
            .files
            .iter()
            .find(|file| file.path == entry.path);
        let agrees = record.is_some_and(|file| {
            Some(&file.digest) == entry.sha256.as_ref()
                && Some(file.byte_length) == entry.byte_length
                && entry.member.as_deref() == Some(format!("{FILES_PREFIX}{}", file.path).as_str())
        });
        if !agrees {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!(
                    "the plan leaves {:?} at bytes the bundle named for apply does not carry; \
                     no effect was made",
                    entry.path
                ),
            ));
        }
    }
    let planned_survivors = planned.iter().filter(|entry| entry.survives()).count();
    if planned_survivors != ready.files.len() {
        return Err(Error::refuse(
            WireReason::DigestMismatch,
            format!(
                "the plan leaves {planned_survivors} files behind and the bundle carries {}; \
                 no effect was made",
                ready.files.len()
            ),
        ));
    }
    Ok(())
}

/// The target identity a selected backup will produce under this scope.
///
/// A slot payload is a transport tree, not always the target's identity tree.
/// Under a shared scope it contains parent directories needed to carry the
/// recorded files, while status hashes the recorded file inventory itself.
/// Hashing the payload wholesale therefore added directory entries that
/// restored status correctly omitted.
fn restore_target_identity(
    harness: &Harness,
    payload: &Path,
    _scope: Option<provider_v3::TargetScope>,
) -> Result<String> {
    let owned = files_in_payload(payload)?;
    Ok(setup_core::digest::of_owned(
        payload,
        &as_paths(&owned),
        &harness.not_our_identity(),
    )?)
}

/// Every regular payload file, relative to the payload root.
fn files_in_payload(payload: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(payload).map_err(|error| {
        setup_core::Error::new(
            setup_core::ReasonCode::StateUnavailable,
            format!("cannot list backup payload {}", payload.display()),
        )
        .with_source(error)
    })? {
        let entry = entry.map_err(|error| {
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!(
                    "cannot read an entry of backup payload {}",
                    payload.display()
                ),
            )
            .with_source(error)
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(Error::from(setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                "a backup payload entry has an unrepresentable name",
            )));
        };
        if entry.path().is_dir() {
            found.extend(files_under(&entry.path(), &name)?);
        } else {
            found.push(name);
        }
    }
    found.sort();
    Ok(found)
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
    /// Empty every declared native namespace. Not the default of composition.
    ResetNamespaces,
    /// Withdraw everything this provider owns, then put back the files a
    /// verified bundle says outlive the setup -- a host file without the key
    /// this setup contributed, at the consumer's reconstructed bytes.
    ///
    /// The same capture-before-effect sequence as everything else here; what
    /// changes is what the target looks like afterwards and what the state
    /// records about it: nothing of ours, because the surviving bytes are the
    /// person's.
    RemoveKeeping {
        /// Each surviving file's bytes and mode, by target-relative path.
        files: &'a BTreeMap<String, (Vec<u8>, u32)>,
    },
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
    /// The scope the plan recorded, when the consumer named one.
    pub target_scope: Option<provider_v3::TargetScope>,
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
    /// The files this operation put on the target.
    ///
    /// Lives here rather than beside the effect because it is the same kind of
    /// fact as the rest of this struct: not what was *planned*, but what the
    /// operation turned out to have done. Empty for a removal and a backup,
    /// which is the true answer rather than a missing one.
    pub written_paths: Vec<String>,
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
/// The scope a plan recorded, when the consumer named one.
///
/// Read back from the plan rather than taken from argv: `apply` is handed a
/// plan and not a scope, so this is the only place a scope can arrive -- and it
/// arrives having been bound into the plan digest the consumer verified, which
/// a second argv flag could not claim.
fn scope_of(artifact: &serde_json::Value) -> Option<provider_v3::TargetScope> {
    artifact
        .get("target_scope")
        .and_then(serde_json::Value::as_str)
        .and_then(provider_v3::TargetScope::parse)
}

#[allow(clippy::too_many_lines)]
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
    let mut verified: Option<Bundle> = None;
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
        return apply_software(
            harness,
            prefix,
            operation,
            plan_digest,
            &artifact,
            downloaded,
        );
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
        Operation::Remove => {
            removal_effect(harness, &artifact, bundle, &mut verified, &mut applied)?
        }
        Operation::Reset => Effect::ResetNamespaces,
        Operation::Install | Operation::Replace => {
            let Some(named) = bundle.as_ref() else {
                return Err(Error::refuse(
                    WireReason::UnsupportedBundleFormat,
                    format!("{operation} arrives as a bundle, and none was named"),
                ));
            };
            // Re-read and re-verify: the plan authorized an identity, not a file
            // that might have changed on disk since.
            verified = Some(verified_bundle(
                harness,
                named,
                Surface::At(scope_of(&artifact)),
            )?);
            let Some(ready) = verified.as_ref() else {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    "bundle vanished",
                ));
            };
            record_bundle_provenance(&mut applied, named, ready);
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
            target_scope: scope_of(&artifact),
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
#[allow(clippy::too_many_lines)]
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
    let owned = owned_here(harness, &resolved, mutation.target_scope)?;
    let identity_paths = snapshot_if_unmanaged_backup(
        harness,
        &resolved,
        mutation.target_scope,
        &owned,
        mutation.operation,
    )?;
    let identity =
        resolved.identity_of_owned(&as_paths(&identity_paths), &harness.not_our_identity())?;
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
    let (previous_setup, previous_definition, previous_written) =
        what_the_target_already_says(harness, &resolved)?;
    // Before the slot exists, not while it is being filled. A capture that met
    // an entry it could not take used to stop halfway, leaving a partial
    // operation and control artifacts for a target shape that was knowable for
    // free -- reported from a Windows target whose owned `config/skills` held
    // four Junctions.
    refuse_a_neighbours_home(harness, &resolved, mutation.target_scope)?;
    let capture = capture_inventory(
        harness,
        &resolved,
        mutation.target_scope,
        &owned,
        &mutation.effect,
    )?;
    refuse_uncapturable(&resolved, &capture)?;
    refuse_an_unrecorded_removal(harness, &resolved, mutation)?;
    let captured = pool.capture(resolved.root(), &as_paths(&capture), |backup_ref| {
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
        target_scope: mutation.target_scope.map(|scope| scope.as_str().to_owned()),
    }
    .publish_prepared(&control)?;

    // What the state will say is applied. A restore learns it from the slot it
    // restores; every other effect was told at plan time.
    let mut applied = mutation.applied.clone();
    let outcome = match &mutation.effect {
        // The capture above *is* the effect. Nothing else is written -- and
        // *nothing written* is not *nothing owned*. This used to record an
        // empty list, which globally cost nothing because a removal reads the
        // namespaces; under a scope the record **is** the inventory, so a
        // backup would have erased the answer `remove` depends on and the next
        // removal would have taken nothing while reporting success. The field
        // means "the files this provider has written at this target", not "the
        // files this operation wrote", and an operation that writes none leaves
        // it as it found it.
        Effect::Backup => Ok(previous_written.clone()),
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
            replace_managed_from(harness, &resolved, &payload, mutation.target_scope, false)
        }
        // Removal puts nothing on the target and leaves nothing of ours there,
        // so an empty list is the true answer rather than a missing one --
        // which is exactly the distinction the schema bump beside this field
        // exists to keep.
        Effect::Remove => {
            remove_managed(harness, &resolved, mutation.target_scope).map(|()| vec![])
        }
        Effect::ResetNamespaces => reset_namespaces(harness, &resolved).map(|()| vec![]),
        // Gone, then present -- and recorded as not ours. `written_paths` is
        // the inventory a later scoped removal deletes from, and a file the
        // person keeps after this setup ended is exactly the file that removal
        // must not take (their ADR-0129: the file stays the user's).
        Effect::RemoveKeeping { files } => {
            remove_keeping_files(harness, &resolved, mutation.target_scope, files)
        }
        Effect::Materialize { setup } => {
            setup.check_within(harness)?;
            replace_managed_from(
                harness,
                &resolved,
                &setup.payload,
                mutation.target_scope,
                true,
            )
        }
        Effect::MaterializeBundle { files } => {
            write_bundle_files(harness, &resolved, files, mutation.target_scope)
        }
        // Keeping a predecessor's stamp aside writes nothing to the target, so
        // the inventory is what it was. Same reason as `Backup` above.
        Effect::Adopt { stamp } => {
            crate::adopt::keep_aside(&control, stamp, harness.predecessor_state_file)
                .map(|_| previous_written.clone())
        }
    };

    // On failure the journal stays in `prepared`, which is what makes the
    // interruption legible: recovery restores the captured pre-operation target.
    applied.written_paths = outcome?;

    // The inventory *after* the effect, which under a scope is the list the
    // effect just returned rather than one re-read from a state file this
    // operation has not written yet.
    let after_owned = applied.written_paths.clone();
    let after = resolved.identity_of_owned(&as_paths(&after_owned), &harness.not_our_identity())?;
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

/// The program lifecycle's answer, carrying the plan echo the wire owes.
///
/// The echo is added here rather than inside `software::apply`, because it is a
/// fact about the wire call and not about the prefix: this layer is where the
/// plan artifact and its digest exist, and `software::apply` deliberately knows
/// about neither.
///
/// **It was missing from both of `software::apply`'s answer shapes, and the cost
/// was carried by every one of the seven.** The consumer requires the echo for
/// every operation, so `harness install`, `harness update` and `harness remove`
/// through `ai-stp` refused *after* the program was installed -- the effect
/// landed and the operation stayed `applied_unverified` over a prefix holding a
/// working build. Reported by the consumer's own session on 2026-08-31 after
/// running the released `0.0.48` through `harness install`, and confirmed here
/// by reading both sites rather than by taking the report: the configuration
/// answer carries `plan_digest`; neither software answer did.
///
/// Nothing on this side could have raised it. The producer tests asked whether
/// the provider does what its own answer says, which it did, and the contract
/// sentence saying the program lifecycle carries *"the same journal, backup and
/// plan-digest"* was read as being about `plan-operation` alone. That is why the
/// test beside it asserts the **wire** shape against the contract's list rather
/// than against this function's output.
fn apply_software(
    harness: &Harness,
    prefix: Option<&Path>,
    operation: Operation,
    plan_digest: &str,
    plan: &serde_json::Value,
    downloaded: &[std::path::PathBuf],
) -> Result<serde_json::Value> {
    let planned_prefix = string_field(plan, "software_prefix")?;
    let planned_version = string_field(plan, "software_version")?;
    let argv_prefix = prefix.ok_or_else(|| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{operation} installs a program, which lives under --prefix, not under --target; \
                 name an absolute --prefix"
            ),
        )
    })?;
    if argv_prefix != Path::new(&planned_prefix) {
        return Err(Error::refuse(
            WireReason::Stale,
            format!(
                "this plan is bound to --prefix {planned_prefix}; --prefix {} is a different \
                 resource; no effect was made",
                argv_prefix.display()
            ),
        ));
    }
    let planned_artifacts = planned_software_artifacts(plan)?;
    let mut answer = software::apply(
        harness,
        prefix,
        operation,
        &planned_version,
        &planned_artifacts,
        downloaded,
    )?;
    if let Some(fields) = answer.as_object_mut() {
        fields.insert(
            "plan_digest".to_owned(),
            serde_json::Value::String(plan_digest.to_owned()),
        );
    }
    Ok(answer)
}

/// What the target's own state says before this operation touches it.
///
/// The setup it names, the definition digest that setup had, and the files this
/// provider recorded writing. The third is not bookkeeping: under a scope it is
/// the inventory every verb acts on, and an operation that writes nothing has
/// to hand it back unchanged rather than record an empty list.
fn what_the_target_already_says(
    harness: &Harness,
    resolved: &Target,
) -> Result<(Option<String>, Option<String>, Vec<String>)> {
    Ok(
        match ProviderState::read(resolved.root(), harness.state_file)? {
            StateReading::Current(current) => (
                current.setup_stable_id,
                current.setup_definition_digest,
                current.written_paths,
            ),
            _ => (None, None, Vec::new()),
        },
    )
}

/// What a bundle says about itself, copied into the state the operation writes.
///
/// Lifted out of `apply` rather than inlined: two of these were null for every
/// bundle install because the passport was a required member nothing read, and
/// a block with its own name is a block somebody can look at.
fn record_bundle_provenance(applied: &mut Applied, named: &ArgvBundle, ready: &Bundle) {
    applied.bundle_format = Some(named.binding.bundle_format.clone());
    applied.bundle_digest = Some(named.binding.bundle_digest.clone());
    applied.artifact_digest = Some(named.binding.artifact_digest.clone());
    if !ready.passport.stable_id.is_empty() {
        applied.setup_id = Some(ready.passport.stable_id.clone());
    }
    if !ready.passport.version.is_empty() {
        applied.setup_version = Some(ready.passport.version.clone());
    }
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

    // The only place a scope can reach a recovery: `recover-operation` takes no
    // arguments, so the interrupted operation had to write down which target it
    // was acting on. A journal from a build that had no scope to act on carries
    // none, and absent means global -- which is what that build did.
    let scope = journal
        .target_scope
        .as_deref()
        .and_then(provider_v3::TargetScope::parse);
    let owned = owned_here(harness, &resolved, scope)?;

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
            replace_managed_from(harness, &resolved, &payload, scope, false)?;
            Journal::clear(&control)?;
            Ok(serde_json::json!({
                "state": "verified",
                "recovered": true,
                "phase": Phase::Prepared.as_str(),
                "restored_from": reference,
                "target_identity_digest": resolved.identity_of_owned(&as_paths(&owned), &harness.not_our_identity())?,
            }))
        }
        Phase::Committed => {
            // The effect is complete. Verify and clear the tails only.
            Journal::clear(&control)?;
            Ok(serde_json::json!({
                "state": "verified",
                "recovered": true,
                "phase": Phase::Committed.as_str(),
                "target_identity_digest": resolved.identity_of_owned(&as_paths(&owned), &harness.not_our_identity())?,
            }))
        }
    }
}

/// Replace this provider's recorded files from a captured tree.
///
/// Withdraws what this provider recorded writing, then copies the payload's
/// recorded members back. A sibling overlay the product or the owner put in
/// the target survives: restoring files this provider never wrote would undo
/// someone else's work. Delegates to [`replace_recorded_from`].
fn replace_managed_from(
    harness: &Harness,
    target: &Target,
    payload: &Path,
    scope: Option<provider_v3::TargetScope>,
    merge_json: bool,
) -> Result<Vec<String>> {
    replace_recorded_from(harness, target, payload, scope, merge_json)
}

/// Put a captured tree back under a named scope, file by file.
///
/// The header above says a restore must not revert files this provider never
/// wrote. Under a scope that sentence needs a different mechanism, not a
/// different rule: the namespace is shared, so clearing it whole and copying
/// the payload over it would revert every neighbour's file to what it was when
/// the slot was taken. The person restoring a codex setup did not ask to move
/// pi's skills back a week.
///
/// So the clear is scoped to this provider's own inventory and the write is the
/// payload's own contents — the payload *is* the record of what was captured,
/// which is why this needs no second list to consult. Copying merges into an
/// existing directory rather than replacing it, so a neighbour's file inside a
/// namespace we write into survives untouched.
fn replace_recorded_from(
    harness: &Harness,
    target: &Target,
    payload: &Path,
    scope: Option<provider_v3::TargetScope>,
    merge_json: bool,
) -> Result<Vec<String>> {
    for relative in &owned_here(harness, target, scope)? {
        withdraw_written(harness, target, relative, merge_json)?;
    }
    let mut written = Vec::new();
    let entries = fs::read_dir(payload).map_err(|error| {
        setup_core::Error::new(
            setup_core::ReasonCode::StateUnavailable,
            format!("cannot list {}", payload.display()),
        )
        .with_source(error)
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!("cannot read an entry of {}", payload.display()),
            )
            .with_source(error)
        })?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err(Error::from(setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!(
                    "{} has a name this kernel cannot represent",
                    entry.path().display()
                ),
            )));
        };
        let source = entry.path();
        let destination = target.root().join(&name);
        if source.is_dir() {
            setup_core::backup::copy_tree(&source, &destination, &[])?;
            // The destination is deliberately a merge: it may contain files
            // another provider or the person owns. Reading it back here made
            // those neighbours part of this provider's `written_paths`, so
            // status widened after a restore and no longer matched the exact
            // BackupRef identity promised by the plan. The slot payload is the
            // complete record of what this provider restored; derive the
            // inventory from it and only it.
            written.extend(files_under(&source, &name)?);
        } else {
            let bytes = fs::read(&source).map_err(|error| {
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot read {}", source.display()),
                )
                .with_source(error)
            })?;
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    setup_core::Error::new(
                        setup_core::ReasonCode::StateUnavailable,
                        format!("cannot create {}", parent.display()),
                    )
                    .with_source(error)
                })?;
            }
            write_host_file(harness, target, &name, &bytes, merge_json)?;
            written.push(name);
        }
    }
    written.sort();
    Ok(written)
}

/// Every file under one written namespace, as a target-relative slash path.
///
/// Read back from the destination rather than counted at the source, because
/// the destination is what the record is about. Slash-separated so a record
/// written on Windows names the same paths as one written on Linux -- the same
/// reason the embedded catalogue joins with `/` rather than with the platform
/// separator.
fn files_under(root: &Path, namespace: &str) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), namespace.to_owned())];
    while let Some((directory, prefix)) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                format!("cannot read {} back", directory.display()),
            )
            .with_source(error)
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot read an entry of {}", directory.display()),
                )
                .with_source(error)
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{prefix}/{name}");
            if entry.path().is_dir() {
                pending.push((entry.path(), relative));
            } else {
                found.push(relative);
            }
        }
    }
    Ok(found)
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
    scope: Option<provider_v3::TargetScope>,
) -> Result<Vec<String>> {
    // Clear only the files this provider recorded writing. Empty inventory is
    // a first install, not a reason to empty declared namespaces.
    let incoming: Vec<String> = files.keys().cloned().collect();
    for relative in &owned_here(harness, target, scope)? {
        if incoming.iter().any(|path| path == relative) && json_object_file(relative) {
            continue;
        }
        withdraw_written(harness, target, relative, true)?;
    }
    for (relative, (bytes, mode)) in files {
        write_host_file(harness, target, relative, bytes, true)?;
        set_mode(&target.root().join(relative), *mode)?;
    }
    Ok(files.keys().cloned().collect())
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

/// Files this provider recorded writing at this target.
///
/// Composition uses this inventory, globally and under a scope. Declared
/// namespaces are permission to write, not a wipe list. Emptying them whole is
/// [`reset_namespaces`], not install, replace or remove.
///
/// A record this build cannot read is a refusal rather than a widening. Absent
/// means nothing was written here, so the inventory is empty.
pub(crate) fn owned_here(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
) -> Result<Vec<String>> {
    match ProviderState::read(target.root(), harness.state_file)? {
        StateReading::Current(state) => Ok(state.written_paths),
        StateReading::Absent => Ok(Vec::new()),
        StateReading::ForeignSchema { .. } => {
            let named = harness.owned_projection(scope).join(", ");
            Err(Error::refuse(
                WireReason::UnsupportedOperation,
                format!(
                    "an operation acts on the files this provider recorded writing, and {} \
                     holds a state file written before this build recorded them. Refused \
                     rather than widened to {named} whole. Reinstall to establish a record, \
                     or point --target at this product's own configuration home.",
                    target.root().display()
                ),
            ))
        }
    }
}

/// Borrow an inventory for the `&[&str]` every walker takes.
fn as_paths(owned: &[String]) -> Vec<&str> {
    owned.iter().map(String::as_str).collect()
}

/// Which scope a target was operated under, read back from the state it carries.
///
/// `status` is handed a target and nothing else — that is the argv contract, and
/// it is also why the consumer may call it twice and require the two answers to
/// be identical. So the scope has to come from the target, and it is already
/// written down: `native_ownership` records the namespaces this provider owns
/// *here*, which under a scope is that scope's set and otherwise the global
/// block. This only has to recognise which.
///
/// An unreadable or absent state answers `None`, which is right rather than
/// merely safe: a target carrying no state of ours is not a target we operated
/// under any scope.
fn scope_recorded_at(harness: &Harness, target: &Target) -> Option<provider_v3::TargetScope> {
    let StateReading::Current(state) =
        ProviderState::read(target.root(), harness.state_file).ok()?
    else {
        return None;
    };
    harness
        .scoped_projections
        .iter()
        .find(|scoped| {
            scoped.native_namespaces.len() == state.native_ownership.len()
                && scoped
                    .native_namespaces
                    .iter()
                    .all(|name| state.native_ownership.iter().any(|owned| owned == name))
        })
        .map(|scoped| scoped.target_scope)
}

fn remove_managed(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
) -> Result<()> {
    // Receipts, not namespaces: a file this provider never wrote stays, under
    // the global profile as under a shared root. JSON host keys this provider
    // did not write are stripped by name rather than by deleting the file.
    for relative in &owned_here(harness, target, scope)? {
        withdraw_written(harness, target, relative, true)?;
    }
    Ok(())
}

/// Empty every declared native namespace. Not composition: an explicit reset.
///
/// Ordinary install, replace and remove use [`remove_managed`]. This is the
/// separately named whole-namespace effect, kept so an authorized agent can
/// still request it without the default verbs silently erasing unrelated files.
#[cfg_attr(not(test), allow(dead_code))]
fn reset_namespaces(harness: &Harness, target: &Target) -> Result<()> {
    for namespace in harness.native_namespaces {
        remove_keeping(
            &target.root().join(namespace),
            target.root(),
            harness.never_touch,
        )?;
    }
    forget_all_written_fields(harness, target);
    Ok(())
}

fn capture_inventory(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
    owned: &[String],
    effect: &Effect<'_>,
) -> Result<Vec<String>> {
    match effect {
        Effect::ResetNamespaces => Ok(harness
            .owned_projection(scope)
            .iter()
            .map(|name| (*name).to_owned())
            .collect()),
        Effect::Backup
            if owned.is_empty()
                && matches!(
                    ProviderState::read(target.root(), harness.state_file)?,
                    StateReading::Absent
                ) =>
        {
            existing_under_projection(harness, target, scope)
        }
        Effect::MaterializeBundle { files } => {
            let mut paths = owned.to_vec();
            for path in files.keys() {
                if target.root().join(path).exists() && !paths.iter().any(|held| held == path) {
                    paths.push(path.clone());
                }
            }
            paths.sort();
            Ok(paths)
        }
        Effect::Materialize { setup } => {
            let mut paths = owned.to_vec();
            overlay_payload_existing(target.root(), &setup.payload, &mut paths);
            paths.sort();
            Ok(paths)
        }
        _ => Ok(owned.to_vec()),
    }
}

fn existing_under_projection(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
) -> Result<Vec<String>> {
    let mut found = Vec::new();
    for namespace in harness.owned_projection(scope) {
        let path = target.root().join(namespace);
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            found.push((*namespace).to_owned());
        } else {
            found.extend(files_under_nofollow(&path, namespace)?);
        }
    }
    found.sort();
    Ok(found)
}

fn files_under_nofollow(root: &Path, namespace: &str) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), namespace.to_owned())];
    while let Some((directory, prefix)) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            Error::from(
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot read {}", directory.display()),
                )
                .with_source(error),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                Error::from(
                    setup_core::Error::new(
                        setup_core::ReasonCode::StateUnavailable,
                        format!("cannot read an entry of {}", directory.display()),
                    )
                    .with_source(error),
                )
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = format!("{prefix}/{name}");
            let meta = fs::symlink_metadata(entry.path()).map_err(|error| {
                Error::from(
                    setup_core::Error::new(
                        setup_core::ReasonCode::StateUnavailable,
                        format!("cannot read {}", entry.path().display()),
                    )
                    .with_source(error),
                )
            })?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                found.push(relative);
            } else {
                pending.push((entry.path(), relative));
            }
        }
    }
    Ok(found)
}

pub(crate) fn snapshot_if_unmanaged_backup(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
    owned: &[String],
    operation: Operation,
) -> Result<Vec<String>> {
    if operation == Operation::Backup
        && owned.is_empty()
        && matches!(
            ProviderState::read(target.root(), harness.state_file)?,
            StateReading::Absent
        )
    {
        existing_under_projection(harness, target, scope)
    } else {
        Ok(owned.to_vec())
    }
}

fn overlay_payload_existing(root: &Path, payload: &Path, paths: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(payload) else {
        return;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if root.join(&name).exists() && !paths.contains(&name) {
            paths.push(name);
        }
    }
}

fn json_object_file(relative: &str) -> bool {
    Path::new(relative)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

fn json_top_keys(bytes: &[u8]) -> Option<Vec<String>> {
    match serde_json::from_slice::<serde_json::Value>(bytes).ok()? {
        serde_json::Value::Object(object) => Some(object.keys().cloned().collect()),
        _ => None,
    }
}

fn merge_json_objects(existing: &[u8], incoming: &[u8]) -> Option<Vec<u8>> {
    let serde_json::Value::Object(mut base) = serde_json::from_slice(existing).ok()? else {
        return None;
    };
    let serde_json::Value::Object(add) = serde_json::from_slice(incoming).ok()? else {
        return None;
    };
    for (key, value) in add {
        base.insert(key, value);
    }
    serde_json::to_vec(&serde_json::Value::Object(base)).ok()
}

fn written_fields_path(harness: &Harness, target: &Target) -> PathBuf {
    target
        .root()
        .join(harness.control_directory)
        .join("written-fields.json")
}

fn remember_written_fields(
    harness: &Harness,
    target: &Target,
    relative: &str,
    keys: Vec<String>,
) -> Result<()> {
    let path = written_fields_path(harness, target);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Error::from(
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot create {}", parent.display()),
                )
                .with_source(error),
            )
        })?;
    }
    let mut map = read_written_fields(&path);
    map.insert(relative.to_owned(), keys);
    let bytes = serde_json::to_vec(&map).map_err(|error| {
        Error::from(
            setup_core::Error::new(
                setup_core::ReasonCode::StateUnavailable,
                "cannot encode written-fields",
            )
            .with_source(error),
        )
    })?;
    lock::atomic_write(&path, &bytes).map_err(Error::from)
}

fn read_written_fields(path: &Path) -> BTreeMap<String, Vec<String>> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn forget_written_fields(harness: &Harness, target: &Target, relative: &str) {
    let path = written_fields_path(harness, target);
    let mut map = read_written_fields(&path);
    if map.remove(relative).is_some() {
        if map.is_empty() {
            let _ = fs::remove_file(&path);
        } else if let Ok(bytes) = serde_json::to_vec(&map) {
            let _ = lock::atomic_write(&path, &bytes);
        }
    }
}

fn forget_all_written_fields(harness: &Harness, target: &Target) {
    let _ = fs::remove_file(written_fields_path(harness, target));
}

fn write_host_file(
    harness: &Harness,
    target: &Target,
    relative: &str,
    bytes: &[u8],
    merge_json: bool,
) -> Result<()> {
    let destination = target.root().join(relative);
    let outgoing = if merge_json && json_object_file(relative) && destination.exists() {
        fs::read(&destination)
            .ok()
            .and_then(|existing| merge_json_objects(&existing, bytes))
            .unwrap_or_else(|| bytes.to_vec())
    } else {
        bytes.to_vec()
    };
    if merge_json && let Some(keys) = json_top_keys(bytes) {
        remember_written_fields(harness, target, relative, keys)?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            Error::from(
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot create {}", parent.display()),
                )
                .with_source(error),
            )
        })?;
    }
    lock::atomic_write(&destination, &outgoing).map_err(Error::from)
}

fn withdraw_written(
    harness: &Harness,
    target: &Target,
    relative: &str,
    preserve_json_keys: bool,
) -> Result<()> {
    let destination = target.root().join(relative);
    if preserve_json_keys && json_object_file(relative) {
        let path = written_fields_path(harness, target);
        if let Some(keys) = read_written_fields(&path).get(relative).cloned() {
            if keys.is_empty() {
                forget_written_fields(harness, target, relative);
                return remove_keeping(&destination, target.root(), harness.never_touch);
            }
            if strip_json_keys(&destination, &keys)? {
                forget_written_fields(harness, target, relative);
                return Ok(());
            }
        }
    }
    if preserve_json_keys {
        forget_written_fields(harness, target, relative);
    }
    remove_keeping(&destination, target.root(), harness.never_touch)
}

fn strip_json_keys(path: &Path, keys: &[String]) -> Result<bool> {
    let Ok(bytes) = fs::read(path) else {
        return Ok(false);
    };
    let Ok(serde_json::Value::Object(mut object)) = serde_json::from_slice(&bytes) else {
        return Ok(false);
    };
    for key in keys {
        object.remove(key);
    }
    if object.is_empty() {
        remove_path(path)?;
    } else {
        let encoded = serde_json::to_vec(&serde_json::Value::Object(object)).map_err(|error| {
            Error::from(
                setup_core::Error::new(
                    setup_core::ReasonCode::StateUnavailable,
                    format!("cannot encode {}", path.display()),
                )
                .with_source(error),
            )
        })?;
        lock::atomic_write(path, &encoded).map_err(Error::from)?;
    }
    Ok(true)
}

/// Withdraw what this provider owns, then put the survivors back.
///
/// Returns the empty inventory on purpose: the bytes written here are the
/// person's, not this provider's, and a later scoped removal must not find
/// them in the record.
fn remove_keeping_files(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
    files: &BTreeMap<String, (Vec<u8>, u32)>,
) -> Result<Vec<String>> {
    remove_managed(harness, target, scope)?;
    for (relative, (bytes, mode)) in files {
        let destination = target.root().join(relative);
        lock::atomic_write(&destination, bytes)?;
        set_mode(&destination, *mode)?;
    }
    Ok(Vec::new())
}

/// Remove `path`, keeping anything the harness promised never to touch.
///
/// `never_touch` named three effects of ownership and protected two of them: a
/// backup does not capture these paths and an identity does not hash them.
/// The third -- deletion -- went straight through, because replacement removes
/// a namespace whole and never asked. The name promised more than it did.
///
/// It matters where a product writes its own record inside a directory this
/// provider owns: grok's `plugins/known_marketplaces.json` is a person's
/// marketplace sources, and a posture switch took it. Measured 2026-08-31 with
/// the released binary.
///
/// Only paths *under* `path` are considered, and each is spared in place: the
/// directory that holds one survives with that file in it and nothing else,
/// which is what preserving a sibling means.
fn remove_keeping(path: &Path, root: &Path, spared: &[&str]) -> Result<()> {
    let keep: Vec<PathBuf> = spared.iter().map(|name| root.join(name)).collect();
    if !keep.iter().any(|held| held.starts_with(path)) {
        return remove_path(path);
    }
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if !metadata.is_dir() {
        // A file that is itself spared, or one nothing spares.
        return if keep.iter().any(|held| held == path) {
            Ok(())
        } else {
            remove_path(path)
        };
    }
    let Ok(entries) = fs::read_dir(path) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        remove_keeping(&entry.path(), root, spared)?;
    }
    // Gone when the last thing in it went; kept when something is still held.
    let _ = fs::remove_dir(path);
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

/// Refuse a target that is a different product's configuration home.
///
/// Every command takes an explicit `--target` so this program never guesses a
/// path. That rule binds this program; it does nothing about a caller who names
/// the wrong place confidently, and Pi and Oh My Pi are one word apart --
/// `~/.pi/agent` against `~/.omp/agent`, the same shape, descended from the
/// same code.
///
/// The reason it must be a refusal rather than a note: Pi reads `settings.json`
/// and Oh My Pi reads `config.yml`. A Pi setup written into an Oh My Pi home is
/// not rejected by anything downstream -- it is **ignored**, and the directory
/// looks configured. A failure that leaves everything looking right is the one
/// worth stopping before it happens.
///
/// Three conditions, and all of them must hold, because the cost of a wrong
/// refusal is a caller who cannot configure their own target:
///
/// - the neighbour's marker is there;
/// - **none** of this provider's own namespaces are -- a home holding
///   `settings.json` is Pi's whatever else is beside it;
/// - the target is not already managed by this provider, which settles it
///   outright.
fn refuse_a_neighbours_home(
    harness: &Harness,
    resolved: &Target,
    scope: Option<provider_v3::TargetScope>,
) -> Result<()> {
    if harness.foreign_homes.is_empty() {
        return Ok(());
    }
    // Already ours: nothing to mistake.
    if matches!(
        ProviderState::read(resolved.root(), harness.state_file)?,
        StateReading::Current(_)
    ) {
        return Ok(());
    }
    // Ours by content, even without our state -- an adopted or hand-made home.
    // The question is asked of the namespaces *this scope* owns: under a scope
    // the global block names nothing that would ever be here, so asking with it
    // would report every scoped target as somebody else's.
    if harness
        .owned_projection(scope)
        .iter()
        .any(|name| resolved.root().join(name).exists())
    {
        return Ok(());
    }

    let found: Vec<&Foreign> = harness
        .foreign_homes
        .iter()
        .filter(|foreign| resolved.root().join(foreign.marker).exists())
        .collect();
    let Some(first) = found.first() else {
        return Ok(());
    };

    Err(Error::refuse(
        WireReason::UnsupportedNativeSurface,
        format!(
            "{} holds {} and none of {}'s own files, which is what {}'s configuration \
             home looks like. {} keeps its configuration in {}; this program configures \
             {} in {}. Nothing has been changed. Name the target you meant.",
            resolved.root().display(),
            found
                .iter()
                .map(|foreign| foreign.marker)
                .collect::<Vec<_>>()
                .join(" and "),
            harness.product,
            first.product,
            first.product,
            first.home,
            harness.product,
            harness.documented_config_home,
        ),
    ))
}

/// Refuse the exact owned paths a backup could not capture, before any of it.
///
/// Named rather than counted, and all of them rather than the first: a caller
/// fixing them one refusal at a time is the same defect as an argv that
/// surfaces one missing flag at a time.
fn refuse_uncapturable(resolved: &Target, owned: &[String]) -> Result<()> {
    let refused = setup_core::backup::uncapturable(resolved.root(), &as_paths(owned))?;
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
        // The same question the plan asks, asked the same way: under a scope
        // this is that scope's profile. It recorded the global one, which made
        // the state disagree with the plan that authorized it.
        projection_profile_digest: Some(
            harness
                .projection_profile_for(mutation.target_scope)?
                .digest,
        ),
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
        // The namespaces owned *at this target*, which under a scope is that
        // scope's set. Recording the global block here was the sixth face of
        // the same defect and the one that made it hard to see from outside:
        // a scoped target's state described namespaces that were never there.
        // It is also what `scope_recorded_at` reads, so a wrong value here
        // would have made `status` answer under the wrong scope.
        native_ownership: harness
            .owned_projection(mutation.target_scope)
            .iter()
            .map(|n| (*n).to_owned())
            .collect(),
        // Namespaces above, files here, and the gap between the two numbers is
        // the whole reason this field exists: under a shared root such as
        // `~/.agents` the namespaces are read by several products at once, so
        // only the files can scope a removal to what this provider put there.
        written_paths: applied.written_paths.clone(),
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

fn planned_software_artifacts(
    plan: &serde_json::Value,
) -> Result<Vec<provider_v3::plan::SoftwareArtifact>> {
    match plan.get("software_artifacts") {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone()).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("the plan artifact has no usable software_artifacts: {source}"),
            )
        }),
    }
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

    /// The earlier release's bytes, and they really are different bytes.
    ///
    /// Same length as [`TEST_PAYLOAD`] and a different digest, which is the
    /// pair that catches a resolver comparing the wrong field: a length check
    /// alone would call these two the same file.
    pub(crate) const TEST_EARLIER_PAYLOAD: &[u8] = b"#!/bin/sh\nexec echo test-harness 1.2.2\n";

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

    /// The release the test harness can move *from*, with its own bytes.
    ///
    /// A different digest from [`TEST_ARTIFACTS`] on purpose: the whole point
    /// of the second pin is that an update crosses two real trees, and a pair
    /// sharing one digest would let a resolution-by-bytes test pass while
    /// resolving nothing.
    pub(crate) const TEST_PREVIOUS_ARTIFACTS: &[setup_core::software::Artifact] = &[
        earlier_artifact("linux/x86_64"),
        earlier_artifact("linux/arm64"),
        earlier_artifact("macos/x86_64"),
        earlier_artifact("macos/arm64"),
        earlier_artifact("windows/x86_64"),
        earlier_artifact("windows/arm64"),
    ];

    const fn earlier_artifact(platform: &'static str) -> setup_core::software::Artifact {
        setup_core::software::Artifact {
            platform,
            url: "https://example.invalid/test-harness-1.2.2",
            bytes: 39,
            sha256: "sha256:42c3e0650b099f95955b0ff86c75499848e1343a6c40af6a7acd10f3c18ce226",
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
            previous: Some(setup_core::software::Previous {
                version: "1.2.2",
                artifacts: TEST_PREVIOUS_ARTIFACTS,
            }),
        };

    pub(crate) const TEST: Harness = Harness {
        launch_binding: crate::facts::LaunchBinding::Complete { how: "a fixture" },
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
        updates_off_env: "",
        config_home_note: "",
        control_directory: ".test-setup-system",
        state_file: "NDDEV-TEST-PROVIDER.json",
        profile_id: "test/native-files/1",
        native_namespaces: &["AGENTS.md", "settings.json", "skills"],
        shadowing_names: &[],
        custody_namespaces: &[],
        never_touch: &[".credentials.json", "sessions"],
        foreign_homes: &[],
        permission_profiles: &["default"],
        component_kinds: &[
            ComponentKind::Instruction,
            ComponentKind::Skill,
            ComponentKind::Setting,
        ],
        projection_kinds: &[ProjectionKind::NativeFiles],
        // A second target, shaped like codex's: a convention root several
        // products read, owning one namespace inside it. It is here because the
        // scoped tests below drove `--target-scope user_root` through a harness
        // that declared no scope at all -- the runtime keyed its behaviour off
        // the *request* rather than off the declaration, so the tests proved
        // the behaviour of a provider that could not exist.
        scoped_projections: &[crate::facts::Scoped {
            target_scope: provider_v3::TargetScope::UserRoot,
            profile_id: "test/native-files/user-root/1",
            component_kinds: &[ComponentKind::Skill],
            projection_kinds: &[ProjectionKind::NativeFiles],
            // Deliberately not `skills`, which this harness owns globally: a
            // filesystem does not know about scopes, so one name in both blocks
            // is two declarations of one path and the surfaces guard refuses it
            // by name. Codex avoids the same collision by declining `skills`
            // under its global home.
            native_namespaces: &["shared"],
        }],
        max_files: 4096,
        max_bytes: 64 * 1024 * 1024,
        kit_identity: r#"{"aggregate_digest":"sha256:aa","protocol_version":3}"#,
    };
}

#[cfg(test)]
mod tests {
    // A test may spawn: several of these drive a real executable, which is the
    // only way to prove what the shipped binary does rather than what this
    // source believes. The lint's subject is the program, not its tests.
    #![allow(
        clippy::unwrap_used,
        clippy::panic,
        clippy::disallowed_types,
        reason = "tests drive real executables to check the shipped behaviour"
    )]

    use std::fs;
    use std::path::{Path, PathBuf};

    use provider_v3::argv;

    use super::*;
    use crate::facts::Shadow;

    use crate::wire::tests_support::{TEST, TEST_EARLIER_PAYLOAD, TEST_PAYLOAD};

    const RELEASE: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";

    /// A per-test directory holding the target and anything written beside it.
    ///
    /// The plan artifact must live *outside* the target: inside, it would change
    /// the target's identity between plan and apply, and the apply would then
    /// correctly refuse its own plan as stale. It must also be unique per test,
    /// because these run in parallel.
    /// A directory no other running test shares.
    ///
    /// The pid alone was not enough. These tests are registered twice in one
    /// binary, so two threads call this with the same `name` at the same time,
    /// land on the same path, and the second one to take the target lock is
    /// refused with *"this process already holds …"* — a failure about the
    /// fixture wearing the shape of a locking defect. It only shows on a test
    /// that holds the lock long enough to overlap, which is why it stayed
    /// invisible until one did: the plan-only tests release before the twin
    /// arrives.
    fn scratch(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let nth = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!(
            "harness-runtime-{name}-{}-{nth}",
            std::process::id()
        ));
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

    /// The plan says what a mutation **takes** before it says what it writes.
    ///
    /// `0.0.24` fixed this sentence on `remove`'s *result* line and left it on
    /// the two plan surfaces, which are the ones a consumer renders before a
    /// person approves. `Operation::Remove` planned *"withdraw every file this
    /// provider owns"* -- which used to mean each namespace whole. Default
    /// `remove_managed` now withdraws recorded files, and `Install` must name
    /// that withdrawal before it enumerates writes.
    ///
    /// Observed red against the shipped wording before it was kept: the old
    /// `remove` line contains neither a namespace name nor the word `whole`,
    /// and the old install effects contained no removal line at all.
    #[test]
    fn a_plan_for_default_remove_names_recorded_files_not_namespaces_whole() {
        let target = seeded("effects-name-what-goes");
        install_global(&target, "effects", &[("AGENTS.md", "# ours\n", 0o644)]);
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
            ],
        ));
        let effects: Vec<String> = planned["plan"]["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_owned())
            .collect();
        let text = effects.join("\n");
        assert!(
            text.contains("only the files this provider recorded writing"),
            "the plan does not name recorded files: {effects:?}"
        );
        assert!(
            !text.contains("go whole, not file by file"),
            "default remove still claimed namespaces go whole: {effects:?}"
        );
        assert!(
            !text.contains("withdraw every file this provider owns"),
            "the false sentence survived: {effects:?}"
        );
    }

    /// Under a scope the same plan promises the opposite, because the
    /// behaviour is the opposite.
    ///
    /// `remove` and `replace` under `user_root` act on the paths this provider
    /// recorded writing: a shared root is read by several products at once. The
    /// global sentence would be false here in the other direction -- it would
    /// promise to take a neighbour's files -- so one wording for both cases is
    /// wrong whichever wording is chosen.
    /// And the state a scoped operation writes records the same profile.
    ///
    /// The plan and the state are read by different callers at different times,
    /// and they disagreed: the plan is what a consumer approves, the state is
    /// what `status` publishes afterwards, and a consumer comparing the two
    /// found one profile in the authorization and another in the record of it.
    /// Split from the plan test rather than folded into it, because fixing the
    /// plan alone would leave this half green-by-omission.
    #[test]
    fn the_state_a_scoped_operation_writes_names_the_scoped_profile() {
        let info = dispatch(&TEST, argv::parse(["provider-info"]).unwrap()).unwrap();
        let global = info["projection_profile"]["digest"].as_str().unwrap();
        let scoped = info["scoped_projection_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["target_scope"] == "user_root")
            .unwrap()["digest"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(scoped, global, "the fixture's two profiles are identical");

        let target = seeded("scoped-profile-state");
        install_scoped(&target, "profile", "one", "# one\n");

        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(TEST.state_file)).unwrap()).unwrap();
        assert_eq!(
            state["projection_profile_digest"], scoped,
            "the state after a scoped install named the global profile"
        );
    }

    /// A scoped plan names the scoped profile, not the global one.
    ///
    /// `provider-info` publishes two profiles for a harness with a scope, and a
    /// consumer compiles its bundle against the scoped one. The plan handed back
    /// carried `projection_profile_digest` from `projection_profile()`, which
    /// answers with the global block whatever it is asked — so the two sides
    /// named different profiles for the same operation, and the state written
    /// afterwards recorded the global one as well.
    ///
    /// Both halves are asserted. Equal to the scoped digest is the claim;
    /// *different from the global* is what makes the first assertion mean
    /// something, since a build whose two profiles happened to agree would pass
    /// the first on its own.
    #[test]
    fn a_scoped_plan_names_the_scoped_profile_and_not_the_global_one() {
        let info = dispatch(&TEST, argv::parse(["provider-info"]).unwrap()).unwrap();
        let global = info["projection_profile"]["digest"].as_str().unwrap();
        let scoped = info["scoped_projection_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["target_scope"] == "user_root")
            .unwrap()["digest"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_ne!(
            scoped, global,
            "the fixture's two profiles are identical, so this test cannot fail"
        );

        let target = seeded("scoped-profile-digest");
        let planned = scoped_plan(&target, "remove", "operation_01SCOPEDPROFILE");
        assert_eq!(
            planned["plan"]["projection_profile_digest"], scoped,
            "a user_root plan named a profile the consumer did not compile against"
        );
    }

    #[test]
    fn under_a_scope_the_plan_promises_the_recorded_files_and_not_the_namespaces() {
        let target = seeded("effects-scoped");
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
                "--target-scope",
                "user_root",
            ],
        ));
        let text = planned["plan"]["effects"].to_string();
        assert!(
            text.contains("only the files this provider recorded writing"),
            "{text}"
        );
        assert!(
            text.contains("left alone"),
            "the scoped plan does not say a neighbour is left alone: {text}"
        );
        assert!(
            !text.contains("go whole, not file by file"),
            "the global sentence reached a scoped plan: {text}"
        );
    }

    /// A removal under a shared root is refused, and the refusal says why.
    ///
    /// `user_root` names a convention's root, not a product's home: four of the
    /// seven products read `~/.agents/skills`. A whole-namespace removal there
    /// takes three neighbours' content, and this build has no per-file record to
    /// scope it to -- provider state records namespaces. Refusing is the answer
    /// until it does.
    ///
    /// Planned with the scope and applied from the plan, because that is the
    /// only path a scope travels: `apply` is handed a plan, never a scope. So
    /// the plan is produced first and `apply` is invoked directly, rather than
    /// through the helper, which unwraps and would turn the refusal into a
    /// panic.
    #[test]
    fn a_removal_under_a_shared_root_is_refused_rather_than_performed() {
        let target = seeded("shared-root-remove");
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                far_future(),
                "--target-scope",
                "user_root",
            ],
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert_eq!(planned["plan"]["target_scope"], "user_root", "{planned}");

        let plan_path = target.join("..").join("plan.json");
        fs::write(&plan_path, serde_json::to_vec(&planned["plan"]).unwrap()).unwrap();
        let done = run(args(
            "apply-operation",
            &target,
            &[
                "--plan",
                plan_path.to_str().unwrap(),
                "--plan-digest",
                planned["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ));
        assert_eq!(done["state"], "verified", "{done}");
        assert!(
            target.join("AGENTS.md").exists(),
            "unrecorded scoped remove took a file this provider never wrote"
        );
    }

    /// With a record, the removal happens and takes only what it wrote.
    ///
    /// The other half of the branch above, and the reason `written_paths`
    /// exists: under a shared root the owned *namespaces* belong to several
    /// products at once, so only the files this provider recorded writing can
    /// scope a removal. A neighbour's file inside an owned namespace is the
    /// case that decides it.
    #[test]
    fn a_removal_under_a_shared_root_takes_only_the_files_this_build_wrote() {
        let target = seeded("shared-root-scoped");

        // Establish a record through the ordinary write path: a backup, then a
        // restore, which materialises the slot's payload and records the files
        // it put there. Any operation that writes would do; this one needs no
        // bundle to construct.
        fs::create_dir_all(target.join("skills").join("ours")).unwrap();
        fs::write(
            target.join("skills").join("ours").join("SKILL.md"),
            b"ours\n",
        )
        .unwrap();
        let captured = plan_then_apply(&target, "backup", &[]);
        assert_eq!(captured["state"], "verified", "{captured}");
        let restored = plan_then_apply(&target, "restore", &[]);
        assert_eq!(restored["state"], "verified", "{restored}");

        // A neighbour writes into a namespace this build owns. Under a shared
        // root that is the ordinary case, not an intrusion.
        let theirs = target.join("skills").join("someone-elses");
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("SKILL.md"), b"another product's skill\n").unwrap();

        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01SCOPED",
                "--expires-at",
                far_future(),
                "--target-scope",
                "user_root",
            ],
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        let plan_path = target.join("..").join("plan-scoped.json");
        fs::write(&plan_path, serde_json::to_vec(&planned["plan"]).unwrap()).unwrap();
        let done = run(args(
            "apply-operation",
            &target,
            &[
                "--plan",
                plan_path.to_str().unwrap(),
                "--plan-digest",
                planned["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ));
        assert_eq!(done["state"], "verified", "{done}");

        assert!(
            theirs.join("SKILL.md").exists(),
            "the removal took a file this build never wrote"
        );
        assert!(
            !target.join("AGENTS.md").exists(),
            "the removal left a file this build did write"
        );
    }

    /// A backup under a scope captures something, and a neighbour's file is
    /// not it.
    ///
    /// **The defect this is written from, measured on the shipped `0.0.27`:**
    /// `owned_projection` answered with the *global* namespaces whatever scope
    /// an operation named, so under `user_root` every verb looked for files
    /// that were never at that root. A `~/.agents` holding skills planned a
    /// backup whose `expected_target_digest` was the digest of the empty string
    /// and applied it into a slot holding `slot.json` and no payload — a backup
    /// that reports success and captures nothing, and therefore a restore that
    /// silently puts nothing back.
    ///
    /// Both halves are asserted, because fixing the first without the second is
    /// what a shared root makes easy: capturing the namespace whole would fill
    /// the slot and revert a neighbour's file on the way back out.
    #[test]
    fn a_backup_under_a_scope_captures_the_files_this_build_wrote_and_no_others() {
        let target = seeded("scoped-capture");
        install_scoped(&target, "cap", "ours", "# ours\n");

        // A neighbour writes into the same shared namespace afterwards. Under a
        // convention root that is the ordinary case, not an intrusion.
        let theirs = target.join("shared").join("someone-elses");
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("SKILL.md"), b"another product's skill\n").unwrap();

        let planned = scoped_plan(&target, "backup", "operation_01SCOPEDCAP");
        assert_ne!(
            planned["expected_target_digest"].as_str().unwrap(),
            EMPTY_TREE,
            "a target holding this provider's own files read as empty"
        );
        let done = scoped_apply(&target, &planned, "cap-backup");
        assert_eq!(done["state"], "verified", "{done}");

        let payload = target
            .join(TEST.control_directory)
            .join("backups")
            .join(done["backup_ref"].as_str().unwrap())
            .join("payload");
        assert!(
            payload
                .join("shared")
                .join("ours")
                .join("SKILL.md")
                .exists(),
            "the capture took nothing this provider had written"
        );
        assert!(
            !payload.join("shared").join("someone-elses").exists(),
            "the capture took a neighbour's file into this provider's slot"
        );
    }

    /// A restore under a scope leaves a neighbour's file exactly as it was.
    ///
    /// `replace_managed_from` says in its own header that a restore must not
    /// revert files this provider never wrote. Under a shared root that needs a
    /// different mechanism, not a different rule: clearing the namespace and
    /// copying the payload over it would move every neighbour's file back to
    /// what it was when the slot was taken.
    #[test]
    fn a_restore_under_a_scope_does_not_revert_a_neighbours_file() {
        let target = seeded("scoped-restore");
        install_scoped(&target, "res", "ours", "# ours\n");

        let theirs = target.join("shared").join("someone-elses");
        fs::create_dir_all(&theirs).unwrap();
        fs::write(theirs.join("SKILL.md"), b"before\n").unwrap();

        let planned = scoped_plan(&target, "backup", "operation_01SCOPEDB");
        let captured = scoped_apply(&target, &planned, "res-backup");
        assert_eq!(captured["state"], "verified", "{captured}");

        // The neighbour moves on after the slot was taken, and this provider's
        // own file is damaged.
        fs::write(theirs.join("SKILL.md"), b"after\n").unwrap();
        fs::write(
            target.join("shared").join("ours").join("SKILL.md"),
            b"# damaged\n",
        )
        .unwrap();

        let planned = scoped_plan(&target, "restore", "operation_01SCOPEDR");
        let promised = planned["plan"]["restore_target_digest"]
            .as_str()
            .unwrap()
            .to_owned();
        let done = scoped_apply(&target, &planned, "res-restore");
        assert_eq!(done["state"], "verified", "{done}");
        assert_eq!(
            fs::read_to_string(target.join("shared").join("ours").join("SKILL.md")).unwrap(),
            "# ours\n",
            "the restore did not return this provider's own file"
        );
        assert_eq!(
            fs::read_to_string(theirs.join("SKILL.md")).unwrap(),
            "after\n",
            "the restore reverted a file this provider never wrote"
        );
        let status = run(args("status", &target, &["--target-scope", "user_root"]));
        assert_eq!(
            status["target_digest"], promised,
            "restore produced bytes different from its BackupRef-bound promise"
        );
    }

    /// A backup writes nothing, so it must not erase the record of what was
    /// written.
    ///
    /// Globally this cost nothing, because a removal reads the namespaces. Under
    /// a scope the record *is* the inventory, so a backup that reset it to the
    /// empty list left the next removal taking nothing while reporting success.
    #[test]
    fn a_backup_leaves_the_inventory_it_found() {
        let target = seeded("inventory-survives-backup");
        install_scoped(&target, "inv", "ours", "# ours\n");
        let before = recorded_written(&target);
        assert!(!before.is_empty(), "the install recorded nothing");

        let planned = scoped_plan(&target, "backup", "operation_01INVENTORY");
        let done = scoped_apply(&target, &planned, "inv-backup");
        assert_eq!(done["state"], "verified", "{done}");
        assert_eq!(
            recorded_written(&target),
            before,
            "the backup erased the record of what this provider had written"
        );
    }

    /// A scope this provider publishes no profile for is refused at plan time.
    ///
    /// The runtime used to key its scoped handling off the *request*: a harness
    /// declaring no scope at all still behaved as though it had one for
    /// `user_root`, and as though it had none for any other name. The
    /// declaration decides, here as everywhere.
    #[test]
    fn a_scope_this_provider_never_declared_is_refused() {
        let target = seeded("undeclared-scope");
        let mut harness = TEST;
        harness.scoped_projections = &[];
        let error = dispatch(
            &harness,
            argv::parse(args(
                "plan-operation",
                &target,
                &[
                    "--operation",
                    "backup",
                    "--provider-release-digest",
                    RELEASE,
                    "--operation-id",
                    "operation_01UNDECLARED",
                    "--expires-at",
                    far_future(),
                    "--target-scope",
                    "user_root",
                ],
            ))
            .unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
        let said = error.to_string();
        for wanted in ["user_root", "only the global one"] {
            assert!(said.contains(wanted), "{said}");
        }
    }

    /// The digest of nothing, which is what every scoped reading used to be.
    const EMPTY_TREE: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    /// Plan one operation under `user_root`.
    fn scoped_plan(target: &Path, operation: &str, operation_id: &str) -> serde_json::Value {
        let planned = run(args(
            "plan-operation",
            target,
            &[
                "--operation",
                operation,
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                operation_id,
                "--expires-at",
                far_future(),
                "--target-scope",
                "user_root",
            ],
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        planned
    }

    /// Apply a plan produced by [`scoped_plan`], with its own plan file.
    fn scoped_apply(target: &Path, planned: &serde_json::Value, tag: &str) -> serde_json::Value {
        let plan_path = target.join("..").join(format!("plan-scoped-{tag}.json"));
        fs::write(&plan_path, serde_json::to_vec(&planned["plan"]).unwrap()).unwrap();
        run(args(
            "apply-operation",
            target,
            &[
                "--plan",
                plan_path.to_str().unwrap(),
                "--plan-digest",
                planned["plan_digest"].as_str().unwrap(),
                "--provider-release-digest",
                RELEASE,
            ],
        ))
    }

    /// Install one skill into the scoped namespace, the way a consumer would.
    ///
    /// The inventory these tests are about has to be *established under the
    /// scope*, not inherited from a global operation: a test whose recorded
    /// files all live in the global namespaces cannot tell a scoped clear from
    /// a global one, and the first version of these three could not.
    fn install_scoped(target: &Path, tag: &str, name: &str, body: &str) {
        let relative = format!("shared/{name}/SKILL.md");
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &TEST,
            Some(provider_v3::TargetScope::UserRoot),
            &[(&relative, body, 0o644)],
            Some("skill"),
        );
        let artifact_path = target.join("..").join(format!("scoped-{tag}.zip"));
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            format!("operation_01SCOPEDIN{}", tag.to_uppercase()),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "user_root".to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");

        let plan_path = target.join("..").join(format!("scoped-{tag}-plan.json"));
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
        let applied = run(args("apply-operation", target, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
    }

    /// The files this provider's state records writing at a target.
    fn recorded_written(target: &Path) -> Vec<String> {
        let bytes = fs::read(target.join(TEST.state_file)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["written_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry.as_str().unwrap().to_owned())
            .collect()
    }

    fn install_global(target: &Path, tag: &str, files: &[(&str, &str, u32)]) {
        let (bytes, bundle_digest, artifact) =
            bundle_bytes_for(&TEST, None, files, Some("instruction"));
        let artifact_path = target.join("..").join(format!("global-{tag}.zip"));
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            format!("operation_01GLOBALIN{}", tag.to_uppercase()),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        let plan_path = target.join("..").join(format!("global-{tag}-plan.json"));
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
        let applied = run(args("apply-operation", target, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
    }

    fn replace_global(target: &Path, tag: &str, files: &[(&str, &str, u32)]) {
        let (bytes, bundle_digest, artifact) =
            bundle_bytes_for(&TEST, None, files, Some("instruction"));
        let artifact_path = target.join("..").join(format!("global-{tag}.zip"));
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let mut plan_args = vec![
            "--operation".to_owned(),
            "replace".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            format!("operation_01GLOBALRP{}", tag.to_uppercase()),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert!(
            !planned["plan"]["effects"]
                .to_string()
                .contains("go whole, not file by file"),
            "replace plan claimed namespaces go whole: {}",
            planned["plan"]["effects"]
        );
        let plan_path = target.join("..").join(format!("global-{tag}-plan.json"));
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
        let applied = run(args("apply-operation", target, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn global_composition_leaves_unrecorded_files_dirs_and_keys_and_reset_clears_them() {
        let target = seeded("global-receipts");
        fs::create_dir_all(target.join("skills").join("person")).unwrap();
        fs::write(
            target.join("skills").join("person").join("SKILL.md"),
            "theirs\n",
        )
        .unwrap();
        fs::write(
            target.join("settings.json"),
            r#"{"theme":"dark","model":"first"}"#,
        )
        .unwrap();

        install_global(
            &target,
            "one",
            &[
                ("AGENTS.md", "# ours\n", 0o644),
                ("settings.json", r#"{"model":"ours"}"#, 0o644),
            ],
        );
        assert_eq!(
            fs::read_to_string(target.join("skills").join("person").join("SKILL.md")).unwrap(),
            "theirs\n"
        );
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["model"], "ours");
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );

        replace_global(
            &target,
            "two",
            &[
                ("AGENTS.md", "# next\n", 0o644),
                ("settings.json", r#"{"model":"next"}"#, 0o644),
            ],
        );
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# next\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("skills").join("person").join("SKILL.md")).unwrap(),
            "theirs\n"
        );
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["model"], "next");

        let restored = plan_then_apply(&target, "restore", &[]);
        assert_eq!(restored["state"], "verified", "{restored}");
        assert_eq!(
            fs::read_to_string(target.join("skills").join("person").join("SKILL.md")).unwrap(),
            "theirs\n",
            "restore after replace took an unrecorded nested file"
        );

        let removed = plan_then_apply(&target, "remove", &[]);
        assert_eq!(removed["state"], "verified", "{removed}");
        assert!(!target.join("AGENTS.md").exists());
        assert!(
            target
                .join("skills")
                .join("person")
                .join("SKILL.md")
                .exists(),
            "remove took an unrecorded nested directory"
        );
        let settings: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert!(settings.get("model").is_none());

        install_global(&target, "again", &[("AGENTS.md", "# again\n", 0o644)]);
        let reset_plan = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "reset",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01RESETTEXT",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(reset_plan["state"], "planned", "{reset_plan}");
        let reset_text = reset_plan["plan"]["effects"].to_string();
        assert!(
            reset_text.contains("go whole, not file by file"),
            "reset plan did not name whole namespaces: {reset_text}"
        );
        let reset = plan_then_apply(&target, "reset", &[]);
        assert_eq!(reset["state"], "verified", "{reset}");
        assert!(
            !target
                .join("skills")
                .join("person")
                .join("SKILL.md")
                .exists(),
            "reset left an unrecorded nested directory"
        );
        assert!(
            !target.join("settings.json").exists(),
            "reset left the settings file"
        );
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );
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
        assert_eq!(answer["reason"], "adaptation_binding_mismatch");

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
    fn replacement_spares_a_never_touch_path_inside_a_namespace_it_empties() {
        const SPARES: Harness = Harness {
            never_touch: &["skills/their-record.json"],
            ..TEST
        };
        // `never_touch` named three effects of ownership and stopped two: a
        // backup does not capture these and an identity does not hash them.
        // Replacement removed a namespace whole and never asked, so the third
        // went through. Measured with the released grok binary: a posture
        // switch took `plugins/known_marketplaces.json`, which the product's
        // own `plugin marketplace` command writes for a person.
        let target = seeded("spared");
        let inside = target.join("skills").join("their-record.json");
        fs::write(&inside, b"a person's own file, inside a namespace we own").unwrap();
        let beside = target.join("skills").join("nothing-spares-this.md");
        fs::write(&beside, b"an ordinary sibling").unwrap();

        // The control first: with nothing spared, replacement takes both.
        let payload = scratch("spared-payload").join("target");
        fs::create_dir_all(payload.join("skills")).unwrap();
        fs::write(payload.join("skills").join("ours.md"), b"ours").unwrap();
        let resolved = Target::resolve(&target, TEST.control_directory).unwrap();
        replace_managed_from(&TEST, &resolved, &payload, None, true).unwrap();
        assert!(inside.exists(), "composition took an unrecorded host file");
        assert!(beside.exists(), "composition took an unrecorded sibling");
        assert!(
            target.join("skills").join("ours.md").exists(),
            "our own payload did not land beside it"
        );

        reset_namespaces(&TEST, &resolved).unwrap();
        assert!(!inside.exists(), "reset left an unrecorded host file");
        assert!(!beside.exists(), "reset left an unrecorded sibling");

        fs::create_dir_all(inside.parent().unwrap()).unwrap();
        fs::write(&inside, b"a person's own file, inside a namespace we own").unwrap();
        fs::write(&beside, b"an ordinary sibling").unwrap();
        reset_namespaces(&SPARES, &resolved).unwrap();
        assert!(inside.exists(), "the named path was taken anyway");
        assert!(!beside.exists(), "a sibling nothing names survived");
    }

    #[test]
    fn status_names_a_file_the_product_reads_and_this_provider_does_not_own() {
        // `state` and `target_digest` are statements about the bytes this
        // provider wrote. Neither can see a *sibling* the product prefers, and
        // for `opencode` that sibling decides: an `opencode.jsonc` beside our
        // `opencode.json` is the one the product keeps.
        const SHADOWED: Harness = Harness {
            shadowing_names: &[Shadow {
                name: "settings.jsonc",
                over: "settings.json",
                effect: "the product keeps the later of the two",
            }],
            ..TEST
        };
        let target = seeded("shadowed");

        // The control first, and it is the whole test: with only the owned
        // name present the answer must be empty. A field that is always
        // populated says nothing, and a field that is never populated cannot
        // be told from one that does not work.
        let quiet = dispatch(
            &SHADOWED,
            argv::parse(args("status", &target, &[])).unwrap(),
        )
        .unwrap();
        assert_eq!(
            quiet["shadowed_by"].as_array().map(Vec::len),
            Some(0),
            "nothing shadows this target and status named something"
        );

        fs::write(target.join("settings.jsonc"), "{\"model\":\"theirs\"}").unwrap();
        let loud = dispatch(
            &SHADOWED,
            argv::parse(args("status", &target, &[])).unwrap(),
        )
        .unwrap();
        assert_eq!(loud["shadowed_by"][0]["name"], "settings.jsonc");
        assert_eq!(loud["shadowed_by"][0]["over"], "settings.json");

        // And it stays a report. The owned bytes did not change, so the digest
        // and the state must not either -- a provider that called this drift
        // would be claiming the person's own file is damage.
        assert_eq!(quiet["target_digest"], loud["target_digest"]);
        assert_eq!(quiet["state"], loud["state"]);
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
        install_global(&target, "drifted", &[("AGENTS.md", "# ours\n", 0o644)]);
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
        install_global(&target, "id", &[("AGENTS.md", "# ours\n", 0o644)]);
        let before = run(args("status", &target, &[]))["target_identity_digest"].clone();

        fs::write(target.join("AGENTS.md"), "# edited\n").unwrap();
        let edited = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_ne!(
            before, edited,
            "an edit of a recorded file left the identity alone"
        );

        fs::write(target.join("skills").join("a.md"), "edited extra").unwrap();
        let extra = run(args("status", &target, &[]))["target_identity_digest"].clone();
        assert_eq!(edited, extra, "an unrecorded extra file moved the identity");
    }

    /// Drift is still drift. The narrower reading must not turn a real change
    /// into a clean target.
    #[test]
    fn drift_inside_an_owned_namespace_is_still_reported() {
        let target = seeded("identity-drift");
        install_global(&target, "drift", &[("AGENTS.md", "# ours\n", 0o644)]);
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

        fs::write(target.join("skills").join("extra.md"), "theirs").unwrap();
        assert_eq!(
            run(args("status", &target, &[]))["provider_state"]["drift_state"],
            "clean",
            "an unrecorded extra file was reported as this provider's drift"
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
    fn a_zero_byte_setup_version_records_everything_a_populated_one_does() {
        let target = seeded("empty-bundle");
        let (bytes, bundle_digest, artifact) = bundle_bytes(&[("AGENTS.md", "", 0o644)]);
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
        install_global(&target, "overlay", &[("AGENTS.md", "# first\n", 0o644)]);
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
        let said = error.to_string();
        assert!(said.contains("one.md"), "{said}");
        assert!(said.contains("two"), "{said}");

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

        // Same prefix the plan bound. A different directory is a different
        // resource and is refused on that ground; this test is about a
        // configuration edit of `--target` not stranding a program install.
        let prefix = ready_prefix(&target);
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
                &prefix,
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

    /// A harness with a near neighbour, for the tests below.
    fn with_a_neighbour() -> Harness {
        let mut harness = TEST;
        harness.foreign_homes = &[
            crate::facts::Foreign {
                marker: "config.yml",
                product: "Oh My Pi",
                home: "~/.omp/agent",
            },
            crate::facts::Foreign {
                marker: "models.yml",
                product: "Oh My Pi",
                home: "~/.omp/agent",
            },
        ];
        harness
    }

    fn refuse_for(harness: &Harness, tokens: Vec<String>) -> provider_v3::Error {
        dispatch(harness, argv::parse(tokens).unwrap()).unwrap_err()
    }

    fn run_for(harness: &Harness, tokens: Vec<String>) -> serde_json::Value {
        dispatch(harness, argv::parse(tokens).unwrap()).unwrap()
    }

    fn backup_plan(target: &Path) -> Vec<String> {
        args(
            "plan-operation",
            target,
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01NEIGHBOUR",
                "--expires-at",
                far_future(),
            ],
        )
    }

    /// A target that is a neighbouring product's home is refused before
    /// anything moves, and the refusal says where each product actually lives.
    ///
    /// Pi and Oh My Pi are one word apart -- `~/.pi/agent` against
    /// `~/.omp/agent` -- the same shape, descended from the same code. What
    /// makes the confusion worth stopping is that it is **silent**: Pi reads
    /// `settings.json` and Oh My Pi reads `config.yml`, so a Pi setup written
    /// into an Oh My Pi home is not rejected by anything. It is ignored, and
    /// the directory looks configured.
    #[test]
    fn a_target_that_is_a_neighbours_home_is_refused_and_says_whose() {
        let harness = with_a_neighbour();
        let target = scratch("neighbour-home").join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("config.yml"), "memory:\n  backend: off\n").unwrap();

        let error = refuse_for(&harness, backup_plan(&target));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        for expected in [
            "config.yml",
            "Oh My Pi",
            "~/.omp/agent",
            "Nothing has been changed",
        ] {
            assert!(
                error.detail().contains(expected),
                "the refusal does not say {expected:?}: {}",
                error.detail()
            );
        }

        // A refusal to start: no slot, no journal, nothing to recover.
        let control = target.join(harness.control_directory);
        assert_eq!(
            fs::read_dir(control.join("backups")).map_or(0, Iterator::count),
            0
        );
        assert!(!control.join("journal.json").exists());
    }

    /// The three ways a target earns the benefit of the doubt.
    ///
    /// The cost of a wrong refusal is a caller who cannot configure their own
    /// target, so each of these is asserted rather than assumed.
    #[test]
    fn a_target_that_is_ours_is_not_mistaken_for_a_neighbours() {
        let harness = with_a_neighbour();

        // One: it holds our own files, whatever else is beside them. A home
        // with both is Pi's -- Oh My Pi would not have written `settings.json`.
        let both = scratch("neighbour-both").join("target");
        fs::create_dir_all(&both).unwrap();
        fs::write(both.join("config.yml"), "x").unwrap();
        fs::write(both.join("settings.json"), "{}").unwrap();
        assert_eq!(
            run_for(&harness, backup_plan(&both))["state"],
            "planned",
            "a target holding our own file was refused as a neighbour's"
        );

        // Two: it is already managed by us, which settles it outright.
        let managed = seeded("neighbour-managed");
        plan_then_apply(&managed, "backup", &[]);
        fs::write(managed.join("config.yml"), "arrived later").unwrap();
        assert_eq!(
            run_for(&harness, backup_plan(&managed))["state"],
            "planned",
            "a target we already manage was refused as a neighbour's"
        );

        // Three: no marker at all. An empty target is the ordinary case and
        // must never be refused.
        let empty = scratch("neighbour-empty").join("target");
        fs::create_dir_all(&empty).unwrap();
        assert_eq!(run_for(&harness, backup_plan(&empty))["state"], "planned");
    }

    /// A harness with no measured neighbour never refuses on this ground.
    ///
    /// Six of the seven declare none. A marker listed without evidence is a
    /// refusal waiting to happen, so the empty case is the one most of the
    /// estate runs and it is asserted too.
    #[test]
    fn a_harness_with_no_neighbour_refuses_nothing_on_this_ground() {
        let target = scratch("neighbour-none").join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("config.yml"), "someone else's file").unwrap();
        assert_eq!(run_for(&TEST, backup_plan(&target))["state"], "planned");
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
        assert!(
            target.join("skills").join("b.md").exists(),
            "restore took a file this provider never recorded writing"
        );
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
        install_global(
            &target,
            "owned",
            &[
                ("AGENTS.md", "# ours\n", 0o644),
                ("settings.json", r#"{"model":"ours"}"#, 0o644),
            ],
        );
        fs::write(target.join("skills").join("person.md"), "theirs\n").unwrap();
        assert_eq!(plan_then_apply(&target, "remove", &[])["state"], "verified");
        assert!(!target.join("AGENTS.md").exists());
        assert!(
            target.join("skills").join("person.md").exists(),
            "remove took an unrecorded skill file"
        );
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
            target_scope: None,
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
    fn an_unsettled_operation_is_named_in_the_key_the_consumer_reads() {
        // `status` has published the journal since the beginning, under
        // `journal`. Measured 2026-08-31 against `ai-stp-cli 0.0.10`: it never
        // reads that key. Both of its recovery paths gate on
        // `state == "recovery_required"` or on `cleanup_state`, and a target
        // holding a prepared journal answered `managed` with neither -- so the
        // fact was in the answer, under a name the reader does not know, and
        // the recovery it exists to trigger could not fire.
        //
        // This asserts the consumer's own condition rather than our field, so
        // it fails if the value stops satisfying the thing that reads it.
        fn recovery_fires(answer: &serde_json::Value) -> bool {
            answer["state"] == "recovery_required"
                || matches!(
                    answer["cleanup_state"].as_str(),
                    Some("pending" | "required" | "in_progress")
                )
        }

        let target = seeded("cleanup-state");
        plan_then_apply(&target, "backup", &[]);

        // The control, and the half that matters: a settled target must not
        // ask for recovery. A field that always fires sends every caller into
        // a restore it does not need.
        let settled = run(args("status", &target, &[]));
        assert_eq!(settled["cleanup_state"], "none");
        assert!(
            !recovery_fires(&settled),
            "a settled target asked for recovery"
        );

        let control = target.join(TEST.control_directory);
        let entry = |phase| Journal {
            schema_version: JOURNAL_SCHEMA,
            phase,
            operation_id: "operation_01INTERRUPTED".to_owned(),
            operation: "restore".to_owned(),
            plan_digest: RELEASE.to_owned(),
            target_precondition_digest: RELEASE.to_owned(),
            backup_ref: None,
            target_scope: None,
        };

        // Prepared: the effect may be partial and a restore is owed.
        entry(Phase::Prepared).publish_prepared(&control).unwrap();
        let interrupted = run(args("status", &target, &[]));
        assert_eq!(interrupted["cleanup_state"], "required");
        assert_eq!(
            interrupted["state"], "managed",
            "state still describes the directory"
        );
        assert!(recovery_fires(&interrupted));

        // Committed: the effect landed and only the tail is left. Promoted
        // rather than written, because `publish_prepared` sets the phase
        // itself -- the first draft of this test wrote `Phase::Committed` into
        // it and got `required` back, which is the API refusing to let a test
        // fake a phase the program never reaches that way.
        entry(Phase::Prepared)
            .promote_to_committed(&control)
            .unwrap();
        let tail = run(args("status", &target, &[]));
        assert_eq!(tail["cleanup_state"], "pending");
        assert!(recovery_fires(&tail));
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
            target_scope: None,
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
        bundle_bytes_for(&TEST, None, files, kind)
    }

    /// A v2 bundle bound to the exact profile selected for one test scope.
    #[allow(
        clippy::too_many_lines,
        reason = "the fixture names every canonical v2 manifest and ZIP member in one place"
    )]
    fn bundle_bytes_for(
        harness: &Harness,
        scope: Option<provider_v3::TargetScope>,
        files: &[(&str, &str, u32)],
        kind: Option<&str>,
    ) -> (Vec<u8>, String, String) {
        use provider_v3::bundle::{BUNDLE_DOMAIN, FILES_PREFIX, MANIFEST_MEMBER, REQUIRED_MEMBERS};
        use provider_v3::zip::build::{Entry, write};

        let owner = "component_00000000000000000000000000";
        let mut member_paths = files.iter().map(|(path, _, _)| *path).collect::<Vec<_>>();
        member_paths.sort_unstable();
        let profile = harness.projection_profile_for(scope).unwrap();
        let records: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, body, mode)| {
                serde_json::json!({
                    "schema_version": 1,
                    "path": path,
                    "digest": setup_core::digest::of_bytes(body.as_bytes()),
                    "byte_length": body.len(),
                    "mode": mode,
                    "owner": owner,
                })
            })
            .collect();
        let mut manifest = serde_json::json!({
            "schema_version": 1,
            "bundle_format": provider_v3::bundle::BUNDLE_FORMAT,
            "protocol_version": provider_v3::bundle::BUNDLE_PROTOCOL_VERSION,
            "harness_id": harness.harness_id,
            "builder_version": "0.1.0",
            "input_digest": "sha256:".to_owned() + &"3".repeat(64),
            "projection_profile": {
                "profile_id": profile.profile_id,
                "profile_digest": profile.digest,
                "target_scope": scope.map_or("global", provider_v3::TargetScope::as_str),
            },
            "component_adaptations": [{
                "stable_id": owner,
                "version": "1.0",
                "passport_digest": "sha256:".to_owned() + &"1".repeat(64),
                "adaptation_id": "adaptation_".to_owned() + &"2".repeat(64),
                "projection_artifact": {
                    "digest": "sha256:".to_owned() + &"3".repeat(64),
                    "size_bytes": 128,
                },
                "provider_component_kind": kind.unwrap_or("instruction"),
                "projection_kind": "native_files",
                "member_paths": member_paths,
            }],
            "managed_paths": files.iter().map(|(path, _, _)| *path).collect::<Vec<_>>(),
            "files": records,
            "limits": {
                "max_files": 2000,
                "max_file_bytes": 4 * 1024 * 1024,
                "max_bundle_bytes": 64 * 1024 * 1024,
            },
        });
        if let Some(scope) = scope {
            manifest["target_scope"] = serde_json::json!(scope.as_str());
        }
        if let Some(kind) = kind {
            manifest["conversion_report"] = serde_json::json!({
                "complete": true,
                "entries": [{
                    "stable_id": owner,
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
                    "harness_id": harness.harness_id,
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
            provider_v3::bundle::BUNDLE_FORMAT.to_owned(),
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
    fn validate_bundle_accepts_a_consistent_bundle_and_echoes_what_it_was_given() {
        // `validate-bundle` is a declared command of the v3 contract and the
        // only caller of its implementation was the dispatch: `Bundle::read` is
        // tested thoroughly one crate down, and the envelope this command hands
        // a consumer was tested nowhere. The envelope is the part a consumer
        // parses.
        let target = seeded("validate-ok");
        let (bytes, bundle_digest, artifact) =
            bundle_bytes(&[("AGENTS.md", "# from a bundle\n", 0o644)]);
        let path = target.join("..").join("valid.zip");
        fs::write(&path, &bytes).unwrap();
        let flags = bundle_flags(&path, &bundle_digest, &artifact, bytes.len());
        let borrowed: Vec<&str> = flags.iter().map(String::as_str).collect();

        let answer = run(args("validate-bundle", &target, &borrowed));
        assert_eq!(answer["valid"], true, "{answer}");
        // The echoes matter as much as the verdict: without them a consumer
        // cannot tell whether the answer concerns the bytes it sent.
        assert_eq!(answer["bundle_digest"], bundle_digest.as_str());
        assert_eq!(answer["artifact_digest"], artifact.as_str());
    }

    #[test]
    fn validate_bundle_refuses_with_a_reason_and_still_echoes_the_claim() {
        let target = seeded("validate-bad");
        let (bytes, bundle_digest, _) = bundle_bytes(&[("AGENTS.md", "# from a bundle\n", 0o644)]);
        let path = target.join("..").join("wrong.zip");
        fs::write(&path, &bytes).unwrap();
        let lie = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let flags = bundle_flags(&path, &bundle_digest, lie, bytes.len());
        let borrowed: Vec<&str> = flags.iter().map(String::as_str).collect();

        let answer = run(args("validate-bundle", &target, &borrowed));
        // A refusal carries `rejected: true` and no `valid` key at all, which
        // is the shape every refusal on this wire uses -- and the shape the
        // consumer reads: `answer.get("valid") is not True` for an acceptance,
        // `answer.get("rejected") is not True` for a refusal. Asserting
        // `valid: false` here failed, which is how the asymmetry got checked
        // against their reader rather than assumed.
        assert!(answer.get("valid").is_none(), "{answer}");
        assert_eq!(answer["rejected"], true, "{answer}");
        assert_eq!(answer["reason"], "digest_mismatch", "{answer}");
        assert_eq!(
            answer["artifact_digest"], lie,
            "a refusal echoes the claim it was given, not the one it wished for"
        );
        assert!(
            answer["detail"].as_str().is_some_and(|d| !d.is_empty()),
            "a refusal carrying only a code says a bundle was wrong without \
             saying which part: {answer}"
        );
    }

    #[test]
    fn a_bundle_on_an_operation_that_reads_none_is_refused_not_echoed() {
        // Measured on the released 0.0.50 before this refusal existed, while
        // the consumer designed remove's `end_state` extension (their
        // ADR-0129): a remove plan carrying all five bundle names answered
        // `planned, valid: true` with the digests echoed, and the apply then
        // removed everything with the bundle bytes untouched -- accept and
        // ignore, exit 0 twice, even for a 20-byte dummy ZIP. A plan that
        // echoes inputs apply will never read lies about what approving it
        // means, and the consumer's rollout story for `end_state` assumed a
        // loud refusal that did not exist. This is that refusal. When remove
        // learns to read a bundle (kit 0.2.8+), it narrows rather than lifts:
        // the operations that read none keep refusing.
        // 0.0.54 taught remove to read one, so the operation that reads none
        // here is backup; the shape of the refusal is the same.
        let target = seeded("bundle-on-backup");
        let (bytes, bundle_digest, artifact) = bundle_bytes(&[("AGENTS.md", "x\n", 0o644)]);
        let artifact_path = target.join("..").join("bundle-on-backup.zip");
        fs::write(&artifact_path, &bytes).unwrap();

        let mut plan_args = vec![
            "--operation".to_owned(),
            "backup".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01NOBUNDLE".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        plan_args.extend(bundle_flags(
            &artifact_path,
            &bundle_digest,
            &artifact,
            bytes.len(),
        ));
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let error = refuse(args("plan-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
        assert!(
            error.detail().contains("reads no bundle"),
            "{}",
            error.detail()
        );
    }

    /// Plan a remove that keeps `files` at the bundle's bytes, and hand back
    /// the plan response with the apply arguments it authorizes.
    fn remove_keeping_plan(
        target: &Path,
        tag: &str,
        files: &[(&str, &str, u32)],
        scope: Option<&str>,
    ) -> (serde_json::Value, Vec<String>) {
        let target_scope = scope.and_then(provider_v3::TargetScope::parse);
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &TEST,
            target_scope,
            files,
            Some(if target_scope.is_some() {
                "skill"
            } else {
                "setting"
            }),
        );
        let artifact_path = target.join("..").join(format!("keep-{tag}.zip"));
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let mut plan_args = vec![
            "--operation".to_owned(),
            "remove".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            format!("operation_01KEEP{}", tag.to_uppercase()),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        if let Some(scope) = scope {
            plan_args.push("--target-scope".to_owned());
            plan_args.push(scope.to_owned());
        }
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        let plan_path = target.join("..").join(format!("keep-{tag}-plan.json"));
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
        (planned, apply_args)
    }

    /// The consumer's `ADR-0129` case, end to end: a contribution put one key
    /// into a file the person also writes, and removing the contribution must
    /// leave the file at the person's remaining bytes rather than delete it.
    /// The bytes arrive as an ordinary bundle on `remove`; the plan says, per
    /// path, what stays and what goes; the apply does exactly that.
    #[test]
    fn a_remove_may_carry_the_bytes_a_path_keeps_and_leaves_them_behind() {
        let target = seeded("remove-keeping");
        install_global(
            &target,
            "keep",
            &[
                ("AGENTS.md", "# ours\n", 0o644),
                ("settings.json", r#"{"model":"setup"}"#, 0o644),
            ],
        );
        let survivor = "{\"model\":\"mine, not the setup's\"}\n";
        let (planned, apply_args) = remove_keeping_plan(
            &target,
            "global",
            &[("settings.json", survivor, 0o644)],
            None,
        );

        let states = planned["plan"]["end_state"].as_array().unwrap();
        let of = |path: &str| {
            states
                .iter()
                .find(|entry| entry["path"] == path)
                .unwrap_or_else(|| panic!("no end state for {path}: {states:?}"))
        };
        assert_eq!(of("AGENTS.md")["end_state"], "removed");
        assert_eq!(of("settings.json")["end_state"], "final_bytes");
        assert_eq!(of("settings.json")["member"], "files/settings.json");
        assert_eq!(
            of("settings.json")["sha256"],
            setup_core::digest::of_bytes(survivor.as_bytes())
        );
        assert_eq!(of("settings.json")["byte_length"], survivor.len());
        assert_eq!(states.len(), 2, "{states:?}");
        assert!(
            planned["effects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|line| line.as_str().unwrap().contains("leave settings.json")),
            "{}",
            planned["effects"]
        );

        let borrowed: Vec<&str> = apply_args.iter().map(String::as_str).collect();
        let applied = run(args("apply-operation", &target, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
        assert_eq!(
            fs::read_to_string(target.join("settings.json")).unwrap(),
            survivor,
            "the surviving file is not at the bytes the bundle carried"
        );
        assert!(!target.join("AGENTS.md").exists());
        assert!(
            target.join("skills").join("a.md").exists(),
            "remove took an unrecorded skill file"
        );
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(
            fs::read_to_string(target.join(".credentials.json")).unwrap(),
            "SECRET"
        );
        // Not ours any more: the record names no file, and the bundle that
        // put the bytes there is named as provenance.
        assert!(recorded_written(&target).is_empty());
        let state: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(TEST.state_file)).unwrap()).unwrap();
        assert_eq!(state["bundle_digest"], planned["bundle_digest"]);
        assert!(state["setup_stable_id"].is_null(), "{state}");
        let after = run(args("status", &target, &[]));
        assert_eq!(after["state"], "managed", "{after}");
        assert_eq!(after["drift_state"], "clean", "{after}");
    }

    /// A remove planned without a bundle carries no `end_state` member at all,
    /// so every plan digest that verified before this build still verifies.
    #[test]
    fn a_remove_without_a_bundle_carries_no_end_state_member() {
        let target = seeded("remove-bare-plan");
        assert_eq!(plan_then_apply(&target, "backup", &[])["state"], "verified");
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01BARE",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert!(
            planned["plan"].get("end_state").is_none(),
            "{}",
            planned["plan"]
        );
    }

    /// The plan the consumer approved is the authorization. A bundle that the
    /// plan never described is refused at apply, and a plan that described one
    /// refuses to apply without it -- both before the lock, with no effect.
    #[test]
    fn a_remove_apply_takes_exactly_the_bundle_its_plan_described() {
        let target = seeded("remove-authorization");
        assert_eq!(plan_then_apply(&target, "backup", &[])["state"], "verified");
        let (bytes, bundle_digest, artifact) =
            bundle_bytes(&[("settings.json", "{\"kept\":true}\n", 0o644)]);
        let artifact_path = target.join("..").join("unplanned.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        // Planned bare, applied with a bundle: never authorized.
        let planned = run(args(
            "plan-operation",
            &target,
            &[
                "--operation",
                "remove",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_01UNPLANNED",
                "--expires-at",
                far_future(),
            ],
        ));
        let plan_path = target.join("..").join("bare-plan.json");
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
        let error = refuse(args("apply-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
        assert!(
            error.detail().contains("never authorized"),
            "{}",
            error.detail()
        );
        assert!(
            target.join("AGENTS.md").exists(),
            "a refusal made an effect"
        );

        // Planned with survivors, applied without the bundle: nothing to
        // leave them at.
        let (_, with_bundle) =
            remove_keeping_plan(&target, "unfed", &[("settings.json", "{}\n", 0o644)], None);
        let bare: Vec<&str> = with_bundle.iter().take(6).map(String::as_str).collect();
        let error = refuse(args("apply-operation", &target, &bare));
        assert_eq!(error.reason(), Some(WireReason::UnsupportedBundleFormat));
        assert!(
            target.join("AGENTS.md").exists(),
            "a refusal made an effect"
        );

        // Planned with one bundle, applied with another whose bytes differ:
        // the plan's end state names bytes this bundle does not carry.
        let (other_bytes, other_digest, other_artifact) =
            bundle_bytes(&[("settings.json", "{\"other\":1}\n", 0o644)]);
        let other_path = target.join("..").join("other.zip");
        fs::write(&other_path, &other_bytes).unwrap();
        let mut swapped: Vec<String> = with_bundle.iter().take(6).cloned().collect();
        swapped.extend(bundle_flags(
            &other_path,
            &other_digest,
            &other_artifact,
            other_bytes.len(),
        ));
        let borrowed: Vec<&str> = swapped.iter().map(String::as_str).collect();
        let error = refuse(args("apply-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        assert!(
            error.detail().contains("does not carry"),
            "{}",
            error.detail()
        );
        assert!(
            target.join("AGENTS.md").exists(),
            "a refusal made an effect"
        );
    }

    /// The consumer binds a plan to the `target_digest` it observed through
    /// `status` a moment before. Reported from their project-scope branch on
    /// 2026-09-02: at a workspace, install passed on an empty target and the
    /// remove that followed was refused for `expected_target_digest`. Both
    /// numbers must come from the same owned set, whatever `status` was told.
    #[test]
    fn status_and_a_scoped_plan_agree_on_the_identity_after_a_scoped_install() {
        let target = seeded("scoped-status-agrees");
        install_scoped(&target, "agree", "ours", "ours\n");
        let observed = run(args("status", &target, &[]));
        let planned = scoped_plan(&target, "remove", "operation_01AGREE");
        assert_eq!(
            planned["plan"]["expected_target_digest"], observed["target_digest"],
            "status {observed}\nplan {planned}"
        );
    }

    /// The same question at a *workspace*: a project-scoped harness shaped like
    /// cursor's, a target holding the person's own source tree, and a scoped
    /// install of one skill under `.cursor/`. This is the exact shape the
    /// consumer's project-scope branch measured on 2026-09-02 and found the
    /// remove plan refused for `expected_target_digest`.
    #[test]
    fn status_and_a_project_plan_agree_on_the_identity_at_a_workspace() {
        let harness = project_shaped();
        let workspace = scratch("project-status-agrees").join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src").join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(workspace.join("README.md"), "# theirs\n").unwrap();

        let before = run_for(&harness, args("status", &workspace, &[]));
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &harness,
            Some(provider_v3::TargetScope::Project),
            &[(".cursor/skills/probe/SKILL.md", "probe\n", 0o644)],
            Some("skill"),
        );
        let artifact_path = workspace.join("..").join("project.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01PROJECTIN".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "project".to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run_for(&harness, args("plan-operation", &workspace, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert_eq!(
            planned["plan"]["expected_target_digest"], before["target_digest"],
            "install: status {before}\nplan {planned}"
        );
        let plan_path = workspace.join("..").join("project-plan.json");
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
        let applied = run_for(&harness, args("apply-operation", &workspace, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
        assert!(workspace.join(".cursor/skills/probe/SKILL.md").exists());

        let after = run_for(&harness, args("status", &workspace, &[]));
        let removal = run_for(
            &harness,
            args(
                "plan-operation",
                &workspace,
                &[
                    "--operation",
                    "remove",
                    "--provider-release-digest",
                    RELEASE,
                    "--operation-id",
                    "operation_01PROJECTRM",
                    "--expires-at",
                    far_future(),
                    "--target-scope",
                    "project",
                ],
            ),
        );
        assert_eq!(removal["state"], "planned", "{removal}");
        assert_eq!(
            removal["plan"]["expected_target_digest"], after["target_digest"],
            "remove: status {after}\nplan {removal}"
        );
    }

    /// A workspace nobody has installed into, whose own tree happens to carry
    /// a top-level directory spelled like one of the global namespaces --
    /// here `skills/`, which is a repository's own business. No record to
    /// read a scope from, so an unasked `status` measures the global set and
    /// hashes those files, while the plan the consumer binds to it is made
    /// under `project`, where an unrecorded target is nothing of ours. Asked,
    /// the two agree.
    #[test]
    fn a_status_asked_about_a_scope_measures_that_scopes_inventory_before_any_record() {
        let harness = project_shaped();
        let workspace = scratch("project-status-asked").join("workspace");
        fs::create_dir_all(workspace.join("skills")).unwrap();
        fs::write(
            workspace.join("skills/theirs.md"),
            "# the repository's own\n",
        )
        .unwrap();
        fs::write(workspace.join("README.md"), "# theirs\n").unwrap();

        let unasked = run_for(&harness, args("status", &workspace, &[]));
        let asked = run_for(
            &harness,
            args("status", &workspace, &["--target-scope", "project"]),
        );
        assert_eq!(
            unasked["target_digest"], asked["target_digest"],
            "unmanaged global and project inventories are both empty receipts"
        );
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &harness,
            Some(provider_v3::TargetScope::Project),
            &[(".cursor/skills/probe/SKILL.md", "probe\n", 0o644)],
            Some("skill"),
        );
        let artifact_path = workspace.join("..").join("asked.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01ASKED".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "project".to_owned(),
        ];
        plan_args.extend(bundle_flags(
            &artifact_path,
            &bundle_digest,
            &artifact,
            bytes.len(),
        ));
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run_for(&harness, args("plan-operation", &workspace, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert_eq!(
            planned["plan"]["expected_target_digest"],
            asked["target_digest"]
        );

        let error = refuse_for(
            &harness,
            args("status", &workspace, &["--target-scope", "user_root"]),
        );
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
    }

    /// A target managed under `project` refuses a plan that names no scope,
    /// and says which one to name -- the consumer's remove plan carried none
    /// and met a digest mismatch instead of this sentence.
    #[test]
    fn a_plan_whose_scope_contradicts_the_record_is_refused_by_name() {
        let harness = project_shaped();
        let workspace = scratch("project-scope-contradiction").join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &harness,
            Some(provider_v3::TargetScope::Project),
            &[(".cursor/skills/probe/SKILL.md", "probe\n", 0o644)],
            Some("skill"),
        );
        let artifact_path = workspace.join("..").join("contra.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01CONTRAIN".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "project".to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run_for(&harness, args("plan-operation", &workspace, &borrowed));
        let plan_path = workspace.join("..").join("contra-plan.json");
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
            run_for(&harness, args("apply-operation", &workspace, &borrowed))["state"],
            "verified"
        );

        let error = refuse_for(
            &harness,
            args(
                "plan-operation",
                &workspace,
                &[
                    "--operation",
                    "remove",
                    "--provider-release-digest",
                    RELEASE,
                    "--operation-id",
                    "operation_01CONTRARM",
                    "--expires-at",
                    far_future(),
                ],
            ),
        );
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
        assert!(
            error
                .detail()
                .contains("managed under target_scope project")
                && error.detail().contains("names the global profile"),
            "{}",
            error.detail()
        );
        assert!(
            workspace.join(".cursor/skills/probe/SKILL.md").exists(),
            "a refusal made an effect"
        );
    }

    /// A kind declared only by a scoped profile -- codex's `skill` under
    /// `~/.agents` -- is one this provider implements, and `validate-bundle`
    /// has no scope to ask about: it must say yes. A scoped plan under that
    /// scope says yes; a global plan says no, by name, because the home does
    /// not route the kind. Found by the consumer's `user_root` slice: codex had
    /// never passed `validate-bundle` with a skill.
    #[test]
    fn a_kind_declared_only_by_a_scope_validates_and_plans_under_that_scope() {
        let mut harness = TEST;
        harness.component_kinds = &[
            provider_v3::ComponentKind::Instruction,
            provider_v3::ComponentKind::Setting,
        ];
        let target = seeded("scoped-only-kind");
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &harness,
            Some(provider_v3::TargetScope::UserRoot),
            &[("shared/probe/SKILL.md", "probe\n", 0o644)],
            Some("skill"),
        );
        let artifact_path = target.join("..").join("scoped-kind.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());
        let borrowed: Vec<&str> = flags.iter().map(String::as_str).collect();

        let validated = run_for(&harness, args("validate-bundle", &target, &borrowed));
        assert_eq!(validated["valid"], true, "{validated}");

        let mut scoped = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01SCOPEDKIND".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "user_root".to_owned(),
        ];
        scoped.extend(flags.clone());
        let borrowed: Vec<&str> = scoped.iter().map(String::as_str).collect();
        let planned = run_for(&harness, args("plan-operation", &target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");

        // The same kind on a path the home owns: the surface passes and the
        // kind is the refusal, named with the profile that lacks it.
        let (bytes, bundle_digest, artifact) =
            bundle_bytes_declaring(&[("skills/probe.md", "probe\n", 0o644)], Some("skill"));
        let home_path = target.join("..").join("global-kind.zip");
        fs::write(&home_path, &bytes).unwrap();
        let mut global = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01GLOBALKIND".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
        ];
        global.extend(bundle_flags(
            &home_path,
            &bundle_digest,
            &artifact,
            bytes.len(),
        ));
        let borrowed: Vec<&str> = global.iter().map(String::as_str).collect();
        let error = refuse_for(&harness, args("plan-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::ProjectionProfileMismatch));
        assert!(
            error
                .detail()
                .contains("different provider projection profile"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn a_removal_without_a_record_leaves_declared_entries_alone() {
        let empty = scratch("remove-nothing-here").join("target");
        fs::write(empty.join("unrelated.txt"), "theirs").unwrap();
        let done = plan_then_apply(&empty, "remove", &[]);
        assert_eq!(done["state"], "verified", "{done}");
        assert_eq!(
            fs::read_to_string(empty.join("unrelated.txt")).unwrap(),
            "theirs"
        );

        let populated = seeded("remove-unrecorded");
        let done = plan_then_apply(&populated, "remove", &[]);
        assert_eq!(done["state"], "verified", "{done}");
        for kept in ["AGENTS.md", "settings.json", "unrelated.txt"] {
            assert!(
                populated.join(kept).exists(),
                "unrecorded remove took {kept}"
            );
        }
    }

    /// A person's own files are not this provider's to withdraw from a target
    /// it never wrote to. Measured on the released 0.0.57 human surface: a
    /// target holding only a person's `AGENTS.md`, `settings.json` and
    /// `skills/` answered *"Removed everything <provider> owns"* and took all
    /// three, recoverable from the slot and under a sentence that did not
    /// describe what happened. The scoped branch of `remove_managed` already
    /// refuses on the same ground -- this build does not know what it wrote --
    /// and this is that refusal where a person types it.
    #[test]
    fn a_human_removal_with_no_record_of_its_own_is_a_noop() {
        let target = seeded("human-remove-unmanaged");
        assert!(target.join(TEST.state_file).symlink_metadata().is_err());

        crate::human::run(
            &TEST,
            crate::human::Command::Remove {
                target: target.clone(),
            },
        )
        .unwrap();
        for kept in ["AGENTS.md", "settings.json", "unrelated.txt"] {
            assert!(target.join(kept).exists(), "unrecorded remove took {kept}");
        }
        assert!(
            target.join("skills").is_dir(),
            "unrecorded remove took skills/"
        );
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );
    }

    /// A harness shaped like cursor's project scope, for the workspace cases.
    fn project_shaped() -> Harness {
        let mut harness = TEST;
        harness.scoped_projections = &[crate::facts::Scoped {
            target_scope: provider_v3::TargetScope::Project,
            profile_id: "test/native-files/project/1",
            component_kinds: &[
                provider_v3::ComponentKind::Skill,
                provider_v3::ComponentKind::Instruction,
            ],
            projection_kinds: &[provider_v3::ProjectionKind::NativeFiles],
            native_namespaces: &[".cursor/skills", ".cursor/rules"],
        }];
        harness
    }

    /// Under a shared root the record is the inventory. A file this build
    /// leaves behind at the person's bytes must leave the record too, or the
    /// next removal would take it -- which is the file the whole extension
    /// exists to keep.
    #[test]
    fn under_a_scope_a_file_left_behind_is_not_this_builds_to_take_next_time() {
        let target = seeded("remove-keeping-scoped");
        install_scoped(&target, "keep", "ours", "the setup's bytes\n");
        assert_eq!(recorded_written(&target), vec!["shared/ours/SKILL.md"]);

        let theirs = "the person's remaining bytes\n";
        let (planned, apply_args) = remove_keeping_plan(
            &target,
            "scoped",
            &[("shared/ours/SKILL.md", theirs, 0o644)],
            Some("user_root"),
        );
        let states = planned["plan"]["end_state"].as_array().unwrap();
        assert_eq!(states.len(), 1, "{states:?}");
        assert_eq!(states[0]["end_state"], "final_bytes");
        let borrowed: Vec<&str> = apply_args.iter().map(String::as_str).collect();
        let applied = run(args("apply-operation", &target, &borrowed));
        assert_eq!(applied["state"], "verified", "{applied}");
        assert_eq!(
            fs::read_to_string(target.join("shared").join("ours").join("SKILL.md")).unwrap(),
            theirs
        );
        assert!(recorded_written(&target).is_empty());

        // The next scoped removal reads the record, finds nothing of ours,
        // and leaves the person's file where it is.
        let again = scoped_plan(&target, "remove", "operation_01AGAIN");
        let done = scoped_apply(&target, &again, "again");
        assert_eq!(done["state"], "verified", "{done}");
        assert!(
            target.join("shared").join("ours").join("SKILL.md").exists(),
            "the second removal took the file the first one left to the person"
        );
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
        assert!(
            target.join("settings.json").exists(),
            "install took an unrecorded host settings file"
        );
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "keep me"
        );
        assert_eq!(
            fs::read_to_string(target.join(".credentials.json")).unwrap(),
            "SECRET"
        );
    }

    /// A bundle routed to a scope installs into that scope's namespace.
    ///
    /// **The defect, and it made the scope unusable end to end:** ownership was
    /// asked of the *global* namespaces whatever scope an operation named. A
    /// consumer routing a skill to codex under `user_root` writes
    /// `skills/<name>` — and codex declines `skills` under its own home, so
    /// the install was refused as writing outside the surface. A scope this
    /// provider declares, publishes a profile for, and could not be installed
    /// into.
    ///
    /// The seeded global files are asserted untouched afterwards, which is the
    /// other half: the clear before the fill used to take the global namespaces
    /// whatever scope it was under.
    #[test]
    fn a_bundle_routed_to_a_scope_installs_into_that_scopes_namespace() {
        let target = seeded("bundle-scoped");
        let (bytes, bundle_digest, artifact) = bundle_bytes_for(
            &TEST,
            Some(provider_v3::TargetScope::UserRoot),
            &[("shared/review/SKILL.md", "# review\n", 0o644)],
            Some("skill"),
        );
        let artifact_path = target.join("..").join("scoped-bundle.zip");
        fs::write(&artifact_path, &bytes).unwrap();
        let flags = bundle_flags(&artifact_path, &bundle_digest, &artifact, bytes.len());

        let mut plan_args = vec![
            "--operation".to_owned(),
            "install".to_owned(),
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--operation-id".to_owned(),
            "operation_01SCOPEDBUNDLE".to_owned(),
            "--expires-at".to_owned(),
            far_future().to_owned(),
            "--target-scope".to_owned(),
            "user_root".to_owned(),
        ];
        plan_args.extend(flags.clone());
        let borrowed: Vec<&str> = plan_args.iter().map(String::as_str).collect();
        let planned = run(args("plan-operation", &target, &borrowed));
        assert_eq!(planned["state"], "planned", "{planned}");

        let plan_path = target.join("..").join("scoped-bundle-plan.json");
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
        assert_eq!(
            fs::read_to_string(target.join("shared").join("review").join("SKILL.md")).unwrap(),
            "# review\n"
        );

        // The global namespaces belong to the other target and this operation
        // did not name it.
        assert!(
            target.join("AGENTS.md").exists(),
            "an install under a scope cleared the global target's files"
        );
        assert!(target.join("settings.json").exists());

        // And the state records the namespaces owned *here*.
        let recorded: serde_json::Value =
            serde_json::from_slice(&fs::read(target.join(TEST.state_file)).unwrap()).unwrap();
        assert_eq!(
            recorded["native_ownership"],
            serde_json::json!(["shared"]),
            "the state described the global namespaces at a scoped target"
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

    /// Apply a plan that was already produced, so a test can inspect it first.
    ///
    /// Split out of [`plan_then_install`] rather than duplicated: an update is
    /// interesting because of what its *plan* says, and a helper that plans and
    /// applies in one breath gives a test no place to look.
    fn apply_planned(
        target: &Path,
        prefix: &str,
        operation: &str,
        planned: &serde_json::Value,
        artifact: Option<&Path>,
    ) -> serde_json::Value {
        let (path, digest, held) = write_plan(target, operation, planned, artifact);
        let mut extra = vec![
            "--plan",
            path.as_str(),
            "--plan-digest",
            digest.as_str(),
            "--provider-release-digest",
            RELEASE,
            "--prefix",
            prefix,
        ];
        if let Some(file) = held.as_ref() {
            extra.push("--software-artifact");
            extra.push(file.as_str());
        }
        run(args("apply-operation", target, &extra))
    }

    fn write_plan(
        target: &Path,
        operation: &str,
        planned: &serde_json::Value,
        artifact: Option<&Path>,
    ) -> (String, String, Option<String>) {
        let plan_path = target.join("..").join(format!("plan-{operation}.json"));
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();
        (
            plan_path.to_string_lossy().into_owned(),
            planned["plan_digest"].as_str().unwrap().to_owned(),
            artifact.map(|file| file.to_string_lossy().into_owned()),
        )
    }

    fn apply_args<'a>(
        target: &'a Path,
        prefix: &'a str,
        operation: &'a str,
        planned: &'a serde_json::Value,
        artifact: Option<&'a Path>,
    ) -> Vec<String> {
        let (path, digest, held) = write_plan(target, operation, planned, artifact);
        let mut extra = vec![
            "--plan".to_owned(),
            path,
            "--plan-digest".to_owned(),
            digest,
            "--provider-release-digest".to_owned(),
            RELEASE.to_owned(),
            "--prefix".to_owned(),
            prefix.to_owned(),
        ];
        if let Some(file) = held {
            extra.push("--software-artifact".to_owned());
            extra.push(file);
        }
        extra
    }

    fn refuse_apply(
        target: &Path,
        prefix: &str,
        operation: &str,
        artifact: Option<&Path>,
    ) -> provider_v3::Error {
        let planned = run(args(
            "plan-operation",
            target,
            &software_plan_args(operation, prefix),
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        let extra = apply_args(target, prefix, operation, &planned, artifact);
        let borrowed: Vec<&str> = extra.iter().map(String::as_str).collect();
        refuse(args("apply-operation", target, &borrowed))
    }

    /// [`plan_then_install`], told which version to plan for.
    fn plan_then_install_at(
        target: &Path,
        operation: &str,
        artifact: Option<&Path>,
        version: Option<&str>,
    ) -> serde_json::Value {
        let prefix = ready_prefix(target);
        let mut arguments = software_plan_args(operation, &prefix);
        if let Some(wanted) = version {
            arguments.extend_from_slice(&["--software-version", wanted]);
        }
        let planned = run(args("plan-operation", target, &arguments));
        assert_eq!(planned["state"], "planned", "plan refused: {planned}");
        apply_planned(target, &prefix, operation, &planned, artifact)
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
        assert_eq!(planned["plan"]["software_version"], "1.2.3");
        assert_eq!(
            planned["plan"]["software_prefix"],
            ready_prefix(&target),
            "the plan must bind the prefix apply will be given"
        );

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

    /// The version pinned before this one is nameable, and names its own bytes.
    ///
    /// This is what makes `software_update` and `rollback` operations that can
    /// be *run* rather than only declared: an update needs a version to move
    /// from, and a rollback a tree to return to. Before the second pin existed
    /// this repository recorded both as measured absences, honestly, for as
    /// long as it did.
    #[test]
    fn the_version_pinned_before_this_one_can_be_planned_and_names_its_own_bytes() {
        let target = seeded("software-previous");
        let prefix = ready_prefix(&target);

        let mut earlier = software_plan_args("software_install", &prefix);
        earlier.extend_from_slice(&["--software-version", "1.2.2"]);
        let planned = run(args("plan-operation", &target, &earlier));
        assert_eq!(planned["state"], "planned");

        let artifacts = planned["plan"]["software_artifacts"].as_array().unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(
            artifacts[0]["url"], "https://example.invalid/test-harness-1.2.2",
            "the earlier version was named and the current bytes were planned"
        );
        assert_eq!(
            artifacts[0]["sha256"],
            "sha256:42c3e0650b099f95955b0ff86c75499848e1343a6c40af6a7acd10f3c18ce226"
        );

        // And a version that is neither is still refused, naming both.
        let mut neither = software_plan_args("software_install", &prefix);
        neither.extend_from_slice(&["--software-version", "9.9.9"]);
        let error = refuse(args("plan-operation", &target, &neither));
        assert!(error.detail().contains("1.2.3"), "{}", error.detail());
        assert!(error.detail().contains("1.2.2"), "{}", error.detail());
    }

    /// An update crosses two real trees, and the exposed command follows.
    ///
    /// The whole transition, in one test, because the interesting failure is
    /// between the steps: the earlier tree must survive the update (a rollback
    /// has nowhere to go otherwise) and the exposed command must actually move.
    #[test]
    fn an_update_moves_the_command_between_two_versions_that_both_stay_on_disk() {
        let target = seeded("software-update-across");
        let prefix = ready_prefix(&target);
        let root = Path::new(&prefix).to_path_buf();

        // Install the earlier release first: `software_update` refuses an empty
        // prefix, and rightly -- an update of nothing is a different operation.
        let earlier_file = downloaded(&target, TEST_EARLIER_PAYLOAD);
        let installed = plan_then_install_at(
            &target,
            "software_install",
            Some(&earlier_file),
            Some("1.2.2"),
        );
        assert_eq!(installed["state"], "verified", "{installed}");
        assert_eq!(installed["version"], "1.2.2");

        // Now update to the pinned one. The plan says what it is replacing,
        // which is the difference between an update and an install.
        let mut arguments = software_plan_args("software_update", &prefix);
        arguments.extend_from_slice(&["--software-version", "1.2.3"]);
        let planned = run(args("plan-operation", &target, &arguments));
        assert_eq!(planned["state"], "planned", "{planned}");
        let effects = serde_json::to_string(&planned["plan"]["effects"]).unwrap();
        assert!(
            effects.contains("1.2.2") && effects.contains("1.2.3"),
            "an update should name both ends of the move: {effects}"
        );

        let current_file = downloaded(&target, TEST_PAYLOAD);
        let updated = apply_planned(
            &target,
            &prefix,
            "software_update",
            &planned,
            Some(&current_file),
        );
        assert_eq!(updated["state"], "verified", "{updated}");
        assert_eq!(updated["version"], "1.2.3");

        // Both trees are there. Losing the earlier one would leave `rollback`
        // declared and unrunnable again, which is the absence this closes.
        assert!(
            root.join("1.2.2").is_dir(),
            "the earlier tree was discarded"
        );
        assert!(root.join("1.2.3").is_dir(), "the new tree is missing");

        let exposed = root.join("bin").join("test-harness");
        assert_eq!(
            fs::read(&exposed).unwrap(),
            TEST_PAYLOAD,
            "the exposed command still runs the version the update moved away from"
        );
    }

    /// A valid artifact for another pin is still not the artifact this plan named.
    ///
    /// The previous behaviour resolved the release from the downloaded bytes
    /// against either compiled pin, so a caller could plan 1.2.3, hand 1.2.2,
    /// and have 1.2.2 installed. The plan is the authorisation; another pin's
    /// bytes are a different effect.
    #[test]
    fn apply_refuses_another_valid_pin_in_place_of_the_planned_artifact() {
        let target = seeded("software-bytes-decide");
        let prefix = ready_prefix(&target);

        let earlier_file = downloaded(&target, TEST_EARLIER_PAYLOAD);
        let error = refuse_apply(&target, &prefix, "software_install", Some(&earlier_file));
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        assert!(
            error.detail().contains("1.2.3"),
            "the refusal should name the planned version: {}",
            error.detail()
        );
        assert!(!Path::new(&prefix).join("1.2.2").exists());
        assert!(!Path::new(&prefix).join("1.2.3").exists());
    }

    /// Planning removal of the previous version must not take the current pin.
    #[test]
    fn removing_the_previous_version_leaves_the_current_pin() {
        let target = seeded("software-remove-previous");
        let prefix = ready_prefix(&target);
        let root = Path::new(&prefix).to_path_buf();

        let earlier_file = downloaded(&target, TEST_EARLIER_PAYLOAD);
        let installed = plan_then_install_at(
            &target,
            "software_install",
            Some(&earlier_file),
            Some("1.2.2"),
        );
        assert_eq!(installed["state"], "verified", "{installed}");

        let current_file = downloaded(&target, TEST_PAYLOAD);
        let updated = plan_then_install_at(
            &target,
            "software_update",
            Some(&current_file),
            Some("1.2.3"),
        );
        assert_eq!(updated["state"], "verified", "{updated}");
        assert!(root.join("1.2.2").is_dir());
        assert!(root.join("1.2.3").is_dir());

        let removed = plan_then_install_at(&target, "software_remove", None, Some("1.2.2"));
        assert_eq!(removed["state"], "verified", "{removed}");
        assert_eq!(removed["version"], "1.2.2");
        assert!(!root.join("1.2.2").exists(), "the planned version stayed");
        assert!(
            root.join("1.2.3").is_dir(),
            "removing 1.2.2 took the current pin"
        );
        assert_eq!(
            fs::read(root.join("bin").join("test-harness")).unwrap(),
            TEST_PAYLOAD,
            "the exposed command was taken with the unplanned version"
        );
    }

    /// A plan bound to prefix A must not mutate prefix B.
    #[test]
    fn apply_at_a_different_prefix_does_not_modify_that_prefix() {
        let target = seeded("software-prefix-bind");
        let planned_prefix = ready_prefix(&target);
        let other = target.join("..").join("other-program");
        fs::create_dir_all(&other).unwrap();
        let other_prefix = fs::canonicalize(&other)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let file = downloaded(&target, TEST_PAYLOAD);
        let planned = run(args(
            "plan-operation",
            &target,
            &software_plan_args("software_install", &planned_prefix),
        ));
        assert_eq!(planned["state"], "planned", "{planned}");
        assert_eq!(planned["plan"]["software_prefix"], planned_prefix);
        assert_eq!(planned["plan"]["software_version"], "1.2.3");

        let extra = apply_args(
            &target,
            &other_prefix,
            "software_install",
            &planned,
            Some(file.as_path()),
        );
        let borrowed: Vec<&str> = extra.iter().map(String::as_str).collect();
        let error = refuse(args("apply-operation", &target, &borrowed));
        assert_eq!(error.reason(), Some(WireReason::Stale));
        assert!(
            error.detail().contains(&planned_prefix),
            "{}",
            error.detail()
        );
        assert!(!Path::new(&other_prefix).join("bin").exists());
        assert!(!Path::new(&planned_prefix).join("bin").exists());
    }

    /// Bytes belonging to no release this build names are refused as such.
    ///
    /// `digest_mismatch` rather than a vaguer reason, and the detail names
    /// every version this build does publish -- a refusal that says only "not
    /// this one" makes the caller guess what would have worked.
    #[test]
    fn an_artifact_from_no_release_this_build_names_is_refused_by_its_digest() {
        let target = seeded("software-stranger");
        let prefix = ready_prefix(&target);
        let planned = run(args(
            "plan-operation",
            &target,
            &software_plan_args("software_install", &prefix),
        ));
        let plan_path = target.join("..").join("plan-stranger.json");
        fs::write(
            &plan_path,
            setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
        )
        .unwrap();
        let digest = planned["plan_digest"].as_str().unwrap().to_owned();
        let path = plan_path.to_string_lossy().into_owned();
        let stranger = downloaded(&target, b"neither release, and not close\n");
        let held = stranger.to_string_lossy().into_owned();

        let error = refuse(args(
            "apply-operation",
            &target,
            &[
                "--plan",
                &path,
                "--plan-digest",
                &digest,
                "--provider-release-digest",
                RELEASE,
                "--prefix",
                &prefix,
                "--software-artifact",
                &held,
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        let detail = error.detail();
        assert!(
            detail.contains("1.2.3") && detail.contains("sha256:"),
            "the refusal should name the planned version and digest: {detail}"
        );
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
    fn every_software_apply_echoes_the_plan_digest_it_was_handed() {
        // **Red before 2026-08-31, and no test here could have been.**
        //
        // The consumer requires the plan echo on *every* apply, and neither of
        // `software::apply`'s two answer shapes carried it. So `harness
        // install`, `harness update` and `harness remove` through `ai-stp`
        // refused after the program had already been installed: the effect
        // landed, and the operation stayed `applied_unverified` over a prefix
        // holding a working build. All seven released providers, and it reached
        // this repository as a measurement from the consumer's own session
        // rather than from anything here.
        //
        // Why nothing caught it: every test around this one asks whether the
        // provider does what its own answer *says*, and it did. The contract
        // owns the list of what an answer must carry -- "the same journal,
        // backup and plan-digest" -- and that sentence was read as being about
        // `plan-operation`. So this asserts the echo against the digest the wire
        // was handed, which is the only value that can disagree, and it does it
        // for all three operations rather than the one that was reported.
        for operation in ["software_install", "software_update", "software_remove"] {
            let target = seeded(&format!("software-echo-{operation}"));
            let file = downloaded(&target, TEST_PAYLOAD);
            if operation != "software_install" {
                plan_then_install(&target, "software_install", Some(&file));
            }
            let prefix = ready_prefix(&target);
            let planned = software_plan(&target, operation);
            assert_eq!(planned["state"], "planned", "plan refused: {planned}");
            let digest = planned["plan_digest"].as_str().unwrap().to_owned();
            let artifact = if operation == "software_remove" {
                None
            } else {
                Some(file.as_path())
            };
            let applied = apply_planned(&target, &prefix, operation, &planned, artifact);
            assert_eq!(applied["state"], "verified", "apply refused: {applied}");
            assert_eq!(
                applied["plan_digest"],
                serde_json::Value::String(digest),
                "{operation} answered without the plan echo the wire owes: {applied}"
            );
        }
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
    /// `status` says which slots retention may not take, and whose they are.
    ///
    /// The pool has known this since held slots shipped; `status` did not
    /// publish it, so a consumer planning a long series could only learn a
    /// baseline was unprotected by watching it evicted -- which is the failure
    /// a hold exists to prevent, discovered the same way.
    #[test]
    fn a_status_says_which_backups_are_held_and_why() {
        let target = seeded("status-publishes-holds");
        plan_then_apply(&target, "backup", &[]);
        plan_then_apply(&target, "backup", &[]);

        let control = target.join(TEST.control_directory);
        let pool = setup_core::backup::Pool::open(&control, facts::BACKUP_SLOTS).unwrap();
        let first = pool.list().unwrap().last().unwrap().backup_ref.clone();
        assert!(pool.hold(&first, "the evidence series baseline").unwrap());

        let listed = run(args("status", &target, &[]));
        let backups = listed["backups"].as_array().unwrap();
        assert!(backups.len() >= 2, "{backups:?}");

        let held: Vec<&serde_json::Value> = backups
            .iter()
            .filter(|entry| entry["held"] == serde_json::json!(true))
            .collect();
        assert_eq!(held.len(), 1, "exactly one slot was held: {backups:?}");
        assert_eq!(held[0]["backup_ref"], serde_json::json!(first.as_str()));
        assert_eq!(
            held[0]["hold_reason"],
            serde_json::json!("the evidence series baseline"),
            "the reason travels with the fact, because a caller deciding \
             whether to release one needs to know whose baseline it is"
        );

        for entry in backups {
            if entry["backup_ref"] != serde_json::json!(first.as_str()) {
                assert_eq!(entry["held"], serde_json::json!(false), "{entry:?}");
                assert_eq!(entry["hold_reason"], serde_json::Value::Null, "{entry:?}");
            }
        }
    }
}
