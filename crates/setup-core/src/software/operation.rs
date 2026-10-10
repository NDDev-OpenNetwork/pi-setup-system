//! Caller-bound software operations and immutable historical outcomes.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{
    Artifact, Software, VerifiedArtifact, Writer, ownership, records, removal, staging, store,
};
use crate::{Result, software_prefix};
use records::{Identity, open_root, present, refuse};

/// The software effect named by an admitted plan.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Install the exact named artifact.
    Install,
    /// Update a previously installed command to the exact named artifact.
    Update,
    /// Remove the exact recorded version.
    Remove,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub id: String,
    pub digest: String,
}

impl Binding {
    pub(super) fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 256
            || self.id.chars().any(char::is_control)
            || !ownership::digest_valid(&self.digest)
        {
            return Err(refuse());
        }
        Ok(())
    }
}

/// An original caller identifier and full canonical plan digest.
///
/// The provider runtime validates the plan's provider, release and resource
/// binding before constructing this value. A digest is not authentication.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub(super) binding: Binding,
    pub(super) command: String,
    pub(super) version: String,
    pub(super) kind: Kind,
}

impl Intent {
    /// Bind one exact software plan to its original caller identifier.
    ///
    /// # Errors
    /// Refuses an unbounded identifier, invalid digest or invalid software name.
    pub fn new(id: &str, digest: &str, software: &Software, kind: Kind) -> Result<Self> {
        let intent = Self {
            binding: Binding {
                id: id.to_owned(),
                digest: digest.to_owned(),
            },
            command: software.command.to_owned(),
            version: software.version.to_owned(),
            kind,
        };
        intent.validate()?;
        Ok(intent)
    }

    fn validate(&self) -> Result<()> {
        self.binding.validate()?;
        if !records::leaf(&self.command) || !records::leaf(&self.version) {
            return Err(refuse());
        }
        Ok(())
    }
}

/// The original semantic result, independent of later filesystem changes.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum Outcome {
    /// The exact planned artifact was installed and exposed.
    Installed {
        /// The archive entry count returned by the original extraction.
        files: usize,
    },
    /// The exact planned version was removed, or was absent at admission.
    Removed {
        /// Whether this operation removed a recorded installation.
        removed: bool,
    },
}

/// The durable state of this exact caller request.
#[derive(Debug, PartialEq, Eq)]
pub enum State {
    /// This exact plan was admitted and still needs completion.
    Pending,
    /// Return this original result without reapplying any effect.
    Completed(Outcome),
}

/// Read a caller receipt without initializing, migrating or recovering metadata.
///
/// # Errors
/// Refuses changed intent, an inconsistent store or a journal needing recovery.
pub fn state(root: &Path, intent: &Intent) -> Result<Option<State>> {
    intent.validate()?;
    let path = root;
    let Some(root) = records::optional_root(root)? else {
        return Ok(None);
    };
    let Some((record, outcome)) = store::operation(&root, &intent.binding)? else {
        return Ok(None);
    };
    if record.intent != *intent || record.root_identity != Identity::of(&root)? {
        return Err(refuse());
    }
    if let Some(allocation) = &record.allocation {
        allocation.location(path)?;
    }
    Ok(Some(outcome.map_or(State::Pending, State::Completed)))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub schema_version: u32,
    pub intent: Intent,
    pub root_identity: Identity,
    pub control: String,
    pub member: String,
    pub bin_was_absent: bool,
    pub initial_digest: String,
    pub outside_digest: String,
    pub allocation: Option<super::allocation::Binding>,
}

impl Record {
    pub(super) fn validate(&self) -> Result<()> {
        self.intent.validate()?;
        if let Some(allocation) = &self.allocation {
            allocation.validate()?;
        }
        if self.schema_version != 1
            || !records::leaf(&self.control)
            || !self.control.starts_with('.')
            || self.control == super::writer::CONTROL
            || !records::member_valid(&self.member)
            || !ownership::digest_valid(&self.initial_digest)
            || !ownership::digest_valid(&self.outside_digest)
        {
            return Err(refuse());
        }
        Ok(())
    }

    pub(super) fn validate_outcome(&self, outcome: &Outcome) -> Result<()> {
        match (self.intent.kind, outcome) {
            (Kind::Install | Kind::Update, Outcome::Installed { files })
                if (1..=65_536).contains(files) =>
            {
                Ok(())
            }
            (Kind::Remove, Outcome::Removed { .. }) => Ok(()),
            _ => Err(refuse()),
        }
    }

    fn outside_paths(&self) -> Vec<String> {
        vec![
            self.intent.version.clone(),
            format!(
                "bin/{}",
                super::exposed_name(&self.intent.command, &self.member)
            ),
            format!("bin/.{}.version", self.intent.command),
            format!("bin/.{}.manifest.json", self.intent.command),
        ]
    }

    fn check_root(&self, writer: &Writer) -> Result<cap_std::fs::Dir> {
        writer.revalidate()?;
        let root = open_root(&writer.root)?;
        if Identity::of(&root)? != self.root_identity {
            return Err(refuse());
        }
        store::check_active(&root, Some(&self.intent.binding))?;
        Ok(root)
    }

    pub(super) fn check_initial(&self, writer: &Writer) -> Result<()> {
        let root = self.check_root(writer)?;
        if store::pending_for(&root, Some(&self.intent.binding))?
            || software_prefix::resume_digest(&root, &self.control, &[], self.bin_was_absent)?
                != self.initial_digest
        {
            return Err(refuse());
        }
        Ok(())
    }
}

impl Writer {
    pub(super) fn recorded_operation(
        &self,
        intent: &Intent,
    ) -> Result<Option<(Record, Option<Outcome>)>> {
        self.revalidate()?;
        intent.validate()?;
        let root = open_root(&self.root)?;
        let found = store::operation(&root, &intent.binding)?;
        if let Some((record, _)) = &found
            && (record.intent != *intent || record.root_identity != Identity::of(&root)?)
        {
            return Err(refuse());
        }
        if let Some((record, _)) = &found
            && let Some(allocation) = &record.allocation
        {
            allocation.location(&self.root)?;
        }
        Ok(found)
    }

    /// Read this request's state after recovering only transactional metadata.
    ///
    /// # Errors
    /// Refuses a reused identifier with different intent or inconsistent state.
    pub fn operation_state(&self, intent: &Intent) -> Result<Option<State>> {
        self.revalidate()?;
        intent.validate()?;
        store::recover(&open_root(&self.root)?)?;
        Ok(self
            .recorded_operation(intent)?
            .map(|(_, outcome)| outcome.map_or(State::Pending, State::Completed)))
    }

    /// Persist original intent before creating any program or launch path.
    ///
    /// The runtime must validate first-admission expiry and target preconditions
    /// before calling this method. The prefix must already exist and be held.
    ///
    /// # Errors
    /// Refuses stale prefix content, another operation, or inconsistent input.
    pub fn admit_operation(
        &self,
        intent: &Intent,
        software: &Software,
        control: &str,
        expected_digest: &str,
    ) -> Result<()> {
        self.admit_at(intent, software, control, expected_digest, None)
    }

    pub(super) fn admit_at(
        &self,
        intent: &Intent,
        software: &Software,
        control: &str,
        expected_digest: &str,
        allocation: Option<super::allocation::Binding>,
    ) -> Result<()> {
        self.revalidate()?;
        intent.validate()?;
        if intent.command != software.command
            || intent.version != software.version
            || !records::leaf(control)
            || !control.starts_with('.')
            || control == super::writer::CONTROL
        {
            return Err(refuse());
        }
        super::require_idle(software, &self.root)?;
        if software_prefix::observe(&self.root, control)?.digest != expected_digest {
            return Err(crate::Error::new(
                crate::ReasonCode::Stale,
                "the software prefix changed before operation admission",
            ));
        }
        let root = open_root(&self.root)?;
        let member = software.member_here();
        let member = if member.is_empty() {
            software.command
        } else {
            member
        };
        let bin_was_absent = present(&root, "bin")?.is_none();
        let mut record = Record {
            schema_version: 1,
            intent: intent.clone(),
            root_identity: Identity::of(&root)?,
            control: control.to_owned(),
            member: member.to_owned(),
            bin_was_absent,
            initial_digest: software_prefix::resume_digest(&root, control, &[], bin_was_absent)?,
            outside_digest: String::new(),
            allocation,
        };
        record.outside_digest = software_prefix::resume_digest(
            &root,
            control,
            &record.outside_paths(),
            bin_was_absent,
        )?;
        if software_prefix::observe(&self.root, control)?.digest != expected_digest {
            return Err(refuse());
        }
        self.revalidate()?;
        store::admit(&root, &record)
    }

    /// Resume only journals bound to this exact request, or return its outcome.
    ///
    /// `None` means the initial prefix is intact and the admitted effect can
    /// start. A partial extraction may be discarded before returning `None`.
    ///
    /// # Errors
    /// Refuses different journals, outside-path drift and foreign replacements.
    pub fn resume_operation(&self, intent: &Intent) -> Result<Option<Outcome>> {
        let (record, outcome) = self.recorded_operation(intent)?.ok_or_else(refuse)?;
        if outcome.is_some() {
            return Ok(outcome);
        }
        let root = record.check_root(self)?;
        store::check_journals(&root, &intent.command)?;
        let installation = staging::Staging::load(&self.root, &intent.command)?;
        let removal = removal::Removal::load(&self.root, &intent.command)?;
        if (installation.is_some() && removal.is_some())
            || store::exists(&root, &store::Key::Switch(intent.command.clone()))?
            || (installation.is_none()
                && store::exists(&root, &store::Key::Preparation(intent.command.clone()))?)
        {
            return Err(refuse());
        }
        let mut paths = record.outside_paths();
        if let Some(installation) = &installation {
            if intent.kind == Kind::Remove
                || installation.binding() != Some(&intent.binding)
                || installation.version() != intent.version
            {
                return Err(refuse());
            }
            paths.extend(installation.recorded_paths()?);
        }
        if let Some(removal) = &removal {
            if intent.kind != Kind::Remove
                || removal.binding() != Some(&intent.binding)
                || removal.version() != intent.version
            {
                return Err(refuse());
            }
            paths.extend(removal.recorded_paths());
        }
        if software_prefix::resume_digest(&root, &record.control, &paths, record.bin_was_absent)?
            != record.outside_digest
        {
            return Err(refuse());
        }
        if let Some(installation) = installation {
            if installation.recover()? {
                installation.expose()?;
                installation.complete()?;
            }
        } else if let Some(removal) = removal {
            removal.complete()?;
        }
        let (_, outcome) = self.recorded_operation(intent)?.ok_or_else(refuse)?;
        if outcome.is_none() {
            record.check_initial(self)?;
        }
        Ok(outcome)
    }

    /// Install one verified input for an admitted request with no pending stage.
    ///
    /// # Errors
    /// Refuses stale state, different intent, changed input or kernel failures.
    pub fn install_operation(
        &self,
        intent: &Intent,
        software: &Software,
        artifact: &Artifact,
        input: VerifiedArtifact,
    ) -> Result<Outcome> {
        if intent.kind == Kind::Remove
            || intent.command != software.command
            || intent.version != software.version
        {
            return Err(refuse());
        }
        let (record, outcome) = self.recorded_operation(intent)?.ok_or_else(refuse)?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        record.check_initial(self)?;
        super::install_for(software, artifact, input, &self.root, Some(&intent.binding))?;
        self.recorded_operation(intent)?
            .and_then(|(_, outcome)| outcome)
            .ok_or_else(refuse)
    }

    /// Remove the exact recorded version for an admitted request.
    ///
    /// # Errors
    /// Refuses different intent, changed owned content or unfinished journals.
    pub fn remove_operation(&self, intent: &Intent, software: &Software) -> Result<Outcome> {
        if intent.kind != Kind::Remove
            || intent.command != software.command
            || intent.version != software.version
        {
            return Err(refuse());
        }
        let (record, outcome) = self.recorded_operation(intent)?.ok_or_else(refuse)?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        record.check_initial(self)?;
        if let Some(removal) =
            removal::Removal::begin_for(&self.root, software, Some(&intent.binding))?
        {
            removal.complete()?;
        } else {
            store::complete(
                &open_root(&self.root)?,
                &intent.binding,
                None,
                &Outcome::Removed { removed: false },
            )?;
        }
        self.recorded_operation(intent)?
            .and_then(|(_, outcome)| outcome)
            .ok_or_else(refuse)
    }
}
