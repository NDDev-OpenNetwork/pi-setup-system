//! The local setup catalog: complete harness states, on disk, beside the binary.
//!
//! A setup here is the whole thing — the system-prompt components and the
//! configuration together, as a verbatim tree. That is what makes selecting one
//! and restoring one mean the same kind of thing: both put a known complete
//! state into the target, rather than adjusting part of it and leaving the rest
//! wherever the last change left it.
//!
//! ```text
//! setups/
//!   <setup-id>/
//!     setup.json    identity and description
//!     home/         copied verbatim into the target
//! ```
//!
//! This is one of the three sources the design admits. The other two — a setup
//! compiled by ai-stp, and a set of ai-stp components — arrive as a
//! `HarnessBundle` over the wire. All three converge on the same immutable
//! definition before any plan is made, so nothing downstream needs to know which
//! one produced it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize};
use setup_core::digest;

use crate::facts::Harness;
use provider_v3::{Error, Result, WireReason};

/// The catalog directory name beside the executable or repository root.
pub const CATALOG_DIRECTORY: &str = "setups";

/// The per-setup manifest file.
pub const SETUP_MANIFEST: &str = "setup.json";

/// The subdirectory copied verbatim into a target.
pub const SETUP_PAYLOAD: &str = "home";

/// The schema this build writes and is willing to read.
pub const SETUP_SCHEMA: u32 = 1;

/// What one setup says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupManifest {
    /// Schema of this manifest.
    pub schema_version: u32,
    /// The setup identity, matching its directory name.
    pub id: String,
    /// One line on what this setup is for.
    pub description: String,
    /// The vendor pages that decided the format of what this setup writes.
    ///
    /// A setup's payload is *content in the product's own language*: a TOML
    /// table the product reads, a JSON key it looks for. Getting the surface
    /// right and then writing the wrong key into it produces a target that
    /// looks configured and is not — the same failure as owning a path nothing
    /// reads, one level down.
    ///
    /// Measured 2026-08-27, two of the seven were exactly that. opencode's
    /// `permission` took a bare string where the product documents an object,
    /// and antigravity's file set `toolPermissions` where the product reads
    /// `toolPermission` with a closed set of four values that does not include
    /// the one we wrote. Both were valid JSON, both installed cleanly, and
    /// neither changed anything about the product.
    ///
    /// Empty for a setup that writes only documents — a `CLAUDE.md` has no
    /// schema to get wrong. [`unsourced`] holds the rest.
    #[serde(default)]
    pub sources: Vec<String>,
}

/// The setups every harness offers, whatever else it also offers.
///
/// Three postures, and a caller who learns them on one product knows them on
/// all seven. That symmetry is the point of the set being named rather than
/// per-harness: `minimal` means the same thing to someone moving from codex to
/// pi as it did before they moved.
///
/// A harness may carry more -- cursor and antigravity each ship a builder
/// toolkit -- but never fewer.
pub const UNIVERSAL_SETUPS: &[&str] = &["baseline", "full-auto", "minimal"];

/// Every universal setup this catalog does not offer, and every one that is
/// indistinguishable from another.
///
/// The second half matters as much as the first. `full-auto` that installs
/// what `baseline` installs is a posture in name only, and it would read as
/// offered.
#[must_use]
pub fn asymmetric(setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for required in UNIVERSAL_SETUPS {
        if !setups.iter().any(|setup| setup.manifest.id == *required) {
            found.push(format!("this harness offers no {required} setup"));
        }
    }
    for (index, setup) in setups.iter().enumerate() {
        for other in &setups[index + 1..] {
            if setup.definition_digest == other.definition_digest {
                found.push(format!(
                    "{} and {} are the same bytes, so one of them is a posture in name only",
                    setup.manifest.id, other.manifest.id
                ));
            }
        }
    }
    found
}

/// The programs that came before this one, by the names their files carry.
///
/// A **closed historical set**: the frozen estate is frozen, so this list can
/// only shrink. That is what makes a denylist honest here — elsewhere in this
/// repository an allowlist is used precisely because the other set can grow.
const FROZEN_ESTATE: &[&str] = &[
    "nddev_claude_cli",
    "nddev_codex_cli",
    "nddev_cursor_cli",
    "nddev_grok_cli",
    "nddev_opencode_cli",
    "nddev_pi_cli",
    "nddev-antigravity-cli-app",
    "nddev-claude-app",
    "nddev-codex-app",
    "nddev-cursor-cli-app",
    "nddev-grok-build-app",
    "nddev-opencode-app",
    "nddev-pi-app",
    "nddev-harnesses",
    "rldyour-ai-cli-tools",
];

/// Every shipped instruction that names something this program is not.
///
/// Setups carry documents an agent reads and acts on — a skill, a rule, a
/// command file. Those documents are content like any other, and nothing was
/// checking them, so one of them told an agent to run
/// `software-status --target <dir> --json` and `list --json` for six releases.
/// Both are refused by the binary; the second says so in those words.
///
/// Two things are refused here:
///
/// * **A frozen-estate name.** The program that came before this one had
///   different commands and a different model, and a document naming it is
///   describing something a reader cannot run.
/// * **An invocation this binary would not parse.** Any line naming the
///   provider followed by a verb is checked against [`crate::human::VERBS`],
///   which is the list `into_command` itself accepts.
///
/// What it deliberately does not do is judge English. `install` in a sentence
/// is a word; `cursor-setup-system install` is an instruction, and only the
/// second is checked.
#[must_use]
pub fn misdirecting(provider_id: &str, setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for setup in setups {
        for (relative, path) in files_by_path(&setup.payload) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let where_it_is = format!("{}/{relative}", setup.manifest.id);
            for name in FROZEN_ESTATE {
                if text.contains(name) {
                    found.push(format!(
                        "{where_it_is} names {name}, which is the program that came before \
                         this one and answers none of its commands"
                    ));
                }
            }
            for line in text.lines() {
                // Only an *invocation*, never prose. The provider must open the
                // line or follow a backtick or a shell prompt; "checking
                // cursor-setup-system behaviour" is a sentence, and an earlier
                // version of this check called it an instruction.
                let Some(at) = line.find(provider_id) else {
                    continue;
                };
                let before = line[..at].trim_end_matches(' ');
                let invoked = before.is_empty()
                    || before.ends_with('`')
                    || before.ends_with('$')
                    || before.ends_with('>');
                if !invoked {
                    continue;
                }
                let rest = &line[at + provider_id.len()..];
                let Some(verb) = rest.split_whitespace().next() else {
                    continue;
                };
                let verb = verb.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if verb.is_empty() || verb.starts_with('-') {
                    continue;
                }
                // Both surfaces, because the binary answers both. The first
                // version knew only `human::VERBS` and so called
                // `provider-info` refused -- a wire command this build answers
                // on demand, and one a toolkit has every reason to document.
                // A guard that names a working command as refused is the same
                // false statement it exists to catch, one level up.
                let human = crate::human::VERBS.contains(&verb);
                let wire = provider_v3::vocabulary::Command::parse(verb).is_some();
                if !human && !wire && !provider_id.contains(verb) {
                    found.push(format!(
                        "{where_it_is} tells a reader to run `{provider_id} {verb}`, \
                         which this binary answers on neither its human nor its wire surface"
                    ));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Every regular file under a payload, with its relative path and full path.
fn files_by_path(root: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                found.push((
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                    path,
                ));
            }
        }
    }
    found
}

/// Every regular file under a payload, as slash-separated relative paths.
fn files_under(root: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(root) {
                found.push(
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/"),
                );
            }
        }
    }
    found
}

/// What [`undescribed`] found, and how many things it looked at.
///
/// The count is here because `assert!(problems.is_empty())` cannot tell a clean
/// tree from an empty walk, and one of the seven harnesses genuinely has
/// nothing for this guard to examine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Examined {
    /// One line per entry point that cannot describe itself.
    pub problems: Vec<String>,
    /// How many entry points the walk actually reached.
    pub entry_points: usize,
}

/// A component entry point that carries no frontmatter, or none the product
/// reads.
///
/// [`unsourced`] exempts documents on the ground that *prose has no keys to
/// spell wrong*. An entry point is not prose: its `description` is what the
/// model reads to decide whether to invoke the thing at all. A `SKILL.md` that
/// lost its frontmatter installs, verifies and restores cleanly, and the
/// product names it after its directory and gives the model nothing to choose
/// on -- the same shape as a correct key under a wrong name, one level up.
///
/// **Which files are entry points is measured, not assumed**, and the negative
/// half is the part that took the measuring:
///
/// * `SKILL.md`, and a file directly under `agents/`, are entry points. Cursor's
///   own generator writes `{name, description}` for exactly these.
/// * A file under `references/` is **not** -- it is a document a skill links to,
///   and requiring frontmatter there would be inventing a rule.
/// * A file under `commands/` is **not**. Cursor's loader builds
///   `{id, name: filename.replace(".md",""), path, scope}`: the name comes from
///   the filename and no frontmatter is read. Requiring it would be a rule the
///   product does not have.
///
/// **It reports how many entry points it looked at, and that is not decoration.**
/// `assert!(problems.is_empty())` is green when the walk found nothing, and
/// codex's own test docstring says outright that it finds nothing there. So six
/// harnesses were asserting a guard whose subject count nobody stated, and a
/// layout change that removed every entry point would have left all seven green.
///
/// A filter that selects nothing produces output identical to a filter that
/// selects correctly. The only thing that tells them apart is asking how many
/// it matched -- and that is a number nobody prints. Returning it forces each
/// caller to state the number its own tree carries, including the harness whose
/// number is zero.
#[must_use]
pub fn undescribed(setups: &[Setup]) -> Examined {
    let mut found = Vec::new();
    let mut entry_points = 0_usize;
    for setup in setups {
        for name in files_under(&setup.payload) {
            if !is_entry_point(&name) {
                continue;
            }
            entry_points += 1;
            let Ok(text) = std::fs::read_to_string(setup.payload.join(&name)) else {
                found.push(format!("{} cannot read {name:?}", setup.manifest.id));
                continue;
            };
            for key in ["name", "description"] {
                if !frontmatter_names(&text, key) {
                    found.push(format!(
                        "{} ships {name:?} with no `{key}` in its frontmatter, and a component \
                         the product cannot describe is one the model cannot choose",
                        setup.manifest.id
                    ));
                }
            }
        }
        // A plugin manifest is the same obligation in a different file format,
        // and it escaped this check entirely until `skills` began routing two
        // kinds: before that every entry under a skills directory was a skill,
        // so every entry had a `SKILL.md` and every one was read. A plugin has
        // no frontmatter and was therefore no entry point, and a component with
        // nothing to describe it is what this function exists to refuse.
        for name in files_under(&setup.payload) {
            if !is_a_plugin_manifest(&name) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(setup.payload.join(&name)) else {
                found.push(format!("{} cannot read {name:?}", setup.manifest.id));
                continue;
            };
            let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
                found.push(format!(
                    "{} ships {name:?} and it is not JSON",
                    setup.manifest.id
                ));
                continue;
            };
            for key in ["name", "description"] {
                let named = manifest
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty());
                if !named {
                    found.push(format!(
                        "{} ships {name:?} with no `{key}`, and a component the product \
                         cannot describe is one the model cannot choose",
                        setup.manifest.id
                    ));
                }
            }
        }
    }
    Examined {
        problems: found,
        entry_points,
    }
}

/// Whether a path is a plugin manifest, in the two shapes that have one.
///
/// The same two arms as [`is_a_plugin_root`], reading a file rather than
/// deciding about its directory: `<name>/plugin.json` for Antigravity and Grok,
/// and `<name>/.<vendor>-plugin/plugin.json` for Claude Code, Cursor and Codex.
/// Matched on the suffix so a fifth vendor works rather than being a silent
/// miss.
///
/// **Two of the six that declare `plugin` have no manifest at all, and saying
/// "the rest" was wrong.** An OpenCode plugin is a JavaScript or TypeScript
/// module — `plugins/<name>.js`, one file exporting functions — and a Pi
/// extension is a package. Neither carries `name` or `description` for a
/// product to read, so there is nothing here to check and no arm to add: a
/// module's identity is its filename and its behaviour is its exports.
///
/// That is a real difference rather than a gap, and it is written down because
/// the first version of this comment claimed the manifest shapes covered every
/// product. They cover four of seven. No setup in this repository ships a
/// plugin for OpenCode, Grok or Pi today — only documentation about authoring
/// one — so nothing has ever tested the claim, which is why it survived being
/// written.
fn is_a_plugin_manifest(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split('/').collect();
    let vendor_prefixed = matches!(
        parts.as_slice(),
        [.., directory, "plugin.json"]
            if directory.starts_with('.') && directory.ends_with("-plugin")
    );
    vendor_prefixed || parts.last().is_some_and(|leaf| *leaf == "plugin.json")
}

/// A shipped instruction naming a sibling file the setup does not carry.
///
/// A skill's routing table sends a reader to `references/surfaces.md`; an agent
/// names the document beside it. If the setup does not ship that file, the
/// instruction sends the reader nowhere -- and the reader is a model, which
/// will not say so.
///
/// **Written because this repository's generator did it.** The agent template
/// pointed every harness at `references/surfaces.md`, and codex ships no skill
/// at all -- its `skill` kind routes only under `target_scope: user_root`, so a
/// setup aimed at its own home carries no `references/` directory. The
/// instruction was correct for six harnesses and false for the seventh, which
/// is the shape this estate keeps finding: a statement true of the thing
/// measured and false of the thing declared.
///
/// Only **relative** paths with a document extension are checked, and only
/// inside a backtick. A URL is somebody else's to serve, an absolute path is
/// not this setup's to carry, and prose that happens to contain a slash is not
/// an instruction -- the same distinction [`misdirecting`] draws for commands,
/// and for the same reason.
#[must_use]
pub fn dangling_references(setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for setup in setups {
        let files = files_under(&setup.payload);
        for (relative, path) in files_by_path(&setup.payload) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let here = relative.rsplit_once('/').map_or("", |(dir, _)| dir);
            for quoted in text.split('`').skip(1).step_by(2) {
                let named = quoted.trim();
                // **Only documents, and that scope was measured rather than
                // assumed.**
                //
                // Cursor's hand-written toolkit named two files that exist
                // nowhere in this estate -- `config/nddev-contract.json` and
                // `build/manifest.json`, inherited from the program this one
                // replaced -- and this guard did not see them because they are
                // not `.md`. The obvious repair is to check every extension.
                //
                // Measured before writing it: across all 28 setups that rule
                // flags **119** backticked paths, of which **117 are correct**.
                // Prose here legitimately names repository paths a reader is
                // told to open (`tools/build_nddev_builder.py`,
                // `scripts/gate.sh`, `references/<harness>-baseline.json`) and
                // product paths a setup deliberately does not ship
                // (`config/hooks.json`, `plugins/installed_plugins.json`,
                // `antigravity-cli/keybindings.json`). A guard with that ratio
                // is one people learn to ignore.
                //
                // The trap inside the trap: two of the 119 are the *corrective*
                // page, which quotes both dead paths in order to say they are
                // dead. The extended guard would refuse the sentence that
                // documents the removal.
                //
                // A document is different because a document named in prose is
                // a file a reader is told to *open*, and the setup either ships
                // it or the instruction goes nowhere. `.json` has no such
                // property. The two dead paths were found by reading, and this
                // note exists so the next author reaches for measurement rather
                // than for the tidier rule.
                let is_document = std::path::Path::new(named)
                    .extension()
                    .and_then(std::ffi::OsStr::to_str)
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"));
                if !is_document || !named.contains('/') || named.starts_with('/') {
                    continue;
                }
                if named.contains("://") || named.contains(' ') {
                    continue;
                }
                // Describing where a product reads is not naming a file to
                // open, and the difference is visible in the path itself. A
                // glob or a placeholder names a *class* of file; a `~`-rooted
                // path is outside this setup; a leading dot-directory is the
                // product's own home, not a document shipped here.
                //
                // Cursor's references are full of all three -- `~/.cursor/
                // skills/<skill>/SKILL.md`, `<plugin>/agents/*.md`,
                // `.cursor/rules/*.mdc` -- and the first version of this guard
                // called every one of them a dangling reference. That is the
                // same distinction `misdirecting` draws between an invocation
                // and prose, applied to paths instead of verbs.
                if named.contains('*')
                    || named.contains('<')
                    || named.contains('>')
                    // Braces are a placeholder too, and this list was one
                    // vendor's convention short. Antigravity writes its path
                    // templates as `{workspace}/.agents/skills/{skill_name}/
                    // SKILL.md`, quoted verbatim from the product's own bytes
                    // into a generated reference -- a *class* of file exactly as
                    // `<name>` is, and refused here because the spelling
                    // differed. Rewording the citation would have been the
                    // wrong repair: it is the product's string, and a quotation
                    // that has been tidied is no longer evidence.
                    || named.contains('{')
                    || named.contains('}')
                    || named.starts_with('~')
                    || named.starts_with('.')
                {
                    continue;
                }
                // An environment variable as the first segment is a root this
                // setup does not contain: `CURSOR_CONFIG_DIR/AGENTS.md` says
                // where a product looks, not what is shipped here.
                let root = named.split('/').next().unwrap_or_default();
                if !root.is_empty()
                    && root
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                {
                    continue;
                }
                // Matched as a **suffix**, anywhere in the setup, and the
                // weakness is deliberate.
                //
                // A relative reference does not resolve the same way in every
                // product: antigravity's plugin rule names
                // `skills/antigravity-surfaces/SKILL.md` relative to the
                // *plugin root*, while a skill's own routing table names
                // `references/surfaces.md` relative to itself. A guard that
                // picked one convention would call the other broken -- it did,
                // on the first run, and the file it named was shipped.
                //
                // So this asks the question it can answer without guessing a
                // convention: **is this document in the setup at all?** That
                // catches the defect the guard exists for -- an instruction
                // naming a file nothing ships -- and stays silent about where
                // a product resolves from, which is not this build's to assert.
                let _ = here;
                if !files
                    .iter()
                    .any(|f| f == named || f.ends_with(&format!("/{named}")))
                {
                    found.push(format!(
                        "{} ships {relative:?}, which sends a reader to {named:?}, and this \
                         setup carries no such file",
                        setup.manifest.id
                    ));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Whether a directory is a plugin root rather than a skill.
///
/// **The products that use a manifest discriminate by it, not by location.**
/// Claude Code: *"Any folder under a skills directory that contains a
/// `.claude-plugin/plugin.json` manifest is loaded as a plugin named
/// `<name>@skills-dir`"* — the same directory holds both kinds and the manifest
/// says which. Cursor and Codex carry the same shape under their own vendor
/// prefix; Antigravity and Grok put `plugin.json` at the plugin root.
///
/// OpenCode and Pi are not directories with manifests — a module file and a
/// package — so this answers `false` for them and correctly: neither can hold a
/// `references/` directory that a walk would then demand a `SKILL.md` beside.
/// See [`is_a_plugin_manifest`] for the same distinction on the file.
///
/// Written because [`unreachable_references`] was one directory, one kind. It
/// reported a plugin folder as a skill that had lost its `SKILL.md`, which is
/// the mirror of a misclassification the consumer session found in its own
/// discovery walk on the same afternoon — both from the same cause, a walk
/// written when the surface routed a single kind.
fn is_a_plugin_root(files: &[String], owner: &str) -> bool {
    files.iter().any(|name| {
        let Some(rest) = name.strip_prefix(&format!("{owner}/")) else {
            return false;
        };
        match rest.split('/').collect::<Vec<_>>().as_slice() {
            // Antigravity's shape: the manifest at the plugin root.
            ["plugin.json"] => true,
            // The vendor-prefixed shape: `.claude-plugin/`, `.cursor-plugin/`,
            // `.codex-plugin/`. Matched on the suffix rather than a list, so a
            // vendor this estate has not met yet is not a silent miss.
            [directory, "plugin.json"] => {
                directory.starts_with('.') && directory.ends_with("-plugin")
            }
            _ => false,
        }
    })
}

/// A `references/` directory no entry point reaches.
///
/// [`undescribed`] requires an entry point to describe itself; this requires
/// the supporting documents to have one. A `skills/x/references/*.md` with no
/// `skills/x/SKILL.md` beside it is prose nothing routes to -- installed,
/// backed up, restored, and read by nobody.
///
/// **Written because a generator in this repository produced exactly that.**
/// `tools/build_nddev_builder.py` wrote three references into a
/// `skills/nddev-builder/` directory of a harness whose skill is called
/// something else, and every other guard passed: the files are documents, so
/// `unsourced` exempts them; there is no `SKILL.md`, so `undescribed` has
/// nothing to check. The absence was invisible precisely because the thing that
/// would have been checked was the thing missing.
#[must_use]
pub fn unreachable_references(setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for setup in setups {
        let files = files_under(&setup.payload);
        for name in &files {
            let parts: Vec<&str> = name.split('/').collect();
            let Some(at) = parts.iter().position(|part| *part == "references") else {
                continue;
            };
            if at == 0 {
                continue;
            }
            let owner = parts[..at].join("/");
            if is_a_plugin_root(&files, &owner) {
                continue;
            }
            let entry = format!("{owner}/SKILL.md");
            if !files.iter().any(|other| other.eq_ignore_ascii_case(&entry)) {
                found.push(format!(
                    "{} ships {name:?} under {owner:?}, which has no SKILL.md, so the \
                     document is reachable from no entry point",
                    setup.manifest.id
                ));
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Whether a relative path inside a setup is a component's entry point.
fn is_entry_point(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split('/').collect();
    let Some(leaf) = parts.last() else {
        return false;
    };
    // Anything under `references/` is a document, whatever it is called.
    if parts.contains(&"references") {
        return false;
    }
    if leaf.eq_ignore_ascii_case("SKILL.md") {
        return true;
    }
    // `agents/<name>.md`, and only directly under it.
    parts.len() >= 2
        && parts[parts.len() - 2] == "agents"
        && std::path::Path::new(leaf)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

/// Whether a file opens with YAML frontmatter naming `key`.
///
/// Read by structure rather than by searching the whole file: a `description:`
/// three hundred lines into the prose is not frontmatter, and a guard that
/// matched it would be measuring the document instead of its header.
fn frontmatter_names(text: &str, key: &str) -> bool {
    let Some(rest) = text.strip_prefix("---\n") else {
        return false;
    };
    let Some(end) = rest.find("\n---") else {
        return false;
    };
    rest[..end]
        .lines()
        .any(|line| line.trim_start().starts_with(&format!("{key}:")))
}

/// Two files in one setup that a case-insensitive filesystem would merge.
///
/// macOS and Windows fold case by default, so `SKILL.md` and `skill.md` in one
/// setup are **one** file there and two on Linux. The setup would then install
/// different content depending on the machine, and its
/// `setup_catalogue_digest` would differ per platform — which the three-OS
/// matrix does catch, but only after the mistake has been written and pushed.
///
/// `provider_v3::bundle` already refuses this for a bundle *arriving* from a
/// consumer, on the same reasoning: *"the second would silently overwrite the
/// first"*. This applies the rule to the files this repository authors, which
/// is the direction it was missing.
#[must_use]
pub fn colliding(setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for setup in setups {
        let mut folded: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for name in files_under(&setup.payload) {
            if let Some(other) = folded.insert(name.to_lowercase(), name.clone())
                && other != name
            {
                found.push(format!(
                    "{} ships {name:?} and {other:?}, which differ only in case and are one \
                     file on macOS and Windows",
                    setup.manifest.id
                ));
            }
        }
    }
    found
}

/// Every setup that writes a configuration file and does not say where its
/// format came from, by setup id.
///
/// A document is exempt: prose has no keys to spell wrong. Anything else is a
/// claim about a product's schema, and a claim with no source is the thing this
/// project spent a release removing from its declarations.
#[must_use]
pub fn unsourced(setups: &[Setup]) -> Vec<String> {
    let mut found = Vec::new();
    for setup in setups {
        // A document has no schema to get wrong. Asked through `extension`
        // rather than by comparing the tail of the name, because a file called
        // `NOTES.MD` is a document on every system this ships to and a string
        // comparison would have said otherwise on two of them.
        let document = |name: &String| {
            std::path::Path::new(name)
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("mdc")
                })
        };
        let writes_configuration = files_under(&setup.payload)
            .iter()
            .any(|name| !document(name));
        if !writes_configuration {
            continue;
        }
        if setup.manifest.sources.is_empty() {
            found.push(format!(
                "{} writes a configuration file and names no source for its format",
                setup.manifest.id
            ));
            continue;
        }
        for source in &setup.manifest.sources {
            if !source.starts_with("https://") {
                found.push(format!(
                    "{} cites {source:?}, which is not a page anyone can open",
                    setup.manifest.id
                ));
            }
        }
    }
    found
}

/// One setup in the catalog, with the digest that identifies its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    /// What the setup says about itself.
    pub manifest: SetupManifest,
    /// Where its verbatim tree lives.
    pub payload: PathBuf,
    /// The digest of that tree.
    ///
    /// Two setups with the same bytes have the same definition digest, whatever
    /// they are called — identity is content, not a name.
    pub definition_digest: String,
    /// How many files the tree holds.
    pub file_count: u64,
    /// Keeps a materialized catalog alive for as long as `payload` names it.
    ///
    /// A `Setup` outlives the `Catalog` it came from — `get` returns one and the
    /// catalog is dropped — and for an embedded catalog the drop deletes the
    /// directory `payload` points into. `list` did not notice, because it reads
    /// everything before returning; `install` failed with *cannot list
    /// …/baseline/home*, having been handed a path to bytes that no longer
    /// existed.
    lifeline: Lifeline,
}

/// A handle that keeps a materialized catalog on disk, and is never identity.
///
/// Two setups with the same bytes are the same setup — that is the whole claim
/// `definition_digest` makes — so where those bytes were written cannot
/// participate in equality. Comparing always-equal is not a shortcut here; it is
/// the statement that provenance is not identity.
#[derive(Debug, Clone, Default)]
struct Lifeline(
    #[expect(
        dead_code,
        reason = "held for its Drop: this is the handle that keeps a materialized \
                  catalog on disk, and dead-code analysis does not count a \
                  destructor as a use"
    )]
    Option<Arc<Materialized>>,
);

impl PartialEq for Lifeline {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Lifeline {}

/// A compiled-in catalog written to a directory this process owns.
///
/// Removed when the last handle goes. A failure to remove it is deliberately
/// silent: the bytes are a copy of something already inside the binary, the
/// directory is inside the system's temporary space, and refusing a command
/// that has already succeeded because a cleanup did not would be the tail
/// wagging the dog.
#[derive(Debug)]
struct Materialized {
    root: PathBuf,
}

impl Materialized {
    /// Write every embedded file under a fresh directory.
    fn write(harness: &Harness) -> Option<Self> {
        // Unique without a dependency: the process cannot collide with itself,
        // and the counter separates two catalogs opened in one process — which
        // the tests do, in threads.
        //
        // The name is kept short on purpose, and the reason is Windows. A
        // classic `MAX_PATH` is 260, and the longest path here is
        // `<temp>/<this directory>/<deepest file in any setup>`. Measured
        // 2026-08-26: the deepest relative path any setup holds is 98 bytes
        // (cursor's `nddev-builder/home/plugins/…/installation-lifecycle.md`)
        // and a normal Windows temp root is around 42, which leaves this name
        // as the only part anyone here controls. `<provider_id>-<pid>-<n>` is
        // 46 for the longest provider id, for a worst case near 186 — enough
        // headroom for a long user name, which `…-setups-…` was eating for a
        // word that says nothing the provider id does not.
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            harness.provider_id,
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // A leftover from a crashed run of the same pid must not be read as
        // this one's catalog.
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).ok()?;

        let held = Self { root };
        for (relative, bytes) in harness.embedded_setups {
            let path = held.root.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).ok()?;
            }
            fs::write(&path, bytes).ok()?;
        }
        Some(held)
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for Materialized {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The catalog this build ships.
#[derive(Debug, Clone)]
pub struct Catalog {
    root: PathBuf,
    /// Kept alive so a materialized catalog outlives every reader of it.
    ///
    /// Shared rather than owned because `Catalog` is cloned, and two owners of
    /// one temporary directory would delete it twice — the second time out from
    /// under whoever still held the first. Handed to every [`Setup`] this
    /// catalog produces, so the bytes outlive the catalog that found them.
    lifeline: Lifeline,
}

impl Catalog {
    /// Open the catalog at an explicit root.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lifeline: Lifeline::default(),
        }
    }

    /// Find the catalog this harness ships.
    ///
    /// An explicit `<PROVIDER_ID>_SETUP_CATALOG` wins, so a caller can point the
    /// binary at a catalog of their own without rebuilding it — the owner's own
    /// setups are as legitimate a source as the shipped ones.
    ///
    /// Otherwise `setups/` is looked for beside the executable and upward, then
    /// in the working directory. Each candidate is tried twice: once as the
    /// catalog itself, and once with the harness id beneath it. A published tree
    /// ships one harness and uses the first shape; the workspace that authors
    /// them all uses the second, and a developer should not have to know which
    /// one they are standing in.
    ///
    /// That second shape was a claim rather than a fact until the embedded
    /// catalog existed: it joins the *harness id*, and two harness ids are not
    /// their tool names — `claude-code` against `setups/claude`, `grok-build`
    /// against `setups/grok`. Two of the seven could not find their own catalog
    /// from the workspace root, and the comment above said they could.
    ///
    /// When nothing is found on disk, the catalog compiled into this binary is
    /// materialized and used. That is the case for every user who installed the
    /// documented way, because the release ships binaries and no `setups/`.
    #[must_use]
    pub fn discover(harness: &Harness) -> Option<Self> {
        Self::on_disk(harness).or_else(|| Self::embedded(harness))
    }

    /// The catalog someone put on a disk, if there is one.
    #[must_use]
    fn on_disk(harness: &Harness) -> Option<Self> {
        let variable = format!(
            "{}_SETUP_CATALOG",
            harness.provider_id.to_uppercase().replace('-', "_")
        );
        if let Ok(explicit) = std::env::var(&variable) {
            let path = PathBuf::from(explicit);
            return path.is_dir().then_some(Self::at(path));
        }

        let mut roots: Vec<PathBuf> = Vec::new();
        if let Ok(executable) = std::env::current_exe()
            && let Some(directory) = executable.parent()
        {
            roots.push(directory.to_path_buf());
            let mut walk = directory;
            for _ in 0..3 {
                let Some(up) = walk.parent() else { break };
                roots.push(up.to_path_buf());
                walk = up;
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            roots.push(cwd);
        }

        roots
            .into_iter()
            .flat_map(|root| {
                let base = root.join(CATALOG_DIRECTORY);
                [base.join(harness.harness_id), base]
            })
            .find(|path| path.is_dir() && Self::holds_a_setup(path))
            .map(Self::at)
    }

    /// Write the compiled-in catalog somewhere real, and read it from there.
    ///
    /// Materializing rather than teaching every reader about a second kind of
    /// catalog is the whole point. A setup's identity is the digest of its
    /// tree, computed by walking a directory; a second in-memory implementation
    /// of that walk would be a second chance to disagree, and the two would
    /// disagree about exactly the thing that decides whether a target has
    /// drifted. Writing the bytes down means `list`, `get`, the digest and the
    /// copy are the same code they have always been, and the embedded catalog
    /// is provably the same setup as the on-disk one because it *is* one.
    ///
    /// The directory belongs to this process and is removed when the last
    /// handle to it goes.
    #[must_use]
    fn embedded(harness: &Harness) -> Option<Self> {
        if harness.embedded_setups.is_empty() {
            return None;
        }
        let root = Materialized::write(harness)?;
        let path = root.path().to_path_buf();
        Some(Self {
            root: path,
            lifeline: Lifeline(Some(Arc::new(root))),
        })
    }

    /// Whether a directory holds at least one readable setup manifest.
    ///
    /// Without this, the workspace's `setups/` — which holds one directory per
    /// harness and no manifests — would be chosen as an empty catalog and the
    /// harness-scoped directory beneath it never reached.
    fn holds_a_setup(path: &Path) -> bool {
        let Ok(read) = fs::read_dir(path) else {
            return false;
        };
        read.flatten()
            .any(|entry| entry.path().join(SETUP_MANIFEST).is_file())
    }

    /// The catalog root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every readable setup, by identity.
    ///
    /// A directory that does not parse is skipped rather than fatal: one broken
    /// setup should not make its neighbours unlistable.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] if the catalog cannot be
    /// listed at all.
    pub fn list(&self) -> Result<Vec<Setup>> {
        let read = match fs::read_dir(&self.root) {
            Ok(read) => read,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(Error::refuse(
                    WireReason::ProviderUnavailable,
                    format!(
                        "cannot list the setup catalog at {}: {source}",
                        self.root.display()
                    ),
                ));
            }
        };
        let mut setups = Vec::new();
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Ok(setup) = read_setup(&path, &self.lifeline) {
                setups.push(setup);
            }
        }
        setups.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(setups)
    }

    /// One setup by identity.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] when no setup carries that
    /// identity, naming the ones that do.
    pub fn get(&self, id: &str) -> Result<Setup> {
        let available = self.list()?;
        available
            .iter()
            .find(|setup| setup.manifest.id == id)
            .cloned()
            .ok_or_else(|| {
                let names: Vec<&str> = available
                    .iter()
                    .map(|setup| setup.manifest.id.as_str())
                    .collect();
                Error::refuse(
                    WireReason::ProviderUnavailable,
                    if names.is_empty() {
                        format!("{id:?} is not a setup; this build ships no catalog")
                    } else {
                        format!(
                            "{id:?} is not a setup; this build ships {}",
                            names.join(", ")
                        )
                    },
                )
            })
    }
}

/// Read one setup directory, or say why it is not one.
fn read_setup(directory: &Path, lifeline: &Lifeline) -> Result<Setup> {
    let manifest_path = directory.join(SETUP_MANIFEST);
    let bytes = fs::read(&manifest_path).map_err(|source| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("cannot read {}: {source}", manifest_path.display()),
        )
    })?;
    let manifest: SetupManifest = serde_json::from_slice(&bytes).map_err(|source| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} does not parse: {source}", manifest_path.display()),
        )
    })?;
    if manifest.schema_version != SETUP_SCHEMA {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is schema {} and this build writes {SETUP_SCHEMA}",
                manifest_path.display(),
                manifest.schema_version
            ),
        ));
    }
    // A setup whose directory and declared identity disagree would be
    // selectable by one name and recorded under another.
    let directory_name = directory.file_name().and_then(|name| name.to_str());
    if directory_name != Some(manifest.id.as_str()) {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} declares id {:?} but sits in {:?}",
                manifest_path.display(),
                manifest.id,
                directory_name.unwrap_or("?")
            ),
        ));
    }

    let payload = directory.join(SETUP_PAYLOAD);
    if !payload.is_dir() {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} has no {SETUP_PAYLOAD} tree", directory.display()),
        ));
    }
    Ok(Setup {
        definition_digest: digest::of_tree(&payload)?,
        file_count: count_files(&payload)?,
        manifest,
        payload,
        lifeline: lifeline.clone(),
    })
}

impl Setup {
    /// Every file this setup would write, as a path relative to the target.
    ///
    /// Files rather than top-level entries, because a harness may own a nested
    /// namespace and nothing else beside it: listing only the first component
    /// cannot tell `antigravity-cli/settings.json`, which is owned, from
    /// `antigravity-cli` as a whole, which is not.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::ProviderUnavailable`] if the payload cannot be
    /// listed, or holds a name this provider cannot represent as a path.
    pub fn relative_paths(&self) -> Result<Vec<String>> {
        let mut found = Vec::new();
        let mut stack = vec![self.payload.clone()];
        while let Some(current) = stack.pop() {
            let read = fs::read_dir(&current).map_err(|source| {
                Error::refuse(
                    WireReason::ProviderUnavailable,
                    format!("cannot list {}: {source}", current.display()),
                )
            })?;
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path.strip_prefix(&self.payload).map_err(|_| {
                    Error::refuse(
                        WireReason::ProviderUnavailable,
                        format!("{} escaped the setup payload", path.display()),
                    )
                })?;
                let Some(text) = relative.to_str() else {
                    return Err(Error::refuse(
                        WireReason::ProviderUnavailable,
                        format!("{} is not representable as a path", relative.display()),
                    ));
                };
                found.push(text.replace('\\', "/"));
            }
        }
        found.sort();
        Ok(found)
    }

    /// Check that every entry this setup writes is one the harness owns.
    ///
    /// A setup that wrote outside the declared namespaces would put files into a
    /// target that `remove` would then leave behind, and that `status` would not
    /// account for. Refusing here keeps ownership and effect the same set.
    ///
    /// # Errors
    ///
    /// Returns [`WireReason::UnsupportedNativeSurface`] naming the first entry
    /// outside the harness's declared surface.
    pub fn check_within(&self, harness: &Harness) -> Result<()> {
        for path in self.relative_paths()? {
            if !harness.owns(&path) {
                return Err(Error::refuse(
                    WireReason::UnsupportedNativeSurface,
                    format!(
                        "setup {:?} writes {path:?}, which is outside the surface {} owns",
                        self.manifest.id, harness.provider_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn count_files(root: &Path) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let read = fs::read_dir(&current).map_err(|source| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("cannot list {}: {source}", current.display()),
            )
        })?;
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                total = total.saturating_add(1);
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("harness-catalog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    /// A path template names a class of file; a relative one names a document.
    ///
    /// `dangling_references` had no unit test at all -- it was exercised only
    /// through each leaf crate against the real setups, which means a change to
    /// its exemption list was checked by whatever the setups happened to
    /// contain that day. It cost a false refusal the day a generated reference
    /// quoted antigravity's own `{workspace}/.agents/skills/{skill_name}/
    /// SKILL.md`: braces are that vendor's placeholder spelling and the list
    /// knew only `<>` and `*`.
    ///
    /// Both directions, because an exemption that swallows the real case is
    /// worse than the refusal it replaced.
    #[test]
    fn a_placeholder_names_a_class_of_file_and_a_relative_path_names_a_document() {
        let root = scratch("dangling-placeholders");
        write_setup(
            &root,
            "templates",
            &[(
                "notes.md",
                "See `{workspace}/.agents/skills/{skill_name}/SKILL.md`, \
                 `<plugin>/agents/*.md` and `~/.cursor/skills/x/SKILL.md`.",
            )],
        );
        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(
            dangling_references(&listed),
            Vec::<String>::new(),
            "a placeholder or an out-of-setup root is not a document this setup carries"
        );

        // The control: a plain relative path, with nothing at the other end.
        let root = scratch("dangling-real");
        write_setup(
            &root,
            "broken",
            &[("notes.md", "Read `references/gone.md`.")],
        );
        let listed = Catalog::at(&root).list().unwrap();
        let found = dangling_references(&listed);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("references/gone.md"), "{found:?}");
    }

    /// A plugin with no manifest is neither checked nor mis-reported.
    ///
    /// Two of the six harnesses that declare `plugin` have no manifest at all:
    /// an OpenCode plugin is a module file exporting functions, and a Pi
    /// extension is a package. The manifest guards answer `false` for both, and
    /// this asserts that answering `false` is harmless rather than a hole —
    /// a module file cannot hold a `references/` directory, so no walk demands
    /// a `SKILL.md` beside it, and it carries no `name` field for `undescribed`
    /// to want.
    ///
    /// Written because the comment on those guards claimed the manifest shapes
    /// covered every product, and no setup here ships a plugin for OpenCode,
    /// Grok or Pi, so nothing had ever exercised the claim.
    #[test]
    fn a_plugin_that_is_a_module_file_needs_neither_guard() {
        let root = scratch("plugin-as-module");
        write_setup(
            &root,
            "module-plugin",
            &[(
                "plugins/nddev-builder.js",
                "export const hooks = () => ({});\n",
            )],
        );
        let listed = Catalog::at(&root).list().unwrap();

        assert!(
            undescribed(&listed).problems.is_empty(),
            "a module plugin was asked for fields it does not have: {:?}",
            undescribed(&listed).problems
        );
        assert!(
            unreachable_references(&listed).is_empty(),
            "a module plugin was walked as though it were a skill: {:?}",
            unreachable_references(&listed)
        );
    }

    /// A plugin manifest has to name itself, like every other entry point.
    ///
    /// `undescribed` requires a `SKILL.md` and an agent file to carry `name`
    /// and `description`, because a component the product names after its
    /// directory gives a model nothing to choose on. A plugin manifest carries
    /// the same two fields for the same reason -- the product shows
    /// `description` when browsing one.
    ///
    /// It escaped the check entirely until `skills` began routing two kinds.
    /// Before that every entry under a skills directory was a skill, so every
    /// entry had frontmatter and every one was read; a plugin has none, so it
    /// was no entry point and nothing looked at it. Recorded first as an
    /// observation that passed, then turned into this when the gap closed.
    ///
    /// Both directions, and both shapes: a manifest that names nothing is
    /// caught under the vendor-prefixed path and under Antigravity's root
    /// `plugin.json`, and one that names both is not.
    #[test]
    fn a_plugin_manifest_has_to_name_itself() {
        let root = scratch("plugin-describes-itself");
        write_setup(
            &root,
            "silent-plugin",
            &[(
                "skills/tool/.claude-plugin/plugin.json",
                "{\"name\": \"tool\", \"version\": \"1.0.0\"}",
            )],
        );
        write_setup(
            &root,
            "silent-at-the-root",
            &[("plugins/tool/plugin.json", "{\"version\": \"1.0.0\"}")],
        );
        write_setup(
            &root,
            "speaking-plugin",
            &[(
                "skills/tool/.claude-plugin/plugin.json",
                "{\"name\": \"tool\", \"description\": \"what it is for\"}",
            )],
        );

        let listed = Catalog::at(&root).list().unwrap();
        let found = undescribed(&listed).problems;

        assert!(
            found
                .iter()
                .any(|p| p.contains("silent-plugin") && p.contains("description")),
            "a manifest with no description was not caught: {found:?}"
        );
        assert!(
            found.iter().any(|p| p.contains("silent-at-the-root")),
            "the root-manifest shape was not checked: {found:?}"
        );
        assert!(
            found.iter().all(|p| !p.contains("speaking-plugin")),
            "a manifest naming both fields was reported anyway: {found:?}"
        );
    }

    /// A plugin folder is not a skill missing its entry point.
    ///
    /// `unreachable_references` was written when one directory held one kind.
    /// Claude Code loads a folder under `skills/` as a *plugin* when it carries
    /// `.claude-plugin/plugin.json`, and as a skill when it carries `SKILL.md`
    /// -- the product's own discriminator is the manifest, not the location. So
    /// a walk that assumes every entry under a skills path is a skill reports
    /// the plugin as one missing its entry point.
    ///
    /// Observed before it was fixed: the same tree, with and without the
    /// manifest, and the assertion is that only the second is a problem. The
    /// first is the case that had no name until `skills` began routing two
    /// kinds.
    #[test]
    fn a_plugin_folder_is_not_a_skill_that_lost_its_entry_point() {
        let root = scratch("plugin-under-skills");
        write_setup(
            &root,
            "with-manifest",
            &[
                (
                    "skills/tool/.claude-plugin/plugin.json",
                    "{\"name\": \"tool\", \"version\": \"1.0.0\"}",
                ),
                ("skills/tool/references/notes.md", "supporting prose"),
            ],
        );
        // The control: the same shape with no manifest is a skill, and a skill
        // with references and no SKILL.md is the defect this guard exists for.
        write_setup(
            &root,
            "without-manifest",
            &[("skills/tool/references/notes.md", "supporting prose")],
        );

        let listed = Catalog::at(&root).list().unwrap();
        let found = unreachable_references(&listed);

        assert!(
            found
                .iter()
                .all(|problem| !problem.contains("with-manifest")),
            "a plugin folder was reported as a skill missing SKILL.md: {found:?}"
        );
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("without-manifest")),
            "the guard stopped catching a skill with no entry point: {found:?}"
        );
    }

    fn write_setup(root: &Path, id: &str, files: &[(&str, &str)]) {
        let directory = root.join(id);
        fs::create_dir_all(directory.join(SETUP_PAYLOAD)).unwrap();
        fs::write(
            directory.join(SETUP_MANIFEST),
            serde_json::to_vec_pretty(&SetupManifest {
                schema_version: SETUP_SCHEMA,
                id: id.to_owned(),
                description: format!("the {id} setup"),
                sources: Vec::new(),
            })
            .unwrap(),
        )
        .unwrap();
        for (relative, content) in files {
            let path = directory.join(SETUP_PAYLOAD).join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, content).unwrap();
        }
    }

    #[test]
    fn an_absent_catalog_lists_nothing_rather_than_failing() {
        let catalog = Catalog::at(scratch("absent").join("nowhere"));
        assert!(catalog.list().unwrap().is_empty());
    }

    #[test]
    fn setups_are_listed_by_identity_with_a_content_digest() {
        let root = scratch("list");
        write_setup(&root, "safe", &[("AGENTS.md", "# safe\n")]);
        write_setup(&root, "full-auto", &[("AGENTS.md", "# full\n")]);

        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|s| s.manifest.id.as_str())
                .collect::<Vec<_>>(),
            vec!["full-auto", "safe"]
        );
        assert!(listed[0].definition_digest.starts_with("sha256:"));
        assert_ne!(listed[0].definition_digest, listed[1].definition_digest);
        assert_eq!(listed[0].file_count, 1);
    }

    #[test]
    fn identity_is_content_so_two_names_over_the_same_bytes_agree() {
        let root = scratch("same-bytes");
        write_setup(&root, "one", &[("AGENTS.md", "identical\n")]);
        write_setup(&root, "two", &[("AGENTS.md", "identical\n")]);
        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(listed[0].definition_digest, listed[1].definition_digest);
    }

    #[test]
    fn a_setup_whose_directory_and_declared_id_disagree_is_not_listed() {
        // It would be selectable by one name and recorded under another.
        let root = scratch("mismatch");
        write_setup(&root, "safe", &[("AGENTS.md", "x")]);
        fs::rename(root.join("safe"), root.join("renamed")).unwrap();
        assert!(Catalog::at(&root).list().unwrap().is_empty());
    }

    #[test]
    fn one_broken_setup_does_not_make_the_others_unlistable() {
        let root = scratch("partly-broken");
        write_setup(&root, "good", &[("AGENTS.md", "x")]);
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken").join(SETUP_MANIFEST), "{ not json").unwrap();

        let listed = Catalog::at(&root).list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].manifest.id, "good");
    }

    #[test]
    fn a_setup_with_no_payload_tree_is_refused() {
        let root = scratch("no-payload");
        write_setup(&root, "empty", &[]);
        fs::remove_dir_all(root.join("empty").join(SETUP_PAYLOAD)).unwrap();
        assert!(Catalog::at(&root).list().unwrap().is_empty());
    }

    #[test]
    fn asking_for_an_unknown_setup_names_the_ones_that_exist() {
        let root = scratch("unknown");
        write_setup(&root, "safe", &[("AGENTS.md", "x")]);
        let error = Catalog::at(&root).get("nope").unwrap_err();
        assert!(error.detail().contains("safe"), "{error}");
    }

    #[test]
    fn a_setup_writing_outside_the_declared_surface_is_refused() {
        let root = scratch("outside");
        write_setup(
            &root,
            "sneaky",
            &[("AGENTS.md", "x"), ("elsewhere.txt", "y")],
        );
        let setup = Catalog::at(&root).get("sneaky").unwrap();
        let harness = crate::wire::tests_support::TEST;
        let error = setup.check_within(&harness).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        assert!(error.detail().contains("elsewhere.txt"));
    }

    #[test]
    fn a_setup_inside_the_declared_surface_is_accepted() {
        let root = scratch("inside");
        write_setup(&root, "fine", &[("AGENTS.md", "x"), ("skills/a.md", "y")]);
        let setup = Catalog::at(&root).get("fine").unwrap();
        assert!(
            setup
                .check_within(&crate::wire::tests_support::TEST)
                .is_ok()
        );
        assert_eq!(setup.file_count, 2);
    }

    /// The same two files a real setup holds, as the build script would embed
    /// them: relative slash paths and bytes, nothing else.
    const EMBEDDED: &[(&str, &[u8])] = &[
        (
            "baseline/setup.json",
            br#"{"schema_version":1,"id":"baseline","description":"the baseline setup"}"#,
        ),
        ("baseline/home/AGENTS.md", b"# instructions\n"),
        ("baseline/home/skills/a.md", b"a skill\n"),
    ];

    fn harness_carrying_the_embedded_catalog() -> Harness {
        let mut harness = crate::wire::tests_support::TEST;
        harness.embedded_setups = EMBEDDED;
        harness
    }

    /// The defect this exists for: `get` returns a `Setup` and the `Catalog`
    /// that produced it is dropped on the next line. For an embedded catalog
    /// that drop deletes the directory `payload` points into, so the caller is
    /// handed a path to bytes that no longer exist.
    ///
    /// `list` never noticed, because it reads everything before returning. The
    /// first thing that did was a real `install` from a binary with no `setups/`
    /// beside it, which refused with *cannot list …/baseline/home*.
    #[test]
    fn a_setup_outlives_the_embedded_catalog_it_came_from() {
        let harness = harness_carrying_the_embedded_catalog();
        let setup = Catalog::embedded(&harness)
            .unwrap()
            .get("baseline")
            .unwrap();

        // Everything that found the setup is gone; only the setup is held.
        assert_eq!(
            fs::read_to_string(setup.payload.join("AGENTS.md")).unwrap(),
            "# instructions\n",
            "the bytes were deleted while a caller still held the path to them"
        );
        assert_eq!(setup.file_count, 2);
    }

    /// A setup is its content, so the same bytes must have the same identity
    /// whether they were shipped inside the binary or found on a disk. If these
    /// two ever disagree, one target configured from the release and another
    /// from a checkout would report different identities for the same setup, and
    /// every drift, restore and `setup_definition_digest` downstream inherits
    /// the disagreement.
    #[test]
    fn the_embedded_catalog_and_the_same_bytes_on_disk_are_one_setup() {
        let root = scratch("embedded-equals-disk");
        write_setup(
            &root,
            "baseline",
            &[
                ("AGENTS.md", "# instructions\n"),
                ("skills/a.md", "a skill\n"),
            ],
        );
        let on_disk = Catalog::at(&root).get("baseline").unwrap();

        let harness = harness_carrying_the_embedded_catalog();
        let embedded = Catalog::embedded(&harness)
            .unwrap()
            .get("baseline")
            .unwrap();

        assert_eq!(
            embedded.definition_digest, on_disk.definition_digest,
            "the binary and the tree disagree about what the baseline setup is"
        );
        assert_eq!(embedded.file_count, on_disk.file_count);
    }

    /// A harness that ships no catalog says so by finding nothing, rather than
    /// by producing an empty directory that reads as a catalog with no setups.
    /// The two are different answers: one is "this build has none", the other is
    /// "this build has a catalog and it is empty".
    #[test]
    fn a_build_carrying_no_embedded_catalog_finds_none() {
        assert!(Catalog::embedded(&crate::wire::tests_support::TEST).is_none());
    }

    /// The temporary directory belongs to the process, and two catalogs opened
    /// in one process must not be handed the same one — the second would delete
    /// the first's bytes when it cleared a stale directory before writing.
    #[test]
    fn two_embedded_catalogs_in_one_process_do_not_share_a_directory() {
        let harness = harness_carrying_the_embedded_catalog();
        let first = Catalog::embedded(&harness).unwrap();
        let second = Catalog::embedded(&harness).unwrap();
        assert_ne!(first.root(), second.root());
        assert!(first.get("baseline").is_ok());
        assert!(second.get("baseline").is_ok());
    }
}
