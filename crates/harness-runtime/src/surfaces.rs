//! The binding between a declaration and the vendor document that decided it.
//!
//! `native_namespaces` says what a provider **owns** inside a target, and every
//! entry of `component_kinds` is a promise of a rollback for components of that
//! kind. Both are published in `provider-info`, which is the authority a
//! consumer plans, verifies and computes target identity against.
//!
//! They were once assembled from a consumer's routing table. Measured against
//! the vendors' pages on 2026-08-27, that had left paths no page documented —
//! `~/.cursor/rules`, `~/.claude/.mcp.json`, `~/.grok/commands` — and omitted
//! paths every page does. Conformance never noticed: its
//! `declared_native_route_is_compilable` case requires **one** declared kind to
//! have a route, not all of them.
//!
//! **Two of those three are real, and finding out took reading the products
//! rather than their pages.** Measured 2026-08-28 against pinned bytes:
//! cursor's own rule-creation code offers a *User Rule* scope whose path is
//! `join(homedir(), ".cursor", "rules")`, hint *"Applies to all your
//! projects"*; grok's own embedded reference lists `~/.grok/commands/` at User
//! tier. Neither page says so, and both products do.
//!
//! So the sentence above is right about pages and was wrong as a conclusion
//! about surfaces, and the removals it justified were right for a reason that
//! did not hold. Both are recorded as declined with the measurement, and
//! whether to own them is a decision rather than an absence — a release and a
//! consumer's routing table, not a record edit.
//!
//! `~/.claude/.mcp.json` is the one of the three that stays removed, and the
//! record was right about it for the right reason. The product names
//! `.mcp.json` in seventy-six places — *"Project config (shared via
//! .mcp.json)"* — and `.claude/.mcp.json` in **none**. MCP servers reach Claude
//! Code through `.claude.json` at user scope and a project-root `.mcp.json` at
//! project scope, which is what the declined row has said all along.
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

/// A credentials file the baseline records and `never_touch` does not name.
///
/// `never_touch` is a safety declaration: `never_captured` is the control
/// directory plus this list, and a slot that held a product's credentials would
/// put them on disk in a second place. The comment beside antigravity's list
/// says it plainly -- *a backup of someone else's credentials is a leak with a
/// schedule*.
///
/// Five of the seven named theirs. Grok and pi did not, and both have one:
/// grok's own embedded reference calls `~/.grok/auth.json` *Authentication
/// credentials (auto-managed)*, and pi joins `agentDir, "auth.json"`. Neither
/// was a live leak, because `capture` walks `native_namespaces` and neither
/// file sat inside one -- which is exactly the kind of safety that holds until
/// somebody widens a declaration.
///
/// So the rule is stated rather than remembered: if the baseline knows of a
/// credentials file, the declaration must disclaim it. The baseline is the only
/// place that knows, because it is where a vendor's page and a product's own
/// reference are written down.
fn credentials_are_disclaimed(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    // Matched on the **path**, never on the row's prose.
    //
    // The first version also searched the `reason` text for "credential", and
    // it fired immediately on antigravity's `transcript.jsonl` -- whose reason
    // says *for the reason the never_touch list gives about a neighbour's
    // credentials*. The word was in a sentence explaining an analogy, and the
    // guard read it as a classification. A guard that matches prose measures
    // how a thing was described rather than what it is, which is the failure
    // this file is otherwise full of examples of.
    //
    // **And this list is a heuristic, which is worth saying rather than
    // implying.** It catches `auth.json`, `.credentials.json` and
    // `oauth_creds.json`; it does not catch antigravity's
    // `google_accounts.json`, which is a credentials file named after neither.
    // That one is disclaimed because a person read the page. The guard is a
    // floor under attention, not a replacement for it.

    let mut named = Vec::new();
    for block in ["surfaces", "declined"] {
        let Some(rows) = baseline
            .get(BLOCK)
            .and_then(|found| found.get(block))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for row in rows {
            let Some(path) = row.get("path").and_then(Value::as_str) else {
                continue;
            };
            let lowered = path.to_lowercase();
            if ["credential", "auth.json", "oauth", "token"]
                .iter()
                .any(|tell| lowered.contains(tell))
            {
                named.push(path.to_owned());
            }
        }
    }

    for path in named {
        // The last component, because `never_touch` names top-level entries.
        let leaf = path.rsplit('/').next().unwrap_or(&path);
        if !harness.never_touch.contains(&leaf) {
            found.push(format!(
                "{path:?} reads as a credentials file and {} does not disclaim it. \
                 never_captured is the control directory plus never_touch, and a slot \
                 holding a product's credentials would put them on disk in a second \
                 place. Add {leaf:?} to never_touch, or say in the row why it is not one.",
                harness.provider_id
            ));
        }
    }
}

/// An owned surface that routes no kind and says nothing about why.
///
/// The mirror of `writes_where_nothing_is_routed`. That one catches a path this
/// provider *writes into* whose surface routes nothing -- the shape that put
/// cursor's `plugin` kind one directory from its own bytes. This catches the
/// other direction: a namespace declared, owned, routing nothing, and written
/// to by nobody.
///
/// Three of them exist: claude's `rules`, opencode's `tui.json`, pi's `themes`.
/// Each is defensible -- claude's row explains that `instruction` already
/// routes to `CLAUDE.md` and the namespace is owned so a setup *could* carry a
/// rule -- and the point is not to remove them. It is that the answer should be
/// written once rather than re-derived by every reader, including the consumer,
/// who asked about two of the three in the same message.
///
/// So: routing nothing is allowed, and being silent about it is not.
/// An administrator's policy, or the signature over one, must not be owned.
///
/// Owning a namespace does three things and only one of them is a benefit here:
/// a backup captures it, an identity hashes it, and **`remove_managed` deletes
/// it**. For a signed policy the third is the one that decides.
///
/// Measured 2026-08-28 on the shipped `grok-setup-system 0.0.11`, against a
/// target holding a managed grok home: `install` removed `managed_config.toml`
/// and `requirements.toml` and **kept** `managed_config.sig.json`,
/// `managed_identity.sig.json` and `managed_config_cache.json`. That is exactly
/// the state the product's own gate refuses -- *"refusing session -- the signed
/// is-managed claim requires an authentic policy sidecar and none is present"*.
/// A `restore` brings the policy back, but the person's next run happens first.
///
/// **Matched on the path, never on the row's prose**, for the reason
/// [`credentials_are_disclaimed`] gives at length: a guard that reads how a
/// thing was described measures the description. A row whose *reason* explains
/// why something is *not* a policy would fire a prose matcher immediately.
///
/// **And this list is a heuristic, said rather than implied.** It catches
/// anything named `managed*` and anything ending `.sig.json`. It does not catch
/// grok's `requirements.toml`, which is an org-enforced fail-closed clamp named
/// after none of that; that one is disclaimed because a person read the page.
/// A floor under attention, not a replacement for it.
fn policy_is_not_owned(harness: &Harness, found: &mut Vec<String>) {
    let owned: Vec<&str> = harness
        .native_namespaces
        .iter()
        .copied()
        .chain(
            harness
                .scoped_projections
                .iter()
                .flat_map(|scope| scope.native_namespaces.iter().copied()),
        )
        .collect();
    for name in owned {
        let leaf = name.rsplit('/').next().unwrap_or(name);
        if leaf.starts_with("managed") || leaf.ends_with(".sig.json") {
            found.push(format!(
                "{name:?} is declared in native_namespaces and reads as an \
                 administrator's policy or the signature over one; owning it \
                 means `remove` deletes it, which on a managed machine leaves \
                 a signature with nothing to verify"
            ));
        }
    }
}

fn silent_about_routing_nothing(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    let Some(rows) = baseline
        .get(BLOCK)
        .and_then(|block| block.get("surfaces"))
        .and_then(Value::as_array)
    else {
        return;
    };
    for row in rows {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !harness.native_namespaces.contains(&path) {
            continue;
        }
        let routes = row
            .get("kinds")
            .and_then(Value::as_array)
            .is_some_and(|kinds| !kinds.is_empty());
        let explained = ["note", "reason"].iter().any(|key| {
            row.get(*key)
                .and_then(Value::as_str)
                .is_some_and(|text| !text.trim().is_empty())
        });
        if !routes && !explained {
            found.push(format!(
                "{path:?} is owned and routes no kind, and the row says nothing about why. \
                 That is allowed -- being silent about it is not, because the next reader \
                 re-derives the answer, and the consumer has already asked about two of \
                 these. Add a note."
            ));
        }
    }
}

/// The home a product reads, the variable that moves it, and the condition
/// under which the first is not the whole answer -- each bound to what was
/// measured.
///
/// Split out of [`disagreements`] when it crossed the hundred-line lint, the
/// same way `owned_surfaces` and `declined_rows` were. Three statements about
/// one thing read better together than interleaved with the surface walk.
fn the_home_and_what_moves_it(
    harness: &Harness,
    block: &serde_json::Map<String, Value>,
    found: &mut Vec<String>,
) {
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

    // A conditional home is bound the same way the home is, and for the same
    // reason: this string is printed to a person choosing a `--target`, so a
    // declaration saying one thing while the baseline says another would put a
    // measurement nobody took in front of them.
    match (
        harness.config_home_note,
        block.get("config_home_note").and_then(Value::as_str),
    ) {
        ("", None) => {}
        (declared, Some(recorded)) if declared == recorded => {}
        ("", Some(recorded)) => found.push(format!(
            "{BLOCK}.config_home_note records {recorded:?} and the declaration \
         carries none, so a condition a person is told about is one this \
         build does not state"
        )),
        (declared, None) => found.push(format!(
            "the declaration says the home is conditional ({declared:?}) and \
         {BLOCK} records no config_home_note saying how that was measured"
        )),
        (declared, Some(recorded)) => found.push(format!(
            "{BLOCK}.config_home_note is {recorded:?} and the declaration says \
         {declared:?}"
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
}

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

/// Baseline keys that share a name with a `provider-info` field.
///
/// A baseline records what a *vendor* does; `provider-info` declares what this
/// *provider* does. They are different axes, and a word on both is a trap --
/// `permission_profiles` was one. In `codex-baseline.json` it held the Codex
/// product's own beta permissions feature (`status`, `configuration_key`,
/// `minimum_codex_version_ref`); in `provider-info` it is the protocol's
/// profile list, `["default"]` on all seven. Nothing read the baseline key, so
/// nothing was wrong yet -- which is exactly when a name collision is cheapest
/// to remove and hardest to notice.
///
/// The estate met this pattern once before, from the other side: the consumer's
/// `projection_capabilities` and the kit's `projection_kinds` overlapped in one
/// token and disagreed everywhere else. The conclusion recorded then was that
/// **the shared name is the trap and no amount of documentation survives the
/// next reader**, and the consumer renamed rather than annotated. This is that
/// conclusion enforced rather than remembered.
///
/// The forbidden names are read from the kit's own schema, not listed here. A
/// list here would be a second copy of the contract, drifting the moment the
/// contract gained a field -- and a guard that has to be updated by hand to
/// keep catching things is a guard that stops catching things.
fn shares_a_name_with_the_protocol(baseline: &Value, found: &mut Vec<String>) {
    // The bytes this program verifies against `SHA256SUMS` before reading, so
    // the forbidden names are the contract's own rather than a copy of them.
    const SCHEMA: &str = include_str!("../../../provider-kit/v3/provider-info.schema.json");

    let Ok(schema) = serde_json::from_str::<Value>(SCHEMA) else {
        found.push(
            "the kit's provider-info schema is unreadable, so no baseline name could be \
             checked against it -- this guard measured nothing"
                .to_owned(),
        );
        return;
    };
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        found.push(
            "the kit's provider-info schema has no properties object, so no name could be \
             checked against it -- this guard measured nothing"
                .to_owned(),
        );
        return;
    };
    let Some(keys) = baseline.as_object() else {
        found.push("the baseline is not an object, so its keys could not be read".to_owned());
        return;
    };
    for name in keys.keys() {
        if properties.contains_key(name) {
            found.push(format!(
                "the baseline key {name:?} is also a provider-info field. A baseline records \
                 what the vendor does and provider-info declares what this provider does, so \
                 one word on two axes will be bound together by somebody. Rename the baseline \
                 key -- product_{name} reads correctly -- rather than documenting the \
                 collision."
            ));
        }
    }
}

/// A surface this provider's own setups write into, which routes no kind.
///
/// The declaration says which paths are owned; the baseline says which *kind*
/// each owned path routes. Nothing compared either with the third fact -- where
/// the bytes this provider ships actually land -- and a kind sitting one
/// directory away from its own files is invisible to every check that does not
/// make that comparison.
///
/// It was invisible twice, in the same shape:
///
/// - **cursor.** `0.0.8` moved the plugin from `plugins` to `plugins/local`,
///   which is the path the vendor names -- *"Create a folder for your plugin:
///   `~/.cursor/plugins/local/my-plugin`"* -- and left `kinds: ["plugin"]` on
///   the parent. Twenty-six shipped files, two releases, no test.
/// - **antigravity.** `kinds: ["plugin"]` sat on `antigravity-cli/plugins`
///   while every shipped plugin file went to `config/plugins`. Measured against
///   the pinned `1.1.22` bytes in a contained home: the product's own
///   `plugin install` creates `config/plugins/<name>` and never the other, and
///   `antigravity-cli/plugins` appears nowhere in the binary.
///
/// Both were declared, both were sourced, both routed a kind from *some* owned
/// surface, so `check_within` and the surface guard passed on each. The
/// consumer could not catch it either: a provider declaring both a right and a
/// wrong namespace makes their cross-check against `provider-info` pass on the
/// wrong row, which is how one of their routing rules stayed wrong for a month.
///
/// So this asks the question neither side was asking: **a path this provider
/// writes must land on a surface that routes something.** A surface we write
/// into and route nothing through is either a mis-recorded row or a write into
/// a directory that means nothing, and both are worth a red.
///
/// The paths come from `embedded_setups`, which `build.rs` generates from the
/// setups directory -- so this measures what the binary *ships*, not what a
/// directory happened to hold when a test ran.
fn writes_where_nothing_is_routed(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    let Some(rows) = baseline
        .get(BLOCK)
        .and_then(|block| block.get("surfaces"))
        .and_then(Value::as_array)
    else {
        // Said rather than skipped: a missing block is already reported by the
        // caller, and silently measuring nothing here would let this guard read
        // as green on a baseline it never opened.
        return;
    };

    for (embedded, _) in harness.embedded_setups {
        // `<setup id>/home/<path the target receives>`.
        let Some((_, relative)) = embedded.split_once("/home/") else {
            continue;
        };
        let mut owner: Option<(&str, usize)> = None;
        for row in rows {
            let Some(path) = row.get("path").and_then(Value::as_str) else {
                continue;
            };
            let covered = relative == path
                || relative
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'));
            if covered && owner.is_none_or(|(_, len)| path.len() > len) {
                owner = Some((path, path.len()));
            }
        }
        let Some((path, _)) = owner else {
            found.push(format!(
                "the setup file {relative:?} lands on no surface {BLOCK} records, so \
                 nothing says what it is or where it was read from"
            ));
            continue;
        };
        let routes = rows
            .iter()
            .find(|row| row.get("path").and_then(Value::as_str) == Some(path))
            .and_then(|row| row.get("kinds"))
            .and_then(Value::as_array)
            .is_some_and(|kinds| !kinds.is_empty());
        if !routes {
            found.push(format!(
                "the setup file {relative:?} lands in {path:?}, which routes no kind. Either \
                 the kind belongs on this row rather than on a neighbour, or these bytes are \
                 being written somewhere the product reads nothing from."
            ));
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
    shares_a_name_with_the_protocol(baseline, &mut found);
    credentials_are_disclaimed(harness, baseline, &mut found);
    policy_is_not_owned(harness, &mut found);
    silent_about_routing_nothing(harness, baseline, &mut found);
    writes_where_nothing_is_routed(harness, baseline, &mut found);

    let Some(block) = baseline.get(BLOCK).and_then(Value::as_object) else {
        found.push(format!(
            "{} has no {BLOCK} object, so nothing this harness claims to own is sourced",
            harness.provider_id
        ));
        return found;
    };

    the_home_and_what_moves_it(harness, block, &mut found);

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
                    // The rows carrying no kind need one, because
                    // `silent_about_routing_nothing` refuses an owned surface
                    // that routes nothing and says nothing. A fixture standing
                    // for a real baseline has to satisfy the same rules the
                    // real ones do, or it stops standing for anything.
                    "note": if kinds.is_empty() {
                        "owned so a backup returns it; no kind describes it"
                    } else {
                        "routes the kinds this fixture declares"
                    },
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

    /// A signed policy in the owned set is refused, by its name.
    ///
    /// The defect this is written against shipped: `grok-setup-system 0.0.11`
    /// owned `managed_config.toml`, so `install` deleted an administrator's
    /// signed policy and kept its signature sidecars -- the state the product's
    /// own gate refuses. Measured on the shipped binary before the guard
    /// existed.
    #[test]
    fn an_administrators_policy_in_the_owned_set_is_refused() {
        let mut found = Vec::new();
        let mut harness = TEST;
        harness.native_namespaces = &["settings.json", "managed_config.toml"];
        policy_is_not_owned(&harness, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("managed_config.toml"), "{found:?}");
        assert!(found[0].contains("signature"), "{found:?}");

        // The signature over one is refused on the same ground, and so is a
        // nested path whose leaf carries the name.
        //
        // `policy.sig.json` is here because the first version of this test
        // could not tell the two branches apart: every signature it named also
        // began with `managed`, so deleting the suffix branch left the test
        // green. A case only the suffix can catch is what makes it a test of
        // two rules rather than of one.
        for owned in [
            &["settings.json", "managed_identity.sig.json"][..],
            &["settings.json", "policy.sig.json"][..],
            &["settings.json", "config/managed_config_cache.json"][..],
        ] {
            let mut nested = Vec::new();
            let mut probe = TEST;
            probe.native_namespaces = owned;
            policy_is_not_owned(&probe, &mut nested);
            assert_eq!(nested.len(), 1, "{owned:?} -> {nested:?}");
        }

        // And a name that merely begins like one is kept: the guard reads the
        // leaf, and `manage.json` is not a policy.
        let mut kept = Vec::new();
        let mut ordinary = TEST;
        ordinary.native_namespaces = &["settings.json", "manage.json", "signals.json"];
        policy_is_not_owned(&ordinary, &mut kept);
        assert!(kept.is_empty(), "{kept:?}");
    }
}
