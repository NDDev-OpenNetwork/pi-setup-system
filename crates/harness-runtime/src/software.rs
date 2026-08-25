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

    let entry_point = format!("bin/{}", declared.command);
    let exposed = root.join(&entry_point);

    if operation == Operation::SoftwareRemove {
        return Ok((
            Vec::new(),
            vec![
                format!(
                    "remove {}, the {} tree this provider installed",
                    root.join(declared.version).display(),
                    declared.version
                ),
                format!("remove {}", exposed.display()),
            ],
        ));
    }

    let (os, arch) = platform_of_this_host();
    let artifact = declared.artifact_for(os, arch)?;
    let effects = vec![
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
    ];

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

    let (os, arch) = platform_of_this_host();
    let artifact = declared.artifact_for(os, arch)?;
    let installed = software::install(&declared, artifact, path, &root)?;

    Ok(serde_json::json!({
        "state": "verified",
        "operation": operation.as_str(),
        "command": declared.command,
        "version": installed.version,
        "entry_point": format!("bin/{}", declared.command),
        "executable": installed.executable.to_string_lossy(),
        "files": installed.files,
    }))
}
