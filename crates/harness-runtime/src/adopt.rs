//! Taking over a target the frozen estate still claims.
//!
//! Before these seven there was `nddev-harnesses`, a Python estate whose module
//! for a product wrote a stamp file beside the configuration it managed. Some of
//! those files are still on disk. This build writes `NDDEV-<TOOL>-PROVIDER.json`
//! and reads only that, so such a target reports `unmanaged`, an install leaves
//! both files, and the old program then sees drift in a directory it no longer
//! owns.
//!
//! Adoption ends that, and it is a command someone types. An install that
//! quietly took over a file this program never wrote would be worse than the
//! honest coexistence, because the person who ran it would not know it had
//! happened.
//!
//! Nothing is deleted. The old stamp is moved into this provider's own control
//! directory, which stops the old program recognising it and leaves the
//! pre-adoption state one `mv` away from being back.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use provider_v3::{Error, Result, WireReason};
use serde::Deserialize;
use setup_core::{digest, target::Target};

use crate::facts::Harness;

/// The schema the frozen estate's stamp files carry.
const PREDECESSOR_SCHEMA: u32 = 1;

/// Where an adopted stamp is kept, inside the control directory.
const KEPT_IN: &str = "adopted";

/// One frozen-estate stamp, as much of it as adoption needs.
///
/// Extra fields are ignored rather than refused: some modules wrote a
/// `content_setup_id` or a `source_setup_id` beside these, and a field this
/// build does not read is not a reason to refuse a file it otherwise
/// understands.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Predecessor {
    /// The schema of the stamp.
    pub schema_version: u32,
    /// The estate module that wrote it.
    pub product_name: String,
    /// The build that wrote it.
    pub build_version: String,
    /// The setup it says is applied.
    pub setup_id: String,
    /// The directory it says it describes.
    pub canonical_target: String,
    /// Every file it claims, by target-relative path, with its `sha256` hex.
    pub managed_files: BTreeMap<String, String>,
}

impl Predecessor {
    /// The directory this stamp was written for, when that is not this one.
    ///
    /// Reported rather than refused. It was a refusal first, and a disposable
    /// copy of a real estate-managed home — which is how this is tested, and
    /// how someone moving a machine would meet it — could not be adopted at
    /// all. The stamp's `canonical_target` is provenance, not authority over
    /// what is on disk: every path it claims is relative, and every one of them
    /// is checked against *this* target before anything is recorded. A stamp
    /// carried somewhere unrelated simply accounts as missing.
    pub(crate) fn written_elsewhere(&self, target: &Target) -> Option<&str> {
        let here = target.root().to_string_lossy();
        (self.canonical_target != here).then_some(self.canonical_target.as_str())
    }
}

/// What one claimed file turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    /// Present, and the bytes are the ones the stamp recorded.
    Intact,
    /// Present, and the bytes are not.
    Changed,
    /// Named by the stamp and not on disk.
    Missing,
}

impl Claim {
    /// The word a report uses.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Intact => "intact",
            Self::Changed => "changed",
            Self::Missing => "missing",
        }
    }
}

/// Read the predecessor's stamp, when this harness had one and it is there.
///
/// # Errors
///
/// Refuses a stamp that is not readable, not JSON, in a schema this build does
/// not understand, or that describes a different directory.
pub(crate) fn read(harness: &Harness, target: &Target) -> Result<Option<(PathBuf, Predecessor)>> {
    if harness.predecessor_state_file.is_empty() {
        return Ok(None);
    }
    let path = target.root().join(harness.predecessor_state_file);
    if !path.is_file() {
        return Ok(None);
    }

    let bytes = std::fs::read(&path).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} cannot be read: {error}", path.display()),
        )
    })?;
    let found: Predecessor = serde_json::from_slice(&bytes).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is not a stamp this build understands: {error}",
                path.display()
            ),
        )
    })?;

    if found.schema_version != PREDECESSOR_SCHEMA {
        return Err(Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} is schema {}, and this build reads {PREDECESSOR_SCHEMA}; adopting a record it \
                 cannot read would be claiming files it did not check",
                path.display(),
                found.schema_version
            ),
        ));
    }

    Ok(Some((path, found)))
}

impl Predecessor {
    /// Check every file the stamp claims against what is on disk.
    ///
    /// # Errors
    ///
    /// Propagates a digest failure.
    pub(crate) fn account_for(&self, target: &Target) -> Result<Vec<(String, Claim)>> {
        let mut found = Vec::with_capacity(self.managed_files.len());
        for (relative, expected) in &self.managed_files {
            let path = target.root().join(relative);
            let claim = if path.is_file() {
                let measured = digest::of_file(&path)?;
                if measured == format!("{}{expected}", digest::PREFIX) {
                    Claim::Intact
                } else {
                    Claim::Changed
                }
            } else {
                Claim::Missing
            };
            found.push((relative.clone(), claim));
        }
        Ok(found)
    }

    /// Every claimed path that falls outside what this provider owns.
    ///
    /// A stamp naming a file this build does not claim is a real conflict:
    /// adopting it would record ownership of something no later operation of
    /// this provider would ever write, restore or remove.
    pub(crate) fn outside(&self, harness: &Harness) -> Vec<&str> {
        self.managed_files
            .keys()
            .map(String::as_str)
            .filter(|relative| !harness.owns(relative))
            .collect()
    }
}

/// Move the adopted stamp out of the product's surface, keeping it.
///
/// # Errors
///
/// Fails if the control directory cannot be written.
pub(crate) fn keep_aside(control: &Path, stamp: &Path, name: &str) -> Result<PathBuf> {
    let kept = control.join(KEPT_IN);
    std::fs::create_dir_all(&kept).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!("{} cannot be created: {error}", kept.display()),
        )
    })?;
    let to = kept.join(name);
    std::fs::rename(stamp, &to).map_err(|error| {
        Error::refuse(
            WireReason::ProviderUnavailable,
            format!(
                "{} could not be moved to {}: {error}",
                stamp.display(),
                to.display()
            ),
        )
    })?;
    Ok(to)
}
