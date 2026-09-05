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
//!   one of the versions this build names -- the pinned one, or the one pinned
//!   before it once the harness has been bumped. The plan records that version
//!   and the artifact digest; apply re-checks both and refuses a neighbour
//!   pin's bytes even when this build also publishes them.
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

use std::fs;
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

/// This build as it describes the version an operation was asked for.
///
/// A build names its pinned version and, once it has been bumped, the one it
/// pinned before. Anything else refuses: installing a version it *does* name
/// instead would be answering a question nobody asked.
fn version_for(declared: &Software, asked: Option<&str>) -> Result<Software> {
    declared.at(asked).ok_or_else(|| {
        let wanted = asked.unwrap_or_default();
        Error::refuse(
            WireReason::UnsupportedOperation,
            format!(
                "this build names {} {}; it cannot install {wanted}, and installing one it does \
                 name instead would be answering a question nobody asked",
                declared.command,
                declared.versions().join(" and "),
            ),
        )
    })
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
) -> Result<(Vec<SoftwareArtifact>, Vec<String>, &'static str)> {
    let declared = version_for(&declared(harness)?, software_version)?;
    let root = program_directory(prefix, operation)?;

    // Not always the command: pi's entry point is JavaScript, and Windows runs
    // a file by its extension rather than by a shebang, so what is exposed
    // there is `pi.cmd`. The plan states what a caller will actually be able to
    // run, which is the whole point of naming it -- and it is derived from
    // **this platform's** member, not the table's first row. The hint
    // derivation promised `bin/agent` on Windows while the apply wrote
    // `bin/agent.cmd`, because cursor's Windows member is a `.cmd` and its
    // Unix members are extensionless; the consumer's matrix caught the plan
    // lying on exactly the one platform where the two rows classify apart.
    let (os, arch) = platform_of_this_host();
    let member = declared.member_on(os, arch);
    let entry_point = format!(
        "bin/{}",
        setup_core::software::exposed_name(declared.command, member)
    );
    let exposed = root.join(&entry_point);

    // Planning may read the local disk and may not reach the network, so what
    // is already under the prefix belongs in the plan. Without it an install
    // and an update produced byte-identical effects -- two names for one act,
    // and neither said what was about to be replaced.
    let present = software::Present::under_named(&root, declared.command, member);

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
        return Ok((Vec::new(), effects, declared.version));
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
            software::Shape::GzipTar | software::Shape::Zip => format!(
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
        declared.version,
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
    planned_version: &str,
    planned_artifacts: &[SoftwareArtifact],
    downloaded: &[PathBuf],
) -> Result<serde_json::Value> {
    let declared = version_for(&declared(harness)?, Some(planned_version))?;
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

    // **Under the lock, before anything reads the prefix.** An interrupted
    // install leaves a staged tree, or a tree that stepped aside for a promote
    // that did not land -- and the second reads as "nothing installed" to every
    // other function here. Until this call the leftovers were cleared by
    // whatever ran next, so the resolution was a side effect of the next
    // operation rather than a decision, and an install could be planned against
    // a prefix whose real state was a version in quarantine.
    //
    // Reported rather than done quietly: a person whose install was interrupted
    // should be told what was found, and the answer is empty on every ordinary
    // run.
    let recovered = setup_core::software::recover(&root)?;

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
            "recovered": recovered,
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
    let [planned] = planned_artifacts else {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{operation} installs the 1 artifact the plan named, and the plan listed {}",
                planned_artifacts.len()
            ),
        ));
    };

    // Re-checked here, not trusted from the plan: applying happens later, and
    // the prefix could have been emptied in between. The plan's digest binds
    // what was decided, not what the disk still holds.
    if operation == Operation::SoftwareUpdate
        && software::Present::under_named(&root, declared.command, declared.member_here())
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

    // The plan named both the version and the artifact. Bytes that belong to
    // another pin this build publishes are still the wrong effect: they are
    // not the bytes this plan authorised. Digest-of-file remains the
    // observation; it no longer selects a neighbour release.
    let digest = setup_core::digest::of_file(path)?;
    if digest != planned.sha256 {
        return Err(Error::refuse(
            WireReason::DigestMismatch,
            format!(
                "the artifact given hashes to {digest}; this plan named {} for {planned_version}",
                planned.sha256
            ),
        ));
    }
    let (os, arch) = platform_of_this_host();
    let artifact = declared.artifact_for(os, arch)?;
    if artifact.sha256 != planned.sha256 {
        return Err(Error::refuse(
            WireReason::DigestMismatch,
            format!(
                "this plan binds {planned_version} but names {}; {} publishes {}",
                planned.sha256, declared.command, artifact.sha256
            ),
        ));
    }
    let installed = software::install(&declared, artifact, path, &root)?;

    Ok(serde_json::json!({
        "state": "verified",
        "recovered": recovered,
        "operation": operation.as_str(),
        "command": declared.command,
        "version": installed.version,
        "entry_point": format!(
            "bin/{}",
            setup_core::software::exposed_name(declared.command, artifact.member)
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
            // The reason comes from the declaration rather than being guessed
            // from one field here. Guessing produced a refusal that told cursor
            // callers *"this build installs no software"* -- false, it installs
            // and removes it, and the actual reason is that the product follows
            // its variable for one of the eight surfaces this provider owns.
            format!(
                "{} does not declare launch: {}",
                harness.provider_id,
                harness.why_no_launch()
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
        declared.member_here(),
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

    verify_installed(&root, declared.command, &executable)?;

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
    for (name, value) in launch_environment(harness, target) {
        command.env(name, value);
    }

    Err(replace_this_process(command, &executable))
}

/// The bytes under the exposed command are the bytes this provider installed.
///
/// `launch` checked that a file was there, that it was a regular file, and that
/// this host could execute it. None of those is a statement about *which* bytes.
/// So a product that replaced itself, a package manager that wrote over the
/// tree, or anything else with the prefix in reach was started and reported as
/// the pinned release -- and the plan that authorised the install, the digest
/// recorded beside it and the rollback to the version next door were all still
/// saying otherwise.
///
/// Read from the record `expose` writes, and the executable it names rather
/// than the exposed command: on Windows the exposure can be a `.cmd` launcher
/// holding a path rather than a copy of the program, so hashing the exposed
/// file would check the launcher.
///
/// **No record is a refusal, not a hole.** A prefix written by an earlier
/// release has none, and launching it would start a chain this build cannot
/// name. Re-run `software_install` to write a receipt, then launch.
fn verify_installed(root: &Path, command: &str, exposed: &Path) -> Result<()> {
    let manifest = match setup_core::software::Manifest::inspect(root, command) {
        setup_core::software::ManifestState::Missing => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{} is missing; the installed chain is unresolved. \
                     Nothing has been started.",
                    setup_core::software::Manifest::path(root, command).display()
                ),
            ));
        }
        setup_core::software::ManifestState::Unreadable => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{} exists and could not be read; the installed chain is unresolved. \
                     Nothing has been started.",
                    setup_core::software::Manifest::path(root, command).display()
                ),
            ));
        }
        setup_core::software::ManifestState::Malformed => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{} is not a usable launch receipt; the installed chain is unresolved. \
                     Nothing has been started.",
                    setup_core::software::Manifest::path(root, command).display()
                ),
            ));
        }
        setup_core::software::ManifestState::Unsupported { schema_version } => {
            return Err(Error::refuse(
                WireReason::ProviderUnavailable,
                format!(
                    "{} names schema {schema_version}, which this build does not verify; \
                     the installed chain is unresolved. Nothing has been started.",
                    setup_core::software::Manifest::path(root, command).display()
                ),
            ));
        }
        setup_core::software::ManifestState::Present(found) => found,
    };
    let executable = root.join(&manifest.executable);
    let found = setup_core::digest::of_file(&executable).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} names {} and it could not be read: {error}",
                setup_core::software::Manifest::path(root, command).display(),
                manifest.executable
            ),
        )
    })?;
    if found != manifest.executable_sha256 {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is not the {} this provider installed: recorded {}, found {}. \
                 Nothing has been started. Re-run software_install to put the pinned \
                 bytes back, or software_remove to take the prefix off.",
                manifest.executable, manifest.version, manifest.executable_sha256, found
            ),
        ));
    }
    verify_exposed_chain(exposed, &executable, &found)
}

/// The file `launch` will exec still names the payload the receipt hashed.
fn verify_exposed_chain(exposed: &Path, payload: &Path, payload_digest: &str) -> Result<()> {
    if let Ok(target) = fs::read_link(exposed) {
        let resolved = if target.is_absolute() {
            target
        } else {
            exposed.parent().unwrap_or(Path::new(".")).join(target)
        };
        let expected = payload.canonicalize().ok();
        let found = resolved.canonicalize().ok();
        if expected.is_some() && expected == found {
            return Ok(());
        }
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} no longer names {}; nothing has been started",
                exposed.display(),
                payload.display()
            ),
        ));
    }
    if exposed
        .extension()
        .is_some_and(|ext| ext == "cmd" || ext == "bat")
    {
        let wrapper = fs::read_to_string(exposed).map_err(|error| {
            Error::refuse(
                WireReason::ProviderUnavailable,
                format!("{} could not be read: {error}", exposed.display()),
            )
        })?;
        let needle = payload.to_string_lossy();
        if wrapper.contains(needle.as_ref()) {
            return Ok(());
        }
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is a launcher that does not name {}; nothing has been started",
                exposed.display(),
                payload.display()
            ),
        ));
    }
    let exposed_digest = setup_core::digest::of_file(exposed).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} could not be read: {error}", exposed.display()),
        )
    })?;
    if exposed_digest == payload_digest {
        return Ok(());
    }
    Err(Error::refuse(
        WireReason::ProviderUnavailable,
        format!(
            "{} is not the payload recorded at {}; nothing has been started",
            exposed.display(),
            payload.display()
        ),
    ))
}

/// Everything this provider adds to the environment of the product it starts.
///
/// A function returning the pairs rather than four `command.env` calls, for the
/// reason `exposed_name_on` and `render_mode_for` are functions: a decision made
/// inline in a builder can only be *demonstrated*, and `launch` ends in an
/// `exec` that replaces this process, so a test can never look at what it set.
/// This one can be asserted.
///
/// Two entries at most:
///
/// * **the configuration home**, because every command here takes an explicit
///   `--target` and the product's own documented variable is how it is told;
/// * **the updater switch**, where the product has one.
///
/// Nothing else. Filtering another program's environment would be deciding what
/// it needs, and only its vendor knows that. The updater is the one exception
/// and it is not a decision about the product's needs: this provider pins a
/// version, records the digest of the artifact it came from, and offers a
/// rollback to the version beside it. A product that replaces those bytes while
/// running makes all three false, and this prefix is a distribution channel its
/// vendor did not build.
fn launch_environment(harness: &Harness, target: &Path) -> Vec<(&'static str, String)> {
    let mut pairs = vec![(
        harness.config_home_env,
        target.to_string_lossy().into_owned(),
    )];
    if !harness.updates_off_env.is_empty() {
        pairs.push((harness.updates_off_env, "1".to_owned()));
    }
    pairs
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::fs;

    use super::*;

    /// A launch refuses bytes that are not the ones this provider installed.
    ///
    /// `launch` checked existence, file-ness and an executable bit, none of
    /// which says *which* bytes. `DISABLE_UPDATES` closed the product's own way
    /// of replacing them; nothing closed anybody else's.
    ///
    /// Four states, because collapsing any of them into "no receipt" would
    /// start a program whose chain this build cannot name:
    ///
    /// * the recorded bytes -- accepted;
    /// * one byte different -- refused, naming both digests;
    /// * no record at all -- refused as missing, distinct from corrupt;
    /// * a present record that cannot be read, parsed, or verified -- refused
    ///   under that state's own sentence.
    #[test]
    fn a_launch_refuses_an_executable_that_is_not_the_one_installed() {
        let root = std::env::temp_dir().join(format!(
            "harness-runtime-verify-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let version = root.join("1.2.3");
        fs::create_dir_all(&version).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let executable = version.join("program");
        fs::write(&executable, b"the installed bytes\n").unwrap();
        let exposed = root.join("bin").join("program");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&executable, &exposed).unwrap();
        #[cfg(not(unix))]
        fs::copy(&executable, &exposed).unwrap();

        let missing = verify_installed(&root, "program", &exposed).unwrap_err();
        assert!(
            missing.detail().contains("is missing"),
            "a prefix with no record must fail as missing, not as a silent skip: {}",
            missing.detail()
        );

        let manifest = setup_core::software::Manifest {
            schema_version: 1,
            version: "1.2.3".to_owned(),
            executable: "1.2.3/program".to_owned(),
            executable_sha256: setup_core::digest::of_file(&executable).unwrap(),
        };
        fs::write(
            setup_core::software::Manifest::path(&root, "program"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            verify_installed(&root, "program", &exposed).is_ok(),
            "the bytes named by the record were refused"
        );

        fs::write(&executable, b"the installed bytes?\n").unwrap();
        let refused = verify_installed(&root, "program", &exposed).unwrap_err();
        assert_eq!(refused.reason(), Some(WireReason::ProviderUnavailable));
        assert!(
            refused.detail().contains("recorded") && refused.detail().contains("found"),
            "the refusal does not say which digest was expected: {}",
            refused.detail()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_corrupt_or_unsupported_manifest_is_not_a_verified_launch() {
        let root = std::env::temp_dir().join(format!(
            "harness-runtime-verify-corrupt-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let version = root.join("1.2.3");
        fs::create_dir_all(&version).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let executable = version.join("program");
        fs::write(&executable, b"the installed bytes\n").unwrap();
        let exposed = root.join("bin").join("program");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&executable, &exposed).unwrap();
        #[cfg(not(unix))]
        fs::copy(&executable, &exposed).unwrap();
        let path = setup_core::software::Manifest::path(&root, "program");

        fs::write(&path, b"{not a manifest").unwrap();
        let malformed = verify_installed(&root, "program", &exposed).unwrap_err();
        assert!(
            malformed.detail().contains("not a usable launch receipt"),
            "{}",
            malformed.detail()
        );

        fs::write(
            &path,
            serde_json::to_vec(&setup_core::software::Manifest {
                schema_version: 2,
                version: "1.2.3".to_owned(),
                executable: "1.2.3/program".to_owned(),
                executable_sha256: setup_core::digest::of_file(&executable).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        let unsupported = verify_installed(&root, "program", &exposed).unwrap_err();
        assert!(
            unsupported.detail().contains("schema 2"),
            "{}",
            unsupported.detail()
        );

        fs::write(
            &path,
            serde_json::to_vec(&setup_core::software::Manifest {
                schema_version: 1,
                version: "1.2.3".to_owned(),
                executable: "1.2.3/program".to_owned(),
                executable_sha256: setup_core::digest::of_file(&executable).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            let other = version.join("other");
            fs::write(&other, b"the installed bytes\n").unwrap();
            fs::remove_file(&exposed).unwrap();
            std::os::unix::fs::symlink(&other, &exposed).unwrap();
            let drifted = verify_installed(&root, "program", &exposed).unwrap_err();
            assert!(
                drifted.detail().contains("no longer names"),
                "{}",
                drifted.detail()
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    /// A launch adds the target and, where the product has one, the updater switch.
    ///
    /// The switch exists because this provider's three strongest statements
    /// about an installation -- the version it pins, the digest of the artifact
    /// it came from, and the rollback to the version beside it -- are all false
    /// the moment the product replaces those bytes itself. Anthropic documents
    /// the variable for exactly this case, a distribution channel somebody else
    /// controls, and the 2.1.251 artifact carries it nine times.
    ///
    /// Both directions, because only the pair says anything. A build that added
    /// the variable to every launch would pass the first assertion while setting
    /// a name nothing reads on six products, and a build that added it to none
    /// would pass the second.
    #[test]
    fn a_launch_sets_the_updater_switch_only_where_the_product_has_one() {
        let target = Path::new("/tmp/nddev-launch-environment");

        let with = Harness {
            config_home_env: "PRODUCT_CONFIG_DIR",
            updates_off_env: "DISABLE_UPDATES",
            ..crate::wire::tests_support::TEST
        };
        assert_eq!(
            launch_environment(&with, target),
            vec![
                ("PRODUCT_CONFIG_DIR", target.display().to_string()),
                ("DISABLE_UPDATES", "1".to_owned()),
            ]
        );

        let without = Harness {
            config_home_env: "PRODUCT_CONFIG_DIR",
            updates_off_env: "",
            ..crate::wire::tests_support::TEST
        };
        assert_eq!(
            launch_environment(&without, target),
            vec![("PRODUCT_CONFIG_DIR", target.display().to_string())],
            "a product with no such variable had its environment written to anyway"
        );
    }

    fn planted_prefix(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "harness-runtime-launch-{tag}-{}",
            std::process::id(),
        ));
        let _ = fs::remove_dir_all(&root);
        let version = root.join("1.2.3");
        fs::create_dir_all(&version).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let payload = version.join("test-harness");
        fs::write(&payload, crate::wire::tests_support::TEST_PAYLOAD).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&payload, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let exposed = root.join("bin").join("test-harness");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&payload, &exposed).unwrap();
        #[cfg(not(unix))]
        fs::copy(&payload, &exposed).unwrap();
        (root, payload, exposed)
    }

    #[test]
    fn launch_refuses_missing_unreadable_malformed_and_unsupported_receipts_as_distinct_states() {
        let target = Path::new("/tmp/nddev-launch-verify-target");
        let (root, payload, exposed) = planted_prefix("receipts");
        let path = setup_core::software::Manifest::path(&root, "test-harness");

        let missing =
            launch(&crate::wire::tests_support::TEST, target, Some(&root), &[]).unwrap_err();
        assert!(
            missing.detail().contains("is missing"),
            "{}",
            missing.detail()
        );

        fs::write(&path, b"{not a manifest").unwrap();
        let malformed =
            launch(&crate::wire::tests_support::TEST, target, Some(&root), &[]).unwrap_err();
        assert!(
            malformed.detail().contains("not a usable launch receipt"),
            "{}",
            malformed.detail()
        );

        fs::write(
            &path,
            serde_json::to_vec(&setup_core::software::Manifest {
                schema_version: 2,
                version: "1.2.3".to_owned(),
                executable: "1.2.3/test-harness".to_owned(),
                executable_sha256: setup_core::digest::of_file(&payload).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        let unsupported =
            launch(&crate::wire::tests_support::TEST, target, Some(&root), &[]).unwrap_err();
        assert!(
            unsupported.detail().contains("schema 2"),
            "{}",
            unsupported.detail()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(
                &path,
                serde_json::to_vec(&setup_core::software::Manifest {
                    schema_version: 1,
                    version: "1.2.3".to_owned(),
                    executable: "1.2.3/test-harness".to_owned(),
                    executable_sha256: setup_core::digest::of_file(&payload).unwrap(),
                })
                .unwrap(),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
            let unreadable =
                launch(&crate::wire::tests_support::TEST, target, Some(&root), &[]).unwrap_err();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                unreadable.detail().contains("could not be read"),
                "{}",
                unreadable.detail()
            );
        }

        let _ = fs::remove_dir_all(&root);
        let _ = exposed;
    }

    #[test]
    fn launch_refuses_another_valid_pin_substituted_for_the_recorded_payload() {
        let target = Path::new("/tmp/nddev-launch-pin-swap-target");
        let (root, payload, exposed) = planted_prefix("pin-swap");
        let digest = setup_core::digest::of_file(&payload).unwrap();
        fs::write(
            setup_core::software::Manifest::path(&root, "test-harness"),
            serde_json::to_vec(&setup_core::software::Manifest {
                schema_version: 1,
                version: "1.2.3".to_owned(),
                executable: "1.2.3/test-harness".to_owned(),
                executable_sha256: digest,
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(&payload, crate::wire::tests_support::TEST_EARLIER_PAYLOAD).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&payload, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let swapped =
            launch(&crate::wire::tests_support::TEST, target, Some(&root), &[]).unwrap_err();
        assert!(
            swapped.detail().contains("recorded") && swapped.detail().contains("found"),
            "{}",
            swapped.detail()
        );
        let _ = fs::remove_dir_all(&root);
        let _ = exposed;
    }

    #[test]
    fn launch_refuses_a_windows_wrapper_that_does_not_name_the_recorded_payload() {
        let root = std::env::temp_dir().join(format!(
            "harness-runtime-wrapper-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_dir_all(&root);
        let version = root.join("1.2.3");
        fs::create_dir_all(&version).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let payload = version.join("test-harness.exe");
        fs::write(&payload, b"payload-bytes\n").unwrap();
        let wrapper = root.join("bin").join("test-harness.cmd");
        fs::write(&wrapper, "@call \"C:\\other\\test-harness.exe\" %*\r\n").unwrap();
        fs::write(
            setup_core::software::Manifest::path(&root, "test-harness"),
            serde_json::to_vec(&setup_core::software::Manifest {
                schema_version: 1,
                version: "1.2.3".to_owned(),
                executable: "1.2.3/test-harness.exe".to_owned(),
                executable_sha256: setup_core::digest::of_file(&payload).unwrap(),
            })
            .unwrap(),
        )
        .unwrap();
        let refused = verify_installed(&root, "test-harness", &wrapper).unwrap_err();
        assert!(
            refused.detail().contains("does not name"),
            "{}",
            refused.detail()
        );
        let _ = fs::remove_dir_all(&root);
    }
}
