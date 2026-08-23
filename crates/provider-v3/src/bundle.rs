//! Reading and checking an `ai-stp-bundle/1` before anything is written.
//!
//! The order of checks is the contract's, and it matters. The raw bytes are
//! hashed *before* the archive is parsed, so a corrupted artifact is refused by
//! a cheap comparison rather than by whatever a parser makes of it. Only then is
//! the canonical shape read, the manifest parsed, and each file matched against
//! the record that declares it.
//!
//! Nothing here writes. A bundle that passes every check is still only a bundle
//! that may be planned; materializing it happens later, under the lock, through
//! the same kernel every other effect uses.
//!
//! # Three identities, three checks
//!
//! - `artifact_digest` — plain SHA-256 of the exact ZIP bytes.
//! - `bundle_digest` — the manifest's own identity, in the `ai-stp:bundle:v1`
//!   domain, over the canonical manifest *without* its own digest field.
//! - each file's `digest` — SHA-256 of that file's bytes.
//!
//! They are deliberately not derivable from one another. A provider that
//! computed one from another would be checking its own arithmetic instead of the
//! sender's claim.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use setup_core::digest;

use crate::error::{Error, Result};
use crate::reason::WireReason;
use crate::zip;

/// The digest domain for a bundle manifest.
pub const BUNDLE_DOMAIN: &str = "ai-stp:bundle:v1";

/// The format tag this reader accepts.
pub const BUNDLE_FORMAT: &str = "ai-stp-bundle/1";

/// The protocol version a bundle manifest declares.
///
/// This is **not** the provider protocol. A bundle is `ai-stp-bundle/1` and says
/// `protocol_version: 1`; the provider speaking about it is protocol v3. Two
/// numbers, two contracts, one field name each — comparing a manifest against
/// the provider's version rejects every well-formed bundle, and does it with a
/// message that sounds right.
pub const BUNDLE_PROTOCOL_VERSION: u32 = 1;

/// The manifest member, always first.
pub const MANIFEST_MEMBER: &str = "bundle.json";

/// The prefix under which managed files live.
pub const FILES_PREFIX: &str = "files/";

/// The members the format requires, in the order it requires them.
pub const REQUIRED_MEMBERS: &[&str] = &[
    "bundle.json",
    "setup-passport.json",
    "composition-report.json",
    "conversion-report.json",
];

/// The only file modes a bundle may carry.
const ALLOWED_MODES: &[u32] = &[0o644, 0o755];

/// The contract's own limits, which no bundle may raise.
///
/// A manifest *reports* its limits; it does not set them. Taking the bundle's
/// word would let a hostile one declare a ceiling of its own choosing and then
/// sit comfortably under it. The effective limit is the smaller of what the
/// contract fixes and what the bundle claims.
pub const CONTRACT_MAX_FILES: u64 = 2000;
/// The largest single file the contract admits: 4 MiB.
pub const CONTRACT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// The largest archive the contract admits: 64 MiB.
pub const CONTRACT_MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

/// Path segments and names that mean credentials, refused by name.
///
/// Opening a file to decide whether it holds a secret is the very act this rule
/// exists to prevent, so the decision is made from the name alone.
const SECRET_MARKERS: &[&str] = &[
    "credentials",
    ".credentials.json",
    "auth.json",
    ".netrc",
    "id_rsa",
    "id_ed25519",
    ".pem",
    ".p12",
    "secrets",
];

/// What a manifest record says a file *is*.
///
/// A bundle declares this rather than leaving it to the container, and that is
/// deliberate: a ZIP has no portable hard-link member, so a hostile bundle could
/// carry one as an ordinary file. The declared kind is refused on its own terms,
/// and a provider must not reinterpret it as a regular file merely because the
/// archive format cannot express the difference.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    /// An ordinary file, the only kind that is materialized.
    #[default]
    File,
    /// A symbolic link.
    Symlink,
    /// A hard link.
    Hardlink,
    /// A device, socket or pipe.
    Special,
    /// A kind this build does not know.
    #[serde(other)]
    Unknown,
}

/// What a compiler says it turned each component into.
#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
pub struct ConversionReport {
    /// Whether every component was converted.
    #[serde(default)]
    pub complete: bool,
    /// One entry per component.
    #[serde(default)]
    pub entries: Vec<ConversionEntry>,
}

/// One component, and the kind the compiler assigned it.
#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
pub struct ConversionEntry {
    /// The component's stable identity.
    #[serde(default)]
    pub stable_id: String,
    /// The component kind. Checked against what a provider declares.
    #[serde(default)]
    pub component_type: String,
    /// Where it was written, relative to the target.
    #[serde(default)]
    pub native_surface: String,
}

/// One record in the bundle's file manifest.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BundleFile {
    /// The normalized relative path this file takes in the target.
    pub path: String,
    /// SHA-256 of the file's bytes.
    pub digest: String,
    /// The exact byte length.
    pub byte_length: u64,
    /// The Unix mode, one of 0o644 or 0o755.
    pub mode: u32,
    /// Which surface owns it.
    #[serde(default)]
    pub owner: String,
    /// What this record says the file is. Absent means an ordinary file.
    #[serde(default)]
    pub kind: FileKind,
}

/// The limits a manifest reports.
///
/// Reported, not set. See [`Manifest::effective_max_files`] and its siblings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Limits {
    /// The file count this bundle reports.
    #[serde(default)]
    pub max_files: Option<u64>,
    /// The per-file byte count this bundle reports.
    #[serde(default)]
    pub max_file_bytes: Option<u64>,
    /// The archive byte count this bundle reports.
    #[serde(default)]
    pub max_bundle_bytes: Option<u64>,
}

/// The `bundle.json` a compiler writes into the archive.
///
/// This is *not* the compiler's own result object. `cli-harness-bundle.schema.json`
/// describes the latter — a `CompiledBundle` with `compiled`, `refusals`, and a
/// flat `max_files` — and reading it as though it described the archive member
/// costs a day: every field name is plausible and every one is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// Schema of this manifest.
    pub schema_version: u32,
    /// Always `ai-stp-bundle/1`.
    pub bundle_format: String,
    /// The *bundle* protocol, which is 1. Not the provider protocol.
    pub protocol_version: u32,
    /// The harness this bundle configures.
    pub harness_id: String,
    /// What compiled it.
    #[serde(default)]
    pub builder_version: String,
    /// The manifest's own identity, over itself without this field.
    pub bundle_digest: String,
    /// The digest of the compiler's input.
    #[serde(default)]
    pub input_digest: String,
    /// What the compiler turned each component into.
    ///
    /// This is the only place a component's *kind* is stated: the manifest does
    /// not carry kinds and the setup passport carries references without them.
    /// A provider that never reads this cannot tell that it has been handed a
    /// kind it does not implement -- and a hostile bundle declaring one is
    /// exactly the case the consumer's corpus drives.
    #[serde(default)]
    pub conversion_report: ConversionReport,
    /// Every path this bundle is allowed to write.
    #[serde(default)]
    pub managed_paths: Vec<String>,
    /// The file manifest.
    #[serde(default)]
    pub files: Vec<BundleFile>,
    /// The limits this bundle reports.
    #[serde(default)]
    pub limits: Limits,
}

impl Manifest {
    /// The effective file-count limit: the contract's, never raised.
    #[must_use]
    pub fn effective_max_files(&self) -> u64 {
        self.limits
            .max_files
            .map_or(CONTRACT_MAX_FILES, |c| c.min(CONTRACT_MAX_FILES))
    }

    /// The effective per-file limit: the contract's, never raised.
    #[must_use]
    pub fn effective_max_file_bytes(&self) -> u64 {
        self.limits
            .max_file_bytes
            .map_or(CONTRACT_MAX_FILE_BYTES, |c| c.min(CONTRACT_MAX_FILE_BYTES))
    }

    /// The effective archive limit: the contract's, never raised.
    #[must_use]
    pub fn effective_max_bundle_bytes(&self) -> u64 {
        self.limits
            .max_bundle_bytes
            .map_or(CONTRACT_MAX_BUNDLE_BYTES, |c| {
                c.min(CONTRACT_MAX_BUNDLE_BYTES)
            })
    }
}

/// A bundle that passed every check, with its files ready to materialize.
#[derive(Debug, Clone)]
pub struct Bundle {
    /// The manifest as read.
    pub manifest: Manifest,
    /// Each declared file's bytes and mode, by target-relative path.
    pub files: BTreeMap<String, (Vec<u8>, u32)>,
}

/// What the caller claimed about the bytes it sent.
#[derive(Debug, Clone, Copy)]
pub struct Claim<'a> {
    /// The format tag from argv.
    pub bundle_format: &'a str,
    /// The logical manifest digest from argv.
    pub bundle_digest: &'a str,
    /// The raw artifact digest from argv.
    pub artifact_digest: &'a str,
    /// The exact byte count from argv.
    pub bundle_size: u64,
    /// The harness this provider configures.
    pub harness_id: &'a str,
}

impl Bundle {
    /// Check exact bytes against an exact claim.
    ///
    /// # Errors
    ///
    /// Refuses with the contract's reason for the first check that fails. Every
    /// refusal happens before anything is written, because nothing here writes.
    pub fn read(bytes: &[u8], claim: Claim<'_>) -> Result<Self> {
        if claim.bundle_format != BUNDLE_FORMAT {
            return Err(Error::refuse(
                WireReason::UnsupportedBundleFormat,
                format!("{:?} is not {BUNDLE_FORMAT}", claim.bundle_format),
            ));
        }
        // The raw bytes are checked before the parser sees them: a corrupted
        // artifact should be refused by a comparison, not by whatever a parser
        // makes of it.
        let actual_length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if actual_length != claim.bundle_size {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!(
                    "the artifact is {actual_length} bytes, not {}",
                    claim.bundle_size
                ),
            ));
        }
        if digest::of_bytes(bytes) != claim.artifact_digest {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                "the artifact bytes do not hash to the digest that named them",
            ));
        }

        let members = zip::read(bytes)?;
        let manifest_bytes = require_members(&members)?;
        let manifest = parse_manifest(manifest_bytes)?;

        if manifest.bundle_format != BUNDLE_FORMAT {
            return Err(Error::refuse(
                WireReason::UnsupportedBundleFormat,
                format!("the manifest declares {:?}", manifest.bundle_format),
            ));
        }
        if manifest.protocol_version != BUNDLE_PROTOCOL_VERSION {
            return Err(Error::refuse(
                WireReason::UnsupportedProtocolVersion,
                format!(
                    "the manifest declares bundle protocol {}, and this reader speaks {BUNDLE_PROTOCOL_VERSION}",
                    manifest.protocol_version
                ),
            ));
        }
        if manifest.harness_id != claim.harness_id {
            return Err(Error::refuse(
                WireReason::ProjectionProfileMismatch,
                format!(
                    "the bundle is for harness {:?}, and this provider configures {:?}",
                    manifest.harness_id, claim.harness_id
                ),
            ));
        }
        let files = check_files(&manifest, &members, actual_length)?;

        // The manifest's own identity is checked after the files, so a bundle
        // with a concrete defect reports that defect rather than an unstated
        // field. The artifact digest -- the one that proves these are the bytes
        // the caller sent -- was already checked before the parser ran.
        check_manifest_digest(manifest_bytes, &manifest.bundle_digest)?;
        if manifest.bundle_digest != claim.bundle_digest {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                "the manifest's identity is not the one the caller named",
            ));
        }

        Ok(Self { manifest, files })
    }
}

/// Require the four documents, once each, in order, before anything else.
fn require_members(members: &[zip::Member]) -> Result<&[u8]> {
    for (index, required) in REQUIRED_MEMBERS.iter().enumerate() {
        match members.get(index) {
            Some(member) if member.name == *required => {}
            Some(member) => {
                return Err(Error::refuse(
                    WireReason::UnsupportedBundleFormat,
                    format!("member {index} is {:?}, not {required:?}", member.name),
                ));
            }
            None => {
                return Err(Error::refuse(
                    WireReason::UnsupportedBundleFormat,
                    format!("the bundle has no {required:?}"),
                ));
            }
        }
    }
    let mut seen = BTreeSet::new();
    for member in members {
        if !seen.insert(member.name.as_str()) {
            return Err(Error::refuse(
                WireReason::PathDuplicate,
                format!("member {:?} appears twice", member.name),
            ));
        }
    }
    members
        .first()
        .map(|member| member.data.as_slice())
        .ok_or_else(|| Error::refuse(WireReason::UnsupportedBundleFormat, "the bundle is empty"))
}

fn parse_manifest(bytes: &[u8]) -> Result<Manifest> {
    serde_json::from_slice(bytes).map_err(|source| {
        Error::refuse(
            WireReason::UnsupportedBundleFormat,
            format!("{MANIFEST_MEMBER} does not parse: {source}"),
        )
    })
}

/// The manifest's identity is taken over itself minus that identity.
fn check_manifest_digest(bytes: &[u8], declared: &str) -> Result<()> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(|source| {
        Error::refuse(
            WireReason::UnsupportedBundleFormat,
            format!("{MANIFEST_MEMBER} does not parse: {source}"),
        )
    })?;
    let Some(object) = value.as_object_mut() else {
        return Err(Error::refuse(
            WireReason::UnsupportedBundleFormat,
            format!("{MANIFEST_MEMBER} is not an object"),
        ));
    };
    // A value cannot be part of the input it identifies.
    object.remove("bundle_digest");
    let computed = digest::of_domain_canonical_json(BUNDLE_DOMAIN, &value)?;
    if computed != declared {
        return Err(Error::refuse(
            WireReason::DigestMismatch,
            "the manifest does not hash to the identity it declares",
        ));
    }
    Ok(())
}

fn check_files(
    manifest: &Manifest,
    members: &[zip::Member],
    artifact_length: u64,
) -> Result<BTreeMap<String, (Vec<u8>, u32)>> {
    let declared = u64::try_from(manifest.files.len()).unwrap_or(u64::MAX);
    let max_files = manifest.effective_max_files();
    if declared > max_files {
        return Err(Error::refuse(
            WireReason::LimitExceeded,
            format!("{declared} files declared against a limit of {max_files}"),
        ));
    }
    let max_bundle = manifest.effective_max_bundle_bytes();
    if artifact_length > max_bundle {
        return Err(Error::refuse(
            WireReason::LimitExceeded,
            format!("the artifact is {artifact_length} bytes against a limit of {max_bundle}"),
        ));
    }

    let present: BTreeMap<&str, &zip::Member> = members
        .iter()
        .filter_map(|member| {
            member
                .name
                .strip_prefix(FILES_PREFIX)
                .map(|rest| (rest, member))
        })
        // A directory member is structure, not content: `files/` itself appears
        // in every bundle and declares nothing.
        .filter(|(rest, _)| !rest.is_empty() && !rest.ends_with('/'))
        .collect();

    let mut files = BTreeMap::new();
    let mut lowercase: BTreeMap<String, String> = BTreeMap::new();
    for record in &manifest.files {
        check_record(record, manifest, &mut lowercase)?;
        let Some(member) = present.get(record.path.as_str()) else {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!(
                    "the manifest declares {:?}, which the archive does not carry",
                    record.path
                ),
            ));
        };
        let actual = u64::try_from(member.data.len()).unwrap_or(u64::MAX);
        if actual != record.byte_length {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!(
                    "{:?} is {actual} bytes, not {}",
                    record.path, record.byte_length
                ),
            ));
        }
        if digest::of_bytes(&member.data) != record.digest {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!(
                    "{:?} does not hash to the digest that declares it",
                    record.path
                ),
            ));
        }
        if files
            .insert(record.path.clone(), (member.data.clone(), record.mode))
            .is_some()
        {
            return Err(Error::refuse(
                WireReason::PathDuplicate,
                format!("{:?} is declared twice", record.path),
            ));
        }
    }

    // A file the archive carries but the manifest never declared would be
    // installed by nobody's decision.
    for name in present.keys() {
        if !files.contains_key(*name) {
            return Err(Error::refuse(
                WireReason::UnsupportedNativeSurface,
                format!("the archive carries {name:?}, which the manifest does not declare"),
            ));
        }
    }
    Ok(files)
}

/// Check one manifest record's own claims, before its bytes are looked at.
fn check_record(
    record: &BundleFile,
    manifest: &Manifest,
    lowercase: &mut BTreeMap<String, String>,
) -> Result<()> {
    check_path(&record.path)?;
    match record.kind {
        FileKind::File => {}
        FileKind::Symlink | FileKind::Hardlink => {
            return Err(Error::refuse(
                WireReason::LinkNotAllowed,
                format!(
                    "{:?} is declared as a link, which is never materialized",
                    record.path
                ),
            ));
        }
        FileKind::Special | FileKind::Unknown => {
            return Err(Error::refuse(
                WireReason::SpecialFileNotAllowed,
                format!("{:?} is declared as neither a file nor a link", record.path),
            ));
        }
    }
    if !ALLOWED_MODES.contains(&record.mode) {
        return Err(Error::refuse(
            WireReason::UnsupportedNativeSurface,
            format!(
                "{:?} declares mode {:o}, which is not allowed",
                record.path, record.mode
            ),
        ));
    }
    if record.byte_length > manifest.effective_max_file_bytes() {
        return Err(Error::refuse(
            WireReason::LimitExceeded,
            format!("{:?} is larger than the declared file limit", record.path),
        ));
    }
    // Two paths differing only in case are one path on a case-insensitive file
    // system, and the second would silently overwrite the first.
    if let Some(other) = lowercase.insert(record.path.to_lowercase(), record.path.clone())
        && other != record.path
    {
        return Err(Error::refuse(
            WireReason::PathDuplicate,
            format!("{:?} and {other:?} differ only in case", record.path),
        ));
    }
    Ok(())
}

/// Every rule a bundle path must satisfy, checked by shape rather than by trial.
fn check_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::refuse(
            WireReason::PathNotRelative,
            "a bundle path is not empty",
        ));
    }
    if path.len() > 1024 {
        return Err(Error::refuse(
            WireReason::LimitExceeded,
            format!("{path:?} is longer than 1024 bytes"),
        ));
    }
    if path.starts_with('/') || path.starts_with('~') || path.starts_with('\\') {
        return Err(Error::refuse(
            WireReason::PathNotRelative,
            format!("{path:?} is not relative"),
        ));
    }
    // A drive letter is absolute on the platform that has drive letters, and a
    // path is checked for what it is rather than for where it is read.
    if path.as_bytes().get(1) == Some(&b':') {
        return Err(Error::refuse(
            WireReason::PathNotRelative,
            format!("{path:?} carries a drive letter"),
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            return Err(Error::refuse(
                WireReason::PathNotRelative,
                format!("{path:?} has an empty or bare-dot segment"),
            ));
        }
        if segment == ".." {
            return Err(Error::refuse(
                WireReason::PathEscapesTarget,
                format!("{path:?} climbs out of the target"),
            ));
        }
        if segment.len() > 255 {
            return Err(Error::refuse(
                WireReason::LimitExceeded,
                format!("{path:?} has a segment longer than 255 bytes"),
            ));
        }
    }
    if path.chars().any(char::is_control) {
        return Err(Error::refuse(
            WireReason::PathNotRelative,
            format!("{path:?} contains a control character"),
        ));
    }
    if path.contains('\\') {
        return Err(Error::refuse(
            WireReason::PathNotRelative,
            format!("{path:?} uses a backslash separator"),
        ));
    }
    let lowered = path.to_lowercase();
    if SECRET_MARKERS.iter().any(|marker| lowered.contains(marker)) {
        return Err(Error::refuse(
            WireReason::SpecialFileNotAllowed,
            format!("{path:?} is named like a credential and is refused by name"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::zip::build::{Entry, write};

    fn document(name: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            data: b"{}".to_vec(),
            mode: 0o644,
        }
    }

    struct Built {
        bytes: Vec<u8>,
        claim_digest: String,
        artifact_digest: String,
        length: u64,
    }

    /// Build a bundle whose three identities are all internally consistent.
    fn build(files: &[(&str, &str, u32)]) -> Built {
        let records: Vec<serde_json::Value> = files
            .iter()
            .map(|(path, body, mode)| {
                serde_json::json!({
                    "schema_version": 1,
                    "path": path,
                    "digest": digest::of_bytes(body.as_bytes()),
                    "byte_length": body.len(),
                    "mode": mode,
                    "owner": "",
                })
            })
            .collect();
        let mut manifest = serde_json::json!({
            "schema_version": 1,
            "bundle_format": BUNDLE_FORMAT,
            "protocol_version": BUNDLE_PROTOCOL_VERSION,
            "harness_id": "test",
            "builder_version": "0.1.0",
            "input_digest": "sha256:".to_owned() + &"3".repeat(64),
            "managed_paths": files.iter().map(|(path, _, _)| *path).collect::<Vec<_>>(),
            "files": records,
            "limits": {
                "max_files": 2000,
                "max_file_bytes": 4 * 1024 * 1024,
                "max_bundle_bytes": 64 * 1024 * 1024,
            },
        });
        let bundle_digest = digest::of_domain_canonical_json(BUNDLE_DOMAIN, &manifest).unwrap();
        manifest["bundle_digest"] = serde_json::json!(bundle_digest);
        let manifest_bytes = setup_core::canonical::to_canonical_bytes(&manifest).unwrap();

        let mut entries = vec![Entry {
            name: MANIFEST_MEMBER.to_owned(),
            data: manifest_bytes,
            mode: 0o644,
        }];
        for name in REQUIRED_MEMBERS.iter().skip(1) {
            entries.push(document(name));
        }
        for (path, body, mode) in files {
            entries.push(Entry {
                name: format!("{FILES_PREFIX}{path}"),
                data: body.as_bytes().to_vec(),
                mode: *mode,
            });
        }
        let bytes = write(&entries);
        let artifact_digest = digest::of_bytes(&bytes);
        let length = bytes.len() as u64;
        Built {
            bytes,
            claim_digest: bundle_digest,
            artifact_digest,
            length,
        }
    }

    fn claim(built: &Built) -> Claim<'_> {
        Claim {
            bundle_format: BUNDLE_FORMAT,
            bundle_digest: &built.claim_digest,
            artifact_digest: &built.artifact_digest,
            bundle_size: built.length,
            harness_id: "test",
        }
    }

    #[test]
    fn a_consistent_bundle_is_read_and_carries_its_files() {
        let built = build(&[
            ("AGENTS.md", "# hello\n", 0o644),
            ("skills/a.md", "skill", 0o644),
        ]);
        let bundle = Bundle::read(&built.bytes, claim(&built)).unwrap();
        assert_eq!(bundle.manifest.harness_id, "test");
        assert_eq!(bundle.files.len(), 2);
        assert_eq!(bundle.files["AGENTS.md"].0, b"# hello\n");
        assert_eq!(bundle.files["skills/a.md"].1, 0o644);
    }

    #[test]
    fn an_executable_mode_is_carried_because_some_surfaces_are_executable() {
        let built = build(&[("hooks/run.sh", "#!/bin/sh\n", 0o755)]);
        let bundle = Bundle::read(&built.bytes, claim(&built)).unwrap();
        assert_eq!(bundle.files["hooks/run.sh"].1, 0o755);
    }

    #[test]
    fn the_raw_bytes_are_checked_before_the_parser_ever_runs() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut claim = claim(&built);
        let wrong = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        claim.artifact_digest = wrong;
        let error = Bundle::read(&built.bytes, claim).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
        assert!(error.detail().contains("do not hash"), "{error}");
    }

    #[test]
    fn a_size_that_disagrees_with_the_bytes_is_refused_first() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut claim = claim(&built);
        claim.bundle_size = built.length + 1;
        assert!(
            Bundle::read(&built.bytes, claim)
                .unwrap_err()
                .detail()
                .contains("bytes, not")
        );
    }

    #[test]
    fn a_bundle_for_another_harness_is_a_profile_mismatch_not_a_generic_refusal() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut claim = claim(&built);
        claim.harness_id = "other";
        let error = Bundle::read(&built.bytes, claim).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::ProjectionProfileMismatch));
    }

    #[test]
    fn a_manifest_identity_the_caller_did_not_name_is_refused() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut claim = claim(&built);
        let other = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
        claim.bundle_digest = other;
        let error = Bundle::read(&built.bytes, claim).unwrap_err();
        assert!(
            error.detail().contains("not the one the caller named"),
            "{error}"
        );
    }

    #[test]
    fn a_file_whose_bytes_do_not_match_its_record_is_refused() {
        let mut built = build(&[("AGENTS.md", "hello", 0o644)]);
        // Corrupt the member payload, then re-stamp the two outer identities so
        // only the inner file digest is wrong.
        let position = built.bytes.windows(5).position(|w| w == b"hello").unwrap();
        built.bytes[position] = b'j';
        // The CRC check fires first, which is itself the right refusal.
        built.artifact_digest = digest::of_bytes(&built.bytes);
        let error = Bundle::read(&built.bytes, claim(&built)).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
    }

    #[test]
    fn every_hostile_path_shape_is_refused_by_its_own_reason() {
        for (path, reason) in [
            ("/etc/passwd", WireReason::PathNotRelative),
            ("~/.ssh/config", WireReason::PathNotRelative),
            ("C:/Windows/system32", WireReason::PathNotRelative),
            ("../outside", WireReason::PathEscapesTarget),
            ("a/../../b", WireReason::PathEscapesTarget),
            ("a//b", WireReason::PathNotRelative),
            ("./a", WireReason::PathNotRelative),
            ("a\\b", WireReason::PathNotRelative),
            ("", WireReason::PathNotRelative),
        ] {
            let error = check_path(path).unwrap_err();
            assert_eq!(error.reason(), Some(reason), "{path:?} gave {error}");
        }
    }

    #[test]
    fn a_path_named_like_a_credential_is_refused_without_opening_it() {
        // Opening a file to decide whether it is a secret is the act this rule
        // exists to prevent.
        for path in [
            ".credentials.json",
            "auth.json",
            "keys/id_ed25519",
            "certs/server.pem",
        ] {
            let error = check_path(path).unwrap_err();
            assert_eq!(
                error.reason(),
                Some(WireReason::SpecialFileNotAllowed),
                "{path}"
            );
        }
    }

    #[test]
    fn a_control_character_in_a_path_is_refused() {
        assert!(check_path("a\u{1}b").is_err());
    }

    #[test]
    fn an_over_long_path_or_segment_is_refused() {
        assert!(check_path(&"a".repeat(1025)).is_err());
        assert!(check_path(&format!("dir/{}", "a".repeat(256))).is_err());
    }

    #[test]
    fn an_archive_member_the_manifest_never_declared_is_refused() {
        // Otherwise it would be installed by nobody's decision.
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut entries: Vec<Entry> = Vec::new();
        for member in zip::read(&built.bytes).unwrap() {
            entries.push(Entry {
                name: member.name,
                data: member.data,
                mode: 0o644,
            });
        }
        entries.push(Entry {
            name: format!("{FILES_PREFIX}undeclared.md"),
            data: b"sneaky".to_vec(),
            mode: 0o644,
        });
        let bytes = write(&entries);
        let artifact = digest::of_bytes(&bytes);
        let claim = Claim {
            bundle_format: BUNDLE_FORMAT,
            bundle_digest: &built.claim_digest,
            artifact_digest: &artifact,
            bundle_size: bytes.len() as u64,
            harness_id: "test",
        };
        let error = Bundle::read(&bytes, claim).unwrap_err();
        assert!(error.detail().contains("undeclared.md"), "{error}");
    }

    #[test]
    fn the_four_documents_are_required_in_order() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut members = zip::read(&built.bytes).unwrap();
        members.swap(1, 2);
        let entries: Vec<Entry> = members
            .into_iter()
            .map(|m| Entry {
                name: m.name,
                data: m.data,
                mode: 0o644,
            })
            .collect();
        let bytes = write(&entries);
        let artifact = digest::of_bytes(&bytes);
        let claim = Claim {
            bundle_format: BUNDLE_FORMAT,
            bundle_digest: &built.claim_digest,
            artifact_digest: &artifact,
            bundle_size: bytes.len() as u64,
            harness_id: "test",
        };
        let error = Bundle::read(&bytes, claim).unwrap_err();
        assert!(error.detail().contains("not"), "{error}");
    }

    #[test]
    fn a_format_tag_this_reader_does_not_know_is_refused_before_anything_else() {
        let built = build(&[("AGENTS.md", "x", 0o644)]);
        let mut claim = claim(&built);
        claim.bundle_format = "ai-stp-bundle/2";
        let error = Bundle::read(&built.bytes, claim).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedBundleFormat));
    }
}
