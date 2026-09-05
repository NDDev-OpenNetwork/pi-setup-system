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
/// Owning a namespace still means a backup can capture it and an identity can
/// hash it as ours. Default `remove_managed` no longer deletes the namespace
/// whole -- it withdraws recorded files -- so a signed policy must not be owned
/// even after that change: capture and identity would still treat it as ours.
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
/// Two owned namespaces a case-insensitive filesystem would merge.
///
/// The declaration is one string set for three operating systems, and two of
/// them fold case. `skills` and `Skills` are two namespaces on Linux and one on
/// macOS and Windows, so the same declaration would mean different things per
/// platform: `remove_managed` would walk one directory twice,
/// `digest::of_owned` reduces by string prefix and would hash the same tree
/// under two names, and a bundle routed to either would land in whichever the
/// filesystem chose.
///
/// The scoped namespaces are folded in with the global ones rather than checked
/// separately, because a filesystem does not know about scopes — if a scoped
/// `Skills` sat beside a global `skills` under one root, they would still be
/// one directory.
/// The file whose grammar was measured must be one this build owns.
///
/// `configuration_format` records what a product's configuration file *is* --
/// JSON, JSONC or TOML, whether the parser takes comments, and the vendor's
/// schema where one exists. Seven measurements that would otherwise live only
/// in a plan, which is where the previous seven ended up before being
/// re-derived.
///
/// Bound rather than merely written, because a record naming a path this build
/// does not own is a measurement about somebody else's file. It cannot check
/// the *grammar* -- this build has no parser for three of them and asserting
/// one would be the claim-without-a-source this estate keeps removing -- so it
/// checks the one thing it can: that the file named is in the declaration.
fn the_measured_format_names_an_owned_file(
    harness: &Harness,
    block: &serde_json::Map<String, Value>,
    found: &mut Vec<String>,
) {
    let Some(format) = block.get("configuration_format") else {
        found.push(format!(
            "{BLOCK} records no configuration_format, so what a file at the owned \
             configuration path *is* -- its grammar, whether it takes comments, whether a \
             vendor publishes a schema -- is a measurement nobody kept"
        ));
        return;
    };
    let Some(named) = format.get("file").and_then(Value::as_str) else {
        found.push(format!("{BLOCK}.configuration_format names no file"));
        return;
    };
    if !harness.native_namespaces.contains(&named) {
        found.push(format!(
            "{BLOCK}.configuration_format measures {named:?}, which is not in \
             native_namespaces -- a grammar recorded for a file this build does not own is a \
             measurement about somebody else's"
        ));
    }
    for key in ["grammar", "note"] {
        if format
            .get(key)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            found.push(format!("{BLOCK}.configuration_format has no {key}"));
        }
    }
    if format
        .get("accepts_comments")
        .and_then(Value::as_bool)
        .is_none()
    {
        found.push(format!(
            "{BLOCK}.configuration_format does not say whether the parser accepts comments, \
             and absent is not the same answer as false"
        ));
    }
}

fn owned_paths_fold_together(harness: &Harness, found: &mut Vec<String>) {
    let mut folded: std::collections::BTreeMap<String, &str> = std::collections::BTreeMap::new();
    let owned = harness.native_namespaces.iter().copied().chain(
        harness
            .scoped_projections
            .iter()
            .flat_map(|scope| scope.native_namespaces.iter().copied()),
    );
    for name in owned {
        if let Some(other) = folded.insert(name.to_lowercase(), name)
            && other != name
        {
            found.push(format!(
                "{name:?} and {other:?} are both owned and differ only in case, so they are one \
                 path on macOS and Windows and two on Linux"
            ));
        }
    }
}

/// A scope's namespaces must not be the global set.
///
/// `status` takes a target and no scope — that is the argv contract — so it
/// recovers the scope from `native_ownership`, the namespaces the state records
/// as owned *here*. That recovery is a lookup, and a lookup needs its keys to be
/// distinct: a scoped profile declaring exactly the global set would make a
/// plain managed target read as scoped, and `status` would then answer about the
/// wrong inventory for a target nobody operated under a scope.
///
/// None of the seven does this, and the reason to state it is that the recovery
/// *depends* on none of them doing it. An invariant a reader has to notice is an
/// invariant somebody removes.
fn a_scope_is_distinguishable_from_the_global_target(harness: &Harness, found: &mut Vec<String>) {
    for scoped in harness.scoped_projections {
        let same = scoped.native_namespaces.len() == harness.native_namespaces.len()
            && scoped
                .native_namespaces
                .iter()
                .all(|name| harness.native_namespaces.contains(name));
        if same {
            found.push(format!(
                "the {} scope owns exactly the namespaces the global target owns, so a                  managed global target reads back as scoped and `status` answers about the                  wrong inventory. Declare what the scope's own root holds.",
                scoped.target_scope.as_str()
            ));
        }
    }
}

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

/// Every owned surface row in a baseline, with the namespaces it is judged by.
///
/// Both guards below walked `surfaces` alone while two harnesses declared a
/// second target. Codex has published `user_root` and antigravity `project` for
/// weeks, and their scoped rows were checked by neither: codex's carried no
/// `evidence` field at all and `evidence_is_recorded` stayed green, because it
/// was green about the rows it looked at and silent about the rest.
///
/// A row is matched against the namespaces of *its own block*: a scoped path is
/// relative to that scope's root and is not a member of the global set, so
/// judging it by the global one would skip every scoped row on the way in.
fn owned_rows<'a>(
    harness: &Harness,
    block: &'a Value,
) -> Vec<(&'a Value, &'static [&'static str])> {
    let mut rows: Vec<(&Value, &'static [&'static str])> = Vec::new();
    if let Some(global) = block.get("surfaces").and_then(Value::as_array) {
        rows.extend(global.iter().map(|row| (row, harness.native_namespaces)));
    }
    for scope in block
        .get("scoped")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
    {
        let Some(named) = scope.get("target_scope").and_then(Value::as_str) else {
            continue;
        };
        let Some(declared) = harness
            .scoped_projections
            .iter()
            .find(|scoped| scoped.target_scope.as_str() == named)
        else {
            continue;
        };
        if let Some(entries) = scope.get("surfaces").and_then(Value::as_array) {
            rows.extend(entries.iter().map(|row| (row, declared.native_namespaces)));
        }
    }
    rows
}

fn silent_about_routing_nothing(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    let Some(block) = baseline.get(BLOCK) else {
        return;
    };
    // Every owned row, global and scoped: `owned_rows` says why both guards
    // needed the second half.
    for (row, owned) in owned_rows(harness, block) {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !owned.contains(&path) {
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
/// Every declared projection kind names an owned surface it unpacks into.
///
/// The component-kind half of this rule has been enforced since the surfaces
/// block existed -- *a declared kind is a promise of a rollback*. The
/// projection half was not, and two harnesses were declaring package families
/// they could not deliver: one whose plugins come from a hosted directory with
/// nothing under the home to hold them, and one that declared `marketplace`
/// while every path a marketplace is registered in sat in its own **declined**
/// list.
///
/// **A declined path cannot serve as the name, and it needs no clause of its
/// own.** That was going to be a second condition, because the case that
/// motivated it names four declined paths; asking the *owned* set excludes them
/// already, which the control confirmed by reporting `plugins/marketplaces` as
/// a path the harness does not own rather than as one it declined. One
/// condition where two were planned, and the second was planned from reasoning
/// rather than from running it.
///
/// A declaration with one real half and one naming nothing is worse than two
/// wrong halves carrying a note, because the note is the thing that gets
/// re-read.
///
/// `native_files` needs no basis: it is the default every provider writes, and
/// requiring it to name one surface would ask which of several. Everything else
/// names a directory the provider unpacks a package into, per the consumer's
/// own definition of the field: *"native packaging selected by the target
/// provider … another package family"*. Where the landing place is a file the
/// provider already owns rather than a package it unpacks -- registering a
/// marketplace by writing a settings key -- the basis names that file, and the
/// distinction is the consumer's: a landing place you already own is a
/// `setting` contribution, and a projection kind names a package.
fn every_projection_names_where_it_lands(
    harness: &Harness,
    block: &serde_json::Map<String, Value>,
    owned: &[Owned<'_>],
    found: &mut Vec<String>,
) {
    let basis = block.get("projection_basis").and_then(Value::as_object);
    let declared: Vec<&str> = harness
        .projection_kinds
        .iter()
        .map(|kind| kind.as_str())
        .filter(|kind| *kind != "native_files")
        .collect();

    let Some(basis) = basis else {
        if !declared.is_empty() {
            found.push(format!(
                "{} declares {declared:?} and {BLOCK} has no projection_basis, so \
                 nothing says where a package of that family would land",
                harness.provider_id
            ));
        }
        return;
    };

    for kind in &declared {
        let Some(path) = basis.get(*kind).and_then(Value::as_str) else {
            found.push(format!(
                "{kind} is a declared projection and projection_basis names no surface \
                 for it; a package family with nowhere to unpack is a promise nothing \
                 can keep"
            ));
            continue;
        };
        if !owned.iter().any(|surface| surface.path == path) {
            found.push(format!(
                "{kind} is declared to land in {path:?}, which this harness does not own"
            ));
        }
    }

    for (kind, _) in basis {
        if !declared.contains(&kind.as_str()) {
            found.push(format!(
                "projection_basis names {kind} and the declaration does not, so the \
                 basis is describing a family this provider does not offer"
            ));
        }
    }
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
        if routes {
            continue;
        }
        // One product reaches a file here by an explicit pointer from another
        // owned file rather than by scanning the directory, so bytes can land
        // in a namespace that routes no kind and still be read. A row says so
        // with `reached_by`, naming the file that does the pointing.
        //
        // The exception is deliberately stronger than the rule it excuses: it
        // does not take the row's word for it, it opens the named file and
        // requires the path to be in it. A pointer that is claimed and absent
        // is worse than no claim, because it reads as routed and is inert --
        // which is the defect this whole guard exists to catch.
        let reached_by = rows
            .iter()
            .find(|row| row.get("path").and_then(Value::as_str) == Some(path))
            .and_then(|row| row.get("reached_by"))
            .and_then(Value::as_str);
        let Some(pointer) = reached_by else {
            found.push(format!(
                "the setup file {relative:?} lands in {path:?}, which routes no kind. Either \
                 the kind belongs on this row rather than on a neighbour, or these bytes are \
                 being written somewhere the product reads nothing from."
            ));
            continue;
        };
        let setup = embedded.split_once("/home/").map_or("", |(id, _)| id);
        let wanted = format!("{setup}/home/{pointer}");
        let points_at_it = harness
            .embedded_setups
            .iter()
            .find(|(name, _)| *name == wanted)
            .is_some_and(|(_, body)| String::from_utf8_lossy(body).contains(relative));
        if !points_at_it {
            found.push(format!(
                "{path:?} says it is reached by {pointer:?}, and the setup file \
                 {relative:?} is not named in it. A surface that routes no kind is read \
                 only through the pointer that names it, so a pointer that is claimed \
                 and missing leaves these bytes inert while the row reads as routed."
            ));
        }
    }
}

/// What actually exercised a row, which is not the same as what cites it.
///
/// A `source` column that mixes a vendor URL, a URL plus "measured", and a
/// bare "measured in the pinned binary" ranks them by nothing, and a reader
/// takes the URL as the strong one. **That ranking is upside down.** The
/// `agent` route this provider carried for codex had a live vendor page behind
/// it and did not exist; antigravity's `agents` route was correct throughout
/// the weeks its citation answered 404; grok's `personas` came out of a
/// binary's own embedded reference and was right.
///
/// So the axis is whether anybody ran the thing:
///
/// - `ran` -- the product was run and the behaviour observed;
/// - `bytes` -- the product's own shipped bytes were read, an embedded
///   reference or a path literal in the binary;
/// - `page` -- a vendor page, and nothing else.
///
/// `page` is the default when a row records no method, because absence of a
/// record of measurement is not evidence of measurement. It is also, today,
/// most of them -- which is the reason to render it rather than to hide it.
fn evidence_is_recorded(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    const ALLOWED: [&str; 3] = ["ran", "bytes", "page"];
    let Some(block) = baseline.get(BLOCK) else {
        return;
    };
    for (row, owned) in owned_rows(harness, block) {
        let Some(path) = row.get("path").and_then(Value::as_str) else {
            continue;
        };
        if !owned.contains(&path) {
            continue;
        }
        match row.get("evidence").and_then(Value::as_str) {
            Some(value) if ALLOWED.contains(&value) => {}
            Some(value) => found.push(format!(
                "{path:?} records evidence {value:?}, which is not one of \
                 `ran`, `bytes` or `page`"
            )),
            None => found.push(format!(
                "{path:?} is owned and does not say what exercised it. Add \
                 `evidence`: `ran` if the product was run and the behaviour \
                 observed, `bytes` if its own shipped bytes were read, `page` \
                 if a vendor page is all there is. A citation is not a \
                 measurement, and the weakest of the three is the one a reader \
                 mistakes for the strongest."
            )),
        }
    }
}

/// A baseline may not record a gap it no longer has.
///
/// `second_pin_absent` exists so a reader can tell a gap from a fact: a provider
/// pinning only one release has nothing for a real `software_update` to cross.
/// The block says so, and says how it will fill -- normally by rotation on the
/// vendor's next bump, or by recovering an earlier exact measurement already
/// present in this repository.
///
/// It filled. Codex's next bump assigned `previous = 0.150.1` exactly as
/// predicted, and the block stayed, so the file both carried a second pin and
/// stated it had none. The prediction was right and nothing read it: a note that
/// describes its own expiry cannot notice the expiry arriving.
///
/// The rule is the smallest one that holds: an absence recorded beside the thing
/// it denies is a contradiction inside one file, and no vendor has to be asked
/// to see it.
fn an_absence_is_not_recorded_beside_the_thing(baseline: &Value, found: &mut Vec<String>) {
    let claims_absent = baseline.get("second_pin_absent").is_some();
    let carries_one = baseline
        .get("previous_software_artifacts")
        .and_then(|previous| previous.get("version"))
        .and_then(Value::as_str)
        .is_some_and(|version| !version.is_empty());
    if claims_absent && carries_one {
        found.push(
            "second_pin_absent records that there is no second release to cross, and \
             previous_software_artifacts names one. The gap it describes has closed: \
             remove the block rather than editing it, because what it says was true and \
             is now a statement about a different day."
                .to_owned(),
        );
    }
}
/// Whether [`Harness::custody_namespaces`] is exactly what nothing can fill.
///
/// A namespace is *fillable* when a component kind routes to it, or when a
/// setup in this build's catalogue carries a file under it. Selecting a setup
/// empties every owned namespace and refills it from the payload, so for a
/// fillable one an empty payload is a statement the posture is entitled to
/// make: *there is nothing here*.
///
/// For the rest it is not. Nothing the provider can install ever lands there,
/// every posture agrees, and the only content is somebody else's -- so the
/// emptying was an opinion no setup held. Twelve of them existed across five
/// harnesses when this was measured, and a `select minimal` took a person's
/// keybindings out of one.
///
/// Checked in **both** directions, because a list like this fails in both: an
/// entry that became fillable would stop being emptied when it should be, and
/// one that stopped being filled would start being emptied when it should not.
/// Neither would be visible in the declaration.
fn custody_is_what_nothing_can_fill(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    let Some(rows) = baseline
        .get(BLOCK)
        .and_then(|block| block.get("surfaces"))
        .and_then(Value::as_array)
    else {
        return;
    };
    // A build with no catalogue cannot answer what is fillable: every namespace
    // would read as unfillable because no setup exists to fill it. Test
    // fixtures are the only builds in that state, and a guard that reports on
    // one is reporting about the fixture rather than about a harness.
    if harness.embedded_setups.is_empty() {
        return;
    }
    let under = |namespace: &str, relative: &str| {
        relative == namespace
            || relative
                .strip_prefix(namespace)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    for namespace in harness.native_namespaces {
        let routes = rows.iter().any(|row| {
            row.get("path").and_then(Value::as_str) == Some(*namespace)
                && row
                    .get("kinds")
                    .and_then(Value::as_array)
                    .is_some_and(|kinds| !kinds.is_empty())
        });
        let filled = harness.embedded_setups.iter().any(|(embedded, _)| {
            embedded
                .split_once("/home/")
                .is_some_and(|(_, relative)| under(namespace, relative))
        });
        let listed = harness.custody_namespaces.contains(namespace);
        if listed && (routes || filled) {
            found.push(format!(
                "{namespace:?} is declared as custody and something can fill it: \
                 {}. A posture is entitled to say a fillable namespace is empty.",
                if routes {
                    "a kind routes there"
                } else {
                    "a setup carries files there"
                }
            ));
        }
        if !listed && !routes && !filled {
            found.push(format!(
                "{namespace:?} is owned, routes no kind and no setup fills it, and is \
                 not declared as custody -- so selecting any posture empties it, and \
                 the only thing ever in it is somebody else's"
            ));
        }
    }
    for namespace in harness.custody_namespaces {
        if !harness.native_namespaces.contains(namespace) {
            found.push(format!(
                "{namespace:?} is declared as custody and is not owned at all"
            ));
        }
    }
}

/// Whether the update switch a launch forces is one the baseline measured.
///
/// `updates_off_env` puts a literal into every provider-managed launch. A name
/// one letter from the real one -- `DISABLE_AUTOUPDATER` where the product
/// reads `DISABLE_AUTOUPDATE`, or the reverse -- looks right, changes nothing,
/// and leaves the product free to replace bytes this provider pinned, recorded
/// the digest of, and offers a rollback beside.
///
/// The list it is checked against exists to hold what was read out of a pinned
/// artifact. It had no reader until 2026-08-31, which is the condition a stale
/// fact needs: the `windows` host row sat under `unsupported` for weeks while
/// the provider installed Windows, for exactly that reason.
fn the_switch_it_sets_was_measured(harness: &Harness, baseline: &Value, found: &mut Vec<String>) {
    if harness.updates_off_env.is_empty() {
        return;
    }
    let measured = baseline
        .get("source_verified_runtime_flags")
        .and_then(Value::as_array)
        .map(|flags| {
            flags
                .iter()
                .filter_map(Value::as_str)
                .any(|flag| flag == harness.updates_off_env)
        });
    match measured {
        Some(true) => {}
        Some(false) => found.push(format!(
            "launch sets {} and the baseline's measured runtime flags do not name it",
            harness.updates_off_env
        )),
        None => found.push(format!(
            "launch sets {} and this baseline records no source-verified runtime flags at all",
            harness.updates_off_env
        )),
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
    a_scope_is_distinguishable_from_the_global_target(harness, &mut found);
    owned_paths_fold_together(harness, &mut found);
    silent_about_routing_nothing(harness, baseline, &mut found);
    writes_where_nothing_is_routed(harness, baseline, &mut found);
    custody_is_what_nothing_can_fill(harness, baseline, &mut found);
    the_switch_it_sets_was_measured(harness, baseline, &mut found);
    evidence_is_recorded(harness, baseline, &mut found);
    an_absence_is_not_recorded_beside_the_thing(baseline, &mut found);

    let Some(block) = baseline.get(BLOCK).and_then(Value::as_object) else {
        found.push(format!(
            "{} has no {BLOCK} object, so nothing this harness claims to own is sourced",
            harness.provider_id
        ));
        return found;
    };

    the_home_and_what_moves_it(harness, block, &mut found);
    the_measured_format_names_an_owned_file(harness, block, &mut found);

    let Some(surfaces) = block.get("surfaces").and_then(Value::as_array) else {
        found.push(format!("{BLOCK}.surfaces is missing or not an array"));
        return found;
    };
    // Every row is checked before any set is compared, so a malformed row is
    // reported as itself rather than as a missing namespace somewhere else.
    let owned = owned_surfaces(surfaces, &mut found);
    against_declaration(harness, &owned, &mut found);
    every_projection_names_where_it_lands(harness, block, &owned, &mut found);

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
                    // A fixture standing for a real baseline satisfies
                    // the same rules the real ones do. `page` is what a
                    // row with only a URL behind it gets, and that is
                    // what this one has.
                    "evidence": "page",
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
                // The fixture has to satisfy every rule a real baseline does,
                // or it stops standing for one. `settings.json` is the first
                // of TEST's owned namespaces, so this names a file the
                // declaration really carries.
                "configuration_format": {
                    "file": TEST.native_namespaces[0],
                    "grammar": "json",
                    "accepts_comments": false,
                    "note": "measured by reading the product",
                },
                "surfaces": surfaces,
                // The second target this harness declares. A fixture standing
                // for a real baseline has to source every scope the harness
                // publishes, because a scope declared and unsourced is exactly
                // the state this guard exists to name.
                "scoped": [
                    {
                        "target_scope": "user_root",
                        "root": "~/.agents",
                        "surfaces": [
                            {
                                "path": "shared",
                                "kinds": ["skill"],
                                "shape": "directory",
                                "source": "this fixture's own contract",
                                "evidence": "page",
                                "note": "the scoped namespace, relative to the scope's own root",
                            },
                        ],
                        "declined": [],
                    },
                ],
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

    /// Two owned namespaces differing only in case are refused.
    ///
    /// The pair is one path on macOS and Windows and two on Linux, so the same
    /// declaration would mean different things per platform.
    /// Observed failing on a scope that owns exactly what the global target does.
    #[test]
    fn a_scope_that_owns_the_global_set_is_refused() {
        let mut clean = Vec::new();
        a_scope_is_distinguishable_from_the_global_target(&TEST, &mut clean);
        assert_eq!(clean, Vec::<String>::new(), "{clean:?}");

        let mut harness = TEST;
        harness.scoped_projections = &[crate::facts::Scoped {
            target_scope: provider_v3::TargetScope::UserRoot,
            profile_id: "test/native-files/user-root/1",
            component_kinds: &[],
            projection_kinds: &[],
            // Reordered as well as equal, because the check is on the set: a
            // permutation is the same declaration and would fool a comparison
            // written on the slices.
            native_namespaces: &["skills", "AGENTS.md", "settings.json"],
        }];
        let mut found = Vec::new();
        a_scope_is_distinguishable_from_the_global_target(&harness, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("reads back as scoped"), "{found:?}");
    }

    #[test]
    fn two_owned_paths_that_fold_together_are_refused() {
        let mut found = Vec::new();
        let mut harness = TEST;
        harness.native_namespaces = &["settings.json", "skills", "Skills"];
        owned_paths_fold_together(&harness, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("Skills"), "{found:?}");

        // A scoped namespace folds against a global one, because a filesystem
        // does not know about scopes.
        let mut across = Vec::new();
        let mut scoped = TEST;
        scoped.native_namespaces = &["skills"];
        scoped.scoped_projections = &[crate::facts::Scoped {
            target_scope: provider_v3::TargetScope::Project,
            profile_id: "test/native-files/project/1",
            component_kinds: &[],
            projection_kinds: &[],
            native_namespaces: &["SKILLS"],
        }];
        owned_paths_fold_together(&scoped, &mut across);
        assert_eq!(across.len(), 1, "{across:?}");

        // And names that merely share a prefix are kept: the rule folds case,
        // it does not merge neighbours.
        let mut kept = Vec::new();
        let mut ordinary = TEST;
        ordinary.native_namespaces = &["skills", "skills-extra", "agents"];
        owned_paths_fold_together(&ordinary, &mut kept);
        assert!(kept.is_empty(), "{kept:?}");
    }

    /// One product reaches an agent's file through a pointer in its settings
    /// rather than by scanning the directory, so `reached_by` lets a namespace
    /// hold setup files while routing no kind. The exception has to be worth
    /// more than the rule it excuses, so it is observed failing here on the
    /// case that makes it dangerous: the claim present and the pointer absent,
    /// which reads as routed and is inert.
    #[test]
    fn a_claimed_pointer_that_names_nothing_is_refused() {
        const NAMES_IT: &[(&str, &[u8])] = &[
            ("kit/home/skills/role.toml", b"the layer"),
            ("kit/home/AGENTS.md", b"config_file = \"skills/role.toml\""),
        ];
        const NAMES_NOTHING: &[(&str, &[u8])] = &[
            ("kit/home/skills/role.toml", b"the layer"),
            (
                "kit/home/AGENTS.md",
                b"a settings file that forgot to point",
            ),
        ];

        let mut baseline = agreeing();
        let rows = baseline["native_surfaces"]["surfaces"]
            .as_array_mut()
            .unwrap();
        for row in rows.iter_mut() {
            if row["path"] == "skills" {
                row["reached_by"] = json!("AGENTS.md");
            }
        }

        let mut pointing = TEST;
        pointing.embedded_setups = NAMES_IT;
        let mut found = Vec::new();
        writes_where_nothing_is_routed(&pointing, &baseline, &mut found);
        assert!(
            found.is_empty(),
            "a pointer that names it is allowed: {found:?}"
        );

        let mut blind = TEST;
        blind.embedded_setups = NAMES_NOTHING;
        let mut found = Vec::new();
        writes_where_nothing_is_routed(&blind, &baseline, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("is not named in it"), "{found:?}");

        // And without the claim at all, the original complaint still stands --
        // widening the guard must not have quietly opened the gate.
        let mut plain = agreeing();
        let rows = plain["native_surfaces"]["surfaces"].as_array_mut().unwrap();
        for row in rows.iter_mut() {
            if row["path"] == "skills" {
                row.as_object_mut().unwrap().remove("reached_by");
            }
        }
        let mut found = Vec::new();
        writes_where_nothing_is_routed(&blind, &plain, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("routes no kind"), "{found:?}");
    }

    /// A citation is not a measurement, and the column that used to carry both
    /// ranked them by nothing. Observed failing on the row that says where it
    /// came from and not whether anybody ran it.
    #[test]
    fn an_owned_row_that_does_not_say_what_exercised_it_is_refused() {
        let mut baseline = agreeing();
        baseline["native_surfaces"]["surfaces"][0]
            .as_object_mut()
            .unwrap()
            .remove("evidence");
        let mut found = Vec::new();
        evidence_is_recorded(&TEST, &baseline, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("what exercised it"), "{found:?}");

        // And the same row in the *scoped* block, which this guard did not
        // read at all while two harnesses declared a second target. Six rows
        // across codex and antigravity carried no `evidence` and it stayed
        // green -- green about the rows it looked at and silent about the rest.
        let mut baseline = agreeing();
        baseline["native_surfaces"]["scoped"][0]["surfaces"][0]
            .as_object_mut()
            .unwrap()
            .remove("evidence");
        let mut found = Vec::new();
        evidence_is_recorded(&TEST, &baseline, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("\"shared\""), "{found:?}");

        // A value outside the three is refused too, so the field cannot become
        // free text meaning whatever the writer felt.
        let mut baseline = agreeing();
        baseline["native_surfaces"]["surfaces"][0]["evidence"] = json!("documented");
        let mut found = Vec::new();
        evidence_is_recorded(&TEST, &baseline, &mut found);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("not one of"), "{found:?}");

        // And the agreeing fixture, untouched, says nothing.
        let mut found = Vec::new();
        evidence_is_recorded(&TEST, &agreeing(), &mut found);
        assert!(found.is_empty(), "{found:?}");
    }
}
