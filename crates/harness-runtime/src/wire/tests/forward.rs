use super::*;

fn request(
    target: &Path,
    id: &str,
    files: &[(&str, &str, u32)],
) -> (serde_json::Value, Vec<String>) {
    request_bytes(target, id, bundle_bytes(files))
}

fn request_bytes(
    target: &Path,
    id: &str,
    (bytes, logical, artifact): (Vec<u8>, String, String),
) -> (serde_json::Value, Vec<String>) {
    let bundle = target.parent().unwrap().join(format!("{id}.zip"));
    fs::write(&bundle, &bytes).unwrap();
    let flags = bundle_flags(&bundle, &logical, &artifact, bytes.len());
    let mut plan = args(
        "plan-operation",
        target,
        &[
            "--operation",
            "replace",
            "--recovery-mode",
            "forward",
            "--provider-release-digest",
            RELEASE,
            "--operation-id",
            id,
            "--expires-at",
            far_future(),
        ],
    );
    plan.extend(flags.clone());
    let planned = run(plan);
    let apply = apply_plan(target, id, &planned, flags);
    (planned, apply)
}

fn apply_plan(
    target: &Path,
    id: &str,
    planned: &serde_json::Value,
    flags: Vec<String>,
) -> Vec<String> {
    let plan_path = target.parent().unwrap().join(format!("{id}.json"));
    fs::write(
        &plan_path,
        setup_core::canonical::to_canonical_bytes(&planned["plan"]).unwrap(),
    )
    .unwrap();
    let mut apply = args(
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
    );
    apply.extend(flags);
    apply
}

fn remove_request(target: &Path, id: &str) -> (serde_json::Value, Vec<String>) {
    let planned = run(args(
        "plan-operation",
        target,
        &[
            "--operation",
            "remove",
            "--recovery-mode",
            "forward",
            "--provider-release-digest",
            RELEASE,
            "--operation-id",
            id,
            "--expires-at",
            far_future(),
        ],
    ));
    let apply = apply_plan(target, id, &planned, Vec::new());
    (planned, apply)
}

fn contribution_bytes(path: &str, host: &[u8], assembled: &str) -> (Vec<u8>, String, String) {
    use provider_v3::zip::{
        self,
        build::{Entry, write},
    };
    let (bytes, _, _) = bundle_bytes(&[(path, assembled, 0o644)]);
    let mut members = zip::read(&bytes).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_slice(&members[0].data).unwrap();
    manifest["files"][0]["contribution"] = serde_json::json!({
        "parser_id": if Path::new(path).extension().is_some_and(|ext| ext == "toml") { "toml/1" } else { "json/1" },
        "ownership_key":"managed", "host_digest":digest::of_bytes(host)
    });
    manifest.as_object_mut().unwrap().remove("bundle_digest");
    let logical =
        digest::of_domain_canonical_json(provider_v3::bundle::BUNDLE_DOMAIN, &manifest).unwrap();
    manifest["bundle_digest"] = logical.clone().into();
    members[0].data = setup_core::canonical::to_canonical_bytes(&manifest).unwrap();
    let entries: Vec<_> = members
        .into_iter()
        .map(|m| Entry {
            name: m.name,
            data: m.data,
            mode: m.mode.unwrap(),
        })
        .collect();
    let bytes = write(&entries);
    let artifact = digest::of_bytes(&bytes);
    (bytes, logical, artifact)
}

#[test]
fn assembled_contributions_do_not_claim_preserved_host_fields() {
    let examples = [
        (
            "settings.json",
            r#"{"user":9007199254740993}"#,
            r#"{"user":9007199254740993,"managed":{"enabled":true}}"#,
        ),
        (
            "skills/host.jsonc",
            "{\n // user comment\n \"user\":9007199254740993,\n}",
            "{\"user\":9007199254740993,\"managed\":{\"enabled\":true}}",
        ),
        (
            "skills/host.toml",
            "# user comment\nuser = 9007199254740993\n",
            "user = 9007199254740993\n[managed]\nenabled = true\n",
        ),
    ];
    for (index, (path, host, assembled)) in examples.into_iter().enumerate() {
        let target = seeded(&format!("forward-contribution-{index}"));
        fs::write(target.join(path), host).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(target.join(path), fs::Permissions::from_mode(0o600)).unwrap();
        }
        let (_, apply) = request_bytes(
            &target,
            "operation_FIELDS_1",
            contribution_bytes(path, host.as_bytes(), assembled),
        );
        fs::write(target.join(path), host.replace("9007199254740993", "123")).unwrap();
        assert_eq!(refuse(apply.clone()).reason(), Some(WireReason::Stale));
        fs::write(target.join(path), host).unwrap();
        assert_eq!(run(apply)["state"], "verified");
        assert_recorded_contribution(&target, path, host);
        let (_, withdraw) = request(
            &target,
            "operation_FIELDS_2",
            &[("skills/next.md", "next", 0o644)],
        );
        assert_eq!(run(withdraw)["state"], "verified");
        assert_eq!(
            run(args("status", &target, &[]))["configuration_ownership"],
            serde_json::json!([{
                "path":"skills/next.md","ownership":"whole","parser_id":null,"ownership_keys":[]
            }])
        );
        let retained = fs::read_to_string(target.join(path)).unwrap();
        assert!(retained.contains("9007199254740993"));
        assert!(!retained.contains("managed"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(target.join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        if Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "jsonc")
            || Path::new(path).extension().is_some_and(|ext| ext == "toml")
        {
            assert!(retained.contains("user comment"));
        }
        assert!(!target.join(TEST.control_directory).join("backups").exists());
        fs::remove_dir_all(target.parent().unwrap()).unwrap();
    }
}

fn assert_recorded_contribution(target: &Path, path: &str, host: &str) {
    let ledger: serde_json::Value = serde_json::from_slice(
        &fs::read(
            target
                .join(TEST.control_directory)
                .join("written-fields.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(ledger[path], serde_json::json!(["managed"]));
    let expected = serde_json::json!([{
        "path":path,"ownership":"contribution",
        "parser_id":if Path::new(path).extension().is_some_and(|ext| ext == "toml") { "toml/1" } else { "json/1" },
        "ownership_keys":["managed"]
    }]);
    assert_eq!(
        run(args("status", target, &[]))["configuration_ownership"],
        expected
    );
    let installed = fs::read(target.join(path)).unwrap();
    fs::write(target.join(path), host).unwrap();
    let drifted = run(args("status", target, &[]));
    assert_eq!(drifted["provider_state"]["drift_state"], "local_drift");
    assert_eq!(drifted["configuration_ownership"], expected);
    fs::write(target.join(path), &installed).unwrap();
    let ledger_path = target
        .join(TEST.control_directory)
        .join("written-fields.json");
    for broken in [
        format!(r#"{{"{path}":["managed"],"{path}":["other"]}}"#),
        format!(r#"{{"{path}":["managed","managed"]}}"#),
        r#"{"unrecorded.json":["managed"]}"#.into(),
        "[".into(),
    ] {
        fs::write(&ledger_path, broken).unwrap();
        assert!(run(args("status", target, &[]))["configuration_ownership"].is_null());
        let error = refuse(args(
            "plan-operation",
            target,
            &[
                "--operation",
                "remove",
                "--recovery-mode",
                "forward",
                "--provider-release-digest",
                RELEASE,
                "--operation-id",
                "operation_INVALID",
                "--expires-at",
                far_future(),
            ],
        ));
        assert_eq!(error.reason(), Some(WireReason::Stale));
        assert_eq!(fs::read(target.join(path)).unwrap(), installed);
    }
    fs::write(&ledger_path, serde_json::to_vec(&ledger).unwrap()).unwrap();
}

fn settings(target: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(target.join("skills/settings.json")).unwrap()).unwrap()
}

#[test]
fn forward_whole_files_replace_exact_bytes_and_replay_only_the_original_receipt() {
    let target = seeded("forward-config");
    assert_eq!(
        run(args("status", &target, &[]))["configuration_ownership"],
        serde_json::json!([])
    );
    let (planned, apply) = request(
        &target,
        "operation_FORWARD_1",
        &[
            (
                "skills/settings.json",
                r#"{ "managed": 1, "retired": true }"#,
                0o644,
            ),
            ("skills/new.md", "owned skill", 0o644),
        ],
    );
    assert_eq!(planned["plan"]["recovery_mode"], "forward");
    assert!(
        !target.join(TEST.control_directory).exists(),
        "planning created control state"
    );
    let first = run(apply.clone());
    assert_eq!(first["state"], "verified");
    assert!(first["backup_ref"].is_null());
    assert_eq!(
        run(args("status", &target, &[]))["configuration_ownership"],
        serde_json::json!([
            {"path":"skills/new.md","ownership":"whole","parser_id":null,"ownership_keys":[]},
            {"path":"skills/settings.json","ownership":"whole","parser_id":null,"ownership_keys":[]}
        ])
    );
    assert_eq!(
        fs::read(target.join("skills/settings.json")).unwrap(),
        br#"{ "managed": 1, "retired": true }"#
    );
    let (_, replace) = request(
        &target,
        "operation_FORWARD_2",
        &[
            ("skills/settings.json", r#"{"managed":2}"#, 0o644),
            ("skills/next.md", "next owned skill", 0o644),
        ],
    );
    assert_eq!(run(replace)["state"], "verified");
    assert_eq!(settings(&target), serde_json::json!({"managed":2}));
    assert!(!target.join("skills/new.md").exists());
    assert_eq!(fs::read(target.join("skills/a.md")).unwrap(), b"skill one");
    let bundle_path = apply.windows(2).find(|pair| pair[0] == "--bundle").unwrap()[1].clone();
    fs::remove_file(bundle_path).unwrap();
    assert_eq!(run(apply), first, "historical replay changed its outcome");
    assert_eq!(
        settings(&target)["managed"],
        2,
        "historical replay overwrote the current setup"
    );
    assert_eq!(
        fs::read(target.join(".credentials.json")).unwrap(),
        b"SECRET"
    );
    assert!(!target.join(TEST.control_directory).join("backups").exists());
    assert!(
        !target
            .join(TEST.control_directory)
            .join("transaction")
            .exists()
    );
    assert!(
        Journal::read(&target.join(TEST.control_directory))
            .unwrap()
            .is_none()
    );
    let StateReading::Current(state) = ProviderState::read(&target, TEST.state_file).unwrap()
    else {
        panic!("provider state is missing")
    };
    assert!(state.backup_ref.is_none());
    assert_eq!(state.operation_id, "operation_FORWARD_2");
    let (_, withdraw) = request(
        &target,
        "operation_FORWARD_3",
        &[("skills/last.md", "last", 0o644)],
    );
    assert_eq!(run(withdraw)["state"], "verified");
    assert!(!target.join("skills/settings.json").exists());
    assert_unknown_content_refused(&target, "skills/a.md", "would overwrite");
    assert_unknown_content_refused(&target, "settings.json", r#"{"model":"changed"}"#);
    let (planned, remove) = remove_request(&target, "operation_FORWARD_4");
    assert!(planned["plan"]["bundle"].is_null());
    let outcome = run(remove.clone());
    assert_eq!(outcome["operation"], "remove");
    assert!(outcome["setup_id"].is_null());
    assert_eq!(run(remove), outcome);
    assert!(!target.join("skills/last.md").exists());
    assert_eq!(fs::read(target.join("skills/a.md")).unwrap(), b"skill one");
    assert!(!target.join(TEST.control_directory).join("backups").exists());
    fs::remove_dir_all(target.parent().unwrap()).unwrap();
}

#[test]
fn forward_removal_preserves_shared_hosts_and_history_after_a_later_install() {
    let examples = [
        (
            "settings.json",
            r#"{"user":9007199254740993}"#,
            r#"{"user":9007199254740993,"managed":{"enabled":true}}"#,
        ),
        (
            "skills/host.jsonc",
            "{\n // user comment\n \"user\":9007199254740993,\n}",
            r#"{"user":9007199254740993,"managed":{"enabled":true}}"#,
        ),
        (
            "skills/host.toml",
            "# user comment\nuser = 9007199254740993\n",
            "user = 9007199254740993\n[managed]\nenabled = true\n",
        ),
    ];
    for (index, (path, host, assembled)) in examples.into_iter().enumerate() {
        let target = seeded(&format!("forward-remove-{index}"));
        fs::write(target.join(path), host).unwrap();
        let (_, install) = request_bytes(
            &target,
            "operation_REMOVE_1",
            contribution_bytes(path, host.as_bytes(), assembled),
        );
        assert_eq!(run(install)["state"], "verified");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(target.join(path), fs::Permissions::from_mode(0o644)).unwrap();
        }
        let (planned, remove) = remove_request(&target, "operation_REMOVE_2");
        assert!(planned["plan"]["bundle"].is_null());
        assert!(planned["plan"].get("end_state").is_none());
        assert!(
            planned["effects"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| !v.as_str().unwrap().contains("capture"))
        );
        let before = fs::read(target.join(path)).unwrap();
        fs::write(target.join(path), host).unwrap();
        assert_eq!(refuse(remove.clone()).reason(), Some(WireReason::Stale));
        fs::write(target.join(path), before).unwrap();
        let outcome = run(remove.clone());
        assert_eq!(outcome["state"], "verified");
        assert_eq!(outcome["operation"], "remove");
        assert!(outcome["setup_id"].is_null());
        assert!(outcome["backup_ref"].is_null());
        assert_eq!(
            run(args("status", &target, &[]))["configuration_ownership"],
            serde_json::json!([])
        );
        let retained = fs::read_to_string(target.join(path)).unwrap();
        assert!(retained.contains("9007199254740993"));
        assert!(!retained.contains("managed"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(target.join(path))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
        }
        if index != 0 {
            assert!(retained.contains("user comment"));
        }
        let StateReading::Current(state) = ProviderState::read(&target, TEST.state_file).unwrap()
        else {
            panic!("the terminal removal record is missing");
        };
        assert!(state.setup_stable_id.is_none());
        assert!(state.bundle_digest.is_none());
        assert!(state.written_paths.is_empty());
        let (_, next) = request(
            &target,
            "operation_REMOVE_3",
            &[("skills/next.md", "later", 0o644)],
        );
        assert_eq!(run(next)["state"], "verified");
        assert_eq!(run(remove), outcome);
        assert_eq!(fs::read(target.join("skills/next.md")).unwrap(), b"later");
        assert_eq!(fs::read(target.join("skills/a.md")).unwrap(), b"skill one");
        assert!(!target.join(TEST.control_directory).join("backups").exists());
        fs::remove_dir_all(target.parent().unwrap()).unwrap();
    }
}

#[test]
fn forward_removal_retains_attached_instructions_and_refuses_ambiguous_markers() {
    let target = seeded("forward-remove-instructions");
    let path = target.join("AGENTS.md");
    let attached = format!("---\nalwaysApply: true\n---\n\n{INSTRUCTION_SECTION}");
    fs::write(&path, &attached).unwrap();
    let (_, apply) = request(
        &target,
        "operation_ATTACHMENT_1",
        &[("AGENTS.md", "setup body\n", 0o644)],
    );
    assert_eq!(run(apply)["state"], "verified");
    assert!(fs::read_to_string(&path).unwrap().contains("setup body"));
    let (planned, remove) = remove_request(&target, "operation_ATTACHMENT_2");
    let installed = fs::read(&path).unwrap();
    fs::write(&path, "setup body\n:::begin-ai-stp\n").unwrap();
    assert_eq!(refuse(remove.clone()).reason(), Some(WireReason::Stale));
    fs::write(&path, installed).unwrap();
    assert_eq!(run(remove.clone())["plan_digest"], planned["plan_digest"]);
    assert_eq!(fs::read_to_string(&path).unwrap(), attached);
    assert_eq!(run(remove)["state"], "verified");
    assert!(!target.join(TEST.control_directory).join("backups").exists());
    fs::remove_dir_all(target.parent().unwrap()).unwrap();
}

fn assert_unknown_content_refused(target: &Path, path: &str, body: &str) {
    let before = fs::read(target.join(path)).unwrap();
    // Unknown content is rejected by planning before it can be admitted.
    let (bytes, logical, artifact) = bundle_bytes(&[(path, body, 0o644)]);
    let bundle = target.parent().unwrap().join("conflict.zip");
    fs::write(&bundle, &bytes).unwrap();
    let mut conflict = args(
        "plan-operation",
        target,
        &[
            "--operation",
            "install",
            "--recovery-mode",
            "forward",
            "--provider-release-digest",
            RELEASE,
            "--operation-id",
            "operation_FORWARD_3",
            "--expires-at",
            far_future(),
        ],
    );
    conflict.extend(bundle_flags(&bundle, &logical, &artifact, bytes.len()));
    assert_eq!(refuse(conflict).reason(), Some(WireReason::Stale));
    assert_eq!(fs::read(target.join(path)).unwrap(), before);
}
