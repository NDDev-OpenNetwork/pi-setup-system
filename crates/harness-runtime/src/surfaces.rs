//! The binding between a declaration and the vendor document that decided it.
//!
//! `native_namespaces` says what a provider **owns** inside a target, and every
//! entry of `component_kinds` is a promise of a rollback for components of that
//! kind. Both are published in `provider-info`, which is the authority a
//! consumer plans, verifies and computes target identity against.
//!
//! They were once assembled from a consumer's routing table. Measured against
//! the products themselves on 2026-08-27, that had left paths no vendor
//! documents — `~/.cursor/rules`, `~/.claude/.mcp.json`, `~/.grok/commands` —
//! and omitted paths every vendor does. Conformance never noticed: its
//! `declared_native_route_is_compilable` case requires **one** declared kind to
//! have a route, not all of them.
//!
//! So each `references/<harness>-baseline.json` now carries a `native_surfaces`
//! block: one row per owned surface with the URL that decided it, and a
//! `declined` list naming every path considered and not owned, with the reason.
//! This module compares a [`Harness`] against that block, and each setup system
//! calls it from one test. A row exists in exactly one place and disagreeing
//! with it is red.
//!
//! **What this cannot do, stated here because the sentence is easy to
//! overclaim.** Both sides of this comparison are written in this repository.
//! It catches a declaration that drifts from its baseline; it cannot catch a
//! baseline row that is wrong, because the declaration will have been written
//! from the same misreading. Shared error is exactly what produced every defect
//! this module was built after -- `~/.claude/.mcp.json` was in a consumer's
//! table *and* ours, both citing the same page, and the agreement between them
//! was worth nothing. Requiring a URL on every owned row is the strongest thing
//! available from inside a test: it makes the claim checkable by a person, not
//! by a run. Nothing here reads the page.
//!
//! Cursor has now produced three of these, and they share a shape worth naming:
//! *the page that looks authoritative for a thing does not say where the thing
//! goes.* The MCP page was about scopes, the rules page about rules, and the
//! plugin reference tells you how to build one and not where it is installed.
//! A row taken from the obvious page reads perfectly and is wrong. When a row's
//! source is a page *about* the feature rather than a page naming its path,
//! that is the row to re-read first.
//!
//! **Declaring a path and a place inside it is legal, and there is one reason
//! to.** A consumer validates a compiler's route against `native_namespaces` by
//! exact membership, so moving a route deeper -- cursor's `plugins` to the
//! `plugins/local` the product actually reads -- has no order that works: the
//! side that moves first refuses every install against the side that has not.
//! One release naming both opens the window, and either side may then move.
//! `setup_core::digest::of_owned` reduces the declaration to a cover before
//! walking it, so the extra name cannot move an installed target's identity;
//! `naming_a_path_already_covered_cannot_move_the_identity` is what holds that.
//!
//! `declined` carries two different reasons and it is worth knowing which you
//! are reading. Most entries are paths no vendor documents — `~/.cursor/rules`,
//! `~/.grok/commands`. Some are documented and deliberately not owned:
//! opencode reads `opencode.jsonc` as readily as `opencode.json`, and owning
//! both would let one target hold two documents that disagree, with the product
//! reading one and this provider reporting the other. The reason text says
//! which, because "we could not find it" and "we chose against it" age
//! differently.
//!
//! Two rules are worth stating because they are the ones that keep being got
//! wrong:
//!
//! * **A key inside a file is not a projection surface.** Grok, codex and
//!   opencode all keep MCP servers under a key of a configuration file this
//!   provider already owns. Owning the file is right; declaring `mcp` would
//!   promise to install, observe and restore a fragment of it, which nothing
//!   here does.
//! * **A surface may be owned and route nothing.** `workflows` under the Grok
//!   home is a real directory a setup may carry, and no component kind projects
//!   there. Such a row lists no kinds rather than borrowing the nearest one.

use provider_v3::ComponentKind;
use serde_json::Value;

use crate::facts::Harness;

/// The baseline key this module reads.
pub const BLOCK: &str = "native_surfaces";

/// The two shapes a surface can have.
const SHAPES: [&str; 2] = ["file", "directory"];

/// Every way a declaration and its baseline can disagree, in a stable order.
///
/// Empty is the only passing answer. Each string names one disagreement and is
/// written to be read on its own, because a test failure shows the list and
/// nothing else.
/// One owned surface, after its row has been checked.
struct Owned<'a> {
    path: &'a str,
    kinds: Vec<&'a str>,
}

/// Read and check every `surfaces` row, collecting what it owns and routes.
fn owned_surfaces<'a>(rows: &'a [Value], found: &mut Vec<String>) -> Vec<Owned<'a>> {
    let mut owned: Vec<Owned<'a>> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            found.push(format!("{BLOCK}.surfaces[{index}] has no path"));
            continue;
        };
        if owned.iter().any(|seen| seen.path == path) {
            found.push(format!("{path} is listed twice among the owned surfaces"));
        }
        match row.get("source").and_then(Value::as_str) {
            Some(source) if !source.is_empty() => {}
            _ => found.push(format!(
                "{path} is owned and cites no source; a surface nobody can \
                 source is not owned"
            )),
        }
        match row.get("shape").and_then(Value::as_str) {
            Some(shape) if SHAPES.contains(&shape) => {}
            other => found.push(format!(
                "{path} has shape {other:?}, which is not one of {SHAPES:?}"
            )),
        }
        let Some(rows) = row.get("kinds").and_then(Value::as_array) else {
            found.push(format!(
                "{path} has no kinds array; a surface that routes no component \
                 kind says so with an empty one"
            ));
            continue;
        };
        let mut kinds = Vec::new();
        for kind in rows {
            let Some(kind) = kind.as_str() else {
                found.push(format!("{path} names a kind that is not a string"));
                continue;
            };
            if ComponentKind::parse(kind).is_none() {
                found.push(format!(
                    "{path} names {kind:?}, which is not a component kind"
                ));
                continue;
            }
            kinds.push(kind);
        }
        owned.push(Owned { path, kinds });
    }
    owned
}

/// Compare the declaration against what the checked rows own and route.
fn against_declaration(harness: &Harness, owned: &[Owned<'_>], found: &mut Vec<String>) {
    for namespace in harness.native_namespaces {
        if !owned.iter().any(|surface| surface.path == *namespace) {
            found.push(format!(
                "{namespace} is declared in native_namespaces and is not in {BLOCK}; \
                 either the vendor documents it and the baseline should say where, \
                 or nothing does and it should not be owned"
            ));
        }
    }
    for surface in owned {
        if !harness.native_namespaces.contains(&surface.path) {
            found.push(format!(
                "{} is a documented surface and is not declared in native_namespaces",
                surface.path
            ));
        }
    }

    // Two surfaces routing one kind is not fatal and is worth naming: a
    // compiler would have to choose, and nothing here tells it how.
    let mut routed: Vec<&str> = Vec::new();
    for surface in owned {
        for kind in &surface.kinds {
            if routed.contains(kind) {
                found.push(format!(
                    "{kind} is routed by more than one surface, the second being {}",
                    surface.path
                ));
            }
            routed.push(kind);
        }
    }

    for kind in harness.component_kinds {
        if !routed.contains(&kind.as_str()) {
            found.push(format!(
                "{} is declared and no owned surface routes it; a declared kind is \
                 a promise of a rollback",
                kind.as_str()
            ));
        }
    }
    for kind in &routed {
        if !harness
            .component_kinds
            .iter()
            .any(|declared| declared.as_str() == *kind)
        {
            found.push(format!(
                "{kind} has an owned surface and is not declared, so a consumer \
                 that compiles one is refused"
            ));
        }
    }
}

/// Compare one scoped declaration against the rows sourced for it.
fn against_scope(
    declared: &crate::facts::Scoped,
    owned: &[Owned<'_>],
    named: &str,
    found: &mut Vec<String>,
) {
    for namespace in declared.native_namespaces {
        if !owned.iter().any(|surface| surface.path == *namespace) {
            found.push(format!(
                "{namespace} is declared for the {named} scope and is not in {BLOCK}"
            ));
        }
    }
    let mut routed: Vec<&str> = Vec::new();
    for surface in owned {
        if !declared.native_namespaces.contains(&surface.path) {
            found.push(format!(
                "{} is sourced for the {named} scope and is not declared",
                surface.path
            ));
        }
        for kind in &surface.kinds {
            routed.push(kind);
        }
    }
    for kind in declared.component_kinds {
        if !routed.contains(&kind.as_str()) {
            found.push(format!(
                "{} is declared for the {named} scope and no owned surface routes it",
                kind.as_str()
            ));
        }
    }
    for kind in &routed {
        if !declared
            .component_kinds
            .iter()
            .any(|held| held.as_str() == *kind)
        {
            found.push(format!(
                "{kind} has a surface in the {named} scope and is not declared there"
            ));
        }
    }
}

/// Check the list that makes a removal stay removed.
fn declined_rows(harness: &Harness, rows: &[Value], owned: &[Owned<'_>], found: &mut Vec<String>) {
    declined_rows_in(harness.native_namespaces, rows, owned, found);
}

/// The same check, against whichever namespace list owns the scope.
fn declined_rows_in(
    namespaces: &[&str],
    rows: &[Value],
    owned: &[Owned<'_>],
    found: &mut Vec<String>,
) {
    for (index, row) in rows.iter().enumerate() {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            found.push(format!("{BLOCK}.declined[{index}] has no path"));
            continue;
        };
        if namespaces.contains(&path) {
            found.push(format!("{path} is declined and owned at the same time"));
        }
        if owned.iter().any(|surface| surface.path == path) {
            found.push(format!("{path} is in both surfaces and declined"));
        }
        for key in ["reason", "source"] {
            match row.get(key).and_then(Value::as_str) {
                Some(text) if !text.is_empty() => {}
                _ => found.push(format!("{path} is declined and carries no {key}")),
            }
        }
    }
}

/// The two paths that are this provider's own must be recorded as declined.
///
/// They are not projection surfaces and never should be: the control directory
/// and the state file are bookkeeping, written by every operation and excluded
/// from target identity so an applied change does not leave the target
/// different from the identity it just recorded.
///
/// Recording them anyway, because the record is where the next reader looks. A
/// peer reviewing an installed target found `NDDEV-CLAUDE-PROVIDER.json` sitting
/// in a home and not in `native_namespaces` -- the same shape as a real defect
/// found the same day -- and had a report half-written before opening the file
/// and seeing `drift_state` and `backup_ref` inside. The code said it, in a doc
/// comment and a per-harness test. The `declined` list, which exists so nobody
/// repeats a search, did not.
fn control_state_is_recorded(harness: &Harness, rows: &[Value], found: &mut Vec<String>) {
    for own in [harness.control_directory, harness.state_file] {
        if !rows
            .iter()
            .any(|row| row.get("path").and_then(Value::as_str) == Some(own))
        {
            found.push(format!(
                "{own} is this provider's own control state and {BLOCK}.declined \
                 does not record it, so a reader has to open the file to learn \
                 it is not a surface"
            ));
        }
    }
}

/// Every path a provider evaluates is relative to `--target`, so a record of
/// one cannot be written relative to anything else.
///
/// This exists because a `$HOME`-relative entry hid a real gap for a release.
/// `references/claude-baseline.json` listed `~/.claude.json` under
/// `never_touch`, and the harness test that binds that list to the declaration
/// skipped it with a stated reason: *sits outside the target and cannot be a
/// top-level entry of it*. That reasoning is true of a person running the
/// product with its default home and false of this provider, which points
/// `CLAUDE_CONFIG_DIR` at a target -- and then the product writes
/// `<target>/.claude.json`. Measured by running it, which is the only way this
/// one could be found: a vendor page says what a product reads, and only a run
/// says what it writes.
///
/// So the skip is gone and the shape that made it look reasonable is refused
/// here. A `~` or a leading `/` in any recorded path is the same defect that has
/// now appeared eight times -- a path is only a path together with what it is
/// relative to.
fn rooted_elsewhere(baseline: &Value, found: &mut Vec<String>) {
    let mut check = |path: &str, whose: &str| {
        if path.starts_with('~') || path.starts_with('/') {
            found.push(format!(
                "{whose} records {path:?}, which is relative to a root this \
                 provider never evaluates against; every recorded path \
                 is relative to the target"
            ));
        }
    };
    if let Some(names) = baseline.get("never_touch").and_then(Value::as_array) {
        for name in names.iter().filter_map(Value::as_str) {
            check(name, "never_touch");
        }
    }
    let Some(block) = baseline.get(BLOCK).and_then(Value::as_object) else {
        return;
    };
    for list in ["surfaces", "declined"] {
        let Some(rows) = block.get(list).and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            if let Some(path) = row.get("path").and_then(Value::as_str) {
                check(path, &format!("{BLOCK}.{list}"));
            }
        }
    }
}

/// Every way a declaration and its baseline can disagree, in a stable order.
///
/// Empty is the only passing answer. Each string names one disagreement and is
/// written to be read on its own, because a test failure shows the list and
/// nothing else.
#[must_use]
pub fn disagreements(harness: &Harness, baseline: &Value) -> Vec<String> {
    let mut found = Vec::new();
    rooted_elsewhere(baseline, &mut found);

    let Some(block) = baseline.get(BLOCK).and_then(Value::as_object) else {
        found.push(format!(
            "{} has no {BLOCK} object, so nothing this harness claims to own is sourced",
            harness.provider_id
        ));
        return found;
    };

    // The variable that points the product at a target, bound the same way the
    // home itself is.
    //
    // It had been a value in code with nothing to check it against, while the
    // seven baselines recorded it under five different names --
    // `config_dir_env`, `configuration.environment_override`,
    // `runtime.grok_home`, `configuration.custom_config_dir_env`, and for two of
    // them nothing at all. The declaration has one slot, so five distinctions
    // collapsed into it, and the one that meant something different --
    // opencode's *custom config dir* -- read as the same thing as the rest. It
    // survived that reading, measured, but only because the product happens to
    // read its configuration there; nothing had checked.
    match block.get("config_home_env").and_then(Value::as_str) {
        Some(name) if name == harness.config_home_env => {}
        Some(name) => found.push(format!(
            "{BLOCK}.config_home_env is {name:?} and the declaration says {:?}",
            harness.config_home_env
        )),
        None => found.push(format!(
            "{BLOCK} records no config_home_env, so the variable this build sets \
             on a launched product is a value nothing checks"
        )),
    }
    match block.get("config_home_env_note").and_then(Value::as_str) {
        Some(note) if !note.is_empty() => {}
        _ => found.push(format!(
            "{BLOCK}.config_home_env carries no note saying how it was \
             established; a variable name is a claim about a product"
        )),
    }

    match block.get("config_home").and_then(Value::as_str) {
        Some(home) if home == harness.documented_config_home => {}
        Some(home) => found.push(format!(
            "{BLOCK}.config_home is {home:?} and the declaration says {:?}",
            harness.documented_config_home
        )),
        None => found.push(format!("{BLOCK}.config_home is missing")),
    }

    let Some(surfaces) = block.get("surfaces").and_then(Value::as_array) else {
        found.push(format!("{BLOCK}.surfaces is missing or not an array"));
        return found;
    };
    // Every row is checked before any set is compared, so a malformed row is
    // reported as itself rather than as a missing namespace somewhere else.
    let owned = owned_surfaces(surfaces, &mut found);
    against_declaration(harness, &owned, &mut found);

    // A second scope is checked exactly as the first, against the declaration
    // that owns it. Written as the same two functions rather than a variant of
    // them: a scope whose rows were checked by looser code would be the place a
    // path travels without its root, which is the defect this whole block was
    // written for.
    let scoped_blocks = block
        .get("scoped")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (index, entry) in scoped_blocks.iter().enumerate() {
        let Some(named) = entry.get("target_scope").and_then(Value::as_str) else {
            found.push(format!("{BLOCK}.scoped[{index}] names no target_scope"));
            continue;
        };
        let Some(declared) = harness
            .scoped_projections
            .iter()
            .find(|scoped| scoped.target_scope.as_str() == named)
        else {
            found.push(format!(
                "{named} is sourced in {BLOCK} and this harness declares no such scope"
            ));
            continue;
        };
        let Some(rows) = entry.get("surfaces").and_then(Value::as_array) else {
            found.push(format!("{BLOCK}.scoped[{index}].surfaces is missing"));
            continue;
        };
        let owned = owned_surfaces(rows, &mut found);
        against_scope(declared, &owned, named, &mut found);
        let empty = Vec::new();
        let rows = entry
            .get("declined")
            .and_then(Value::as_array)
            .unwrap_or(&empty);
        declined_rows_in(declared.native_namespaces, rows, &owned, &mut found);
    }
    for declared in harness.scoped_projections {
        let named = declared.target_scope.as_str();
        let sourced = scoped_blocks
            .iter()
            .any(|entry| entry.get("target_scope").and_then(Value::as_str) == Some(named));
        if !sourced {
            found.push(format!(
                "this harness declares the {named} scope and {BLOCK} sources none"
            ));
        }
    }

    let Some(declined) = block.get("declined").and_then(Value::as_array) else {
        found.push(format!(
            "{BLOCK}.declined is missing or not an array; a harness that declined \
             nothing says so with an empty one"
        ));
        return found;
    };
    declined_rows(harness, declined, &owned, &mut found);
    control_state_is_recorded(harness, declined, &mut found);

    found
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use serde_json::json;

    use super::*;
    use crate::wire::tests_support::TEST;

    /// A block that agrees with [`TEST`], built here so each test below can
    /// break exactly one thing about it.
    fn agreeing() -> Value {
        let surfaces: Vec<Value> = TEST
            .native_namespaces
            .iter()
            .enumerate()
            .map(|(index, path)| {
                // Every declared kind is routed, and by exactly one surface:
                // the first namespace carries them all, the rest carry none.
                let kinds: Vec<&str> = if index == 0 {
                    TEST.component_kinds
                        .iter()
                        .map(|kind| kind.as_str())
                        .collect()
                } else {
                    Vec::new()
                };
                json!({
                    "path": path,
                    "kinds": kinds,
                    "shape": "file",
                    "source": "https://example.invalid/docs",
                })
            })
            .collect();
        json!({
            BLOCK: {
                "verified_at": "2026-08-27",
                "config_home": TEST.documented_config_home,
                "config_home_env": TEST.config_home_env,
                "config_home_env_note": "measured against the product",
                "surfaces": surfaces,
                // The two paths that are the provider's own. Every real
                // baseline records them, so the fixture that stands for one
                // must too.
                "declined": [
                    {
                        "path": TEST.state_file,
                        "reason": "this provider's own state file",
                        "source": "this provider's own contract",
                    },
                    {
                        "path": TEST.control_directory,
                        "reason": "this provider's own control directory",
                        "source": "this provider's own contract",
                    },
                ],
            }
        })
    }

    #[test]
    fn a_declaration_that_matches_its_baseline_has_nothing_to_say() {
        assert_eq!(disagreements(&TEST, &agreeing()), Vec::<String>::new());
    }

    #[test]
    fn a_missing_block_is_reported_rather_than_passing_silently() {
        let problems = disagreements(&TEST, &json!({}));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("no native_surfaces"));
    }

    #[test]
    fn an_owned_namespace_the_baseline_does_not_source_is_named() {
        let mut baseline = agreeing();
        let surfaces = baseline[BLOCK]["surfaces"].as_array_mut().unwrap();
        let dropped = surfaces.pop().unwrap();
        let path = dropped["path"].as_str().unwrap().to_owned();
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems
                .iter()
                .any(|line| line.starts_with(&path)
                    && line.contains("is declared in native_namespaces")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_documented_surface_the_declaration_omits_is_named() {
        let mut baseline = agreeing();
        baseline[BLOCK]["surfaces"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "path": "a-surface-nobody-declared",
                "kinds": [],
                "shape": "directory",
                "source": "https://example.invalid/docs",
            }));
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems
                .iter()
                .any(|line| line.contains("is not declared in native_namespaces")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_path_recorded_against_another_root_is_refused() {
        // Three places record paths and all three are read relative to the
        // target. A `~` entry is the shape that hid a real gap for a release:
        // it reads as a fact about the product's default home, and the harness
        // test that binds `never_touch` to the declaration skipped it for
        // exactly that reason -- while this provider was redirecting the home
        // and the product was writing the file inside the target.
        for (field, mutate) in [
            (
                "never_touch",
                Box::new(|b: &mut Value| b["never_touch"] = json!(["~/.thing.json"]))
                    as Box<dyn Fn(&mut Value)>,
            ),
            (
                "surfaces",
                Box::new(|b: &mut Value| b[BLOCK]["surfaces"][0]["path"] = json!("/etc/thing")),
            ),
            (
                "declined",
                Box::new(|b: &mut Value| {
                    b[BLOCK]["declined"] = json!([{
                        "path": "~/thing",
                        "reason": "measured, and recorded against the wrong root",
                        "source": "https://example.invalid/docs",
                    }]);
                }),
            ),
        ] {
            let mut baseline = agreeing();
            mutate(&mut baseline);
            let problems = disagreements(&TEST, &baseline);
            assert!(
                problems
                    .iter()
                    .any(|line| line.contains("relative to a root this provider never")),
                "{field}: {problems:?}"
            );
        }
    }

    #[test]
    fn a_surface_with_no_source_is_not_owned() {
        let mut baseline = agreeing();
        baseline[BLOCK]["surfaces"][0]["source"] = json!("");
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems.iter().any(|line| line.contains("cites no source")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_declared_kind_no_surface_routes_is_named() {
        let mut baseline = agreeing();
        baseline[BLOCK]["surfaces"][0]["kinds"] = json!([]);
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems
                .iter()
                .any(|line| line.contains("promise of a rollback")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_routed_kind_the_declaration_omits_is_named() {
        let mut baseline = agreeing();
        // Whichever kind this harness does not declare; there is always one,
        // because no harness declares all eight.
        let absent = ComponentKind::ALL
            .iter()
            .find(|kind| !TEST.component_kinds.contains(kind))
            .unwrap();
        baseline[BLOCK]["surfaces"][0]["kinds"]
            .as_array_mut()
            .unwrap()
            .push(json!(absent.as_str()));
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems.iter().any(|line| line.contains("is refused")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_path_that_is_owned_and_declined_at_once_is_named() {
        let mut baseline = agreeing();
        baseline[BLOCK]["declined"] = json!([{
            "path": TEST.native_namespaces[0],
            "reason": "a reason",
            "source": "https://example.invalid/docs",
        }]);
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems
                .iter()
                .any(|line| line.contains("declined and owned at the same time")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_declined_path_with_no_reason_is_named() {
        let mut baseline = agreeing();
        baseline[BLOCK]["declined"] = json!([{
            "path": "something-considered",
            "source": "https://example.invalid/docs",
        }]);
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems
                .iter()
                .any(|line| line.contains("carries no reason")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_config_home_that_drifted_from_the_declaration_is_named() {
        let mut baseline = agreeing();
        baseline[BLOCK]["config_home"] = json!("~/somewhere-else");
        let problems = disagreements(&TEST, &baseline);
        assert!(
            problems.iter().any(|line| line.contains("config_home")),
            "{problems:?}"
        );
    }
}
