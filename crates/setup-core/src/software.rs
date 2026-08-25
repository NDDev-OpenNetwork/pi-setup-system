//! Installing the product itself, from bytes a plan named in advance.
//!
//! The contract splits every software operation into three phases: `plan` with
//! no network, `download` with `artifact_download`, and `apply` with no network
//! again. That split is the whole design. `plan` states the exact artifact for
//! the running platform -- url, byte length, `sha256` -- offline, from a table
//! compiled into this binary. `download` fetches precisely that. `apply`
//! re-checks the digest and extracts, offline. Nothing about what gets
//! installed is decided while the network is reachable.
//!
//! Where it lands is a directory of its own, named by the caller as `--prefix`
//! and distinct from the configuration target. That separation is in the agreed
//! contract and it is the right shape: a setup system owns `native_namespaces`
//! inside a target and preserves everything else verbatim, so installing a
//! program there would claim a path it has promised not to touch -- and one
//! program can serve several targets, which a program living inside one of them
//! could not.

use std::fs;
use std::path::{Path, PathBuf};

use crate::archive::{self, Limits};
use crate::digest;
use crate::error::{Error, ReasonCode, Result};

/// How an artifact becomes a program on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// The artifact's bytes *are* the executable.
    ///
    /// Grok publishes this way. Its npm package ships the same program
    /// Brotli-compressed to fit the registry's tarball ceiling; the direct
    /// distribution its own installer uses needs no decompression at all, and
    /// the two were measured byte-identical.
    Raw,
    /// A gzip-compressed tar. The executable is one member inside it.
    ///
    /// The rest of the tree is not incidental and is never discarded: codex
    /// ships `rg`, `zsh` and `bwrap` beside its binary, and cursor's executable
    /// is a shell launcher that runs a bundled `node`.
    GzipTar,
}

/// One platform's artifact, exactly as a plan will state it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Artifact {
    /// The platform key, spelled the way the consumer spells it: `linux/x86_64`.
    pub platform: &'static str,
    /// Where the bytes come from.
    pub url: &'static str,
    /// How many bytes to expect.
    pub bytes: u64,
    /// The `sha256:`-prefixed digest of those bytes.
    pub sha256: &'static str,
    /// How to turn the artifact into a program.
    pub shape: Shape,
    /// The executable's path inside the archive. Empty when [`Shape::Raw`].
    pub member: &'static str,
}

/// How a product's software reaches a machine.
#[derive(Debug, Clone, Copy)]
pub enum Delivery {
    /// Artifacts this provider fetches and places itself.
    Artifacts(&'static [Artifact]),
    /// A package manager resolves a dependency closure.
    ///
    /// Recorded rather than attempted. Running one means executing whatever the
    /// registry resolves to, which is a different security question from
    /// fetching bytes whose digest was decided in advance, and it is not
    /// answered by pretending the operation is the same shape.
    Manager {
        /// The tool the product's own documentation names.
        tool: &'static str,
        /// Why this provider does not run it yet.
        reason: &'static str,
    },
}

/// A product's software lifecycle, as the runtime needs to know it.
#[derive(Debug, Clone, Copy)]
pub struct Software {
    /// The version this build installs.
    pub version: &'static str,
    /// The command name the installed program answers to.
    pub command: &'static str,
    /// How it is delivered.
    pub delivery: Delivery,
    /// Platforms the vendor does not publish for.
    ///
    /// Said out loud rather than left as an absence: cursor ships no Windows
    /// build, and a caller deserves that answer instead of "not found".
    pub unsupported: &'static [&'static str],
}

/// What an install produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The version now on disk.
    pub version: String,
    /// The directory holding this version.
    pub root: PathBuf,
    /// The executable a caller runs.
    pub executable: PathBuf,
    /// How many files the artifact produced.
    pub files: usize,
}

impl Software {
    /// The artifact for one platform, or the reason there is not one.
    ///
    /// # Errors
    ///
    /// Refuses with `unsupported_platform` when the vendor publishes nothing
    /// for this operating system, and `unsupported_architecture` when it
    /// publishes for the system but not this machine.
    pub fn artifact_for(&self, os: &str, arch: &str) -> Result<&'static Artifact> {
        let Delivery::Artifacts(artifacts) = self.delivery else {
            return Err(Error::new(
                ReasonCode::UnsupportedOperation,
                match self.delivery {
                    Delivery::Manager { tool, reason } => {
                        format!("{} is delivered by {tool}: {reason}", self.command)
                    }
                    Delivery::Artifacts(_) => unreachable!(),
                },
            ));
        };

        let wanted = format!("{os}/{arch}");
        if let Some(found) = artifacts.iter().find(|entry| entry.platform == wanted) {
            return Ok(found);
        }

        // Distinguish "no build for this system" from "no build for this
        // machine". The consumer's closed reason set separates them, and the
        // two are different problems for whoever reads the refusal.
        let prefix = format!("{os}/");
        let system_is_published = artifacts
            .iter()
            .any(|entry| entry.platform.starts_with(&prefix));
        let reason = if system_is_published {
            ReasonCode::UnsupportedArchitecture
        } else {
            ReasonCode::UnsupportedPlatform
        };
        Err(Error::new(
            reason,
            format!(
                "{} publishes no build for {wanted}; it publishes {}",
                self.command,
                artifacts
                    .iter()
                    .map(|entry| entry.platform)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }
}

impl Artifact {
    /// Check downloaded bytes against what the plan said they would be.
    ///
    /// Both the length and the digest, in that order: a truncated download is
    /// the common case and saying so is more useful than a digest mismatch.
    ///
    /// # Errors
    ///
    /// Refuses with `integrity_mismatch` when either disagrees.
    pub fn verify(&self, downloaded: &Path) -> Result<()> {
        let found = fs::metadata(downloaded)
            .map_err(|error| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("downloaded artifact could not be read: {error}"),
                )
                .with_source(error)
            })?
            .len();
        if found != self.bytes {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!(
                    "{} is {found} bytes; the plan named {}",
                    downloaded.display(),
                    self.bytes
                ),
            ));
        }
        let measured = digest::of_file(downloaded)?;
        if measured != self.sha256 {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!(
                    "{} hashes to {measured}; the plan named {}",
                    downloaded.display(),
                    self.sha256
                ),
            ));
        }
        Ok(())
    }

    /// How much this artifact is allowed to produce.
    ///
    /// A compressed length says nothing about what it inflates to, so the limit
    /// is the caller's rather than the archive's. Sixteen times the compressed
    /// size with a floor covers every measured artifact -- the widest is
    /// claude's, at a little over three -- and stops a stream that would
    /// otherwise keep producing until the disk filled.
    #[must_use]
    pub const fn limits(&self) -> Limits {
        Limits {
            entries: 65_536,
            bytes: match self.bytes.checked_mul(16) {
                Some(scaled) if scaled > 64 * 1024 * 1024 => scaled,
                _ => 64 * 1024 * 1024,
            },
        }
    }
}

/// Install a verified artifact under `root`, replacing any same-version tree.
///
/// The digest is checked here rather than trusted from the download phase,
/// because this phase is the one that runs without a network and is therefore
/// the one whose check means something.
///
/// # Errors
///
/// Refuses an artifact that does not match the plan, an archive this reader
/// will not accept, or an archive that does not contain the member it named.
pub fn install(
    software: &Software,
    artifact: &Artifact,
    downloaded: &Path,
    root: &Path,
) -> Result<Installed> {
    artifact.verify(downloaded)?;

    let version_root = root.join(software.version);
    if version_root.exists() {
        fs::remove_dir_all(&version_root).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!(
                    "the existing {} tree could not be cleared: {error}",
                    software.version
                ),
            )
            .with_source(error)
        })?;
    }

    let source = fs::File::open(downloaded).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("downloaded artifact could not be opened: {error}"),
        )
        .with_source(error)
    })?;

    let (executable, files) = match artifact.shape {
        Shape::Raw => {
            let placed = version_root.join(software.command);
            archive::place_executable(source, &placed)?;
            (placed, 1)
        }
        Shape::GzipTar => {
            let entries = archive::extract_gzip_tar(source, &version_root, artifact.limits())?;
            let found = entries
                .iter()
                .any(|entry| entry.path == artifact.member && entry.kind == archive::Kind::File);
            if !found {
                return Err(Error::new(
                    ReasonCode::IntegrityMismatch,
                    format!(
                        "the archive does not contain {}, which the plan named as the executable",
                        artifact.member
                    ),
                ));
            }
            (version_root.join(artifact.member), entries.len())
        }
    };

    let exposed = root.join("bin").join(software.command);
    expose(&executable, &exposed)?;

    Ok(Installed {
        version: software.version.to_owned(),
        root: version_root,
        executable: exposed,
        files,
    })
}

/// Remove one installed version, and the exposed command if it pointed at it.
///
/// # Errors
///
/// Fails if the tree exists and cannot be removed.
pub fn remove(software: &Software, root: &Path) -> Result<bool> {
    let version_root = root.join(software.version);
    if !version_root.exists() {
        return Ok(false);
    }
    fs::remove_dir_all(&version_root).map_err(|error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!(
                "the {} tree could not be removed: {error}",
                software.version
            ),
        )
        .with_source(error)
    })?;
    let exposed = root.join("bin").join(software.command);
    if exposed.symlink_metadata().is_ok() {
        fs::remove_file(&exposed).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("{} could not be removed: {error}", exposed.display()),
            )
            .with_source(error)
        })?;
    }
    Ok(true)
}

/// Point one stable path at the executable inside a versioned tree.
///
/// The member is left where the archive put it. Codex's binary needs the `rg`
/// and `bwrap` beside it and cursor's launcher needs its bundled `node`, so
/// moving the executable out of its tree would produce a file that runs on the
/// machine it was built on and nowhere else.
fn expose(executable: &Path, exposed: &Path) -> Result<()> {
    let fail = |error: std::io::Error| {
        Error::new(
            ReasonCode::StateUnavailable,
            format!("{} could not be exposed: {error}", exposed.display()),
        )
        .with_source(error)
    };

    if let Some(parent) = exposed.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    if exposed.symlink_metadata().is_ok() {
        fs::remove_file(exposed).map_err(fail)?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(executable, exposed).map_err(fail)
    }
    #[cfg(not(unix))]
    {
        // Windows reserves symlink creation for privileged or developer-mode
        // processes, so a hard link is what actually works; a copy is the last
        // resort and costs a second copy of a large binary.
        fs::hard_link(executable, exposed)
            .or_else(|_| fs::copy(executable, exposed).map(|_| ()))
            .map_err(fail)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::archive::build::{Dialect, Item, gzip_tar};

    /// Codex's real shape: the executable is deep inside a tree whose siblings
    /// it needs at runtime.
    const CODEX_MEMBER: &str = "package/vendor/x86_64-unknown-linux-musl/bin/codex";

    const ARTIFACTS: &[Artifact] = &[
        Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/linux-x86_64.tgz",
            bytes: 0,
            sha256: "sha256:0",
            shape: Shape::GzipTar,
            member: CODEX_MEMBER,
        },
        Artifact {
            platform: "linux/arm64",
            url: "https://example.invalid/linux-arm64.tgz",
            bytes: 0,
            sha256: "sha256:0",
            shape: Shape::GzipTar,
            member: CODEX_MEMBER,
        },
    ];

    fn software() -> Software {
        Software {
            version: "1.2.3",
            command: "codex",
            delivery: Delivery::Artifacts(ARTIFACTS),
            unsupported: &["windows/x86_64"],
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("setup-core-software-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    /// Write an archive to disk and return it with an artifact describing it.
    fn staged(name: &str, body: &[u8], member: &'static str) -> (PathBuf, Artifact) {
        let raw = gzip_tar(
            &[
                Item::directory("package"),
                Item::file(member, body, 0o755),
                Item::file("package/README.md", b"read me", 0o644),
            ],
            Dialect::Gnu,
        );
        let at = scratch(name);
        fs::create_dir_all(&at).unwrap();
        let file = at.join("artifact.tgz");
        fs::write(&file, &raw).unwrap();
        let artifact = Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/artifact.tgz",
            bytes: raw.len() as u64,
            sha256: Box::leak(digest::of_bytes(&raw).into_boxed_str()),
            shape: Shape::GzipTar,
            member,
        };
        (at, artifact)
    }

    #[test]
    fn the_artifact_for_this_platform_is_the_one_named_for_it() {
        let found = software().artifact_for("linux", "x86_64").unwrap();
        assert_eq!(found.url, "https://example.invalid/linux-x86_64.tgz");
    }

    #[test]
    fn a_system_the_vendor_does_not_build_for_is_an_unsupported_platform() {
        let error = software().artifact_for("windows", "x86_64").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::UnsupportedPlatform);
        assert!(
            error.detail().contains("linux/x86_64"),
            "{}",
            error.detail()
        );
    }

    #[test]
    fn a_machine_the_vendor_does_not_build_for_is_an_unsupported_architecture() {
        // The system is published, this machine is not. The consumer's closed
        // set separates these and so must the refusal.
        let error = software().artifact_for("linux", "riscv64").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::UnsupportedArchitecture);
    }

    #[test]
    fn a_product_delivered_by_a_package_manager_says_so_rather_than_pretending() {
        let pi = Software {
            version: "0.84.3",
            command: "pi",
            delivery: Delivery::Manager {
                tool: "npm",
                reason: "its dependency closure is resolved at install time",
            },
            unsupported: &[],
        };
        let error = pi.artifact_for("linux", "x86_64").unwrap_err();
        assert_eq!(error.reason(), ReasonCode::UnsupportedOperation);
        assert!(error.detail().contains("npm"), "{}", error.detail());
    }

    #[test]
    fn an_archive_installs_and_exposes_one_stable_command() {
        let (at, artifact) = staged("install", b"#!/bin/sh\necho hi\n", CODEX_MEMBER);
        let root = at.join("software");
        let installed = install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        assert_eq!(installed.version, "1.2.3");
        assert_eq!(installed.files, 3);
        assert_eq!(installed.executable, root.join("bin/codex"));
        // The executable stays inside its tree; only a link leaves it. Codex
        // needs `rg` and `bwrap` beside it, so moving the binary would break it.
        assert!(root.join("1.2.3").join(CODEX_MEMBER).is_file());
        assert!(root.join("1.2.3/package/README.md").is_file());
        assert_eq!(
            fs::read(&installed.executable).unwrap(),
            b"#!/bin/sh\necho hi\n"
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn installing_twice_replaces_the_tree_rather_than_merging_into_it() {
        let (at, artifact) = staged("twice", b"first", CODEX_MEMBER);
        let root = at.join("software");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        let stray = root.join("1.2.3/package/left-over");
        fs::write(&stray, b"from an older install").unwrap();

        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        assert!(!stray.exists(), "a replaced tree must not keep older files");
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn bytes_that_are_not_the_ones_the_plan_named_are_refused() {
        let (at, mut artifact) = staged("digest", b"payload", CODEX_MEMBER);
        artifact.sha256 = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let error = install(
            &software(),
            &artifact,
            &at.join("artifact.tgz"),
            &at.join("s"),
        )
        .unwrap_err();
        assert_eq!(error.reason(), ReasonCode::IntegrityMismatch);
        assert!(error.detail().contains("hashes to"), "{}", error.detail());
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn a_truncated_download_is_named_as_a_length_problem_not_a_digest_one() {
        let (at, artifact) = staged("length", b"payload", CODEX_MEMBER);
        let file = at.join("artifact.tgz");
        let mut bytes = fs::read(&file).unwrap();
        bytes.truncate(bytes.len() - 4);
        fs::write(&file, &bytes).unwrap();

        let error = install(&software(), &artifact, &file, &at.join("s")).unwrap_err();
        assert!(
            error.detail().contains("bytes; the plan named"),
            "{}",
            error.detail()
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn an_archive_without_the_member_the_plan_named_is_refused() {
        let (at, mut artifact) = staged("member", b"payload", CODEX_MEMBER);
        artifact.member = "package/vendor/somewhere-else/bin/codex";
        let error = install(
            &software(),
            &artifact,
            &at.join("artifact.tgz"),
            &at.join("s"),
        )
        .unwrap_err();
        assert!(
            error.detail().contains("does not contain"),
            "{}",
            error.detail()
        );
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn removing_takes_the_tree_and_the_exposed_command_with_it() {
        let (at, artifact) = staged("remove", b"payload", CODEX_MEMBER);
        let root = at.join("software");
        let installed = install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        assert!(installed.executable.symlink_metadata().is_ok());

        assert!(remove(&software(), &root).unwrap());
        assert!(!root.join("1.2.3").exists());
        assert!(installed.executable.symlink_metadata().is_err());
        // Removing what is already gone is not a failure, and says so.
        assert!(!remove(&software(), &root).unwrap());
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn the_inflation_limit_scales_with_the_artifact_but_never_below_a_floor() {
        let small = Artifact {
            bytes: 10,
            ..ARTIFACTS[0]
        };
        assert_eq!(small.limits().bytes, 64 * 1024 * 1024);
        let large = Artifact {
            bytes: 121_422_431,
            ..ARTIFACTS[0]
        };
        // Claude's is the widest measured expansion, a little over three times.
        assert!(large.limits().bytes > 391_948_592);
    }
}
