//! Explicit configuration install, replacement and removal through the forward kernel.

mod fields;

use std::collections::{BTreeMap, BTreeSet};

use setup_core::forward::{Admission, Change, Content, Observation, Progress, Writer};

use super::{
    Applied, ArgvBundle, Bundle, Error, Harness, Operation, Path, PlanRequest, Result, StatePlan,
    Surface, Target, WireReason, as_paths, build_state, json_object_file, operation_of, owned_here,
    record_bundle_provenance, refuse_a_neighbours_home, refuse_another_scopes_record, scope_of,
    string_field, validate_expiry, verified_bundle,
};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Context {
    written_fields: BTreeMap<String, Vec<String>>,
    ledger_before: Observation,
    state_before: Observation,
    applied: Applied,
}

struct Assembly {
    changes: Vec<Change>,
    context: Context,
}

fn refused() -> Error {
    Error::refuse(
        WireReason::Stale,
        "forward configuration requires the original bound plan and unchanged owned state",
    )
}

/// Compatibility writers understand JSON ledgers only. They must not withdraw
/// a shared TOML/JSONC host as though it were a whole owned file.
pub(super) fn require_legacy_target(harness: &Harness, target: &Target) -> Result<()> {
    let (_, content) = setup_core::forward::observe(
        target,
        &format!("{}/written-fields.json", harness.control_directory),
    )?;
    let ledger: BTreeMap<String, Vec<String>> = content
        .map(|content| serde_json::from_slice(&content.bytes).map_err(|_| refused()))
        .transpose()?
        .unwrap_or_default();
    if ledger.keys().any(|path| !json_object_file(path)) {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            "this target has structured contributions that require forward recovery",
        ));
    }
    Ok(())
}

pub(super) fn validate_request(request: &PlanRequest) -> Result<()> {
    if let Some(mode) = &request.recovery_mode
        && (mode != "forward"
            || !match request.operation {
                Operation::Install | Operation::Replace => request.bundle.is_some(),
                Operation::Remove => request.bundle.is_none(),
                _ => false,
            }
            || request.backup_ref.is_some()
            || request.capture_mode.is_some()
            || request.permission_profile.is_some()
            || request.prefix.is_some()
            || request.software_version.is_some()
            || request.instruction_section.is_some())
    {
        return Err(refused());
    }
    Ok(())
}

fn validate_binding(
    harness: &Harness,
    target: &Target,
    plan: &serde_json::Value,
    release: &str,
    named: Option<&ArgvBundle>,
) -> Result<()> {
    let scope = match plan.get("target_scope") {
        None => None,
        Some(value) => Some(
            provider_v3::TargetScope::parse(value.as_str().ok_or_else(refused)?)
                .ok_or_else(refused)?,
        ),
    };
    if scope.is_some() && harness.scoped_for(scope).is_none() {
        return Err(refused());
    }
    let expected = serde_json::json!({
        "format": provider_v3::PLAN_FORMAT, "protocol_version": provider_v3::PROTOCOL_VERSION,
        "provider_id": harness.provider_id, "provider_version": harness.version,
        "provider_build_digest": harness.build_digest()?, "provider_release_digest": release,
        "canonical_target": target.root().to_string_lossy(),
        "projection_profile_digest": harness.projection_profile_for(scope)?.digest,
        "platform": provider_v3::platform::echo(), "bundle": named.map(|bundle| &bundle.binding),
        "backup_ref": null, "restore_target_digest": null, "permission_profile": null,
        "recovery_mode": "forward",
    });
    let expected = expected.as_object().ok_or_else(refused)?;
    let fields = plan.as_object().ok_or_else(refused)?;
    if expected
        .iter()
        .any(|(key, value)| fields.get(key) != Some(value))
        || fields.keys().any(|key| {
            !expected.contains_key(key)
                && !matches!(
                    key.as_str(),
                    "operation"
                        | "operation_id"
                        | "expected_target_digest"
                        | "expires_at"
                        | "effects"
                        | "target_scope"
                )
        })
        || !match operation_of(plan)? {
            Operation::Install | Operation::Replace => named.is_some(),
            Operation::Remove => named.is_none(),
            _ => false,
        }
        || plan
            .get("effects")
            .and_then(serde_json::Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn apply(
    harness: &Harness,
    path: &Path,
    plan: &serde_json::Value,
    plan_digest: &str,
    release: &str,
    bundle: Option<&ArgvBundle>,
) -> Result<serde_json::Value> {
    let target = Target::resolve(path, harness.control_directory)?;
    validate_binding(harness, &target, plan, release, bundle)?;
    let scope = scope_of(plan);
    let admission = Admission {
        operation_id: string_field(plan, "operation_id")?,
        plan_digest: plan_digest.into(),
        operation: operation_of(plan)?.as_str().into(),
        target_precondition_digest: string_field(plan, "expected_target_digest")?,
        target_scope: scope.map(|value| value.as_str().into()),
    };
    let mut writer = Writer::acquire(&target, harness.control_directory)?;
    if let Progress::Completed(outcome) = writer.progress(&admission)? {
        return Ok(outcome);
    }
    if matches!(writer.progress(&admission)?, Progress::Absent) {
        validate_expiry(plan)?;
        let ready = bundle
            .map(|named| verified_bundle(harness, named, Surface::At(scope)))
            .transpose()?;
        refuse_another_scopes_record(harness, &target, scope)?;
        refuse_a_neighbours_home(harness, &target, scope)?;
        require_ownership(harness, &target)?;
        let owned = owned_here(harness, &target, scope)?;
        if target.identity_of_owned(&as_paths(&owned), &harness.not_our_identity())?
            != admission.target_precondition_digest
        {
            return Err(refused());
        }
        let assembly = assemble(
            harness,
            &|path| writer.read(path),
            bundle.zip(ready.as_ref()),
            &owned,
            scope,
        )?;
        let context = serde_json::to_value(&assembly.context).map_err(|_| refused())?;
        writer.admit(&admission, &context, &assembly.changes)?;
    }
    if let Progress::Applying(context) = writer.progress(&admission)? {
        let context: Context = serde_json::from_value(context).map_err(|_| refused())?;
        if writer.read(harness.state_file)?.0 != context.state_before
            || writer
                .read(&format!(
                    "{}/written-fields.json",
                    harness.control_directory
                ))?
                .0
                != context.ledger_before
        {
            return Err(refused());
        }
    }
    writer.apply(&admission)?;
    if let Progress::Applying(context) = writer.progress(&admission)? {
        finalize(harness, &target, &mut writer, &admission, &context, plan)?;
        writer.apply(&admission)?;
    }
    writer.complete(&admission).map_err(Error::from)
}

pub(super) fn preflight(
    harness: &Harness,
    target: &Target,
    named: &ArgvBundle,
    bundle: &Bundle,
    scope: Option<provider_v3::TargetScope>,
) -> Result<()> {
    refuse_a_neighbours_home(harness, target, scope)?;
    require_ownership(harness, target)?;
    let owned = owned_here(harness, target, scope)?;
    assemble(
        harness,
        &|path| setup_core::forward::observe(target, path),
        Some((named, bundle)),
        &owned,
        scope,
    )?;
    Ok(())
}

/// Plan withdrawal through the same assembly used at admission. A remove has
/// no replacement bundle and cannot acquire ownership of surviving host fields.
pub(super) fn removal_effects(
    harness: &Harness,
    target: &Target,
    scope: Option<provider_v3::TargetScope>,
) -> Result<Vec<String>> {
    refuse_a_neighbours_home(harness, target, scope)?;
    require_ownership(harness, target)?;
    let owned = owned_here(harness, target, scope)?;
    let assembly = assemble(
        harness,
        &|path| setup_core::forward::observe(target, path),
        None,
        &owned,
        scope,
    )?;
    let mut effects = vec!["withdraw recorded configuration without creating a backup".into()];
    effects.extend(assembly.changes.into_iter().map(|change| {
        if change.after.is_some() {
            format!(
                "withdraw owned content and retain the remaining bytes at {}",
                change.path
            )
        } else {
            format!("remove the recorded file {}", change.path)
        }
    }));
    Ok(effects)
}

fn require_ownership(harness: &Harness, target: &Target) -> Result<()> {
    let state = super::ProviderState::read(target.root(), harness.state_file)?;
    if super::ownership::observe(harness, target, &state).is_null() {
        return Err(refused());
    }
    Ok(())
}

fn permitted(harness: &Harness, path: &str, scope: Option<provider_v3::TargetScope>) -> bool {
    let under = |name: &&str| {
        path == *name
            || path
                .strip_prefix(*name)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    harness.owned_projection(scope).iter().any(under)
        && !harness.never_touch.iter().any(under)
        && path != harness.state_file
        && path != harness.control_directory
        && !path.starts_with(&format!("{}/", harness.control_directory))
}

fn encoded<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| refused())
}

fn assemble(
    harness: &Harness,
    read: &impl Fn(&str) -> setup_core::Result<(Observation, Option<Content>)>,
    bundle: Option<(&ArgvBundle, &Bundle)>,
    owned: &[String],
    scope: Option<provider_v3::TargetScope>,
) -> Result<Assembly> {
    let ledger_path = format!("{}/written-fields.json", harness.control_directory);
    let (ledger_before, old_ledger) = read(&ledger_path)?;
    let (state_before, _) = read(harness.state_file)?;
    let old_fields: BTreeMap<String, Vec<String>> = old_ledger
        .map(|content| serde_json::from_slice(&content.bytes).map_err(|_| refused()))
        .transpose()?
        .unwrap_or_default();
    if old_fields.keys().any(|path| !owned.contains(path)) {
        return Err(refused());
    }
    let paths: BTreeSet<&str> = owned
        .iter()
        .map(String::as_str)
        .chain(
            bundle
                .into_iter()
                .flat_map(|(_, bundle)| bundle.files.keys().map(String::as_str)),
        )
        .collect();
    let mut fields = BTreeMap::new();
    let mut changes = Vec::new();
    let contributions: BTreeMap<_, _> = bundle
        .into_iter()
        .flat_map(|(_, bundle)| bundle.manifest.files.iter())
        .filter_map(|file| {
            file.contribution
                .as_ref()
                .map(|claim| (file.path.as_str(), claim))
        })
        .collect();
    for path in paths {
        if !permitted(harness, path, scope) {
            return Err(refused());
        }
        let (before, existing) = read(path)?;
        let incoming = bundle.and_then(|(_, bundle)| bundle.files.get(path));
        let was_owned = owned.iter().any(|name| name == path);
        let after = if let Some(claim) = contributions.get(path) {
            let result = contributed(
                path,
                existing.as_ref(),
                incoming,
                claim,
                old_fields.get(path),
                was_owned,
            )?;
            fields.insert(path.into(), vec![claim.ownership_key.clone()]);
            Some(result)
        } else if incoming.is_none()
            && let Some(keys) = old_fields.get(path)
        {
            existing
                .as_ref()
                .map(|existing| -> Result<Content> {
                    Ok(Content::new(
                        fields::merge(path, Some(existing), None, keys)?,
                        preserved_mode(Some(existing), incoming),
                    )?)
                })
                .transpose()?
        } else {
            // Whole-path ownership cannot silently take over a shared host.
            // Remaining fields still belong to the user, regardless of format.
            if old_fields.contains_key(path) {
                return Err(refused());
            }
            desired(harness, path, existing.as_ref(), incoming, was_owned)?
        };
        changes.push(Change {
            path: path.into(),
            before,
            after,
        });
    }
    setup_core::forward::validate_changes(&changes)?;
    let applied = bundle.map_or_else(Applied::default, |(named, bundle)| {
        provenance(named, bundle)
    });
    Ok(Assembly {
        changes,
        context: Context {
            written_fields: fields,
            ledger_before,
            state_before,
            applied,
        },
    })
}

fn provenance(named: &ArgvBundle, bundle: &Bundle) -> Applied {
    let mut applied = Applied::default();
    record_bundle_provenance(&mut applied, named, bundle);
    applied.component_refs = bundle
        .manifest
        .conversion_report
        .entries
        .iter()
        .map(|entry| entry.stable_id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    applied.written_paths = bundle.files.keys().cloned().collect();
    applied
}

fn contributed(
    path: &str,
    existing: Option<&Content>,
    incoming: Option<&(Vec<u8>, u32)>,
    claim: &provider_v3::bundle::Contribution,
    keys: Option<&Vec<String>>,
    was_owned: bool,
) -> Result<Content> {
    fields::check_host(path, existing, claim)?;
    if was_owned && keys.is_none() {
        return Err(refused());
    }
    let (bytes, _) = incoming.ok_or_else(refused)?;
    let result = fields::merge(
        path,
        existing,
        Some((bytes, &claim.ownership_key)),
        keys.map_or(&[], Vec::as_slice),
    )?;
    Content::new(result, preserved_mode(existing, incoming)).map_err(Error::from)
}

fn desired(
    harness: &Harness,
    path: &str,
    existing: Option<&Content>,
    incoming: Option<&(Vec<u8>, u32)>,
    was_owned: bool,
) -> Result<Option<Content>> {
    if crate::instruction_region::is_attachment(path, harness.instruction_region) {
        let previous = existing
            .map(|c| std::str::from_utf8(&c.bytes).map_err(|_| refused()))
            .transpose()?
            .unwrap_or("");
        if !crate::instruction_region::markers_well_formed(previous) {
            return Err(refused());
        }
        let bytes = match incoming {
            Some((bytes, _)) => {
                let next = std::str::from_utf8(bytes).map_err(|_| refused())?;
                let without_region = crate::instruction_region::remove_region(previous);
                let unmarked = without_region.as_deref().unwrap_or(previous).trim();
                if !was_owned
                    && existing.is_some()
                    && !unmarked.is_empty()
                    && unmarked != next.trim()
                {
                    return Err(refused());
                }
                if !crate::instruction_region::markers_well_formed(next) {
                    return Err(refused());
                }
                Some(
                    crate::instruction_region::preserve_in_replacement(previous, next).into_bytes(),
                )
            }
            None => {
                crate::instruction_region::keep_region_on_withdraw(previous).map(String::into_bytes)
            }
        };
        return bytes
            .map(|bytes| {
                Content::new(bytes, preserved_mode(existing, incoming)).map_err(Error::from)
            })
            .transpose();
    }
    if !was_owned
        && let (Some(existing), Some((incoming, _))) = (existing, incoming)
        && existing.bytes != *incoming
    {
        return Err(Error::refuse(
            WireReason::Stale,
            format!("{path} already contains unrecorded bytes; explicit adoption is required"),
        ));
    }
    incoming
        .map(|(bytes, mode)| Content::new(bytes.clone(), *mode).map_err(Error::from))
        .transpose()
}

fn preserved_mode(existing: Option<&Content>, incoming: Option<&(Vec<u8>, u32)>) -> u32 {
    let requested = incoming.map_or(0o600, |(_, mode)| *mode);
    existing.map_or(requested, |content| {
        incoming.map_or(content.mode, |_| content.mode & requested)
    })
}

fn finalize(
    harness: &Harness,
    target: &Target,
    writer: &mut Writer,
    admission: &Admission,
    context: &serde_json::Value,
    plan: &serde_json::Value,
) -> Result<()> {
    let context: Context = serde_json::from_value(context.clone()).map_err(|_| refused())?;
    let applied = &context.applied;
    let after = target.identity_of_owned(
        &as_paths(&applied.written_paths),
        &harness.not_our_identity(),
    )?;
    let binding = StatePlan {
        artifact: plan,
        digest: &admission.plan_digest,
        scope: scope_of(plan),
    };
    let state = build_state(
        harness,
        target,
        &binding,
        &admission.target_precondition_digest,
        &after,
        applied,
    )?;
    let changes = [
        Change {
            path: format!("{}/written-fields.json", harness.control_directory),
            before: context.ledger_before,
            after: Some(Content::new(encoded(&context.written_fields)?, 0o600)?),
        },
        Change {
            path: harness.state_file.into(),
            before: context.state_before,
            after: Some(Content::new(encoded(&state)?, 0o600)?),
        },
    ];
    let outcome = serde_json::json!({"state":"verified", "operation":admission.operation,
        "plan_digest":admission.plan_digest, "expected_target_digest":admission.target_precondition_digest,
        "target_identity_digest":after, "backup_ref":null, "setup_id":applied.setup_id});
    writer
        .finish(admission, &changes, &outcome)
        .map_err(Error::from)
}
