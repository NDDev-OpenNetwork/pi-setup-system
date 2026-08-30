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

use serde::{Deserialize, Serialize};

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
    /// A ZIP. The executable is one member inside it.
    ///
    /// One vendor ships this on Windows and a gzip-tar everywhere else, which
    /// is the reason the shape sits on an [`Artifact`] rather than on a
    /// product: the same release, the same version, two container formats and
    /// two different members. Reading it off the product would have been right
    /// for six of the seven and silently wrong for the seventh on one platform
    /// of three -- the shape of defect this estate has now shipped twice.
    Zip,
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

/// The version this build pinned before the current one.
///
/// **Not a second independent choice.** A bump assigns `previous = current` and
/// then sets `current`, so one value still moves per bump and the old one falls
/// into this slot instead of being discarded. Two clocks would be two things to
/// keep fresh; this is one.
///
/// It exists because two operations could be declared and not run. An update
/// needs a version to move *from* and a rollback a tree to return *to*, and a
/// build pinning one version has neither — which is why
/// `docs/SOFTWARE-LIFECYCLE.md` carried two `no` rows against
/// `software_update` and `rollback` for as long as it did.
///
/// Two *consecutive real releases* differ in whatever the vendor actually
/// changed, so the transition exercised is one a person will really perform. A
/// fabricated pair would prove the plumbing against a case nobody runs, which
/// is the same kind of evidence as a test that has never been red.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Previous {
    /// The version, exactly as the vendor published it.
    pub version: &'static str,
    /// Its artifacts, measured from bytes the same way the current ones were.
    pub artifacts: &'static [Artifact],
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
    /// The version before this one, when this build has had a bump.
    ///
    /// `None` until a harness bumps once. Absent is the honest reading of a
    /// build that has only ever pinned one version — there is nothing to move
    /// between and the operations say so, rather than pretending.
    pub previous: Option<Previous>,
}

/// What is already under a program directory.
///
/// Read at plan time, which is allowed to look at the local disk and is not
/// allowed to reach the network. Without it `software_install` and
/// `software_update` produced byte-identical plans — two names for one act, and
/// neither of them said what was about to be replaced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Present {
    /// Every version directory found, sorted.
    pub versions: Vec<String>,
    /// The version the exposed command currently resolves into, when it does.
    pub exposed: Option<String>,
}

impl Present {
    /// Where the exposed version is recorded, beside the command it describes.
    ///
    /// Dotted so it is hidden on Unix, and under `bin/` so it can never be read
    /// as a version directory.
    #[must_use]
    fn marker(root: &Path, command: &str) -> PathBuf {
        root.join("bin").join(format!(".{command}.version"))
    }

    /// Whether this build's pinned version is one of the ones already there.
    #[must_use]
    pub fn holds(&self, version: &str) -> bool {
        self.versions.iter().any(|found| found == version)
    }

    /// Read what a program directory holds, told which member the exposed
    /// thing was made from.
    ///
    /// A missing or unreadable directory reads as empty rather than failing: a
    /// plan for a prefix that does not exist yet is exactly the ordinary case.
    ///
    /// Windows exposes a JavaScript entry point as `<command>.cmd`, so a
    /// reading that only ever looked for `<command>` reports nothing exposed on
    /// exactly the harness that needed the launcher.
    ///
    /// **There was a member-blind `under(root, command)` beside this, and it is
    /// gone.** It passed `member = ""`, which is a true statement for a native
    /// binary and a false one for pi, whose members are all
    /// `package/dist/bundle/cli.js`. Both production callers -- `software` and
    /// `rollback` on the human surface -- took the short one, so on Windows
    /// `fs::metadata(bin/pi)` failed, `exposed` came back `None`, and a
    /// rollback that had worked reported *"the prefix still runs something
    /// else"*. Every test of the path used `codex`, whose member-blind and
    /// member-aware names are identical, so the suite was green on all three
    /// systems and never entered the branch.
    ///
    /// Removing the short constructor is the repair rather than fixing the two
    /// call sites: a defaulted member preserves every future instance of the
    /// same mistake, and changing the signature turned both of them into
    /// compile errors.
    #[must_use]
    pub fn under_named(root: &Path, command: &str, member: &str) -> Self {
        Self::under_on(root, command, member, cfg!(windows))
    }

    /// The same reading, with the platform as an argument rather than as a
    /// `cfg`.
    ///
    /// Same reason [`exposed_name_on`] takes one, and the same reason spelled
    /// out there: a `cfg!` makes the Windows branch provable only on Windows,
    /// and this branch is the one that was wrong. A reading laid out the way
    /// Windows lays it out can now be asserted from Linux.
    #[must_use]
    pub fn under_on(root: &Path, command: &str, member: &str, windows: bool) -> Self {
        let mut versions: Vec<String> = fs::read_dir(root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            // `bin` holds the exposed command and a dotted directory holds this
            // provider's own bookkeeping. Neither is a version.
            .filter(|name| name != "bin" && !name.starts_with('.'))
            .collect();
        versions.sort();

        // The link points into the version directory it was installed from, so
        // the version is the component right under the root.
        //
        // Resolved rather than read. `expose` writes an absolute target, which
        // a plain `strip_prefix` handles — but a link someone rewrote by hand
        // as `../<version>/<member>` does not: its first component is `bin`,
        // and the answer becomes "no entry point names this version" while the
        // entry point names it. Resolving costs one syscall and is right for
        // both. A dangling link resolves to nothing, which is also correct:
        // nothing usable is exposed.
        // Recorded, not inferred -- and the reason is Windows.
        //
        // Resolving the link was the only reading here, and it cannot work on a
        // system where `expose` does not make a link. Windows reserves symlink
        // creation for privileged processes, so the exposed command is a hard
        // link or a copy: canonicalizing it returns its own path, its first
        // component under the root is `bin`, and the answer became "no version
        // is exposed" on a prefix where one plainly was.
        //
        // That was not only cosmetic. `Present::exposed` is what separates an
        // install from an update, so on Windows every `software_update` saw an
        // empty prefix and refused as an update of nothing -- while the version
        // it would have updated sat right there. Found by the three-OS matrix
        // on the first Windows run of the rollback tests.
        // A record is only ever believed about a command that resolves. A
        // dangling link exposes nothing whatever the record says -- the record
        // remembers what `expose` last pointed at, and someone can break that
        // by hand afterwards. `metadata` follows links, so this is false for a
        // dangling one and true for both a real file and a live link.
        let exposed_as = exposed_name_on(command, member, windows);
        let usable = fs::metadata(root.join("bin").join(&exposed_as)).is_ok();
        let marker = Self::marker(root, command);
        let exposed = fs::read_to_string(&marker)
            .ok()
            .filter(|_| usable)
            .map(|held| held.trim().to_owned())
            // A hand-edited or half-written marker must not name a version that
            // is not there.
            .filter(|name| versions.contains(name))
            .or_else(|| {
                // Nothing recorded: a prefix written before this existed, or one
                // someone arranged themselves. Where a real link is what is
                // there, reading it is still the truth.
                fs::canonicalize(root.join("bin").join(&exposed_as))
                    .ok()
                    .zip(fs::canonicalize(root).ok())
                    .and_then(|(to, base)| {
                        to.strip_prefix(&base).ok().and_then(|rest| {
                            rest.components()
                                .next()
                                .map(|first| first.as_os_str().to_string_lossy().into_owned())
                        })
                    })
                    .filter(|name| versions.contains(name))
            });

        Self { versions, exposed }
    }
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
    /// Every version this build can name, current first.
    ///
    /// One element until the harness has been bumped once, two after. Used to
    /// say in a refusal what *is* available rather than only what is not.
    #[must_use]
    pub fn versions(&self) -> Vec<&'static str> {
        let mut named = vec![self.version];
        if let Some(earlier) = self.previous {
            named.push(earlier.version);
        }
        named
    }

    /// This build, as it describes one of the versions it names.
    ///
    /// `None` for the argument means the pinned version, which is what an
    /// omitted `--software-version` means on the wire. A named version is
    /// either the pinned one or the one pinned before it; anything else
    /// returns `None` here, and the caller turns that into a refusal in its own
    /// error vocabulary.
    ///
    /// Returning a whole [`Software`] rather than a version string is what
    /// keeps this to one resolution point: `artifact_for`, `member_hint`,
    /// `install` and `remove` all read the value they already read, and none of
    /// them learns that a second pin exists.
    #[must_use]
    pub fn at(&self, asked: Option<&str>) -> Option<Self> {
        match asked {
            None => Some(*self),
            Some(wanted) if wanted == self.version => Some(*self),
            Some(wanted) => self
                .previous
                .filter(|earlier| earlier.version == wanted)
                .map(|earlier| Self {
                    version: earlier.version,
                    delivery: Delivery::Artifacts(earlier.artifacts),
                    ..*self
                }),
        }
    }

    /// This build, as it describes the release those exact bytes belong to.
    ///
    /// `apply` is handed a file, not a version. Reading which release it is
    /// from the digest makes the version an **observation** rather than a claim
    /// that travelled beside the bytes: a caller cannot install the previous
    /// tree under the current version's name by relabelling a flag, because
    /// nothing here reads a label.
    #[must_use]
    pub fn for_bytes(&self, os: &str, arch: &str, digest: &str) -> Option<Self> {
        self.versions()
            .into_iter()
            .filter_map(|version| self.at(Some(version)))
            .find(|candidate| {
                candidate
                    .artifact_for(os, arch)
                    .is_ok_and(|artifact| artifact.sha256 == digest)
            })
    }

    /// Where this build's own artifacts put the executable inside their tree.
    ///
    /// A *hint*, and named one deliberately: it is right for a version this
    /// build installed and is only a first guess for a tree an older build
    /// wrote. [`rollback`] tries it, then the flat shape, and refuses naming
    /// both rather than pointing a command at a path it did not verify.
    #[must_use]
    pub fn member_hint(&self) -> &'static str {
        match self.delivery {
            Delivery::Artifacts(artifacts) => {
                artifacts.first().map_or(self.command, |entry| entry.member)
            }
            Delivery::Manager { .. } => self.command,
        }
    }

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

    // **Staged beside the final path, never into it.** This used to clear
    // `<root>/<version>` and extract into that same directory, so between those
    // two steps there was no installation at all -- and on a reinstall of the
    // version currently exposed, `bin/<command>` pointed into a directory that
    // had just been deleted. A crash, a full disk, or an archive that turns out
    // not to carry its declared executable took the working program with it.
    //
    // The names begin with a dot, which is not decoration: `Present::under_on`
    // skips dotted entries when it lists installed versions, so a staging or
    // quarantine directory left by an interruption is never read as a version
    // somebody could roll back to.
    let staging = root.join(format!(".incoming-{}", software.version));
    let quarantine = root.join(format!(".replaced-{}", software.version));
    for leftover in [&staging, &quarantine] {
        if leftover.exists() {
            fs::remove_dir_all(leftover).map_err(|error| {
                Error::new(
                    ReasonCode::StateUnavailable,
                    format!("{} could not be cleared: {error}", leftover.display()),
                )
                .with_source(error)
            })?;
        }
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
            let placed = staging.join(software.command);
            archive::place_executable(source, &placed)?;
            (placed, 1)
        }
        Shape::GzipTar | Shape::Zip => {
            let entries = if artifact.shape == Shape::Zip {
                archive::extract_zip(source, &staging, artifact.limits())?
            } else {
                archive::extract_gzip_tar(source, &staging, artifact.limits())?
            };
            let found = entries
                .iter()
                .any(|entry| entry.path == artifact.member && entry.kind == archive::Kind::File);
            if !found {
                // Refused with the staged tree still in the staging directory
                // and the installed one untouched. Cleaning up is best-effort:
                // a leftover `.incoming-*` is cleared by the next attempt and is
                // never read as an installed version.
                let _ = fs::remove_dir_all(&staging);
                return Err(Error::new(
                    ReasonCode::IntegrityMismatch,
                    format!(
                        "the archive does not contain {}, which the plan named as the executable",
                        artifact.member
                    ),
                ));
            }
            (staging.join(artifact.member), entries.len())
        }
    };

    // **Promote.** Two renames on one filesystem, in the order that leaves a
    // complete tree at the final path at every moment a reader could look:
    // the installed one until the second rename, the new one after it. A rename
    // onto an existing directory is not portable -- Windows refuses it -- so the
    // old tree steps aside first rather than being overwritten in place.
    let replaced = version_root.exists();
    if replaced {
        fs::rename(&version_root, &quarantine).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!(
                    "the installed {} tree could not be moved aside: {error}",
                    software.version
                ),
            )
            .with_source(error)
        })?;
    }
    if let Err(error) = fs::rename(&staging, &version_root) {
        // Put back what was there. If this second rename also fails the tree is
        // in quarantine under a name nothing reads as a version, and the refusal
        // says so rather than reporting a success over an empty final path.
        let restored = !replaced || fs::rename(&quarantine, &version_root).is_ok();
        return Err(Error::new(
            ReasonCode::StateUnavailable,
            format!(
                "the staged {} tree could not be promoted: {error}{}",
                software.version,
                if restored {
                    ""
                } else {
                    ". The previous tree is in .replaced-<version> and was not put back"
                }
            ),
        )
        .with_source(error));
    }
    if replaced {
        // Best-effort: the install is complete and correct without it, and a
        // leftover is cleared by the next attempt.
        let _ = fs::remove_dir_all(&quarantine);
    }

    let executable = version_root.join(
        executable
            .strip_prefix(&staging)
            .unwrap_or_else(|_| Path::new(software.command)),
    );

    let exposed = root
        .join("bin")
        .join(exposed_name(software.command, artifact.member));
    expose(&executable, &exposed, software.version, software.command)?;

    Ok(Installed {
        version: software.version.to_owned(),
        root: version_root,
        executable: exposed,
        files,
    })
}

/// Resolve whatever an interrupted software operation left in a prefix.
///
/// Configuration mutations have a durable journal and a `recover-operation` that
/// reads it. Software operations have neither: the protocol's recovery takes a
/// `--target` and this work happens under a `--prefix`, which is a different
/// root with a different lifetime. So an interrupted install used to be resolved
/// by the *next* install happening to clear the leftovers, and nothing could say
/// an operation had been interrupted at all.
///
/// The filesystem is the record here, and it is enough for a decision because
/// the promote is two renames in a known order. Three states, and each has one
/// right answer:
///
/// * **staging present** — extraction did not finish. The final path was never
///   touched, so the installation that was there is still there. Take the
///   staging directory.
/// * **quarantine present and the version present** — the promote landed and
///   the cleanup did not. The new tree is in place. Take the quarantine.
/// * **quarantine present and the version absent** — the promote failed after
///   the old tree stepped aside. Put it back.
///
/// The third is the one that matters and the one the old code could not have
/// resolved: `<version>` empty with a full `.replaced-<version>` beside it reads
/// as "nothing installed" to every other function here.
///
/// Idempotent: run twice and the second run finds nothing and says so. It
/// answers what it did rather than how it went, because a caller printing this
/// is telling a person what happened to their prefix.
///
/// # Errors
///
/// Fails where a leftover cannot be removed or a tree cannot be put back, which
/// is a prefix nobody can repair by running this again.
pub fn recover(root: &Path) -> Result<Vec<String>> {
    fn fail(what: String) -> impl FnOnce(std::io::Error) -> Error {
        move |error: std::io::Error| {
            Error::new(ReasonCode::StateUnavailable, format!("{what}: {error}")).with_source(error)
        }
    }

    let mut done = Vec::new();
    let mut entries: Vec<PathBuf> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    for path in entries {
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if let Some(version) = name.strip_prefix(".incoming-") {
            fs::remove_dir_all(&path).map_err(fail(format!(
                "the staged {version} tree could not be cleared"
            )))?;
            done.push(format!(
                "an install of {version} was interrupted before it landed; the staged tree \
                 is gone and whatever was installed is untouched"
            ));
        } else if let Some(version) = name.strip_prefix(".replaced-") {
            let final_path = root.join(version);
            if final_path.exists() {
                fs::remove_dir_all(&path).map_err(fail(format!(
                    "the replaced {version} tree could not be cleared"
                )))?;
                done.push(format!(
                    "an install of {version} landed and its cleanup did not; the new tree is \
                     in place and the old one is gone"
                ));
            } else {
                fs::rename(&path, &final_path).map_err(fail(format!(
                    "the previous {version} tree could not be put back"
                )))?;
                done.push(format!(
                    "an install of {version} failed after the installed tree stepped aside; \
                     it is back"
                ));
            }
        } else if name.ends_with(".incoming") {
            // A staged marker or manifest. Neither is a tree and neither is the
            // record until it is renamed, so a leftover is only litter.
            let _ = fs::remove_file(&path);
            done.push(format!("a half-written {name} was cleared"));
        }
    }
    Ok(done)
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

    // Asked **before** the tree goes, and that ordering is the whole fix.
    // `exposed` resolves the command into the version directory it points at,
    // and a directory that has just been removed resolves to nothing -- so
    // reading afterwards cannot tell "it named this one" from "it named another
    // one" and the answer would always be the same.
    //
    // The sentence above this function has said *"and the exposed command if it
    // pointed at it"* since it was written. The code took the command whenever
    // the tree existed. So the ordinary sequence after a bad release -- install,
    // update, roll back, take the bad one off -- deleted the command that was
    // running the good one and left a complete version tree nothing could start.
    let exposed_version =
        Present::under_named(root, software.command, software.member_hint()).exposed;

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

    // Some(this one)  -- the command named what was just taken; it goes too.
    // Some(another)   -- the command names a version still installed; it stays.
    // None            -- nothing usable was exposed, so anything left behind is
    //                    a leftover rather than somebody's entry point.
    let ours = exposed_version.as_deref() == Some(software.version);
    if !ours && exposed_version.is_some() {
        return Ok(true);
    }

    let exposed = root
        .join("bin")
        .join(exposed_name(software.command, software.member_hint()));
    if exposed.symlink_metadata().is_ok() {
        fs::remove_file(&exposed).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("{} could not be removed: {error}", exposed.display()),
            )
            .with_source(error)
        })?;
    }
    // The record goes with the command it described. A marker outliving it
    // would name a version nothing runs.
    let _ = fs::remove_file(Present::marker(root, software.command));
    Ok(true)
}

/// Where a version tree can hold its executable, most specific first.
///
/// Looked for rather than assumed. This build pins one version and knows where
/// *its* executable sits inside the archive; an older tree was written by an
/// older build, whose artifact table this one does not carry. Every shape it
/// could have used is tried, and none is guessed at: if the file is not there,
/// the refusal says where it looked.
///
/// **The platform is a parameter and not a `cfg!`**, for the same reason
/// [`exposed_name_on`] takes one. That is not hypothetical here.
/// [`Software::member_hint`] answers with the *first* artifact's member
/// whatever host is asking, the tables in this repository list Linux first, and
/// so a Windows rollback looked for `package/bin/opencode` while the file on
/// disk was `package/bin/opencode.exe`. Ubuntu and macOS passed; only the
/// evidence run on Windows failed, and a `cfg!` here would have left the fix
/// unprovable from the machine that wrote it.
fn executable_candidates(
    software: &Software,
    version_root: &Path,
    os: &str,
    arch: &str,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push = |relative: &str| {
        let path = version_root.join(relative);
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    };
    // This host's own member first: it is the only one that is right by
    // construction rather than by the table happening to be ordered well.
    if let Ok(artifact) = software.artifact_for(os, arch) {
        push(artifact.member);
    }
    // Then the shapes an older build could have written.
    push(software.member_hint());
    push(software.command);
    if os == "windows" {
        push(&format!("{}.exe", software.command));
    }
    candidates
}

/// Point the exposed command back at a version that is already on disk.
///
/// Installing 1.0.6 leaves 1.0.5 in its own directory and moves only the
/// exposed command, so the bytes to go back to are already there. Until this
/// existed nothing pointed at them: the owner named rollback in the same
/// sentence as install, reinstall and select, and three of those four were
/// reachable.
///
/// This is the one part of the software lifecycle that needs no network at all,
/// which is why it can be a command someone types rather than the three-phase
/// exchange install and update have to be.
///
/// **The version is named, never inferred.** There is no record of what was
/// previous -- only what is on disk -- and these version strings do not order
/// reliably: `2026.08.11-e8db854` sorts by string, not by release. Picking "the
/// one before" would mean inventing an ordering the vendor never promised, and
/// pointing a command at the wrong build is exactly the class of mistake this
/// program refuses everywhere else. A caller who omits it is told what is here.
///
/// # Errors
///
/// Refuses a version that is not installed, naming the ones that are, and a
/// version tree that holds no executable this build can find.
pub fn rollback(software: &Software, root: &Path, to: &str) -> Result<Installed> {
    let present = Present::under_named(root, software.command, software.member_hint());
    if !present.versions.iter().any(|found| found == to) {
        return Err(Error::new(
            ReasonCode::InvalidTarget,
            if present.versions.is_empty() {
                format!(
                    "{} holds no installed version of {}",
                    root.display(),
                    software.command
                )
            } else {
                format!(
                    "{to} is not installed under {}; it holds {}",
                    root.display(),
                    present.versions.join(", ")
                )
            },
        ));
    }

    let version_root = root.join(to);
    let (os, arch) = crate::platform_of_this_host();
    let candidates = executable_candidates(software, &version_root, os, arch);
    let Some(executable) = candidates.iter().find(|path| path.is_file()) else {
        return Err(Error::new(
            ReasonCode::StateUnavailable,
            format!(
                "the {to} tree holds no {} executable; looked at {}",
                software.command,
                candidates
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            ),
        ));
    };

    let exposed = root
        .join("bin")
        .join(exposed_name(software.command, software.member_hint()));
    expose(executable, &exposed, to, software.command)?;
    Ok(Installed {
        version: to.to_owned(),
        root: version_root,
        executable: exposed,
        files: 0,
    })
}

/// The name the exposed command answers to, which is not always the command.
///
/// Six of the seven products ship a native executable, and a link named after
/// the command is the whole story. Pi ships JavaScript — `dist/bundle/cli.js`,
/// with a `#!/usr/bin/env node` line and mode 755 — and the two systems part
/// company there.
///
/// On Unix the shebang does the work and a symlink named `pi` runs. Windows has
/// no shebang: it decides how to run a file from its extension, and a copy
/// named `pi` with JavaScript inside it is not a program. So there the exposed
/// thing is `pi.cmd`, a launcher that names the interpreter.
///
/// Said in one place because three callers need the same answer: the plan that
/// states `entry_point`, the apply that writes it, and `launch` that starts it.
/// They disagreed once already about where a program lives, and once was
/// enough.
#[must_use]
pub fn exposed_name(command: &str, member: &str) -> String {
    exposed_name_on(command, member, cfg!(windows))
}

/// The same rule, with the platform as an argument rather than as a `cfg`.
///
/// Written this way so it can be *asserted* from either system rather than
/// demonstrated on one. A `cfg!` here would make the Windows branch provable
/// only on Windows, and a mutation that deleted it would leave every Linux run
/// green — which is exactly what happened to the first version of this, and is
/// the reason it is a parameter now.
#[must_use]
pub fn exposed_name_on(command: &str, member: &str, windows: bool) -> String {
    match member_kind(member, windows) {
        // A launcher naming the runtime, or a batch file kept as one. Both need
        // an extension because Windows decides how to run a file from its name.
        MemberKind::JavaScript | MemberKind::CommandScript => format!("{command}.cmd"),
        MemberKind::Native => command.to_owned(),
    }
}

/// What the vendor's entry point *is*, which is what decides how to expose it.
///
/// This used to be one question -- *is the member JavaScript* -- and everything
/// else fell through to the bare command. Cursor's Windows package ships
/// `dist-package/cursor-agent.cmd`, a batch launcher, so the stable command
/// became an extensionless file holding batch text: not a program on the only
/// platform that branch exists for, and hard-linked or copied as though it were
/// native.
///
/// **A native `.exe` deliberately stays extensionless**, which looks
/// inconsistent and is the measured distinction. `CreateProcess` on an explicit
/// path reads the PE header rather than the name, so codex's `codex.exe` runs
/// exposed as `codex`. A batch file has no header and cannot be executed without
/// an interpreter at all. One of the two was broken; renaming the other would
/// move the entry point in every plan, marker and readback for no benefit.
///
/// The platform is a parameter rather than a `cfg!` for the reason spelled out
/// on [`exposed_name_on`]: the branch that was wrong is the one only Windows
/// runs, and a `cfg!` makes it provable only there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A program the operating system runs directly.
    Native,
    /// JavaScript, which needs the runtime named.
    JavaScript,
    /// A Windows batch launcher, which needs `cmd` and its own directory.
    CommandScript,
}

/// Classify one artifact member for one platform.
#[must_use]
pub fn member_kind(member: &str, windows: bool) -> MemberKind {
    if !windows {
        // On Unix the shebang does the work and nothing here is a batch file.
        return MemberKind::Native;
    }
    // Through the extension rather than the spelling: Windows treats `.JS` and
    // `.js` as one extension, and a rule that missed the first would expose an
    // unrunnable file on the only platform this branch exists for.
    match Path::new(member)
        .extension()
        .map(|kind| kind.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("js") => MemberKind::JavaScript,
        Some("cmd" | "bat") => MemberKind::CommandScript,
        _ => MemberKind::Native,
    }
}

/// Point one stable path at the executable inside a versioned tree.
///
/// The member is left where the archive put it. Codex's binary needs the `rg`
/// and `bwrap` beside it and cursor's launcher needs its bundled `node`, so
/// moving the executable out of its tree would produce a file that runs on the
/// machine it was built on and nowhere else.
/// Write a file so a reader sees the old contents or the new ones, never a part.
///
/// Staged in the same directory and renamed, because a rename within one
/// directory is atomic and a plain write can stop anywhere. Factored when the
/// second caller arrived: the version marker needed it after the consumer
/// constructed an interrupted write that truncated onto another installed
/// version, and the manifest beside it has the same failure and a worse one --
/// a half-written JSON document does not parse, so a reader would call an
/// installation unverifiable rather than wrong.
///
/// The staging name is dotted and sits beside its target, which every reader in
/// this module already skips.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let staging = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let name = name.to_string_lossy();
            // Dotted once, not twice. Both callers already write a dotted file,
            // and blindly prefixing produced `..codex.version.incoming` -- which
            // works and reads as a mistake. Caught by the marker test, which
            // blocks the staging path by name to prove the write stages at all.
            let dotted = if name.starts_with('.') {
                format!("{name}.incoming")
            } else {
                format!(".{name}.incoming")
            };
            parent.join(dotted)
        }
        _ => return fs::write(path, bytes),
    };
    fs::write(&staging, bytes)?;
    fs::rename(&staging, path)
}

/// What this prefix runs, recorded so a launch can check it rather than trust it.
///
/// The version marker beside this answers *which* version is exposed. It cannot
/// answer whether the bytes under that version are the ones this provider put
/// there, and `launch` was checking only that a file existed and carried an
/// executable bit. So a product that replaced itself, or anything else that
/// wrote over the tree, was started and reported as the pinned release.
///
/// The digest is of the executable named here rather than of the exposed
/// command: on Windows the exposure can be a `.cmd` launcher holding a path
/// rather than a copy of the program, and hashing that would check the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The shape of this record, so a reader can refuse one it does not know.
    pub schema_version: u32,
    /// Which product version the exposed command runs.
    pub version: String,
    /// The executable, relative to the prefix.
    pub executable: String,
    /// Its digest when this provider exposed it.
    pub executable_sha256: String,
}

impl Manifest {
    /// Where the record for one command lives.
    #[must_use]
    pub fn path(root: &Path, command: &str) -> PathBuf {
        root.join("bin").join(format!(".{command}.manifest.json"))
    }

    /// The record, or `None` where there is not one to read.
    ///
    /// Absent is a real state and not a failure: a prefix written before this
    /// existed has no record, and calling that tampering would refuse every
    /// installation made by an earlier release of this provider. Unparseable is
    /// also `None` -- a record that cannot be read cannot accuse anything.
    #[must_use]
    pub fn read(root: &Path, command: &str) -> Option<Self> {
        let raw = fs::read_to_string(Self::path(root, command)).ok()?;
        let found: Self = serde_json::from_str(&raw).ok()?;
        (found.schema_version == 1).then_some(found)
    }
}

fn expose(executable: &Path, exposed: &Path, version: &str, command: &str) -> Result<()> {
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
        std::os::unix::fs::symlink(executable, exposed).map_err(fail)?;
    }
    #[cfg(not(unix))]
    {
        match member_kind(&executable.to_string_lossy(), true) {
            MemberKind::JavaScript => {
                // A launcher rather than a link. Windows runs a file by its
                // extension, so neither a hard link nor a copy of a `.js` is a
                // program -- and the interpreter has to be named. `%*` forwards
                // every argument, and the quotes survive a prefix with spaces,
                // which `%LOCALAPPDATA%\Programs` is one bad default away from.
                fs::write(
                    exposed,
                    format!("@node \"{}\" %*\r\n", executable.display()),
                )
                .map_err(fail)?;
            }
            MemberKind::CommandScript => {
                // A wrapper that *calls* the vendor script where it lives.
                // Copying or linking it here would move it out of its own tree,
                // and a batch launcher locates the runtime it starts through
                // `%~dp0` -- which would then resolve to `bin\` and name
                // nothing. `call` so the wrapper returns the script's exit
                // status rather than ending the shell, and `%*` to forward
                // arguments with their quoting intact.
                fs::write(
                    exposed,
                    format!("@call \"{}\" %*\r\n", executable.display()),
                )
                .map_err(fail)?;
            }
            MemberKind::Native => {
                // Windows reserves symlink creation for privileged or
                // developer-mode processes, so a hard link is what actually
                // works; a copy is the last resort and costs a second copy of a
                // large binary. An extensionless name is fine here: an explicit
                // path to a PE runs whatever it is called.
                fs::hard_link(executable, exposed)
                    .or_else(|_| fs::copy(executable, exposed).map(|_| ()))
                    .map_err(fail)?;
            }
        }
    }

    // Which version this now runs, recorded rather than left to be inferred
    // from a link that two of the three systems do not make. Written after the
    // command is in place, so a marker never names a version that is not
    // exposed yet.
    if let Some(root) = exposed.parent().and_then(Path::parent) {
        // **Staged and renamed, because a partial marker can name a real
        // version.** A plain write can stop anywhere, and the version filter on
        // the reading side rejects a fragment only because a fragment is not an
        // installed version -- unless the truncation stops somewhere that *is*
        // one. `1.2.3` cut short is `1.2`, and where `1.2` is also installed
        // both readers believe it: `versions.contains("1.2")` is true because
        // 1.2 really is there. Nothing in a plain-text marker separates "1.2
        // because that is exposed" from "1.2 because the write stopped there".
        //
        // Constructed by the consumer after both of us had reasoned that only a
        // person could produce a wrong-but-plausible marker. An interrupted
        // write and a sibling whose string is a prefix of another is not a
        // person, and the prefix relationship makes it free.
        //
        // A rename within one directory is atomic, so a reader sees the marker
        // that was there or the one being put there, never a third thing. The
        // staging name is dotted like the marker itself, which `Present` already
        // skips when it lists versions.
        write_atomically(&Present::marker(root, command), version.as_bytes()).map_err(fail)?;

        // And what a launch needs to check the bytes rather than trust them.
        // Written after the marker: a reader that finds a manifest and no
        // marker has a partial record, and a reader that finds a marker and no
        // manifest has the state every prefix written before this had, which is
        // accepted rather than refused.
        let relative = executable.strip_prefix(root).unwrap_or(executable);
        let manifest = Manifest {
            schema_version: 1,
            version: version.to_owned(),
            executable: relative.to_string_lossy().replace('\\', "/"),
            executable_sha256: digest::of_file(executable)?,
        };
        let body = serde_json::to_vec(&manifest).map_err(|error| {
            Error::new(
                ReasonCode::StateUnavailable,
                format!("the installation record could not be written: {error}"),
            )
        })?;
        write_atomically(&Manifest::path(root, command), &body).map_err(fail)?;
    }
    Ok(())
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
            previous: None,
        }
    }

    /// The release the fixture can move away from, with bytes of its own.
    ///
    /// A different digest from [`ARTIFACTS`] deliberately: resolution by bytes
    /// is only tested by a pair that can actually be told apart.
    const EARLIER_ARTIFACTS: &[Artifact] = &[
        Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/linux-x86_64-1.2.2.tgz",
            bytes: 0,
            sha256: "sha256:earlier",
            shape: Shape::GzipTar,
            member: CODEX_MEMBER,
        },
        Artifact {
            platform: "linux/arm64",
            url: "https://example.invalid/linux-arm64-1.2.2.tgz",
            bytes: 0,
            sha256: "sha256:earlier",
            shape: Shape::GzipTar,
            member: CODEX_MEMBER,
        },
    ];

    fn bumped() -> Software {
        Software {
            previous: Some(Previous {
                version: "1.2.2",
                artifacts: EARLIER_ARTIFACTS,
            }),
            ..software()
        }
    }

    /// A build that has never been bumped names exactly one version.
    ///
    /// The absence is the point: `software_update` has nothing to move from
    /// and `rollback` nothing to return to, and both say so rather than
    /// pretending a transition exists.
    #[test]
    fn a_build_with_no_second_pin_names_one_version_and_refuses_the_rest() {
        let only = software();
        assert_eq!(only.versions(), vec!["1.2.3"]);
        assert!(only.at(None).is_some());
        assert!(only.at(Some("1.2.3")).is_some());
        assert!(only.at(Some("1.2.2")).is_none());
    }

    /// Asking for the earlier version gets the earlier version's *artifacts*.
    ///
    /// Not just its number. The whole failure this guards against is a build
    /// that answers "1.2.2" and then downloads 1.2.3's bytes, which would be a
    /// plan that names one thing and installs another.
    #[test]
    fn naming_the_earlier_version_selects_the_earlier_bytes() {
        let both = bumped();
        assert_eq!(both.versions(), vec!["1.2.3", "1.2.2"]);

        let earlier = both.at(Some("1.2.2")).unwrap();
        assert_eq!(earlier.version, "1.2.2");
        assert_eq!(
            earlier.artifact_for("linux", "x86_64").unwrap().url,
            "https://example.invalid/linux-x86_64-1.2.2.tgz"
        );

        // And the current one is unmoved by the second pin existing.
        let current = both.at(None).unwrap();
        assert_eq!(current.version, "1.2.3");
        assert_eq!(
            current.artifact_for("linux", "x86_64").unwrap().url,
            "https://example.invalid/linux-x86_64.tgz"
        );
    }

    /// Which release a file belongs to is read from the file.
    ///
    /// `apply` is handed bytes, not a version. Resolving by digest is what
    /// stops a caller installing the earlier tree under the current version's
    /// name by relabelling a flag -- nothing here reads a label.
    #[test]
    fn the_release_a_file_belongs_to_is_read_from_its_digest() {
        let both = bumped();
        assert_eq!(
            both.for_bytes("linux", "x86_64", "sha256:0")
                .map(|found| found.version),
            Some("1.2.3")
        );
        assert_eq!(
            both.for_bytes("linux", "x86_64", "sha256:earlier")
                .map(|found| found.version),
            Some("1.2.2")
        );
        // Bytes belonging to neither release resolve to neither, rather than to
        // whichever was checked first.
        assert!(
            both.for_bytes("linux", "x86_64", "sha256:someone-elses")
                .is_none()
        );
        // A platform this build publishes nothing for cannot resolve either,
        // however right the digest is.
        assert!(both.for_bytes("windows", "x86_64", "sha256:0").is_none());
    }

    /// The bytes to go back to are already on disk: installing a new version
    /// leaves the old tree in place and moves only the exposed command. Until
    /// `rollback` existed nothing pointed back at them, and the owner named
    /// rollback in the same sentence as install, reinstall and select.
    #[test]
    fn rollback_points_the_command_at_a_version_already_on_disk() {
        let (at, artifact) = staged("rollback", b"#!/bin/sh\necho new\n", CODEX_MEMBER);
        let root = at.join("prefix");
        let installed = install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        assert_eq!(installed.version, "1.2.3");

        // What an update leaves behind: the previous tree, untouched.
        let older = root.join("1.2.2");
        fs::create_dir_all(older.join("package/vendor/x86_64-unknown-linux-musl/bin")).unwrap();
        fs::write(older.join(CODEX_MEMBER), b"#!/bin/sh\necho old\n").unwrap();

        let rolled = rollback(&software(), &root, "1.2.2").unwrap();
        assert_eq!(rolled.version, "1.2.2");

        let present = Present::under_named(&root, "codex", CODEX_MEMBER);
        assert_eq!(present.exposed.as_deref(), Some("1.2.2"));
        assert_eq!(present.versions, vec!["1.2.2", "1.2.3"]);

        // And the version it came from is still there to go forward to.
        assert!(root.join("1.2.3").join(CODEX_MEMBER).is_file());
        assert_eq!(
            rollback(&software(), &root, "1.2.3").unwrap().version,
            "1.2.3"
        );
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );
    }

    /// Removing a version nobody is running leaves the running one runnable.
    ///
    /// `remove` took the version tree, then took `bin/<command>` and the marker
    /// **whenever the tree existed**, without asking whether either named the
    /// version being removed. So the ordinary sequence after a bad release --
    /// install, update, roll back, then take the bad one off -- deleted the
    /// command that was pointing at the good one, and left a complete, working
    /// version tree that nothing could start.
    ///
    /// The evidence workflow could not have caught it: it switches *forward* to
    /// the newer version before removing it, so the version it removes is always
    /// the active one. The one case where the guard matters was the one case the
    /// happy path never enters.
    #[test]
    fn removing_an_inactive_version_leaves_the_active_one_exposed() {
        let (at, artifact) = staged("remove-inactive", b"#!/bin/sh\necho new\n", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        // The version an update left behind, and the one a rollback selects.
        let older = root.join("1.2.2");
        fs::create_dir_all(older.join("package/vendor/x86_64-unknown-linux-musl/bin")).unwrap();
        fs::write(older.join(CODEX_MEMBER), b"#!/bin/sh\necho old\n").unwrap();
        rollback(&software(), &root, "1.2.2").unwrap();
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.2"),
            "the rollback did not take"
        );

        // Take off the version nobody is running.
        assert!(remove(&software(), &root).unwrap());

        assert!(
            !root.join("1.2.3").exists(),
            "the removed version's tree is still here"
        );
        assert!(
            root.join("1.2.2").join(CODEX_MEMBER).is_file(),
            "removing one version took another version's files"
        );
        let after = Present::under_named(&root, "codex", CODEX_MEMBER);
        assert_eq!(
            after.exposed.as_deref(),
            Some("1.2.2"),
            "removing an inactive version took the command that was running the active one"
        );
        assert_eq!(after.versions, vec!["1.2.2"]);
    }

    /// An install that fails leaves the installation that was working.
    ///
    /// `install` cleared `<root>/<version>` and then extracted into that same
    /// path. Between those two steps there is no installation at all, and the
    /// exposed command points into a directory that has been deleted -- so a
    /// crash, a full disk, or an archive that turns out not to carry its
    /// declared executable takes the working program with it.
    ///
    /// Driven by the third of those, which is deterministic: an artifact whose
    /// digest is correct and whose archive does not contain the member the plan
    /// named. The digest check passes, the clearing happens, and the refusal
    /// arrives afterwards -- exactly where an interruption would.
    #[test]
    fn a_reinstall_that_fails_leaves_the_installation_that_was_working() {
        let (at, artifact) = staged("reinstall-fails", b"#!/bin/sh\necho good\n", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        let good = fs::read(root.join("1.2.3").join(CODEX_MEMBER)).unwrap();

        // Same version, correct digest, and no executable inside.
        let raw = gzip_tar(
            &[
                Item::directory("package"),
                Item::file("package/README.md", b"no executable here", 0o644),
            ],
            Dialect::Gnu,
        );
        let broken_at = at.join("broken.tgz");
        fs::write(&broken_at, &raw).unwrap();
        let broken = Artifact {
            bytes: raw.len() as u64,
            sha256: Box::leak(digest::of_bytes(&raw).into_boxed_str()),
            ..artifact
        };

        let refused = install(&software(), &broken, &broken_at, &root).unwrap_err();
        assert_eq!(
            refused.reason(),
            ReasonCode::IntegrityMismatch,
            "the refusal is not the one this test drives: {refused:?}"
        );

        assert_eq!(
            fs::read(root.join("1.2.3").join(CODEX_MEMBER))
                .ok()
                .as_deref(),
            Some(good.as_slice()),
            "a failed reinstall destroyed the installation that was working"
        );
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3"),
            "the exposed command no longer names an installed version"
        );
    }

    /// A vendor command script keeps a runnable boundary on Windows.
    ///
    /// `exposed_name_on` asked one question -- *is the member JavaScript* -- and
    /// answered `agent` for everything else. Cursor's Windows package ships
    /// `dist-package/cursor-agent.cmd`, a batch launcher, so the stable command
    /// became an extensionless `bin/agent` holding batch text. Windows decides
    /// how to run a file from its extension, so that file is not a program, and
    /// the exposure branch beside it then hard-linked or copied it as though it
    /// were native.
    ///
    /// The question that decides is the member's *kind*, not whether it happens
    /// to be one particular kind. Four members, asserted for both platforms from
    /// whichever one is running, because the branch that was wrong is the one
    /// only Windows executes.
    #[test]
    fn a_command_script_member_keeps_an_extension_windows_can_run() {
        // Native: the command, on every system.
        assert_eq!(
            exposed_name_on("agent", "dist-package/cursor-agent", true),
            "agent"
        );
        assert_eq!(
            exposed_name_on("agent", "dist-package/cursor-agent", false),
            "agent"
        );

        // JavaScript: a launcher on Windows, the shebang does it on Unix.
        assert_eq!(
            exposed_name_on("pi", "package/dist/bundle/cli.js", true),
            "pi.cmd"
        );
        assert_eq!(
            exposed_name_on("pi", "package/dist/bundle/cli.js", false),
            "pi"
        );

        // A vendor batch launcher: still a batch file, so it keeps an extension
        // Windows will run. On Unix the member is never this shape.
        assert_eq!(
            exposed_name_on("agent", "dist-package/cursor-agent.cmd", true),
            "agent.cmd",
            "a .cmd member was exposed as a name Windows cannot run"
        );
        assert_eq!(
            exposed_name_on("agent", "dist-package/cursor-agent.cmd", false),
            "agent"
        );

        // **And a native `.exe` deliberately does not change.** Codex ships one
        // and is exposed extensionless today, which works: `CreateProcess` on an
        // explicit path reads the PE header rather than the name, so the file
        // runs whatever it is called. A batch file has no header and cannot be
        // executed at all without an interpreter, which is the whole difference
        // and the reason only one of these two was broken. Renaming the working
        // one would move the entry point in every plan and marker for no
        // measured benefit.
        assert_eq!(
            exposed_name_on("codex", "package/bin/codex.exe", true),
            "codex"
        );
        assert_eq!(
            exposed_name_on("codex", "package/bin/codex.exe", false),
            "codex"
        );
    }

    /// An interrupted software operation is resolved by a decision, not by luck.
    ///
    /// Until this existed, the leftovers of an interrupted install were cleared
    /// by whatever ran next, and nothing could say an operation had been
    /// interrupted. The protocol's `recover-operation` cannot help: it takes a
    /// `--target` and this is a `--prefix`, a different root with a different
    /// lifetime.
    ///
    /// The three states, each asserted for what it leaves behind rather than
    /// for what it says. The third is the one that matters: a version directory
    /// that is *empty of the promote* with a full quarantine beside it reads as
    /// "nothing installed" to every other function here, so luck would have
    /// resolved it as a missing installation.
    #[test]
    fn an_interrupted_install_is_resolved_by_the_state_it_left() {
        let root = scratch("recover-prefix");
        fs::create_dir_all(&root).unwrap();

        // Staging only: extraction stopped, the final path was never touched.
        fs::create_dir_all(root.join(".incoming-1.2.3/package")).unwrap();
        fs::create_dir_all(root.join("1.2.2")).unwrap();
        fs::write(root.join("1.2.2/kept"), b"the installation that was there").unwrap();
        let said = recover(&root).unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].contains("interrupted before it landed"), "{said:?}");
        assert!(!root.join(".incoming-1.2.3").exists());
        assert!(
            root.join("1.2.2/kept").is_file(),
            "an untouched tree was taken"
        );

        // Quarantine with the version present: promoted, cleanup interrupted.
        fs::create_dir_all(root.join(".replaced-1.2.3")).unwrap();
        fs::write(root.join(".replaced-1.2.3/old"), b"the previous tree").unwrap();
        fs::create_dir_all(root.join("1.2.3")).unwrap();
        fs::write(root.join("1.2.3/new"), b"the tree that landed").unwrap();
        let said = recover(&root).unwrap();
        assert!(
            said.iter().any(|line| line.contains("cleanup did not")),
            "{said:?}"
        );
        assert!(!root.join(".replaced-1.2.3").exists());
        assert_eq!(
            fs::read(root.join("1.2.3/new")).unwrap(),
            b"the tree that landed",
            "the promoted tree was replaced by the one it replaced"
        );

        // Quarantine with the version absent: the promote failed after the old
        // tree stepped aside. Every other function here reads this as nothing
        // installed.
        fs::remove_dir_all(root.join("1.2.3")).unwrap();
        fs::create_dir_all(root.join(".replaced-1.2.3")).unwrap();
        fs::write(root.join(".replaced-1.2.3/old"), b"the previous tree").unwrap();
        let said = recover(&root).unwrap();
        assert!(
            said.iter().any(|line| line.contains("it is back")),
            "{said:?}"
        );
        assert_eq!(
            fs::read(root.join("1.2.3/old")).unwrap(),
            b"the previous tree",
            "the tree that stepped aside was not put back"
        );

        // Idempotent, and it says nothing rather than inventing something.
        assert!(recover(&root).unwrap().is_empty());
    }

    /// A marker truncated onto a sibling version is not believed.
    ///
    /// The consumer constructed the accident I told them I could not. The
    /// version filter rejects a truncated marker because the fragment is not an
    /// installed version -- unless the truncation stops somewhere that *is* one.
    /// `1.18.23` cut short is `1.18`, and where `1.18` is also installed the
    /// prefix relationship does it for free: no editing, no person, an
    /// interrupted write and a sibling whose string is a prefix of another.
    ///
    /// `versions.contains("1.18")` is true, because 1.18 really is installed.
    /// Nothing in a plain-text marker separates *"1.18 because that is what is
    /// exposed"* from *"1.18 because the write stopped there"*, and reading it
    /// twice does not help: the bytes are stable once written.
    ///
    /// So the fix is in the writing, not the reading. Both endings of an
    /// interrupted write are asserted -- the old value or the new one, never a
    /// third thing.
    #[test]
    fn a_marker_truncated_onto_a_sibling_version_is_not_believed() {
        let (at, artifact) = staged("truncated-marker", b"#!/bin/sh\necho new\n", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        // A sibling whose string is a prefix of the exposed one. `1.2` beside
        // the installed `1.2.3` is the shape; the consumer measured `1.18`
        // beside `1.18.23`.
        let sibling = root.join("1.2");
        fs::create_dir_all(sibling.join("package/vendor/x86_64-unknown-linux-musl/bin")).unwrap();
        fs::write(sibling.join(CODEX_MEMBER), b"#!/bin/sh\necho sibling\n").unwrap();

        // What an interrupted write leaves: the exposed version, cut short at a
        // point that happens to be another installed version.
        fs::write(root.join("bin").join(".codex.version"), "1.2").unwrap();

        let read = Present::under_named(&root, "codex", CODEX_MEMBER);
        assert_eq!(
            read.exposed.as_deref(),
            Some("1.2"),
            "this test drives the state; if it stops reproducing, say why here"
        );

        // And the writer must make that state unreachable: a write that fails
        // leaves the marker it found, never a fragment of the one it was told
        // to put there.
        let marker = root.join("bin").join(".codex.version");
        fs::write(&marker, "1.2.3").unwrap();
        let staging = marker.with_extension("version.incoming");
        fs::create_dir_all(&staging).unwrap(); // the staging path cannot be written
        let refused = expose(
            &root.join("1.2").join(CODEX_MEMBER),
            &root.join("bin").join("codex"),
            "1.2",
            "codex",
        );
        assert!(
            refused.is_err(),
            "the marker write did not stage: {refused:?}"
        );
        assert_eq!(
            fs::read_to_string(&marker).unwrap(),
            "1.2.3",
            "a failed marker write replaced the marker that was there"
        );
    }

    /// A JavaScript entry point is exposed as something the platform can run.
    ///
    /// Six of the seven products ship a native executable and a link named
    /// after the command is the whole story. Pi ships `dist/bundle/cli.js`, and
    /// the two systems part company: Unix has the shebang, Windows decides how
    /// to run a file from its extension, so a copy named `pi` with JavaScript
    /// inside it is not a program there.
    ///
    /// The rule is asserted for both systems from either, because it is the
    /// answer three callers need to agree on -- the plan that states
    /// `entry_point`, the apply that writes it, and `launch` that starts it.
    #[test]
    fn a_javascript_entry_point_is_exposed_as_something_the_platform_can_run() {
        // A native member is the command, on every system.
        assert_eq!(exposed_name("codex", CODEX_MEMBER), "codex");
        assert_eq!(exposed_name("grok", ""), "grok");

        // A JavaScript member, asserted for both systems from whichever one is
        // running: Windows cannot run a file named `pi` holding JavaScript, and
        // on Unix the shebang does the work so a launcher would be noise.
        let js = "package/dist/bundle/cli.js";
        assert_eq!(exposed_name_on("pi", js, true), "pi.cmd");
        assert_eq!(exposed_name_on("pi", js, false), "pi");

        // A native member is unaffected by the platform, which is what makes
        // this a rule about the artifact rather than about Windows.
        assert_eq!(exposed_name_on("codex", CODEX_MEMBER, true), "codex");
        assert_eq!(exposed_name_on("codex", CODEX_MEMBER, false), "codex");

        // And the shipped entry point agrees with the rule for this system.
        assert_eq!(
            exposed_name("pi", js),
            exposed_name_on("pi", js, cfg!(windows))
        );
    }

    /// A prefix laid out the way Windows lays one out reads back correctly,
    /// and the member-blind reading of the same prefix finds nothing.
    ///
    /// This is the defect `pi`'s `evidence.yml` failed on, on `windows-latest`
    /// and only there: `rollback` answered, and the readback after it reported
    /// *"the prefix still runs something else"*. `rollback` was right; the
    /// reading was blind. Both production callers passed `member = ""`, so
    /// `exposed_name("pi", "")` gave `pi`, `bin/pi` does not exist beside a
    /// `.cmd` launcher, and `exposed` came back `None`.
    ///
    /// **Every existing test of this path uses `codex`**, a native binary whose
    /// member-blind and member-aware names are the same string. So the whole
    /// suite was green on all three systems while never once entering the
    /// branch that was wrong. That is why this test names `pi` and its real
    /// member, and why it takes the platform as an argument: on Linux
    /// `cfg!(windows)` is false and a `cfg`-gated version of this could not
    /// have run where it was written.
    ///
    /// The control is the second half. A positive that says "the reading found
    /// it" proves nothing unless the reading can also fail — so the same
    /// directory is read once with the member and once without, and the
    /// member-blind reading must come back empty. If someone reintroduces a
    /// blind constructor, that assertion is what stops it.
    #[test]
    fn a_windows_shaped_prefix_is_read_back_by_the_member_and_not_by_the_command() {
        let member = "package/dist/bundle/cli.js";
        let at = scratch("windows-readback");
        let root = at.join("prefix");
        fs::create_dir_all(root.join("0.84.4").join("package/dist/bundle")).unwrap();
        fs::write(root.join("0.84.4").join(member), b"// the bundle").unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();

        // What `expose` leaves on Windows: a launcher named by the extension
        // rule, plus the marker that records which version it points into.
        let launcher = exposed_name_on("pi", member, true);
        assert_eq!(launcher, "pi.cmd");
        fs::write(root.join("bin").join(&launcher), b"@echo off\r\n").unwrap();
        fs::write(root.join("bin").join(".pi.version"), b"0.84.4\n").unwrap();

        assert_eq!(
            Present::under_on(&root, "pi", member, true)
                .exposed
                .as_deref(),
            Some("0.84.4"),
            "the reading told the member did not find the launcher beside it"
        );

        // The control: the same prefix, read the way the two callers read it
        // before this commit.
        assert_eq!(
            Present::under_on(&root, "pi", "", true).exposed,
            None,
            "a member-blind reading found something, so this test proves nothing"
        );

        // And both readings agree about what is installed, which is what makes
        // the difference above specifically about the exposed name.
        assert_eq!(
            Present::under_on(&root, "pi", "", true).versions,
            vec!["0.84.4".to_owned()]
        );
    }

    /// Pi installs from one tarball and the thing that lands runs.
    ///
    /// The declaration this replaces said pi could not be installed because
    /// "its dependency closure is resolved at install time, so there is no
    /// single artifact whose digest can be decided in advance". The published
    /// package ships `npm-shrinkwrap.json`, so the closure is fixed -- and it
    /// does not matter, because the bundle imports only Node built-ins and runs
    /// with no `node_modules` at all.
    #[test]
    #[cfg(unix)]
    fn a_javascript_program_installs_from_one_archive_and_runs() {
        let member = "package/dist/bundle/cli.js";
        let (at, artifact) = staged(
            "js-entry",
            b"#!/usr/bin/env sh\necho 'js-stand-in 9.9.9'\n",
            member,
        );
        let software = Software {
            version: "9.9.9",
            command: "jsprog",
            delivery: Delivery::Artifacts(&[]),
            unsupported: &[],
            previous: None,
        };
        let root = at.join("prefix");
        let installed = install(&software, &artifact, &at.join("artifact.tgz"), &root).unwrap();

        // Exposed under the name the rule gives, pointing into the archive's
        // own layout rather than a copy lifted out of it.
        assert_eq!(
            installed.executable,
            root.join("bin").join(exposed_name("jsprog", member))
        );
        assert_eq!(
            fs::canonicalize(&installed.executable).unwrap(),
            fs::canonicalize(root.join("9.9.9").join(member)).unwrap()
        );

        // And the reading of what is exposed agrees with what was written.
        assert_eq!(
            Present::under_named(&root, "jsprog", member)
                .exposed
                .as_deref(),
            Some("9.9.9")
        );
    }

    /// The exposed version is recorded, so it is readable where no link exists.
    ///
    /// This is the Windows defect written as a test that fails on Linux too.
    /// `expose` makes a symlink on Unix and a hard link or a copy on Windows,
    /// and the old reading resolved the link -- so on Windows the answer was
    /// always "nothing is exposed", on a prefix where something plainly was.
    ///
    /// It was not cosmetic: `Present::exposed` is what separates an install
    /// from an update, so every `software_update` on Windows saw an empty
    /// prefix and refused as an update of nothing.
    #[test]
    fn the_exposed_version_is_readable_without_a_link_to_resolve() {
        let (at, artifact) = staged("exposed-marker", b"#!/bin/sh\necho hi\n", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );

        // Exactly what Windows leaves behind: a real file where Unix has a
        // link. Nothing to resolve, and the answer must not change.
        let exposed = root.join("bin").join("codex");
        let bytes = fs::read(root.join("1.2.3").join(CODEX_MEMBER)).unwrap();
        fs::remove_file(&exposed).unwrap();
        fs::write(&exposed, &bytes).unwrap();
        assert!(!exposed.symlink_metadata().unwrap().is_symlink());
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3"),
            "the exposed version was unreadable without a link to resolve"
        );

        // A record naming a version that is not there is not believed.
        fs::write(root.join("bin").join(".codex.version"), "9.9.9").unwrap();
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER).exposed,
            None
        );

        // And removing takes the record with the command it described.
        fs::write(root.join("bin").join(".codex.version"), "1.2.3").unwrap();
        remove(&software(), &root).unwrap();
        assert!(!root.join("bin").join(".codex.version").exists());
    }

    /// A version that is not there is refused, and the refusal says what is --
    /// otherwise a caller's only way to find out is to guess again.
    #[test]
    fn rollback_to_a_version_that_is_not_installed_names_the_ones_that_are() {
        let (at, artifact) = staged("rollback-missing", b"x", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        let error = rollback(&software(), &root, "9.9.9").unwrap_err();
        assert!(error.detail().contains("9.9.9"), "{}", error.detail());
        assert!(error.detail().contains("1.2.3"), "{}", error.detail());
        // Nothing moved.
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );
    }

    /// A tree an older build wrote may not put the executable where this
    /// build's artifacts do. Both shapes are tried and neither is guessed at:
    /// if the file is not there, the refusal says where it looked.
    #[test]
    fn a_version_tree_with_no_executable_is_refused_naming_where_it_looked() {
        let (at, artifact) = staged("rollback-empty", b"x", CODEX_MEMBER);
        let root = at.join("prefix");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        fs::create_dir_all(root.join("1.2.2")).unwrap();

        let error = rollback(&software(), &root, "1.2.2").unwrap_err();
        assert!(error.detail().contains("1.2.2"), "{}", error.detail());
        assert!(error.detail().contains("looked at"), "{}", error.detail());
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );
    }

    /// Two platforms whose executables are named differently inside the
    /// archive, which is the ordinary case and not an exotic one: every
    /// `windows/*` artifact in this repository ends in `.exe` and no other
    /// does.
    const CROSS_ARTIFACTS: &[Artifact] = &[
        Artifact {
            platform: "linux/x86_64",
            url: "https://example.invalid/linux.tgz",
            bytes: 0,
            sha256: "sha256:0",
            shape: Shape::GzipTar,
            member: "package/bin/tool",
        },
        Artifact {
            platform: "windows/x86_64",
            url: "https://example.invalid/windows.tgz",
            bytes: 0,
            sha256: "sha256:0",
            shape: Shape::GzipTar,
            member: "package/bin/tool.exe",
        },
    ];

    fn cross() -> Software {
        Software {
            version: "1.2.3",
            command: "tool",
            delivery: Delivery::Artifacts(CROSS_ARTIFACTS),
            unsupported: &[],
            previous: None,
        }
    }

    /// A rollback on Windows looked for the Linux member and refused a tree
    /// that held the executable all along.
    ///
    /// `member_hint` answers with the *first* artifact's member whatever host
    /// asks, and every table here lists Linux first. The evidence run caught it
    /// on `windows-latest` while Ubuntu and macOS passed, which is exactly the
    /// shape a `cfg!` would have made unprovable from the machine that fixes
    /// it -- so the platform is a parameter and this runs everywhere.
    #[test]
    fn the_candidate_list_is_this_hosts_member_and_not_the_tables_first() {
        let root = Path::new("/prefix/1.2.2");

        let windows = executable_candidates(&cross(), root, "windows", "x86_64");
        assert_eq!(
            windows.first(),
            Some(&root.join("package/bin/tool.exe")),
            "this host's own member comes first: {windows:?}"
        );
        assert!(
            windows.contains(&root.join("tool.exe")),
            "the bare command with its extension is a shape an older tree may \
             have used: {windows:?}"
        );

        let linux = executable_candidates(&cross(), root, "linux", "x86_64");
        assert_eq!(
            linux.first(),
            Some(&root.join("package/bin/tool")),
            "{linux:?}"
        );
        assert!(
            !linux.contains(&root.join("tool.exe")),
            "the Windows shape is not offered to a platform that cannot run it: {linux:?}"
        );

        // The defect, stated as the thing that must not come back: the hint is
        // the Linux member on both hosts, so a list built from it alone sends a
        // Windows rollback to a file that is not there.
        assert_eq!(cross().member_hint(), "package/bin/tool");
        assert_ne!(
            windows.first(),
            Some(&root.join(cross().member_hint())),
            "a Windows candidate list must not start at the hint"
        );
    }

    /// An unsupported platform still gets a usable list rather than an empty
    /// one: the tree may have been written by a build that supported it.
    #[test]
    fn a_platform_with_no_artifact_still_offers_the_older_shapes() {
        let root = Path::new("/prefix/1.2.2");
        let found = executable_candidates(&software(), root, "windows", "x86_64");
        assert!(!found.is_empty(), "{found:?}");
        assert!(found.contains(&root.join("codex.exe")), "{found:?}");
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
            previous: None,
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
    fn an_empty_or_absent_prefix_reads_as_holding_nothing() {
        // A plan for a prefix that does not exist yet is the ordinary first
        // case, not a failure.
        let nowhere = scratch("present-absent");
        assert_eq!(
            Present::under_named(&nowhere, "codex", CODEX_MEMBER),
            Present::default()
        );
    }

    #[test]
    fn what_is_under_a_prefix_is_read_including_which_version_is_exposed() {
        let (at, artifact) = staged("present-read", b"payload", CODEX_MEMBER);
        let root = at.join("software");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        // A second version, installed but not exposed.
        fs::create_dir_all(root.join("9.9.9")).unwrap();

        let found = Present::under_named(&root, "codex", CODEX_MEMBER);
        assert_eq!(found.versions, vec!["1.2.3".to_owned(), "9.9.9".to_owned()]);
        assert!(found.holds("1.2.3"));
        assert!(!found.holds("0.0.1"));
        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn the_exposed_version_is_read_through_the_link_however_it_was_written() {
        let (at, artifact) = staged("present-link", b"payload", CODEX_MEMBER);
        let root = at.join("software");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();
        let link = root.join("bin").join("codex");

        // What `expose` writes: an absolute target.
        assert!(fs::read_link(&link).unwrap().is_absolute());
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );

        // What a person might write instead. Taking the first component of
        // `../1.2.3/...` without resolving it gives `bin`, and the answer
        // becomes "no entry point names this version" while one does.
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(
            std::path::Path::new("..").join("1.2.3").join(CODEX_MEMBER),
            &link,
        )
        .unwrap();
        assert!(!fs::read_link(&link).unwrap().is_absolute());
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER)
                .exposed
                .as_deref(),
            Some("1.2.3")
        );

        // A dangling link exposes nothing, which is what it means.
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(root.join("9.9.9").join("codex"), &link).unwrap();
        assert_eq!(
            Present::under_named(&root, "codex", CODEX_MEMBER).exposed,
            None
        );

        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn bin_and_the_control_directory_are_not_versions() {
        let at = scratch("present-notversions");
        fs::create_dir_all(at.join("bin")).unwrap();
        fs::create_dir_all(at.join(".codex-setup-system")).unwrap();
        fs::create_dir_all(at.join("1.2.3")).unwrap();
        assert_eq!(
            Present::under_named(&at, "codex", CODEX_MEMBER).versions,
            vec!["1.2.3".to_owned()]
        );
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
