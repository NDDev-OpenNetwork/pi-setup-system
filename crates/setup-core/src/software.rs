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

    /// Read what a program directory holds.
    ///
    /// A missing or unreadable directory reads as empty rather than failing: a
    /// plan for a prefix that does not exist yet is exactly the ordinary case.
    #[must_use]
    pub fn under(root: &Path, command: &str) -> Self {
        Self::under_named(root, command, "")
    }

    /// The same reading, told which member the exposed thing was made from.
    ///
    /// Windows exposes a JavaScript entry point as `<command>.cmd`, so a
    /// reading that only ever looked for `<command>` would report nothing
    /// exposed on exactly the harness that needed the launcher.
    #[must_use]
    pub fn under_named(root: &Path, command: &str, member: &str) -> Self {
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
        let exposed_as = exposed_name(command, member);
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
        Shape::GzipTar | Shape::Zip => {
            let entries = if artifact.shape == Shape::Zip {
                archive::extract_zip(source, &version_root, artifact.limits())?
            } else {
                archive::extract_gzip_tar(source, &version_root, artifact.limits())?
            };
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
    // Through the extension rather than the spelling: Windows treats `.JS` and
    // `.js` as one extension, and a rule that missed the first would expose an
    // unrunnable file on the only platform this branch exists for.
    let javascript = Path::new(member)
        .extension()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("js"));
    if windows && javascript {
        format!("{command}.cmd")
    } else {
        command.to_owned()
    }
}

/// Point one stable path at the executable inside a versioned tree.
///
/// The member is left where the archive put it. Codex's binary needs the `rg`
/// and `bwrap` beside it and cursor's launcher needs its bundled `node`, so
/// moving the executable out of its tree would produce a file that runs on the
/// machine it was built on and nowhere else.
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
        if exposed.extension().is_some_and(|kind| kind == "cmd") {
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
        } else {
            // Windows reserves symlink creation for privileged or
            // developer-mode processes, so a hard link is what actually works;
            // a copy is the last resort and costs a second copy of a large
            // binary.
            fs::hard_link(executable, exposed)
                .or_else(|_| fs::copy(executable, exposed).map(|_| ()))
                .map_err(fail)?;
        }
    }

    // Which version this now runs, recorded rather than left to be inferred
    // from a link that two of the three systems do not make. Written after the
    // command is in place, so a marker never names a version that is not
    // exposed yet.
    if let Some(root) = exposed.parent().and_then(Path::parent) {
        fs::write(Present::marker(root, command), version).map_err(fail)?;
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

        let present = Present::under(&root, "codex");
        assert_eq!(present.exposed.as_deref(), Some("1.2.2"));
        assert_eq!(present.versions, vec!["1.2.2", "1.2.3"]);

        // And the version it came from is still there to go forward to.
        assert!(root.join("1.2.3").join(CODEX_MEMBER).is_file());
        assert_eq!(
            rollback(&software(), &root, "1.2.3").unwrap().version,
            "1.2.3"
        );
        assert_eq!(
            Present::under(&root, "codex").exposed.as_deref(),
            Some("1.2.3")
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
            Present::under(&root, "codex").exposed.as_deref(),
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
            Present::under(&root, "codex").exposed.as_deref(),
            Some("1.2.3"),
            "the exposed version was unreadable without a link to resolve"
        );

        // A record naming a version that is not there is not believed.
        fs::write(root.join("bin").join(".codex.version"), "9.9.9").unwrap();
        assert_eq!(Present::under(&root, "codex").exposed, None);

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
            Present::under(&root, "codex").exposed.as_deref(),
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
            Present::under(&root, "codex").exposed.as_deref(),
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
        assert_eq!(Present::under(&nowhere, "codex"), Present::default());
    }

    #[test]
    fn what_is_under_a_prefix_is_read_including_which_version_is_exposed() {
        let (at, artifact) = staged("present-read", b"payload", CODEX_MEMBER);
        let root = at.join("software");
        install(&software(), &artifact, &at.join("artifact.tgz"), &root).unwrap();

        // A second version, installed but not exposed.
        fs::create_dir_all(root.join("9.9.9")).unwrap();

        let found = Present::under(&root, "codex");
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
            Present::under(&root, "codex").exposed.as_deref(),
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
            Present::under(&root, "codex").exposed.as_deref(),
            Some("1.2.3")
        );

        // A dangling link exposes nothing, which is what it means.
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(root.join("9.9.9").join("codex"), &link).unwrap();
        assert_eq!(Present::under(&root, "codex").exposed, None);

        fs::remove_dir_all(&at).unwrap();
    }

    #[test]
    fn bin_and_the_control_directory_are_not_versions() {
        let at = scratch("present-notversions");
        fs::create_dir_all(at.join("bin")).unwrap();
        fs::create_dir_all(at.join(".codex-setup-system")).unwrap();
        fs::create_dir_all(at.join("1.2.3")).unwrap();
        assert_eq!(
            Present::under(&at, "codex").versions,
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
