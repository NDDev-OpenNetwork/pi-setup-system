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

use crate::adopt;
use crate::catalog::{CATALOG_DIRECTORY, Catalog};
use crate::expiry;
use crate::facts::{self, Harness};
use crate::wire::{self, Effect, Mutation};

/// How long a plan this surface makes stays applicable.
///
/// It is applied within the same process, so the window only has to cover that.
/// A long one would mean a plan could outlive the state it was made against.
const PLAN_WINDOW_SECONDS: u64 = 600;
/// Every verb this surface answers.
///
/// Named once so nothing has to remember it. `into_command` is the authority
/// and a test below proves this list is exactly what that match accepts, in
/// both directions -- a verb here the match refuses, or a verb the match takes
/// that is missing here, is red.
///
/// Read by [`crate::catalog::misdirecting`], because a setup that tells an
/// agent to run a command this binary refuses is shipping an instruction that
/// fails. One of them did, for six releases.
pub const VERBS: &[&str] = &[
    "adopt",
    "backups",
    "diff",
    "hold",
    "install",
    "list",
    "reinstall",
    "release",
    "remove",
    "restore",
    "rollback",
    "select",
    "software",
    "status",
];

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
    /// Keep one backup until it is released, so retention cannot reclaim it.
    Hold {
        /// The target whose pool holds it.
        target: PathBuf,
        /// The slot to keep. Named, never inferred.
        backup: Option<String>,
        /// Why it is held, so whoever meets a full pool knows the cost.
        reason: Option<String>,
    },
    /// Let retention have a held backup back.
    Release {
        /// The target whose pool holds it.
        target: PathBuf,
        /// The slot to let go.
        backup: Option<String>,
    },
    /// Report which versions of the product are installed, and which is exposed.
    Software {
        /// The program directory to read.
        prefix: PathBuf,
    },
    /// Point the exposed command back at a version already on disk.
    Rollback {
        /// The program directory to change.
        prefix: PathBuf,
        /// The version to expose. Named, never inferred.
        to: Option<String>,
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
    /// Take over a target the frozen estate's program still claims.
    Adopt {
        /// The directory to take over.
        target: PathBuf,
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
        "list"
            | "install"
            | "reinstall"
            | "select"
            | "backups"
            | "restore"
            | "remove"
            | "adopt"
            | "diff"
            | "software"
            | "rollback"
            | "hold"
            | "release"
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
    prefix: Option<PathBuf>,
    to: Option<String>,
    reason: Option<String>,
    positional: Vec<String>,
}

impl Arguments {
    /// Split an argument list into flags and positionals, refusing anything odd.
    fn scan(name: &str, rest: &[String]) -> Result<Self> {
        let mut parsed = Self {
            target: None,
            backup: None,
            prefix: None,
            to: None,
            reason: None,
            positional: Vec::new(),
        };
        let mut index = 0;
        while index < rest.len() {
            let Some(token) = rest.get(index) else { break };
            match token.as_str() {
                "--target" | "--backup" | "--prefix" | "--to" | "--reason" => {
                    let Some(value) = rest.get(index + 1) else {
                        return Err(local(format!("{token} has no value")));
                    };
                    if value.starts_with("--") {
                        return Err(local(format!("{token} has no value")));
                    }
                    match token.as_str() {
                        "--target" => {
                            if parsed.target.is_some() {
                                return Err(local("--target was given twice"));
                            }
                            parsed.target = Some(PathBuf::from(value));
                        }
                        "--prefix" => {
                            if parsed.prefix.is_some() {
                                return Err(local("--prefix was given twice"));
                            }
                            parsed.prefix = Some(PathBuf::from(value));
                        }
                        "--to" => {
                            if parsed.to.is_some() {
                                return Err(local("--to was given twice"));
                            }
                            parsed.to = Some(value.clone());
                        }
                        "--reason" => {
                            if parsed.reason.is_some() {
                                return Err(local("--reason was given twice"));
                            }
                            parsed.reason = Some(value.clone());
                        }
                        _ => {
                            if parsed.backup.is_some() {
                                return Err(local("--backup was given twice"));
                            }
                            parsed.backup = Some(value.clone());
                        }
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

    fn prefix(&self, name: &str) -> Result<PathBuf> {
        self.prefix
            .clone()
            .ok_or_else(|| local(format!("{name} requires --prefix <absolute-directory>")))
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
        if self.backup.is_some() && !matches!(name, "restore" | "hold" | "release") {
            return Err(local(format!("--backup is not an argument of {name}")));
        }
        if self.prefix.is_some() && !matches!(name, "software" | "rollback") {
            return Err(local(format!("--prefix is not an argument of {name}")));
        }
        if self.to.is_some() && name != "rollback" {
            return Err(local(format!("--to is not an argument of {name}")));
        }
        if self.reason.is_some() && name != "hold" {
            return Err(local(format!("--reason is not an argument of {name}")));
        }
        match name {
            "list" => {
                self.no_setup(name)?;
                Ok(Command::List)
            }
            "hold" => {
                self.no_setup(name)?;
                Ok(Command::Hold {
                    target: self.target(name)?,
                    backup: self.backup,
                    reason: self.reason,
                })
            }
            "release" => {
                self.no_setup(name)?;
                Ok(Command::Release {
                    target: self.target(name)?,
                    backup: self.backup,
                })
            }
            "software" => {
                self.no_setup(name)?;
                Ok(Command::Software {
                    prefix: self.prefix(name)?,
                })
            }
            "rollback" => {
                self.no_setup(name)?;
                Ok(Command::Rollback {
                    prefix: self.prefix(name)?,
                    to: self.to,
                })
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
            "adopt" => {
                self.no_setup(name)?;
                Ok(Command::Adopt {
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
        Command::Adopt { target } => adopt_target(harness, &target),
        Command::Remove { target } => remove(harness, &target),
        Command::Hold {
            target,
            backup,
            reason,
        } => hold(harness, &target, backup.as_deref(), reason.as_deref()),
        Command::Release { target, backup } => release(harness, &target, backup.as_deref()),
        Command::Software { prefix } => software(harness, &prefix),
        Command::Rollback { prefix, to } => rollback(harness, &prefix, to.as_deref()),
    }
}

/// Keep one backup until it is released.
///
/// The pool rolls: ten slots, oldest evicted. A long series of captures makes
/// more than that, so a baseline someone means to return to at the end is gone
/// by the time they get there. A hold is the smallest thing that stops it —
/// one marker eviction has to read.
fn hold(
    harness: &Harness,
    target: &Path,
    backup: Option<&str>,
    reason: Option<&str>,
) -> Result<()> {
    let (resolved, pool) = pool_of(harness, target)?;
    let reference = named_slot(&pool, backup, "hold")?;
    // A pool can be held by more than one run. Without a reason, whoever meets
    // a full pool knows which slots to release and not what releasing one
    // would cost.
    let reason = reason.unwrap_or("no reason recorded");
    if pool.hold(&reference, reason)? {
        println!(
            "{} is held ({reason}). Retention will not reclaim it until it is released.",
            reference.as_str()
        );
    } else {
        let why = pool
            .held_reason(&reference)?
            .unwrap_or_else(|| "no reason recorded".to_owned());
        println!("{} was already held ({why}).", reference.as_str());
    }
    println!(
        "  release it with:  release --backup {} --target {}",
        reference.as_str(),
        resolved.root().display()
    );
    Ok(())
}

/// Let retention have a held backup back.
fn release(harness: &Harness, target: &Path, backup: Option<&str>) -> Result<()> {
    let (_, pool) = pool_of(harness, target)?;
    let reference = named_slot(&pool, backup, "release")?;
    if pool.release(&reference)? {
        println!(
            "{} is released and will be reclaimed like any other slot.",
            reference.as_str()
        );
    } else {
        // Not a refusal: a run cleaning up after itself should not have to tell
        // "nothing to do" apart from "something is wrong".
        println!("{} was not held; nothing to release.", reference.as_str());
    }
    Ok(())
}

fn pool_of(harness: &Harness, target: &Path) -> Result<(Target, Pool)> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let pool = Pool::observe(&resolved.control_directory(), facts::BACKUP_SLOTS)?;
    Ok((resolved, pool))
}

/// The slot a caller named, or the list of the ones they could have named.
///
/// Never inferred. `restore` may default to the newest because that is the one
/// thing a caller wanting "undo" can mean; keeping a slot is a decision about a
/// specific capture, and guessing which would be guessing what a run is for.
fn named_slot(
    pool: &Pool,
    backup: Option<&str>,
    verb: &str,
) -> Result<setup_core::backup::BackupRef> {
    let available = pool.list()?;
    let Some(text) = backup else {
        return Err(local(if available.is_empty() {
            format!("{verb} requires --backup <ref>, and this target has no backups")
        } else {
            format!(
                "{verb} requires --backup <ref>; this target holds {}",
                available
                    .iter()
                    .map(|record| record.backup_ref.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }));
    };
    setup_core::backup::BackupRef::parse(text)
        .map_err(|error| local(format!("{text:?} is not a backup reference: {error}")))
}

/// What the program directory holds, and which version answers to the command.
///
/// Install and update are not here, and their absence is the design rather than
/// an omission: both need bytes fetched over a network this program never
/// touches, so they are the three-phase exchange the wire already carries.
/// Rollback and this reading need no network at all, which is exactly why they
/// can be commands someone types.
fn software(harness: &Harness, prefix: &Path) -> Result<()> {
    let declared = declared_software(harness)?;
    let present = setup_core::software::Present::under(prefix, declared.command);

    if present.versions.is_empty() {
        println!(
            "No version of {} is installed under {}.",
            declared.command,
            prefix.display()
        );
        return Ok(());
    }

    println!("{} under {}:", declared.command, prefix.display());
    println!();
    for version in &present.versions {
        let mark = if present.exposed.as_ref() == Some(version) {
            "* "
        } else {
            "  "
        };
        println!("  {mark}{version}");
    }
    println!();
    match &present.exposed {
        Some(version) => println!(
            "  {}/bin/{} runs {version}",
            prefix.display(),
            declared.command
        ),
        // Worth saying rather than leaving blank: the versions are on disk and
        // nothing answers to the command, which is a different situation from
        // having nothing installed.
        None => println!(
            "  Nothing is exposed: {}/bin/{} names no installed version.",
            prefix.display(),
            declared.command
        ),
    }
    if present.versions.len() > 1 {
        println!();
        println!(
            "  Change it with:  rollback --to <version> --prefix {}",
            prefix.display()
        );
    }
    Ok(())
}

/// Point the exposed command back at a version that is already on disk.
fn rollback(harness: &Harness, prefix: &Path, to: Option<&str>) -> Result<()> {
    let declared = declared_software(harness)?;
    let present = setup_core::software::Present::under(prefix, declared.command);

    // Named, never inferred. There is no record of what was previous -- only
    // what is on disk -- and these version strings do not order reliably:
    // cursor's `2026.08.11-e8db854` sorts by string, not by release. Choosing
    // "the one before" would invent an ordering the vendor never promised.
    let Some(version) = to else {
        return Err(local(if present.versions.is_empty() {
            format!(
                "rollback requires --to <version>, and {} holds none",
                prefix.display()
            )
        } else {
            format!(
                "rollback requires --to <version>; {} holds {}",
                prefix.display(),
                present.versions.join(", ")
            )
        }));
    };

    if present.exposed.as_deref() == Some(version) {
        println!(
            "{} already runs {version}; nothing to do.",
            declared.command
        );
        return Ok(());
    }

    let rolled = setup_core::software::rollback(&declared, prefix, version)?;
    println!(
        "{} now runs {}.",
        rolled.executable.display(),
        rolled.version
    );
    if let Some(previous) = present.exposed {
        println!("  it ran {previous} before this, and that tree is still here");
    }
    Ok(())
}

/// The software this build installs, or the reason it installs none.
///
/// Two different absences, and they are worth separating. A harness with no
/// `software` at all does not offer the lifecycle. A harness whose delivery is a
/// package manager does -- the product is installable, just not by fetching
/// bytes whose digest was fixed in advance -- and answering "nothing is
/// installed under this prefix" would suggest this build could put something
/// there. It cannot, and pi is the one that would have been told so.
fn declared_software(harness: &Harness) -> Result<setup_core::software::Software> {
    let declared = harness.software.ok_or_else(|| {
        local(format!(
            "{} configures {} and does not install it",
            harness.provider_id, harness.product
        ))
    })?;
    if let setup_core::software::Delivery::Manager { tool, reason } = declared.delivery {
        return Err(local(format!(
            "{} is delivered by {tool}: {reason}",
            declared.command
        )));
    }
    Ok(declared)
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

/// Break a description into lines a terminal can hold, at word boundaries.
///
/// `list` printed each description as one `println!`, which was fine while
/// every description was a sentence. The `full-auto` postures say what they
/// turn off *and* which axis they are, and the longest reached 330 characters
/// on one line -- unreadable in exactly the moment someone is choosing what to
/// install.
///
/// Fixed width rather than the terminal's: asking the terminal makes the
/// output depend on where it ran, and two runs of `list` that disagree are
/// worse than a line that is occasionally short. A word longer than the width
/// is emitted whole rather than cut, because a broken URL helps nobody.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.is_empty() {
            line.push_str(word);
        } else if line.chars().count() + 1 + word.chars().count() <= width {
            line.push(' ');
            line.push_str(word);
        } else {
            lines.push(std::mem::take(&mut line));
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The width a description is wrapped to, indent excluded.
const DESCRIPTION_WIDTH: usize = 72;

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
        for line in wrapped(&setup.manifest.description, DESCRIPTION_WIDTH) {
            println!("      {line}");
        }
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
    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
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
            // One condition, one sentence. A target restored to a slot captured
            // before any setup existed holds exactly what an untouched target
            // holds, and used to read `(unnamed)` there while the same emptiness
            // one command earlier read `none applied`. Two sentences for one
            // state, and the reader has to know which path got them here to tell
            // that they mean the same thing.
            //
            // So both now open with `none`, and the clause after it says why
            // this one has a state file at all.
            match state.setup_stable_id.as_deref() {
                Some(id) => println!("Setup    {id}"),
                None => println!("Setup    none — the last operation named no setup"),
            }
            println!("Applied  operation {}", state.operation_id);
            if identity == state.target_identity_digest {
                println!("Drift    none");
            } else {
                println!("Drift    the target has changed since it was applied");
                println!("         run `diff` to see where, or `reinstall` to put it back");
            }
        }
    }

    // Observing, not opening: reporting how many backups exist must not create
    // the place they would live. Asking a question should not change its answer.
    let pool = Pool::observe(&resolved.control_directory(), facts::BACKUP_SLOTS)?;
    println!("Backups  {}", pool.list()?.len());
    Ok(())
}

fn backups(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let records = Pool::observe(&resolved.control_directory(), facts::BACKUP_SLOTS)?.list()?;
    if records.is_empty() {
        println!("No backups. One is captured before every change.");
        return Ok(());
    }
    println!("Backups of {}, newest first:", resolved.root().display());
    println!();
    let pool = Pool::observe(&resolved.control_directory(), facts::BACKUP_SLOTS)?;
    for (position, record) in records.iter().enumerate() {
        let mut marker = String::new();
        if position == 0 {
            marker.push_str("  (restored by default)");
        }
        if let Some(why) = pool.held_reason(&record.backup_ref)? {
            use std::fmt::Write as _;
            let _ = write!(marker, "  (held: {why})");
        }
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
    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
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
        wire::Applied {
            setup_id: Some(setup_id.to_owned()),
            setup_definition_digest: Some(setup.definition_digest.clone()),
            ..wire::Applied::default()
        },
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
        wire::Applied::default(),
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
    let report = mutate(
        harness,
        target,
        Operation::Remove,
        Effect::Remove,
        wire::Applied::default(),
    )?;
    println!(
        "Removed everything {} owns from {}.",
        harness.provider_id,
        target.display()
    );
    // **Say what "owns" means, because the sentence above is true and is heard
    // wrong.** A person reads "removed everything it owns" as "removed the
    // files it installed". `remove_managed` walks `native_namespaces` and calls
    // `remove_dir_all` on each, so a namespace goes whole -- including whatever
    // the person put there themselves.
    //
    // It matters most where a declaration keeps a transition window open. Cursor
    // owns `plugins` *and* `plugins/local`; the bytes this provider writes are
    // all under the second, and `remove` takes the first, which is where a
    // marketplace plugin lives. Nothing is lost -- the capture named below runs
    // before the removal, over exactly these namespaces, and `restore` returns
    // them byte-exact -- but "nothing is lost" is only true if the person knows
    // to restore, and they only know if they are told what went.
    println!(
        "  taken whole, not file by file: {}",
        harness.native_namespaces.join(", ")
    );
    println!("  anything you put under those went too, and is in the capture below");
    println!(
        "  previous state captured as {}",
        report_field(&report, "backup_ref")
    );
    println!("  restore it with:  restore --target {}", target.display());
    Ok(())
}

/// Take over a target the frozen estate's program still claims.
///
/// Everything that could refuse happens before anything is captured or moved:
/// the stamp is read, its schema and its directory are checked, every file it
/// claims is accounted for against what is on disk, and any path it names that
/// this provider does not own is a refusal rather than a silent partial claim.
fn adopt_target(harness: &Harness, target: &Path) -> Result<()> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let Some((stamp, found)) = adopt::read(harness, &resolved)? else {
        return Err(local(if harness.predecessor_state_file.is_empty() {
            format!(
                "{} had no module in the frozen estate, so there is no stamp to adopt",
                harness.product
            )
        } else {
            format!(
                "{} holds no {}; there is nothing to adopt",
                resolved.root().display(),
                harness.predecessor_state_file
            )
        }));
    };

    let outside = found.outside(harness);
    if !outside.is_empty() {
        return Err(local(format!(
            "{} claims {}, which {} does not own; adopting it would record ownership of files no              later operation of this provider would write, restore or remove",
            stamp.display(),
            outside.join(", "),
            harness.provider_id
        )));
    }

    if let Some(elsewhere) = found.written_elsewhere(&resolved) {
        println!(
            "This stamp was written for {elsewhere}, and is being adopted at {}.",
            resolved.root().display()
        );
        println!("  every path it claims is checked against this target, not that one");
    }

    let accounted = found.account_for(&resolved)?;
    println!(
        "{} wrote {} for setup {:?}, build {}.",
        found.product_name, harness.predecessor_state_file, found.setup_id, found.build_version
    );
    for (relative, claim) in &accounted {
        println!("  {:-8} {relative}", claim.as_str());
    }
    let changed = accounted
        .iter()
        .filter(|(_, claim)| *claim != adopt::Claim::Intact)
        .count();
    if changed > 0 {
        println!(
            "  {changed} of {} are not what the stamp recorded; they are adopted as they are, and              the backup below holds them",
            accounted.len()
        );
    }

    let report = mutate(
        harness,
        target,
        Operation::Install,
        Effect::Adopt {
            stamp: stamp.clone(),
        },
        wire::Applied {
            setup_id: Some(found.setup_id.clone()),
            ..wire::Applied::default()
        },
    )?;

    println!();
    println!(
        "{} now owns {}.",
        harness.provider_id,
        resolved.root().display()
    );
    println!(
        "  previous state captured as {}",
        report_field(&report, "backup_ref")
    );
    println!(
        "  the old stamp is kept at {}/adopted/{}",
        harness.control_directory, harness.predecessor_state_file
    );
    Ok(())
}

/// Build a plan and apply it through the one write path.
fn mutate(
    harness: &Harness,
    target: &Path,
    operation: Operation,
    effect: Effect<'_>,
    applied: wire::Applied,
) -> Result<serde_json::Value> {
    let resolved = Target::resolve(target, harness.control_directory)?;
    let identity =
        resolved.identity_of_owned(harness.owned_projection(), &harness.not_our_identity())?;
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
        // The human surface drives configuration, never the product's own
        // install: that arrives over the wire, with artifacts somebody else
        // downloaded between planning and applying.
        software_artifacts: Vec::new(),
        effects: effect_lines(harness, &effect, applied.setup_id.as_deref()),
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
            applied,
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
        Effect::Adopt { stamp } => vec![
            capture,
            format!(
                "record {} as the setup this target holds, without writing one file of it",
                setup_id.unwrap_or("the setup the old stamp names")
            ),
            format!(
                "move {} into {}/adopted, where the old program no longer sees it",
                stamp.display(),
                harness.control_directory
            ),
        ],
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

    /// `rollback` names its version and never infers one.
    ///
    /// There is no record of what was previous -- only what is on disk -- and
    /// these version strings do not order reliably: cursor's
    /// `2026.08.11-e8db854` sorts by string, not by release. Choosing "the one
    /// before" would invent an ordering the vendor never promised, and pointing
    /// a command at the wrong build is the class of mistake this program
    /// refuses everywhere else.
    #[test]
    fn rollback_requires_the_version_it_is_going_to() {
        let parsed = parse(["rollback", "--prefix", "/tmp/prefix"]).unwrap();
        assert_eq!(
            parsed,
            Command::Rollback {
                prefix: std::path::PathBuf::from("/tmp/prefix"),
                to: None
            }
        );
        let with = parse(["rollback", "--to", "1.2.2", "--prefix", "/tmp/prefix"]).unwrap();
        assert_eq!(
            with,
            Command::Rollback {
                prefix: std::path::PathBuf::from("/tmp/prefix"),
                to: Some("1.2.2".to_owned())
            }
        );
    }

    /// The two new flags belong to the two new commands and nowhere else.
    #[test]
    fn prefix_and_to_are_refused_on_commands_that_do_not_take_them() {
        for tokens in [
            vec![
                "install", "baseline", "--target", "/tmp/t", "--prefix", "/tmp/p",
            ],
            vec!["restore", "--target", "/tmp/t", "--to", "1.2.2"],
            vec!["status", "--target", "/tmp/t", "--to", "1.2.2"],
        ] {
            let error = parse(tokens.clone()).unwrap_err();
            assert!(
                error.detail().contains("is not an argument of"),
                "{tokens:?} was accepted: {}",
                error.detail()
            );
        }
    }

    /// Two different absences, and the wrong one is misleading.
    ///
    /// Pi's `software` is `Some` and its delivery is npm, so before this was
    /// separated `software --prefix` answered *no version of pi is installed
    /// under /tmp/x* -- which reads as an invitation to install one. This build
    /// cannot, and says so in the same words `plan-operation` uses.
    #[test]
    fn a_product_delivered_by_a_package_manager_says_so_rather_than_looking_empty() {
        let mut managed = crate::wire::tests_support::TEST;
        managed.software = Some(setup_core::software::Software {
            version: "1.0.0",
            command: "managed",
            delivery: setup_core::software::Delivery::Manager {
                tool: "npm",
                reason: "its closure is resolved at install time",
            },
            unsupported: &[],
        });
        assert!(!managed.installs_a_program());

        let error = declared_software(&managed).unwrap_err();
        assert!(error.detail().contains("npm"), "{}", error.detail());
        assert!(
            !error.detail().contains("no version"),
            "it read as an empty prefix: {}",
            error.detail()
        );

        // And a build that installs nothing at all is a third answer again.
        let mut none = crate::wire::tests_support::TEST;
        none.software = None;
        assert!(!none.installs_a_program());
        assert!(
            declared_software(&none)
                .unwrap_err()
                .detail()
                .contains("does not install it")
        );
    }

    /// Both new commands take a program directory, and neither guesses one.
    #[test]
    fn software_and_rollback_require_a_prefix() {
        for name in ["software", "rollback"] {
            let error = parse([name]).unwrap_err();
            assert!(error.detail().contains("--prefix"), "{}", error.detail());
        }
    }

    use crate::wire::tests_support::TEST;

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
            "adopt",
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
            "adopt",
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
        let base =
            std::env::temp_dir().join(format!("harness-human-{name}-{}", std::process::id()));
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
            wire::Applied {
                setup_id: Some(id.to_owned()),
                setup_definition_digest: Some(setup.definition_digest.clone()),
                ..wire::Applied::default()
            },
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
            wire::Applied {
                setup_id: Some("exact".to_owned()),
                ..wire::Applied::default()
            },
        )
        .unwrap();

        let resolved = Target::resolve(&target, harness().control_directory).unwrap();
        let identity = resolved
            .identity_of_owned(harness().owned_projection(), &harness().not_our_identity())
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
            wire::Applied::default(),
        )
        .unwrap();
        assert!(!target.join("AGENTS.md").exists());
        assert!(!target.join("settings.json").exists());
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
    }

    /// Read the provider state a mutation left behind.
    fn state(target: &Path) -> setup_core::stamp::ProviderState {
        match ProviderState::read(target, harness().state_file).unwrap() {
            StateReading::Current(current) => *current,
            other => panic!("no current state: {other:?}"),
        }
    }

    #[test]
    fn a_restore_names_the_setup_the_slot_it_restored_wrote_down() {
        // A restore returns exact bytes, and until this guard it returned them
        // anonymously: the slot records which setup was in effect when it was
        // captured, and the restore threw that away. `status` then said
        // "(unnamed)" about a target that was byte-for-byte a known setup.
        let (catalog, target) = world("restore-names-its-setup");
        install(&catalog, &target, "baseline", Operation::Install);
        let installed = state(&target);
        install(&catalog, &target, "minimal", Operation::Replace);

        mutate(
            &harness(),
            &target,
            Operation::Restore,
            Effect::Restore { backup_ref: None },
            wire::Applied::default(),
        )
        .unwrap();

        let restored = state(&target);
        assert_eq!(
            restored.target_identity_digest, installed.target_identity_digest,
            "the bytes must come back exactly"
        );
        assert_eq!(
            restored.setup_stable_id.as_deref(),
            Some("baseline"),
            "and so must the name the slot recorded"
        );
        assert_eq!(
            restored.setup_definition_digest, installed.setup_definition_digest,
            "and the definition it was identified by"
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
            wire::Applied::default(),
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

        mutate(
            &harness(),
            &target,
            Operation::Remove,
            Effect::Remove,
            wire::Applied::default(),
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
            wire::Applied {
                setup_id: Some("sneaky".to_owned()),
                ..wire::Applied::default()
            },
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

    // ── adoption ─────────────────────────────────────────────────────────────

    /// A target holding what the frozen estate's stamp claims.
    fn estate_managed(name: &str, files: &[(&str, &str)], claimed: &[(&str, &str)]) -> PathBuf {
        let target = std::env::temp_dir().join(format!("adopt-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).unwrap();
        for (relative, body) in files {
            let at = target.join(relative);
            if let Some(parent) = at.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(at, body).unwrap();
        }
        let managed: serde_json::Map<String, serde_json::Value> = claimed
            .iter()
            .map(|(relative, hex)| ((*relative).to_owned(), serde_json::json!(hex)))
            .collect();
        fs::write(
            target.join(TEST.predecessor_state_file),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "product_name": "nddev-test-app",
                "build_version": "0.1.0",
                "setup_id": "full-auto",
                "canonical_target": target.to_string_lossy(),
                "managed_files": managed,
            }))
            .unwrap(),
        )
        .unwrap();
        target
    }

    fn hex_of(body: &str) -> String {
        setup_core::digest::of_bytes(body.as_bytes())
            .trim_start_matches(setup_core::digest::PREFIX)
            .to_owned()
    }

    #[test]
    fn adopt_takes_over_a_target_the_old_program_still_claims() {
        let body = "# from the estate\n";
        let target = estate_managed(
            "takeover",
            &[("AGENTS.md", body)],
            &[("AGENTS.md", &hex_of(body))],
        );

        run(
            &TEST,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap();

        // The setup the old stamp named is now what this provider records.
        let resolved = Target::resolve(&target, TEST.control_directory).unwrap();
        let StateReading::Current(state) =
            ProviderState::read(resolved.root(), TEST.state_file).unwrap()
        else {
            panic!("adoption left no state");
        };
        assert_eq!(state.setup_stable_id.as_deref(), Some("full-auto"));

        // The old program looks for its stamp at the top level and no longer
        // finds it — but nothing was destroyed.
        assert!(!target.join(TEST.predecessor_state_file).exists());
        assert!(
            target
                .join(TEST.control_directory)
                .join("adopted")
                .join(TEST.predecessor_state_file)
                .is_file()
        );

        // The file the stamp claimed is untouched: adoption changes who owns
        // the target, not what is in it.
        assert_eq!(fs::read_to_string(target.join("AGENTS.md")).unwrap(), body);

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn adopt_captures_the_state_it_took_over() {
        let body = "# before\n";
        let target = estate_managed(
            "captured",
            &[("AGENTS.md", body)],
            &[("AGENTS.md", &hex_of(body))],
        );
        run(
            &TEST,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap();

        let resolved = Target::resolve(&target, TEST.control_directory).unwrap();
        let slots = resolved.root().join(TEST.control_directory).join("backups");
        let taken = fs::read_dir(&slots).map_or(0, Iterator::count);
        assert_eq!(taken, 1, "adoption captured nothing to return to");

        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn a_stamp_claiming_what_this_provider_does_not_own_is_refused() {
        // Recording ownership of a path no later operation would write,
        // restore or remove is a claim this build could not keep.
        let target = estate_managed(
            "outside",
            &[("AGENTS.md", "x"), ("somebody-elses.toml", "y")],
            &[
                ("AGENTS.md", &hex_of("x")),
                ("somebody-elses.toml", &hex_of("y")),
            ],
        );
        let error = run(
            &TEST,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap_err();
        assert!(
            error.detail().contains("somebody-elses.toml"),
            "{}",
            error.detail()
        );
        // Nothing was taken over, and the stamp is where it was.
        assert!(target.join(TEST.predecessor_state_file).is_file());
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn adopt_on_a_target_with_no_stamp_says_there_is_nothing_to_adopt() {
        let target = std::env::temp_dir().join(format!("adopt-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).unwrap();
        let error = run(
            &TEST,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap_err();
        assert!(
            error.detail().contains("nothing to adopt"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn a_harness_with_no_predecessor_says_so_rather_than_looking_for_a_file() {
        let mut fresh = TEST;
        fresh.predecessor_state_file = "";
        let target = std::env::temp_dir().join(format!("adopt-fresh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&target);
        fs::create_dir_all(&target).unwrap();
        let error = run(
            &fresh,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap_err();
        assert!(
            error.detail().contains("no module in the frozen estate"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&target);
    }

    #[test]
    fn a_file_that_drifted_since_the_stamp_is_accounted_for_not_assumed() {
        // The estate's own digest is the only record of what it wrote. A file
        // that no longer matches is adopted as it is, and the backup holds it —
        // but the difference is stated rather than passed over.
        let target = estate_managed(
            "drifted",
            &[("AGENTS.md", "what is there now")],
            &[("AGENTS.md", &hex_of("what the estate wrote"))],
        );
        let resolved = Target::resolve(&target, TEST.control_directory).unwrap();
        let (_, found) = adopt::read(&TEST, &resolved).unwrap().unwrap();
        let accounted = found.account_for(&resolved).unwrap();
        assert_eq!(
            accounted,
            vec![("AGENTS.md".to_owned(), adopt::Claim::Changed)]
        );

        // And adoption still succeeds: it is a takeover, not a verification.
        run(
            &TEST,
            Command::Adopt {
                target: target.clone(),
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "what is there now"
        );
        let _ = fs::remove_dir_all(&target);
    }

    // ── a populated home, through the whole lifecycle ────────────────────────

    /// Fingerprint a tree using nothing this program owns.
    ///
    /// `setup_core::digest::of_tree` is what the provider itself uses to decide
    /// a target is unchanged, so comparing against it would be asking the same
    /// function twice and believing the answer. This walks with `std::fs` and
    /// hashes with `sha2` directly. A restore that returned *almost* the right
    /// tree — a mode dropped, an empty directory lost, a byte reordered — is
    /// caught here and would not be caught by the other.
    ///
    /// The provider's own bookkeeping is skipped: the control directory and the
    /// state file are this build's, not the target's, and they are supposed to
    /// appear.
    fn independent_fingerprint(root: &Path, harness: &Harness) -> String {
        fn walk(at: &Path, base: &Path, skip: &[&str], into: &mut Vec<String>) {
            let mut entries: Vec<_> = fs::read_dir(at)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if skip.iter().any(|name| relative == *name) {
                    continue;
                }
                let found = fs::symlink_metadata(&path).unwrap();
                if found.is_dir() {
                    into.push(format!("d {relative}"));
                    walk(&path, base, skip, into);
                } else if found.is_symlink() {
                    let to = fs::read_link(&path).unwrap();
                    into.push(format!("l {relative} -> {}", to.to_string_lossy()));
                } else {
                    let bytes = fs::read(&path).unwrap();
                    let readonly = found.permissions().readonly();
                    into.push(format!(
                        "f {relative} {} {} ro={readonly}",
                        bytes.len(),
                        setup_core::digest::of_bytes(&bytes)
                    ));
                }
            }
        }
        let skip = [harness.control_directory, harness.state_file];
        let mut lines = Vec::new();
        walk(root, root, &skip, &mut lines);
        setup_core::digest::of_bytes(lines.join("\n").as_bytes())
    }

    /// A target with the awkward content a real configuration home grows.
    fn populated(name: &str) -> (PathBuf, PathBuf) {
        let base = scratch(name);
        let catalog = base.join("setups");
        fs::create_dir_all(&catalog).unwrap();
        write_setup(
            &catalog,
            "baseline",
            &[("AGENTS.md", "# baseline\n"), ("settings.json", "{}")],
        );
        let target = base.join("target");

        // Inside what this provider owns.
        let deep = target.join("skills/one/two/three/four/five/six");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("leaf.md"), "a long way down\n").unwrap();
        fs::write(target.join("skills/пример.md"), "имя не в ASCII\n").unwrap();
        fs::write(target.join("skills/empty.md"), b"").unwrap();
        // CRLF on purpose: a restore that normalized line endings would be
        // returning a different file and reporting success.
        fs::write(target.join("skills/crlf.md"), b"one\r\ntwo\r\n").unwrap();
        fs::write(target.join("skills/big.md"), vec![b'x'; 300_000]).unwrap();
        fs::create_dir_all(target.join("skills/an-empty-directory")).unwrap();
        fs::write(target.join("AGENTS.md"), "# what was here first\n").unwrap();

        // Beside it, and none of this provider's business.
        fs::write(target.join("unrelated.txt"), "mine").unwrap();
        fs::write(target.join(".credentials.json"), "SECRET").unwrap();
        fs::create_dir_all(target.join("sessions/2026")).unwrap();
        fs::write(target.join("sessions/2026/log.jsonl"), "{}\n").unwrap();

        (catalog, target)
    }

    #[test]
    fn a_populated_target_comes_back_byte_for_byte_after_a_restore() {
        let (catalog, target) = populated("populated-restore");
        let harness = harness();
        let before = independent_fingerprint(&target, &harness);

        install(&catalog, &target, "baseline", Operation::Install);

        // What the setup declares is now what is there, and the deep tree that
        // was in the same namespace is gone with it — that is what owning a
        // namespace means.
        assert_eq!(
            fs::read_to_string(target.join("AGENTS.md")).unwrap(),
            "# baseline\n"
        );
        assert!(!target.join("skills/one").exists());

        // Nothing outside those namespaces moved.
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
        assert_eq!(
            fs::read_to_string(target.join(".credentials.json")).unwrap(),
            "SECRET"
        );
        assert!(target.join("sessions/2026/log.jsonl").is_file());

        run(
            &harness,
            Command::Restore {
                target: target.clone(),
                backup: None,
            },
        )
        .unwrap();

        assert_eq!(
            independent_fingerprint(&target, &harness),
            before,
            "the restored target is not the one that was captured"
        );

        let _ = fs::remove_dir_all(target.parent().unwrap());
    }

    #[test]
    fn a_backup_never_holds_what_it_could_not_put_back() {
        // The slot is what a restore replays. A credential swept into one would
        // be a secret this program copied without being asked, and a restore
        // would then write it back over whatever the product had since stored.
        let (catalog, target) = populated("populated-slot");
        install(&catalog, &target, "baseline", Operation::Install);

        let slots = target.join(harness().control_directory).join("backups");
        let mut swept = Vec::new();
        for slot in fs::read_dir(&slots).unwrap() {
            let slot = slot.unwrap().path();
            if slot.is_dir() {
                for found in walkdir(&slot) {
                    let name = found.to_string_lossy().into_owned();
                    if name.contains("credentials") || name.contains("sessions") {
                        swept.push(name);
                    }
                }
            }
        }
        assert!(swept.is_empty(), "a backup slot holds {swept:?}");

        let _ = fs::remove_dir_all(target.parent().unwrap());
    }

    /// Every path under a root, for a test that needs to look at all of them.
    fn walkdir(root: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(at) = pending.pop() {
            let Ok(entries) = fs::read_dir(&at) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path.clone());
                }
                found.push(path);
            }
        }
        found
    }

    #[test]
    fn remove_takes_a_populated_namespace_and_leaves_the_rest() {
        let (catalog, target) = populated("populated-remove");
        let harness = harness();
        install(&catalog, &target, "baseline", Operation::Install);

        run(
            &harness,
            Command::Remove {
                target: target.clone(),
            },
        )
        .unwrap();

        for owned in ["AGENTS.md", "settings.json", "skills"] {
            assert!(
                !target.join(owned).exists(),
                "{owned} survived a remove that claims to own it"
            );
        }
        assert_eq!(
            fs::read_to_string(target.join("unrelated.txt")).unwrap(),
            "mine"
        );
        assert!(target.join("sessions/2026/log.jsonl").is_file());

        let _ = fs::remove_dir_all(target.parent().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn a_read_only_file_in_an_owned_namespace_is_still_replaced() {
        // A product, or a person, can leave a file unwritable. The namespace is
        // still this provider's to replace, and failing there would leave a
        // half-applied target behind a permission bit.
        use std::os::unix::fs::PermissionsExt;
        let (catalog, target) = populated("populated-readonly");
        let stubborn = target.join("skills/пример.md");
        fs::set_permissions(&stubborn, fs::Permissions::from_mode(0o444)).unwrap();

        install(&catalog, &target, "baseline", Operation::Install);
        assert!(!stubborn.exists(), "a read-only file blocked the install");

        let _ = fs::remove_dir_all(target.parent().unwrap());
    }
    /// A description a terminal can hold, broken where words end.
    #[test]
    fn a_long_description_is_wrapped_at_word_boundaries() {
        let text = "Full auto: nothing is asked and nothing is sandboxed. This is a \
                    setup posture -- keys in this product's own configuration file.";
        let lines = wrapped(text, 40);
        assert!(lines.len() > 1, "{lines:?}");
        for line in &lines {
            assert!(line.chars().count() <= 40, "{line:?}");
            assert!(!line.starts_with(' ') && !line.ends_with(' '), "{line:?}");
        }
        assert_eq!(
            lines.join(" "),
            text.split_whitespace().collect::<Vec<_>>().join(" "),
            "wrapping must not lose or invent a word"
        );
    }

    /// A word longer than the width is emitted whole.
    ///
    /// Cutting it would produce a broken URL, and a description that cites a
    /// vendor page is exactly where a long word appears.
    #[test]
    fn a_word_longer_than_the_width_is_not_cut() {
        let long = "https://learn.chatgpt.com/docs/config-file/config-reference";
        // Both positions, because they take different branches: a long word
        // that *starts* a line is pushed into an empty buffer, and one that
        // follows a word is pushed after the buffer is flushed. An earlier
        // version of this test asserted only the second, and a mutation that
        // truncated the first left it green.
        for text in [
            format!("see {long} for this"),
            format!("{long} is the page"),
        ] {
            let lines = wrapped(&text, 20);
            assert!(
                lines.iter().any(|line| line == long),
                "{text:?} became {lines:?}"
            );
        }
    }

    /// Nothing in, nothing out -- rather than one empty line.
    #[test]
    fn an_empty_description_wraps_to_nothing() {
        assert!(wrapped("", 40).is_empty());
        assert!(wrapped("   ", 40).is_empty());
    }
    /// `VERBS` is exactly what `into_command` accepts, both ways.
    ///
    /// A list beside a match is two expressions of one fact, and this project
    /// has paid for that five times. Here the match is the authority and the
    /// list is checked against it, so a verb added to one and not the other is
    /// red rather than merely inconsistent.
    #[test]
    fn the_named_verbs_are_the_ones_the_parser_takes() {
        for verb in VERBS {
            let refused = Arguments::scan(verb, &[])
                .unwrap()
                .into_command(verb)
                .is_err_and(|error| error.detail().contains("is not a command"));
            assert!(!refused, "{verb} is named and the parser does not know it");
        }
        // The other direction: anything not named must be refused as unknown,
        // rather than quietly parsed.
        for absent in [
            "migrate",
            "switch",
            "plan",
            "update",
            "install-cli",
            "software-status",
        ] {
            let error = Arguments::scan(absent, &[])
                .unwrap()
                .into_command(absent)
                .unwrap_err();
            assert!(
                error.detail().contains("is not a command"),
                "{absent} is not named and the parser did not refuse it as unknown: {}",
                error.detail()
            );
        }
    }
}
