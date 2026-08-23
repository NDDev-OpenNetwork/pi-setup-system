//! The commands an owner types.
//!
//! These are the reason the project exists: choose a complete harness state,
//! put it in place, change your mind, and get the previous one back. The wire
//! surface serves a consumer; this one serves a person.
//!
//! It is not a second implementation. Every mutation here builds the same plan
//! the wire surface builds and hands it to [`wire::perform`], so the lock, the
//! journal, the backup and the recovery story are identical. What differs is
//! only where the setup came from and who is reading the output.
//!
//! # The target is always explicit
//!
//! Every command takes `--target <absolute-directory>`. There is no default and
//! no fallback to a configuration home, because a mutation aimed at a guessed
//! path is a mutation aimed at someone else's state. The documented home is
//! printed by `--help` so it can be copied, not resolved.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use provider_v3::plan::{PlanArtifact, PlanInputs};
use provider_v3::{Error, Operation, Result, WireReason};
use setup_core::backup::Pool;
use setup_core::stamp::{ProviderState, StateReading};
use setup_core::target::Target;

use crate::catalog::{CATALOG_DIRECTORY, Catalog};
use crate::expiry;
use crate::facts::{self, Harness};
use crate::wire::{self, Effect, Mutation};

/// How long a plan this surface makes stays applicable.
///
/// It is applied within the same process, so the window only has to cover that.
/// A long one would mean a plan could outlive the state it was made against.
const PLAN_WINDOW_SECONDS: u64 = 600;

/// One parsed human command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Show the setups this build ships.
    List,
    /// Report what is in a target.
    Status {
        /// The directory to read.
        target: PathBuf,
    },
    /// Put a setup into a target.
    Install {
        /// The directory to write.
        target: PathBuf,
        /// The setup to apply.
        setup: String,
    },
    /// Re-apply whatever setup is already recorded, repairing drift.
    Reinstall {
        /// The directory to repair.
        target: PathBuf,
    },
    /// Replace the applied setup with another.
    Select {
        /// The directory to write.
        target: PathBuf,
        /// The setup to switch to.
        setup: String,
    },
    /// List the backups a target holds.
    Backups {
        /// The directory to read.
        target: PathBuf,
    },
    /// Put a captured state back.
    Restore {
        /// The directory to write.
        target: PathBuf,
        /// The slot to read, or the most recent when absent.
        backup: Option<String>,
    },
    /// Withdraw everything this provider owns.
    Remove {
        /// The directory to clear.
        target: PathBuf,
    },
    /// Report what differs between the target and the setup recorded in it.
    Diff {
        /// The directory to compare.
        target: PathBuf,
    },
}

/// Whether a first argument names a human command.
#[must_use]
pub fn is_human_command(name: &str) -> bool {
    matches!(
        name,
        "list" | "install" | "reinstall" | "select" | "backups" | "restore" | "remove" | "diff"
    )
}

/// Parse a human invocation.
///
/// # Errors
///
/// Refuses an unknown command, a missing `--target`, a missing setup name and
/// any unknown flag. Nothing is guessed.
pub fn parse<I, S>(arguments: I) -> Result<Command>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let tokens: Vec<String> = arguments.into_iter().map(Into::into).collect();
    let Some(name) = tokens.first().cloned() else {
        return Err(local("no command was given"));
    };
    let parsed = Arguments::scan(&name, tokens.get(1..).unwrap_or_default())?;
    parsed.into_command(&name)
}

/// The flags and positionals one invocation carried.
struct Arguments {
    target: Option<PathBuf>,
    backup: Option<String>,
    positional: Vec<String>,
}

impl Arguments {
    /// Split an argument list into flags and positionals, refusing anything odd.
    fn scan(name: &str, rest: &[String]) -> Result<Self> {
        let mut parsed = Self {
            target: None,
            backup: None,
            positional: Vec::new(),
        };
        let mut index = 0;
        while index < rest.len() {
            let Some(token) = rest.get(index) else { break };
            match token.as_str() {
                "--target" | "--backup" => {
                    let Some(value) = rest.get(index + 1) else {
                        return Err(local(format!("{token} has no value")));
                    };
                    if value.starts_with("--") {
                        return Err(local(format!("{token} has no value")));
                    }
                    if token == "--target" {
                        if parsed.target.is_some() {
                            return Err(local("--target was given twice"));
                        }
                        parsed.target = Some(PathBuf::from(value));
                    } else {
                        if parsed.backup.is_some() {
                            return Err(local("--backup was given twice"));
                        }
                        parsed.backup = Some(value.clone());
                    }
                    index += 2;
                }
                other if other.starts_with("--") => {
                    return Err(local(format!("{other} is not an argument of {name}")));
                }
                other => {
                    parsed.positional.push(other.to_owned());
                    index += 1;
                }
            }
        }
        Ok(parsed)
    }

    fn target(&self, name: &str) -> Result<PathBuf> {
        self.target
            .clone()
            .ok_or_else(|| local(format!("{name} requires --target <absolute-directory>")))
    }

    fn setup(&self, name: &str) -> Result<String> {
        match self.positional.as_slice() {
            [only] => Ok(only.clone()),
            [] => Err(local(format!(
                "{name} requires a setup name; run `list` to see them"
            ))),
            _ => Err(local(format!("{name} takes one setup name"))),
        }
    }

    fn no_setup(&self, name: &str) -> Result<()> {
        if self.positional.is_empty() {
            Ok(())
        } else {
            Err(local(format!("{name} takes no setup name")))
        }
    }

    fn into_command(self, name: &str) -> Result<Command> {
        if self.backup.is_some() && name != "restore" {
            return Err(local(format!("--backup is not an argument of {name}")));
        }
        match name {
            "list" => {
                self.no_setup(name)?;
                Ok(Command::List)
            }
            "status" => {
                self.no_setup(name)?;
                Ok(Command::Status {
                    target: self.target(name)?,
                })
            }
            "install" => Ok(Command::Install {
                target: self.target(name)?,
                setup: self.setup(name)?,
            }),
            "select" => Ok(Command::Select {
                target: self.target(name)?,
                setup: self.setup(name)?,
            }),
            "reinstall" => {
                self.no_setup(name)?;
                Ok(Command::Reinstall {
                    target: self.target(name)?,
                })
            }
            "backups" => {
                self.no_setup(name)?;
                Ok(Command::Backups {
                    target: self.target(name)?,
                })
            }
            "restore" => {
                self.no_setup(name)?;
                let target = self.target(name)?;
                Ok(Command::Restore {
                    target,
                    backup: self.backup,
                })
            }
            "remove" => {
                self.no_setup(name)?;
                Ok(Command::Remove {
                    target: self.target(name)?,
                })
            }
            "diff" => {
                self.no_setup(name)?;
                Ok(Command::Diff {
                    target: self.target(name)?,
                })
            }
            other => Err(local(format!("{other:?} is not a command"))),
        }
    }
}

/// Run one human command, writing its report to stdout.
///
/// # Errors
///
/// Propagates the refusal. The caller renders it.
pub fn run(harness: &Harness, command: Command) -> Result<()> {
    match command {
        Command::List => list(harness),
        Command::Status { target } => status(harness, &target),
        Command::Backups { target } => backups(harness, &target),
        Command::Diff { target } => diff(harness, &target),
        Command::Install { target, setup } => {
            apply_setup(harness, &target, &setup, Operation::Install)
        }
        Command::Select { target, setup } => {
            apply_setup(harness, &target, &setup, Operation::Replace)
        }
        Command::Reinstall { target } => reinstall(harness, &target),
        Command::Restore { target, backup } => restore(harness, &target, backup),
        Command::Remove { target } => remove(harness, &target),
    }
}

fn local(detail: impl Into<String>) -> Error {
    Error::refuse(WireReason::ProviderUnavailable, detail)
}

fn catalog(harness: &Harness) -> Result<Catalog> {
    Catalog::discover(harness).ok_or_else(|| {
        local(format!(
            "no setup catalog was found; point {}_SETUP_CATALOG at one, or run from a \
             tree that ships {CATALOG_DIRECTORY}/",
            harness.provider_id.to_uppercase().replace('-', "_")
        ))
    })
}

fn list(harness: &Harness) -> Result<()> {
    let setups = catalog(harness)?.list()?;
    if setups.is_empty() {
        println!("{}: no setups in the catalog.", harness.provider_id);
        return Ok(());
    }
    println!("Setups for {} ({}):", harness.product, harness.vendor);
    println!();
    for setup in &setups {
        println!("  {}", setup.manifest.id);
        println!("      {}", setup.manifest.description);
        println!(
            "      {} files, definition {}",
            setup.file_count,
            short(&setup.definition_digest)
        );
    }
    println!();
    println!("Install one with:  install <id> --target <absolute-directory>");
    Ok(())
}

fn status(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    println!("Target   {}", resolved.root().display());
    println!("Identity {}", short(&identity));

    match ProviderState::read(resolved.root(), harness.state_file)? {
        StateReading::Absent => {
            println!("Setup    none applied by {}", harness.provider_id);
        }
        StateReading::ForeignSchema { found_schema } => {
            println!("Setup    recorded in schema {found_schema}, which this build does not write");
            println!("         reported as found; nothing was migrated");
        }
        StateReading::Current(state) => {
            println!(
                "Setup    {}",
                state.setup_stable_id.as_deref().unwrap_or("(unnamed)")
            );
            println!("Applied  operation {}", state.operation_id);
            if identity == state.target_identity_digest {
                println!("Drift    none");
            } else {
                println!("Drift    the target has changed since it was applied");
                println!("         run `diff` to see where, or `reinstall` to put it back");
            }
        }
    }

    let control = resolved.ensure_control_directory()?;
    let pool = Pool::open(&control, facts::BACKUP_SLOTS)?;
    println!("Backups  {}", pool.list()?.len());
    Ok(())
}

fn backups(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let control = resolved.ensure_control_directory()?;
    let records = Pool::open(&control, facts::BACKUP_SLOTS)?.list()?;
    if records.is_empty() {
        println!("No backups. One is captured before every change.");
        return Ok(());
    }
    println!("Backups of {}, newest first:", resolved.root().display());
    println!();
    for (position, record) in records.iter().enumerate() {
        let marker = if position == 0 {
            "  (restored by default)"
        } else {
            ""
        };
        println!("  {}{marker}", record.backup_ref.as_str());
        println!(
            "      before {}, setup {}",
            record.operation,
            record.setup_id.as_deref().unwrap_or("none")
        );
    }
    println!();
    println!("Restore a specific one with:  restore --backup <ref> --target <dir>");
    Ok(())
}

fn diff(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let StateReading::Current(state) = ProviderState::read(resolved.root(), harness.state_file)?
    else {
        println!(
            "Nothing to compare: {} has applied no setup here.",
            harness.provider_id
        );
        return Ok(());
    };
    if identity == state.target_identity_digest {
        println!("The target matches the setup recorded in it.");
        return Ok(());
    }

    println!(
        "The target has changed since {} was applied.",
        state
            .setup_stable_id
            .as_deref()
            .unwrap_or("the recorded setup")
    );
    println!("  recorded {}", short(&state.target_identity_digest));
    println!("  observed {}", short(&identity));
    println!();
    println!("This provider owns:");
    for namespace in harness.native_namespaces {
        let path = resolved.root().join(namespace);
        let mark = if path.exists() { "present" } else { "absent " };
        println!("  {mark}  {namespace}");
    }
    println!();
    println!("`reinstall` puts the recorded setup back. Everything else is left alone.");
    Ok(())
}

fn apply_setup(
    harness: &Harness,
    target: &Path,
    setup_id: &str,
    operation: Operation,
) -> Result<()> {
    let setup = catalog(harness)?.get(setup_id)?;
    setup.check_within(harness)?;
    let report = mutate(
        harness,
        target,
        operation,
        Effect::Materialize { setup: &setup },
        Some(setup_id),
    )?;
    println!("Applied setup {setup_id} to {}.", target.display());
    println!(
        "  previous state captured as {}",
        report_field(&report, "backup_ref")
    );
    println!(
        "  target identity now {}",
        short(&report_field(&report, "target_identity_digest"))
    );
    Ok(())
}

fn reinstall(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let StateReading::Current(state) = ProviderState::read(resolved.root(), harness.state_file)?
    else {
        return Err(local(
            "nothing is recorded here to reinstall; use `install <setup>` to choose one",
        ));
    };
    let Some(setup_id) = state.setup_stable_id else {
        return Err(local(
            "the recorded state names no setup, so there is nothing to re-apply",
        ));
    };
    apply_setup(harness, target, &setup_id, Operation::Replace)
}

fn restore(harness: &Harness, target: &Path, backup: Option<String>) -> Result<()> {
    let named = backup.clone();
    let report = mutate(
        harness,
        target,
        Operation::Restore,
        Effect::Restore { backup_ref: backup },
        None,
    )?;
    match named {
        Some(reference) => println!("Restored {reference} into {}.", target.display()),
        None => println!("Restored the most recent backup into {}.", target.display()),
    }
    println!(
        "  state before this restore captured as {}",
        report_field(&report, "backup_ref")
    );
    Ok(())
}

fn remove(harness: &Harness, target: &Path) -> Result<()> {
    let report = mutate(harness, target, Operation::Remove, Effect::Remove, None)?;
    println!(
        "Removed everything {} owns from {}.",
        harness.provider_id,
        target.display()
    );
    println!(
        "  previous state captured as {}",
        report_field(&report, "backup_ref")
    );
    println!("  restore it with:  restore --target {}", target.display());
    Ok(())
}

/// Build a plan and apply it through the one write path.
fn mutate(
    harness: &Harness,
    target: &Path,
    operation: Operation,
    effect: Effect<'_>,
    setup_id: Option<&str>,
) -> Result<serde_json::Value> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let identity = resolved.identity_digest_excluding(&harness.not_our_identity())?;
    let build_digest = harness.build_digest()?;
    let profile = harness.projection_profile()?;
    let operation_id = operation_id(harness, &identity);

    // A restore names the state it will produce; every other operation must not.
    //
    // The slot is resolved *here*, at plan time, and pinned into the effect. It
    // has to be: applying a restore captures the current state first, so a
    // reference resolved after that capture would name the capture itself and
    // the restore would be a no-op. "The most recent backup" means the most
    // recent one that existed when the restore was asked for.
    let (effect, restore_target_digest) = if operation == Operation::Restore {
        let control = resolved.ensure_control_directory()?;
        let pool = Pool::open(&control, facts::BACKUP_SLOTS)?;
        let Effect::Restore { backup_ref } = &effect else {
            return Err(local("a restore effect is required to plan a restore"));
        };
        let record = wire::chosen_backup(&pool, backup_ref.as_deref())?;
        let digest = setup_core::digest::of_tree(&pool.payload_of(&record.backup_ref)?)?;
        let pinned = record.backup_ref.as_str().to_owned();
        (
            Effect::Restore {
                backup_ref: Some(pinned),
            },
            Some(digest),
        )
    } else {
        (effect, None)
    };

    let artifact = PlanArtifact::new(PlanInputs {
        provider_id: harness.provider_id,
        provider_version: harness.version,
        provider_build_digest: &build_digest,
        // No release verified a locally invoked binary; recording the build
        // digest here says what ran without claiming a release it did not have.
        provider_release_digest: &build_digest,
        operation_id: &operation_id,
        operation,
        canonical_target: &resolved.root().to_string_lossy(),
        expected_target_digest: &identity,
        projection_profile_digest: &profile.digest,
        bundle: None,
        backup_ref: match &effect {
            Effect::Restore { backup_ref } => backup_ref.clone(),
            _ => None,
        },
        restore_target_digest,
        permission_profile: None,
        expires_at: &expiry::deadline_in(PLAN_WINDOW_SECONDS, SystemTime::now()),
        effects: effect_lines(harness, &effect, setup_id),
    })?;
    let plan_digest = artifact.digest()?;
    let provenance = serde_json::to_value(&artifact)
        .map_err(|source| local(format!("the plan artifact cannot be encoded: {source}")))?;

    wire::perform(
        harness,
        target,
        &Mutation {
            operation,
            operation_id,
            plan_digest,
            expected_target_digest: identity,
            effect,
            provenance,
            setup_id: setup_id.map(str::to_owned),
        },
    )
}

fn effect_lines(harness: &Harness, effect: &Effect<'_>, setup_id: Option<&str>) -> Vec<String> {
    let capture = "capture the current target into a new backup slot".to_owned();
    match effect {
        Effect::Backup => vec![capture],
        Effect::Remove => {
            vec![
                capture,
                format!(
                    "withdraw the {} entries this provider owns",
                    harness.native_namespaces.len()
                ),
            ]
        }
        Effect::Restore { backup_ref } => vec![
            capture,
            match backup_ref {
                Some(reference) => format!("restore the target from {reference}"),
                None => "restore the target from the most recent backup".to_owned(),
            },
        ],
        // The human surface never builds one of these: a bundle arrives over the
        // wire, and the plan for it is made there.
        Effect::MaterializeBundle { files } => vec![
            capture,
            format!("write the {} files the bundle declares", files.len()),
        ],
        Effect::Materialize { setup } => vec![
            capture,
            format!(
                "write setup {} over the entries this provider owns",
                setup_id.unwrap_or(setup.manifest.id.as_str())
            ),
        ],
    }
}

/// A stable identifier for one operation.
///
/// Derived from the harness, the target identity and the clock rather than a
/// random source: the same command against the same target at the same second
/// produces the same id, which makes a repeated invocation recognisable in a
/// journal rather than looking like a second operation.
fn operation_id(harness: &Harness, identity: &str) -> String {
    let seconds = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let seed = format!("{}:{identity}:{seconds}", harness.provider_id);
    let digest = setup_core::digest::of_bytes(seed.as_bytes());
    format!(
        "operation_{}",
        digest
            .trim_start_matches("sha256:")
            .get(..24)
            .unwrap_or("unknown")
    )
}

fn report_field(report: &serde_json::Value, name: &str) -> String {
    report
        .get(name)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("(none)")
        .to_owned()
}

fn short(digest: &str) -> String {
    digest.get(..19).unwrap_or(digest).to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn every_mutating_command_requires_an_explicit_target() {
        for name in [
            "status",
            "install",
            "reinstall",
            "select",
            "backups",
            "restore",
            "remove",
            "diff",
        ] {
            let tokens = if matches!(name, "install" | "select") {
                vec![name.to_owned(), "some-setup".to_owned()]
            } else {
                vec![name.to_owned()]
            };
            let error = parse(tokens).unwrap_err();
            assert!(error.detail().contains("--target"), "{name}: {error}");
        }
    }

    #[test]
    fn list_needs_no_target_because_it_reads_no_target() {
        assert_eq!(parse(["list"]).unwrap(), Command::List);
    }

    #[test]
    fn install_and_select_require_exactly_one_setup_name() {
        assert!(
            parse(["install", "--target", "/tmp/x"])
                .unwrap_err()
                .detail()
                .contains("setup name")
        );
        assert!(
            parse(["install", "a", "b", "--target", "/tmp/x"])
                .unwrap_err()
                .detail()
                .contains("one setup name")
        );
        assert_eq!(
            parse(["install", "safe", "--target", "/tmp/x"]).unwrap(),
            Command::Install {
                target: PathBuf::from("/tmp/x"),
                setup: "safe".to_owned()
            }
        );
    }

    #[test]
    fn commands_that_take_no_setup_refuse_one() {
        for name in ["reinstall", "remove", "diff", "backups", "status"] {
            let error = parse([name, "stray", "--target", "/tmp/x"]).unwrap_err();
            assert!(error.detail().contains("no setup name"), "{name}: {error}");
        }
    }

    #[test]
    fn only_restore_accepts_a_backup_reference() {
        assert_eq!(
            parse([
                "restore",
                "--target",
                "/tmp/x",
                "--backup",
                "slot-000000000002"
            ])
            .unwrap(),
            Command::Restore {
                target: PathBuf::from("/tmp/x"),
                backup: Some("slot-000000000002".to_owned())
            }
        );
        assert!(
            parse([
                "remove",
                "--target",
                "/tmp/x",
                "--backup",
                "slot-000000000002"
            ])
            .unwrap_err()
            .detail()
            .contains("--backup")
        );
    }

    #[test]
    fn restore_without_a_reference_means_the_most_recent() {
        assert_eq!(
            parse(["restore", "--target", "/tmp/x"]).unwrap(),
            Command::Restore {
                target: PathBuf::from("/tmp/x"),
                backup: None
            }
        );
    }

    #[test]
    fn an_unknown_flag_or_command_is_refused_rather_than_ignored() {
        assert!(
            parse(["status", "--target", "/tmp/x", "--force"])
                .unwrap_err()
                .detail()
                .contains("--force")
        );
        assert!(
            parse(["frobnicate"])
                .unwrap_err()
                .detail()
                .contains("not a command")
        );
        assert!(
            parse(Vec::<String>::new())
                .unwrap_err()
                .detail()
                .contains("no command")
        );
    }

    #[test]
    fn a_repeated_flag_is_refused_rather_than_last_one_winning() {
        let error = parse(["status", "--target", "/a", "--target", "/b"]).unwrap_err();
        assert!(error.detail().contains("twice"));
    }

    #[test]
    fn the_human_command_names_are_exactly_what_is_dispatched() {
        for name in [
            "list",
            "install",
            "reinstall",
            "select",
            "backups",
            "restore",
            "remove",
            "diff",
        ] {
            assert!(
                is_human_command(name),
                "{name} is dispatched but not recognised"
            );
        }
        // `status` is served by both surfaces and is routed by its flags, not here.
        assert!(!is_human_command("status"));
        assert!(!is_human_command("provider-info"));
        assert!(!is_human_command("plan-operation"));
    }

    // ---- end to end, over a real catalog and a real target ----

    use std::fs;

    use crate::catalog::{SETUP_MANIFEST, SETUP_PAYLOAD, SETUP_SCHEMA, SetupManifest};

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("harness-human-{name}"));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("target")).unwrap();
        fs::canonicalize(&base).unwrap()
    }

    fn write_setup(catalog: &Path, id: &str, files: &[(&str, &str)]) {
        let directory = catalog.join(id);
        fs::create_dir_all(directory.join(SETUP_PAYLOAD)).unwrap();
        fs::write(
            directory.join(SETUP_MANIFEST),
            serde_json::to_vec(&SetupManifest {
                schema_version: SETUP_SCHEMA,
                id: id.to_owned(),
                description: format!("the {id} setup"),
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

    /// A world with a catalog, a target, and one file the provider does not own.
    fn world(name: &str) -> (PathBuf, PathBuf) {
        let base = scratch(name);
        let catalog = base.join("setups");
        fs::create_dir_all(&catalog).unwrap();
        write_setup(
            &catalog,
            "baseline",
            &[("AGENTS.md", "# baseline\n"), ("settings.json", "{}")],
        );
        write_setup(&catalog, "minimal", &[("AGENTS.md", "# minimal\n")]);
        let target = base.join("target");
        fs::write(target.join("unrelated.txt"), "mine").unwrap();
        (catalog, target)
    }

    fn harness() -> Harness {
        crate::wire::tests_support::TEST
    }

    fn setup_at(catalog: &Path, id: &str) -> crate::catalog::Setup {
        Catalog::at(catalog).get(id).unwrap()
    }

    fn install(catalog: &Path, target: &Path, id: &str, operation: Operation) {
        let setup = setup_at(catalog, id);
        mutate(
            &harness(),
            target,
            operation,
            Effect::Materialize { setup: &setup },
            Some(id),
        )
        .unwrap();
    }

    #[test]
    fn installing_a_setup_writes_it_and_leaves_everything_else_alone() {
        let (catalog, target) = world("install");
        install(&catalog, &target, "baseline", Operation::Install);

        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# baseline\n"
        );
        assert!(target.join("settings.json").exists());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn a_setup_installed_into_an_otherwise_empty_target_becomes_its_identity() {
        // The strongest statement this design can make: a target holding nothing
        // but one setup *is* that setup, byte for byte. If these ever diverge,
        // either the materializer changed something on the way in or the digest
        // is measuring something other than what was written.
        let base = scratch("identity-equals-definition");
        let catalog = base.join("setups");
        fs::create_dir_all(&catalog).unwrap();
        write_setup(
            &catalog,
            "exact",
            &[("AGENTS.md", "# exact\n"), ("skills/a.md", "one")],
        );
        let target = base.join("target");

        let setup = setup_at(&catalog, "exact");
        mutate(
            &harness(),
            &target,
            Operation::Install,
            Effect::Materialize { setup: &setup },
            Some("exact"),
        )
        .unwrap();

        let resolved = Target::resolve(&target, harness().control_directory).unwrap();
        let identity = resolved
            .identity_digest_excluding(&harness().not_our_identity())
            .unwrap();
        assert_eq!(identity, setup.definition_digest);
    }

    #[test]
    fn selecting_another_setup_reaches_its_complete_state_not_a_merge() {
        // baseline ships settings.json and minimal does not, so after selecting
        // minimal that file must be gone. A merge would leave it behind, and the
        // target would then be a state neither setup describes.
        let (catalog, target) = world("select");
        install(&catalog, &target, "baseline", Operation::Install);
        assert!(target.join("settings.json").exists());

        install(&catalog, &target, "minimal", Operation::Replace);
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# minimal\n"
        );
        assert!(
            !target.join("settings.json").exists(),
            "select left a file minimal does not own"
        );
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn every_change_captures_what_was_there_before_it() {
        let (catalog, target) = world("captures");
        install(&catalog, &target, "baseline", Operation::Install);
        install(&catalog, &target, "minimal", Operation::Replace);

        let control = target.join(harness().control_directory);
        let records = Pool::open(&control, facts::BACKUP_SLOTS)
            .unwrap()
            .list()
            .unwrap();
        assert_eq!(records.len(), 2);
        // Newest first: the second capture preceded the replace and knew which
        // setup was applied at the time.
        assert_eq!(records[0].operation, "replace");
        assert_eq!(records[0].setup_id.as_deref(), Some("baseline"));
        assert_eq!(
            records[1].setup_id, None,
            "the first capture predates any setup"
        );
    }

    #[test]
    fn restoring_a_named_backup_reaches_the_state_that_backup_holds() {
        let (catalog, target) = world("restore-named");
        install(&catalog, &target, "baseline", Operation::Install);
        install(&catalog, &target, "minimal", Operation::Replace);

        // The first slot predates every install, so restoring it empties what we own.
        mutate(
            &harness(),
            &target,
            Operation::Restore,
            Effect::Restore {
                backup_ref: Some("slot-000000000001".to_owned()),
            },
            None,
        )
        .unwrap();
        assert!(!target.join("AGENTS.md").exists());
        assert!(!target.join("settings.json").exists());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn restoring_without_a_reference_reaches_the_most_recent_capture() {
        let (catalog, target) = world("restore-latest");
        install(&catalog, &target, "baseline", Operation::Install);
        install(&catalog, &target, "minimal", Operation::Replace);

        mutate(
            &harness(),
            &target,
            Operation::Restore,
            Effect::Restore { backup_ref: None },
            None,
        )
        .unwrap();
        // The newest capture preceded the replace, so baseline comes back.
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# baseline\n"
        );
        assert!(target.join("settings.json").exists());
    }

    #[test]
    fn reinstall_repairs_a_hand_edit_without_being_told_which_setup() {
        let (catalog, target) = world("reinstall");
        install(&catalog, &target, "minimal", Operation::Install);
        fs::write(target.join("AGENTS.md"), "# edited by hand\n").unwrap();

        let recorded = match ProviderState::read(&target, harness().state_file).unwrap() {
            StateReading::Current(state) => state.setup_stable_id,
            other => panic!("expected recorded state, got {other:?}"),
        };
        assert_eq!(recorded.as_deref(), Some("minimal"));

        install(&catalog, &target, &recorded.unwrap(), Operation::Replace);
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# minimal\n"
        );
    }

    #[test]
    fn removing_withdraws_what_is_owned_and_keeps_what_is_not() {
        let (catalog, target) = world("remove");
        install(&catalog, &target, "baseline", Operation::Install);

        mutate(&harness(), &target, Operation::Remove, Effect::Remove, None).unwrap();
        assert!(!target.join("AGENTS.md").exists());
        assert!(!target.join("settings.json").exists());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn a_setup_writing_outside_the_declared_surface_never_reaches_the_target() {
        let (catalog, target) = world("outside");
        write_setup(
            &catalog,
            "sneaky",
            &[("AGENTS.md", "x"), ("elsewhere.txt", "y")],
        );
        let setup = setup_at(&catalog, "sneaky");

        let error = mutate(
            &harness(),
            &target,
            Operation::Install,
            Effect::Materialize { setup: &setup },
            Some("sneaky"),
        )
        .unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedNativeSurface));
        assert!(!target.join("elsewhere.txt").exists());
        assert!(
            !target.join("AGENTS.md").exists(),
            "a refused setup wrote part of itself"
        );
    }

    #[test]
    fn the_state_records_which_setup_is_applied_so_reinstall_has_something_to_read() {
        let (catalog, target) = world("state");
        install(&catalog, &target, "baseline", Operation::Install);
        match ProviderState::read(&target, harness().state_file).unwrap() {
            StateReading::Current(state) => {
                assert_eq!(state.setup_stable_id.as_deref(), Some("baseline"));
                assert!(state.backup_ref.is_some());
            }
            other => panic!("expected recorded state, got {other:?}"),
        }
    }

    #[test]
    fn an_operation_id_is_stable_for_the_same_target_and_second() {
        let harness = crate::wire::tests_support::TEST;
        let one = operation_id(&harness, "sha256:abc");
        let two = operation_id(&harness, "sha256:abc");
        assert_eq!(one, two);
        assert_ne!(one, operation_id(&harness, "sha256:def"));
        assert!(one.starts_with("operation_"));
    }
}
