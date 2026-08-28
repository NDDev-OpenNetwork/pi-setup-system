//! The Pi Coding Agent setup system.
//!
//! This file is the harness's *facts*. Every command over them lives in
//! [`harness_runtime`], shared with every other setup system, so a change to
//! behaviour lands once and a change to Pi Coding Agent's surface lands here.
//!
//! The program lifecycle is the one thing this harness does not offer. Pi is a
//! Node package whose dependency closure npm resolves at install time, so there
//! is no single artifact whose digest could be named in a plan, and declaring
//! an operation on that footing would promise something this build cannot keep.
//! `src/software.rs` records that as a fact rather than leaving it an absence.

use std::process::ExitCode;

mod software;

use harness_runtime::{Foreign, Harness};
use provider_v3::{ComponentKind, ProjectionKind};

/// Everything specific to Pi Coding Agent, verified against `pi-baseline.json`.
pub const PI: Harness = Harness {
    harness_id: "pi",
    provider_id: "pi-setup-system",
    version: env!("CARGO_PKG_VERSION"),
    product: "Pi Coding Agent",
    vendor: "Earendil Works",
    documented_config_home: "~/.pi/agent",
    config_home_env: "PI_CODING_AGENT_DIR",
    // One home, one variable: nothing here is conditional.
    config_home_note: "",
    control_directory: ".pi-setup-system",
    state_file: "NDDEV-PI-PROVIDER.json",
    predecessor_state_file: "NDDEV-PI-SETUP.json",
    profile_id: "pi/native-files/1",
    // Everything outside this list is a sibling overlay preserved verbatim.
    // `extensions` is where Pi's plugins live. It was missing here while the
    // consumer's own two descriptions of that route disagreed -- the composition
    // rule said `packages`, the catalog layout said `extensions` -- and claiming
    // a route while the product's own sources contradicted each other would have
    // been guessing at someone else's directory. Settled on their side by the
    // layout 1.1 correction, and the canonical compiler now answers `extensions`
    // for `plugin`, so it is declared.
    //
    // `prompts` and `themes` were missing rather than invented: Pi resolves
    // every path in `settings.json` relative to `~/.pi/agent`, and pi was the
    // only one of the seven short of a documented surface rather than carrying
    // one that does not exist.
    native_namespaces: &[
        "AGENTS.md",
        "settings.json",
        "skills",
        "extensions",
        "prompts",
        "themes",
    ],
    // The product's own: credentials, session history and runtime caches. Never
    // read, never written, and never copied into a backup slot.
    // `auth.json` first, and it took a sweep across all seven to notice it was
    // missing here. Pi joins it against its agent directory
    // (`agentDir, "auth.json"` in the pinned 0.84.3 bundle) and five of the
    // seven providers already listed their equivalent. No live leak --
    // `capture` walks `native_namespaces` and this file is inside none of them
    // -- but a safety list that depends on a namespace never widening is a
    // safety list waiting for one declaration change.
    never_touch: &["auth.json", "trust.json", "sessions"],
    // Oh My Pi is a separate product descended from the same code: package
    // `@oh-my-pi/pi-coding-agent`, command `omp`, home `~/.omp/agent`. Its
    // shape is Pi's -- both keep their configuration one directory down under
    // `agent` -- and the two homes are one word apart.
    //
    // What makes the confusion silent rather than loud is the file. Pi reads
    // `settings.json`; Oh My Pi reads `config.yml` and `models.yml`. A Pi setup
    // written into an Oh My Pi home is not rejected by anything: it is ignored,
    // and the directory looks configured.
    //
    // Measured 2026-08-27 from the published package and the project's own
    // documentation, not inferred from the name.
    foreign_homes: &[
        Foreign {
            marker: "config.yml",
            product: "Oh My Pi",
            home: "~/.omp/agent",
        },
        Foreign {
            marker: "models.yml",
            product: "Oh My Pi",
            home: "~/.omp/agent",
        },
    ],
    permission_profiles: &["default"],
    // `prompts` holds Pi's prompt templates, each `*.md` a command, so
    // `Command` is declared with the surface that carries it.
    component_kinds: &[
        ComponentKind::Instruction,
        ComponentKind::Skill,
        ComponentKind::Plugin,
        ComponentKind::Setting,
        ComponentKind::Command,
    ],
    projection_kinds: &[ProjectionKind::NativeFiles, ProjectionKind::Package],
    // One scope. Pi's project surfaces live under `.pi/` in a workspace, which is a
    // different root rather than a second scope of this target.
    //
    // Empty rather than absent: a harness that owns one target says so.
    scoped_projections: &[],
    max_files: 8192,
    max_bytes: 64 * 1024 * 1024,
    kit_identity: include_str!("../../../provider-kit/v3/KIT-IDENTITY.json"),
    // Generated by `build.rs` from this harness's `setups/` directory, so the
    // binary carries the catalog it is named after instead of hoping to find
    // one on a disk it was never shipped to.
    embedded_setups: include!(concat!(env!("OUT_DIR"), "/embedded_setups.rs")),
    software: Some(software::SOFTWARE),
};

fn main() -> ExitCode {
    harness_runtime::run(&PI, std::env::args().skip(1).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    /// The directory name this harness's setups live under in the workspace.
    const TOOL: &str = "pi";
    /// The declaration under test, named once so the shared test below reads
    /// the same in all seven crates.
    const HARNESS: Harness = PI;

    /// `build.rs` put the whole catalog in, under the paths it will be read by.
    ///
    /// This does **not** test for staleness, and an earlier version of this
    /// comment claimed it did. It cannot: `build.rs` declares
    /// `rerun-if-changed` on the catalog directory, so editing a setup rebuilds
    /// the table before this runs, and the test would be comparing the tree
    /// with itself. Observed — a deliberately edited setup left it green.
    ///
    /// What it does test is the build script, against a walk written
    /// independently of it: every file present, none invented, bytes exact, and
    /// paths relative and slash-separated. That last one is the one that would
    /// really break — `join("/")` is the only reason these keys are usable on
    /// Windows, and a path built with the platform separator would still look
    /// perfectly correct in the generated source.
    /// The bytes this harness ships, pinned so they cannot change unseen.
    ///
    /// A setup's `definition_digest` is what makes two setups the same setup,
    /// and it appears in `list`, in a plan and in provider state -- and until
    /// this, nothing compared it to anything. A stray character in a setup file
    /// changed what the estate installs and every test stayed green.
    ///
    /// One aggregate rather than one per setup, because the claim is about the
    /// catalogue: sorted definition digests, joined by a newline, hashed. A
    /// deliberate change to a setup updates the line in the baseline, which is
    /// the point -- the peer calls this a golden and it earns itself the first
    /// time a row moves without anyone meaning it to.
    ///
    /// **And it is the three-OS check nothing else makes.** The setups are
    /// embedded with `include_bytes!`, so whatever the checkout holds is what
    /// ships; `.gitattributes` pins `eol=lf` to keep a Windows checkout from
    /// rewriting them, and this is the assertion that would notice if it ever
    /// stopped working. The matrix runs it on all three systems, so a digest
    /// that differed by platform could not stay hidden.
    #[test]
    fn the_catalogue_this_harness_ships_is_the_one_the_baseline_records() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let mut digests: Vec<String> = catalog
            .list()
            .unwrap()
            .iter()
            .map(|setup| setup.definition_digest.clone())
            .collect();
        digests.sort();
        let joined = digests.join("\n");
        let aggregate = harness_runtime::digest_of_bytes(&joined);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references")
            .join(format!("{TOOL}-baseline.json"));
        let baseline: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let recorded = baseline["setup_catalogue_digest"].as_str().unwrap_or("");
        assert_eq!(
            aggregate, recorded,
            "the setups this binary ships are not the ones {TOOL}-baseline.json \
             records; if the change was meant, put this digest there"
        );
    }

    #[test]
    fn the_catalog_this_binary_carries_is_the_one_in_the_tree() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // The workspace holds one directory per harness; a rendered public tree
        // ships one harness and holds it flat. Same two candidates `build.rs`
        // chooses between, asked the same way.
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };

        // Only the setup directories, which is what the reader lists and what
        // `build.rs` embeds. A rendered public tree also carries a
        // `setups/README.md` at the catalog root, which belongs to no setup.
        let mut on_disk = Vec::new();
        let mut stack: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.join("setup.json").is_file())
            .collect();
        while let Some(directory) = stack.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    on_disk.push(path);
                }
            }
        }

        assert_eq!(
            HARNESS.embedded_setups.len(),
            on_disk.len(),
            "the binary carries {} files and the tree holds {}",
            HARNESS.embedded_setups.len(),
            on_disk.len()
        );

        for (relative, bytes) in HARNESS.embedded_setups {
            assert!(
                !relative.contains('\\') && !relative.starts_with('/'),
                "{relative:?} is not a relative slash path; a key built with the \
                 platform separator reads correctly on Unix and finds nothing on Windows"
            );
            let path = root.join(relative);
            let found = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("{relative} is compiled in but not in the tree: {e}"));
            assert_eq!(
                &found, bytes,
                "{relative} differs between the binary and the tree"
            );
        }
    }

    #[test]
    fn the_declaration_is_valid_and_names_this_host() {
        let info = PI.provider_info().unwrap();
        assert_eq!(info.provider_id, env!("CARGO_PKG_NAME"));
        assert_eq!(info.harness_id, "pi");
        assert_eq!(info.protocol_version, 3);
        assert!(info.supports_this_host());
    }

    #[test]
    fn no_namespace_is_both_owned_and_disclaimed() {
        for name in PI.never_touch {
            assert!(
                !PI.native_namespaces.contains(name),
                "{name} is claimed and disclaimed"
            );
        }
    }

    /// Everything this harness claims to own, against the vendor page that
    /// decided it.
    ///
    /// What this replaced only checked that the baseline parsed. The block it
    /// reads now is hand-authored beside the rest of the baseline, and this is
    /// what keeps that block from being decoration: a namespace no vendor
    /// document names, or a declared kind no owned surface routes, is red here.
    ///
    /// Both directions, because the defect it was written for ran both ways --
    /// `~/.cursor/rules` was owned and does not exist, `~/.pi/agent/prompts`
    /// exists and was not owned. Conformance caught neither: its
    /// `declared_native_route_is_compilable` case asks for **one** route, not
    /// every one.
    #[test]
    fn every_surface_this_harness_owns_is_one_the_vendor_documents() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references")
            .join(format!("{TOOL}-baseline.json"));
        let baseline: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let problems = harness_runtime::surfaces::disagreements(&HARNESS, &baseline);
        assert!(
            problems.is_empty(),
            "the declaration and {TOOL}-baseline.json disagree:
  {}",
            problems.join(
                "
  "
            )
        );
    }

    #[test]
    fn the_control_directory_and_state_file_are_provider_owned_not_product_owned() {
        assert!(PI.control_directory.contains("setup-system"));
        assert!(PI.state_file.starts_with("NDDEV-"));
        assert!(!PI.native_namespaces.contains(&PI.state_file));
    }
    /// A setup that writes a configuration file says where its format came from.
    ///
    /// The release before this one made the *surfaces* sourced: a path this
    /// provider owns cites the page that documents it. This is the same rule
    /// one level down, and it was written because two of the seven failed it.
    ///
    /// opencode's baseline set `"permission": "ask"` where the product
    /// documents an object of tool names, and antigravity's set
    /// `toolPermissions` where the product reads `toolPermission` with four
    /// values, none of them the one written. Both were valid JSON in the right
    /// file at the right path. Both installed, verified and restored cleanly.
    /// Neither changed anything about the product — a target that looks
    /// configured and is not, which is the failure this estate refuses one
    /// level up and had been shipping one level down.
    /// Two files in one setup that a case-insensitive filesystem would merge.
    ///
    /// macOS and Windows fold case, so such a pair is one file there and two on
    /// Linux -- the setup would install different content depending on the
    /// machine, and its catalogue digest would differ per platform. The bundle
    /// reader has refused this for an arriving bundle since 0.0.11; this is the
    /// same rule applied to what this repository authors.
    /// Every component entry point describes itself.
    ///
    /// A `SKILL.md` or an agent whose frontmatter lost its `description` still
    /// installs, verifies and restores cleanly -- and the product names it after
    /// its directory and gives the model nothing to choose on. Documents under
    /// `references/` and files under `commands/` are exempt, because the
    /// products measured do not read frontmatter from either.
    /// Supporting documents are reachable from an entry point.
    ///
    /// A `references/` folder whose skill has no `SKILL.md` is prose nothing
    /// routes to. A generator in this repository produced exactly that, and
    /// every other guard passed it: the files are documents, so `unsourced`
    /// exempts them, and there is no `SKILL.md`, so `undescribed` has nothing
    /// to check.
    #[test]
    fn every_reference_folder_has_an_entry_point() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems = harness_runtime::catalog::unreachable_references(&catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }

    #[test]
    fn every_component_entry_point_describes_itself() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems = harness_runtime::catalog::undescribed(&catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }

    #[test]
    fn no_two_files_in_a_setup_differ_only_in_case() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems = harness_runtime::catalog::colliding(&catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }

    #[test]
    fn a_setup_that_writes_configuration_says_where_its_format_came_from() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems = harness_runtime::catalog::unsourced(&catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }
    /// Three postures, on every one of the seven.
    ///
    /// `baseline` is a working floor, `minimal` is the product's own defaults,
    /// and `full-auto` asks nothing and sandboxes nothing. A caller who learns
    /// them on one product knows them on all seven, which is the whole reason
    /// the names are the estate's rather than each harness's.
    ///
    /// The second half of the check is the one worth having: two setups with
    /// the same bytes mean one of them is a posture in name only, and it would
    /// still read as offered in `list`.
    #[test]
    fn the_three_postures_are_offered_and_are_actually_different() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems = harness_runtime::catalog::asymmetric(&catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }
    /// Nothing this setup ships tells a reader to run something that is not here.
    ///
    /// A setup carries documents an agent reads and acts on -- a skill, a rule,
    /// a command file -- and nothing was checking them. One shipped
    /// `software-status --target <dir> --json` and `list --json` for six
    /// releases; the binary refuses both, and says so in those words.
    ///
    /// Two refusals: a name belonging to the frozen estate, and any line naming
    /// this provider followed by a verb `into_command` does not accept. English
    /// is not judged -- `install` in a sentence is a word, and only
    /// `<provider> install` is an instruction.
    #[test]
    fn nothing_this_harness_ships_names_a_command_it_refuses() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest.join("../../setups").join(TOOL);
        let root = if root.is_dir() {
            root
        } else {
            manifest.join("../../setups")
        };
        let catalog = harness_runtime::Catalog::at(&root);
        let problems =
            harness_runtime::catalog::misdirecting(HARNESS.provider_id, &catalog.list().unwrap());
        assert!(problems.is_empty(), "{}", problems.join("\n  "));
    }
}
