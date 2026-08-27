//! The product's own lifecycle: planning what to fetch, and installing it.
//!
//! The argv and the plan shape here are not this program's invention. They were
//! proposed on `ai-engineers-guild/ai_stp#414`, agreed, and recorded in that
//! project's `docs/contracts/provider-protocol.md`. Where an earlier version of
//! this file guessed, it guessed differently -- one `--artifact`, a single
//! object, the program under the target -- and the agreement is what this now
//! follows:
//!
//! * `--target` is the configuration directory; `--prefix` is the program
//!   directory. Different paths with different lifetimes, both absolute.
//! * The plan carries an array. One element is one file, and `apply` receives
//!   one repeated `--software-artifact` per element **in that order**, so which
//!   file answers which entry is never inferred.
//! * `--software-version` omitted means the pinned version; given means exactly
//!   that one.
//! * An unpinned platform refuses with `unsupported_platform`.
//! * `software_remove` plans and applies with no download and no artifact.
//!
//! It is deliberately not routed through [`crate::wire::perform`], and the
//! reason is not convenience. That path exists to mutate the namespaces this
//! provider owns inside a *target*: it captures a backup slot, re-checks the
//! target's identity, and journals. A software install writes under `--prefix`
//! and touches no namespace, so running it through that path would spend one of
//! ten backup slots on a capture nobody can use. Installing ten times would
//! evict every configuration backup the target had.
//!
//! What it does keep is a lock, because two installs racing into one directory
//! is a real failure. It needs nothing more, because the layout makes the
//! operation atomic by construction: bytes land in a directory named for their
//! version, and the entry point is pointed at them only once every byte is
//! written. An interrupted install leaves a partial directory the next one
//! replaces, and an entry point still naming the version that worked.

use std::path::{Path, PathBuf};

use provider_v3::plan::SoftwareArtifact;
use provider_v3::{Error, Operation, Result, WireReason};
use setup_core::platform_of_this_host;
use setup_core::software::{self, Delivery, Software};

use crate::facts::Harness;

/// The software this harness installs, or the reason it installs none.
fn declared(harness: &Harness) -> Result<Software> {
    match harness.software {
        Some(
            found @ Software {
                delivery: Delivery::Artifacts(_),
                ..
            },
        ) => Ok(found),
        Some(Software {
            delivery: Delivery::Manager { tool, reason },
            command,
            ..
        }) => Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "{command} is installed by {tool}, which resolves a dependency closure: {reason}"
            ),
        )),
        None => Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "{} does not implement the software lifecycle",
                harness.provider_id
            ),
        )),
    }
}

/// The program directory a software operation was given.
fn program_directory(prefix: Option<&Path>, operation: Operation) -> Result<PathBuf> {
    prefix.map(Path::to_path_buf).ok_or_else(|| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{operation} installs a program, which lives under --prefix, not under --target; \
                 name an absolute --prefix"
            ),
        )
    })
}

/// The version this operation is for, refusing one this build does not pin.
fn version_for(declared: &Software, asked: Option<&str>) -> Result<()> {
    match asked {
        None => Ok(()),
        Some(wanted) if wanted == declared.version => Ok(()),
        Some(wanted) => Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "this build pins {} {}; it cannot install {wanted}, and installing the pinned one \
                 instead would be answering a question nobody asked",
                declared.command, declared.version
            ),
        )),
    }
}

/// Plan one software operation: name the exact bytes, with no network open.
///
/// # Errors
///
/// Refuses when this harness declares no software lifecycle, when no `--prefix`
/// was given, when a version other than the pinned one was asked for, or when
/// the vendor publishes no build for the running platform.
pub(crate) fn plan(
    harness: &Harness,
    prefix: Option<&Path>,
    operation: Operation,
    software_version: Option<&str>,
) -> Result<(Vec<SoftwareArtifact>, Vec<String>)> {
    let declared = declared(harness)?;
    let root = program_directory(prefix, operation)?;
    version_for(&declared, software_version)?;

    // Not always the command: pi's entry point is JavaScript, and Windows runs
    // a file by its extension rather than by a shebang, so what is exposed
    // there is `pi.cmd`. The plan states what a caller will actually be able to
    // run, which is the whole point of naming it.
    let entry_point = format!(
        "bin/{}",
        setup_core::software::exposed_name(declared.command, declared.member_hint())
    );
    let exposed = root.join(&entry_point);

    // Planning may read the local disk and may not reach the network, so what
    // is already under the prefix belongs in the plan. Without it an install
    // and an update produced byte-identical effects -- two names for one act,
    // and neither said what was about to be replaced.
    let present = software::Present::under_named(&root, declared.command, declared.member_hint());

    if operation == Operation::SoftwareRemove {
        let mut effects = vec![
            format!(
                "remove {}, the {} tree this provider installed",
                root.join(declared.version).display(),
                declared.version
            ),
            format!("remove {}", exposed.display()),
        ];
        // Other versions are left, and saying which is the difference between
        // leaving them and losing track of them. This build pins one version
        // and cannot know whether an older tree is still wanted.
        let kept: Vec<&str> = present
            .versions
            .iter()
            .map(String::as_str)
            .filter(|version| *version != declared.version)
            .collect();
        if !kept.is_empty() {
            effects.push(format!(
                "leave {} in place: this build pins {} and does not decide about versions it \
                 does not pin",
                kept.join(", "),
                declared.version
            ));
        }
        return Ok((Vec::new(), effects));
    }

    // An update of nothing is a request that cannot be honoured as asked.
    // Installing instead would be doing something else and calling it done.
    if operation == Operation::SoftwareUpdate && present.versions.is_empty() {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "there is no {} under {} to update; software_install is the operation that \
                 puts one there",
                declared.command,
                root.display()
            ),
        ));
    }

    let (os, arch) = platform_of_this_host();
    let artifact = declared.artifact_for(os, arch)?;
    let mut effects = Vec::new();
    if present.holds(declared.version) {
        effects.push(format!(
            "replace {}, which is already installed",
            declared.version
        ));
    } else if let Some(running) = present.exposed.as_deref() {
        effects.push(format!(
            "move {} from {running} to {}, keeping {running} where it is",
            declared.command, declared.version
        ));
    } else if !present.versions.is_empty() {
        effects.push(format!(
            "install {} beside {}, which no entry point names",
            declared.version,
            present.versions.join(", ")
        ));
    }
    effects.extend([
        format!(
            "download {} ({} bytes) in the operation's own download phase",
            artifact.url, artifact.bytes
        ),
        format!("check those bytes against {}", artifact.sha256),
        match artifact.shape {
            software::Shape::Raw => format!(
                "place them as {}",
                root.join(declared.version).join(declared.command).display()
            ),
            software::Shape::GzipTar => format!(
                "extract them into {}, whose {} is the program",
                root.join(declared.version).display(),
                artifact.member
            ),
        },
        format!("point {} at it", exposed.display()),
    ]);

    Ok((
        vec![SoftwareArtifact {
            platform: artifact.platform.to_owned(),
            url: artifact.url.to_owned(),
            sha256: artifact.sha256.to_owned(),
            byte_length: artifact.bytes,
            entry_point,
        }],
        effects,
    ))
}

/// Apply one software operation under a lock, with no network open.
///
/// # Errors
///
/// Refuses a missing `--prefix`, a count of downloaded files that does not match
/// what the plan named, bytes that are not the ones it named, or an archive that
/// does not hold the member it named.
pub(crate) fn apply(
    harness: &Harness,
    prefix: Option<&Path>,
    operation: Operation,
    downloaded: &[PathBuf],
) -> Result<serde_json::Value> {
    let declared = declared(harness)?;
    let root = program_directory(prefix, operation)?;

    // The lock lives in this provider's own dotted directory inside the prefix,
    // not at its root. `acquire` takes a control directory, and a `target.lock`
    // sitting beside `bin/` in a program directory would be both misnamed and
    // in the way of whoever looks in there for a program.
    let control = root.join(harness.control_directory);
    std::fs::create_dir_all(&control).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("--prefix {} cannot be created: {error}", root.display()),
        )
    })?;
    let mut guard = setup_core::lock::TargetLock::acquire(&control)?;
    guard.annotate(&format!("{} {operation}", harness.provider_id))?;

    if operation == Operation::SoftwareRemove {
        if !downloaded.is_empty() {
            return Err(Error::refuse(
                WireReason::UnsupportedOperation,
                "software_remove downloads nothing, so it takes no --software-artifact",
            ));
        }
        let removed = software::remove(&declared, &root)?;
        return Ok(serde_json::json!({
            "state": "verified",
            "operation": operation.as_str(),
            "command": declared.command,
            "version": declared.version,
            "removed": removed,
        }));
    }

    // One file per entry the plan named. This build's table names one, so a
    // second file is a caller holding a plan from a different build -- which the
    // plan digest already refuses, but saying which mismatch it is costs a line.
    let [path] = downloaded else {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{operation} installs the 1 artifact the plan named, and {} were given; \
                 pass one --software-artifact per entry, in the plan's order",
                downloaded.len()
            ),
        ));
    };

    // Re-checked here, not trusted from the plan: applying happens later, and
    // the prefix could have been emptied in between. The plan's digest binds
    // what was decided, not what the disk still holds.
    if operation == Operation::SoftwareUpdate
        && software::Present::under_named(&root, declared.command, declared.member_hint())
            .versions
            .is_empty()
    {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "there is no {} under {} to update any more; software_install is the operation \
                 that puts one there",
                declared.command,
                root.display()
            ),
        ));
    }

    let (os, arch) = platform_of_this_host();
    let artifact = declared.artifact_for(os, arch)?;
    let installed = software::install(&declared, artifact, path, &root)?;

    Ok(serde_json::json!({
        "state": "verified",
        "operation": operation.as_str(),
        "command": declared.command,
        "version": installed.version,
        "entry_point": format!(
            "bin/{}",
            setup_core::software::exposed_name(declared.command, declared.member_hint())
        ),
        "executable": installed.executable.to_string_lossy(),
        "files": installed.files,
    }))
}

/// Start the exact program a software install placed, replacing this process.
///
/// Not a name looked up on `PATH`: that starts whatever else shares the
/// spelling, which is the failure this command exists to avoid. The path comes
/// from the same table the plan was built from, and it must resolve to a
/// regular file this host can execute — an existing but non-executable file is
/// a refusal with a reason, not a process error surfacing from somewhere else.
///
/// On success this does not return. The caller's stdio and exit status become
/// the product's, which is what starting a program means; everything that could
/// refuse has already refused by then.
///
/// # Errors
///
/// Refuses when this build does not declare `launch`, when no `--prefix` was
/// given, when nothing is installed there, or when what is there cannot be run.
pub(crate) fn launch(
    harness: &Harness,
    target: &Path,
    prefix: Option<&Path>,
    arguments: &[String],
) -> Result<serde_json::Value> {
    if !harness.can_launch() {
        return Err(Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "{} does not declare launch: {}",
                harness.provider_id,
                if harness.config_home_env.is_empty() {
                    format!(
                        "{} documents no environment variable for its configuration home, so a \
                         launch could not point it at the target this command was given",
                        harness.product
                    )
                } else {
                    "this build installs no software, and launching a name found on PATH would \
                     start whatever else shares it"
                        .to_owned()
                },
            ),
        ));
    }

    let declared = declared(harness)?;
    let root = program_directory(prefix, Operation::Launch)?;
    // The same name the plan stated and the apply wrote. Three readings of one
    // fact, and they have to be one expression or they will drift apart on the
    // one platform where they differ.
    let executable = root.join("bin").join(setup_core::software::exposed_name(
        declared.command,
        declared.member_hint(),
    ));

    let found = executable.metadata().map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is not installed under {}: {error}; run software_install first",
                declared.command,
                root.display()
            ),
        )
    })?;
    if !found.is_file() {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} is not a regular file", executable.display()),
        ));
    }
    if !is_executable(&found) {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} exists but this host cannot execute it",
                executable.display()
            ),
        ));
    }

    // One of the two places in this program that may spawn, and the reason it
    // may: `launch` starts the product, which the contract declares
    // `runtime_external` rather than a local phase. The lint refuses the type
    // everywhere else so a `tar` shell-out cannot arrive quietly in an unpack.
    #[allow(
        clippy::disallowed_types,
        reason = "launch starts the product, and is declared as doing so"
    )]
    let mut command = std::process::Command::new(&executable);
    command.args(arguments);
    // The target is what this provider configured, and the product's own
    // documented variable is how it is told. Nothing else in the environment is
    // touched: filtering another program's environment would be deciding what
    // it needs, and only its vendor knows that.
    command.env(harness.config_home_env, target);

    Err(replace_this_process(command, &executable))
}

/// Whether the mode bits say this host can run it.
#[cfg(unix)]
fn is_executable(found: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    found.mode() & 0o111 != 0
}

/// Windows decides by extension, not by a mode bit there is none of.
#[cfg(not(unix))]
fn is_executable(_found: &std::fs::Metadata) -> bool {
    true
}

/// Hand this process to the product, and only return if that failed.
#[cfg(unix)]
#[allow(
    clippy::disallowed_types,
    reason = "the command `launch` built, handed to exec"
)]
fn replace_this_process(mut command: std::process::Command, executable: &Path) -> Error {
    use std::os::unix::process::CommandExt;
    // `exec` returns only on failure. The product inherits this process, so its
    // stdio and its exit status are the ones the caller sees, with nothing of
    // this program's left in between.
    let failure = command.exec();
    Error::refuse(
        WireReason::ProviderUnavailable,
        format!("{} could not be started: {failure}", executable.display()),
    )
}

/// Windows has no `exec`, so the status is carried back by hand.
#[cfg(not(unix))]
fn replace_this_process(mut command: std::process::Command, executable: &Path) -> Error {
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(failure) => Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} could not be started: {failure}", executable.display()),
        ),
    }
}
