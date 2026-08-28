//! The closed sets the wire boundary is allowed to name.
//!
//! These enums exist because a program must name its variants to match on them.
//! The authority is still `provider-kit/v3/manifest.json`: [`tests`] binds every
//! set here to the vendored manifest, so adding a command in the kit and
//! forgetting it here is a test failure rather than a silent divergence.
//!
//! Nothing else in this crate hard-codes a member of these sets.

use std::fmt;

/// The wire protocol version this crate implements.
pub const PROTOCOL_VERSION: u32 = 3;

/// The digest domain for a provider plan artifact.
pub const PLAN_DOMAIN: &str = "ai-stp:provider-plan:v3";

/// The digest domain for a projection profile declaration.
pub const PROJECTION_DOMAIN: &str = "ai-stp:provider-projection:v3";

/// The plan artifact format tag.
pub const PLAN_FORMAT: &str = "ai-stp-provider-plan/3";

/// A command the consumer may invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Command {
    /// Report capabilities. The only command that takes no target.
    ProviderInfo,
    /// Check a bundle without touching the target.
    ValidateBundle,
    /// Produce a plan. Always pure.
    PlanOperation,
    /// Apply an exact plan under the target lock.
    ApplyOperation,
    /// Resolve an interrupted operation from its journal.
    RecoverOperation,
    /// Report the target's current state. Never migrates it.
    Status,
    /// Start the product through its native boundary. Optional.
    Launch,
}

impl Command {
    /// Every command, core first.
    pub const ALL: &'static [Self] = &[
        Self::ProviderInfo,
        Self::ValidateBundle,
        Self::PlanOperation,
        Self::ApplyOperation,
        Self::RecoverOperation,
        Self::Status,
        Self::Launch,
    ];

    /// The commands every provider must implement.
    pub const CORE: &'static [Self] = &[
        Self::ProviderInfo,
        Self::ValidateBundle,
        Self::PlanOperation,
        Self::ApplyOperation,
        Self::RecoverOperation,
        Self::Status,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderInfo => "provider-info",
            Self::ValidateBundle => "validate-bundle",
            Self::PlanOperation => "plan-operation",
            Self::ApplyOperation => "apply-operation",
            Self::RecoverOperation => "recover-operation",
            Self::Status => "status",
            Self::Launch => "launch",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|command| command.as_str() == text)
    }

    /// Whether this command may change the target.
    ///
    /// Only apply and recover may. Everything else is a read, and the
    /// conformance kit runs the read commands against a caller's real target on
    /// that promise.
    #[must_use]
    pub const fn mutates(self) -> bool {
        matches!(self, Self::ApplyOperation | Self::RecoverOperation)
    }

    /// Whether the consumer passes `--target` and `--json` for this command.
    ///
    /// `provider-info` describes the provider, not a target, so it receives
    /// neither. Reporting capabilities that depended on a target would make the
    /// answer unusable for choosing one.
    #[must_use]
    pub const fn takes_target(self) -> bool {
        !matches!(self, Self::ProviderInfo)
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An operation a plan may describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    /// Materialize a setup into an empty-of-ours target.
    Install,
    /// Swap the applied setup for another.
    Replace,
    /// Capture the target without changing it.
    Backup,
    /// Return the target to a captured state.
    Restore,
    /// Withdraw everything this provider owns.
    Remove,
    /// Install the product itself. Optional.
    SoftwareInstall,
    /// Update the product itself. Optional.
    SoftwareUpdate,
    /// Uninstall the product itself. Optional.
    SoftwareRemove,
    /// Start the product. Optional.
    Launch,
}

impl Operation {
    /// Every operation, core first.
    pub const ALL: &'static [Self] = &[
        Self::Install,
        Self::Replace,
        Self::Backup,
        Self::Restore,
        Self::Remove,
        Self::SoftwareInstall,
        Self::SoftwareUpdate,
        Self::SoftwareRemove,
        Self::Launch,
    ];

    /// The operations every provider must support.
    pub const CORE: &'static [Self] = &[
        Self::Backup,
        Self::Install,
        Self::Remove,
        Self::Replace,
        Self::Restore,
    ];

    /// Core plus the software lifecycle, for a provider that performs both.
    pub const CORE_AND_SOFTWARE: &'static [Self] = &[
        Self::Backup,
        Self::Install,
        Self::Remove,
        Self::Replace,
        Self::Restore,
        Self::SoftwareInstall,
        Self::SoftwareUpdate,
        Self::SoftwareRemove,
    ];

    /// The optional operations that install the product itself.
    ///
    /// Declared together or not at all. A provider offering to install but not
    /// to remove leaves a caller holding something it cannot put down.
    pub const SOFTWARE: &'static [Self] = &[
        Self::SoftwareInstall,
        Self::SoftwareUpdate,
        Self::SoftwareRemove,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Replace => "replace",
            Self::Backup => "backup",
            Self::Restore => "restore",
            Self::Remove => "remove",
            Self::SoftwareInstall => "software_install",
            Self::SoftwareUpdate => "software_update",
            Self::SoftwareRemove => "software_remove",
            Self::Launch => "launch",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|operation| operation.as_str() == text)
    }

    /// Whether a plan for this operation must name the target it restores to.
    #[must_use]
    pub const fn requires_restore_target_digest(self) -> bool {
        matches!(self, Self::Restore)
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A kind of component a setup may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentKind {
    /// An instruction document.
    Instruction,
    /// A skill.
    Skill,
    /// An MCP server entry.
    Mcp,
    /// A hook.
    Hook,
    /// A command.
    Command,
    /// A subagent.
    Agent,
    /// A plugin.
    Plugin,
    /// A settings value.
    Setting,
}

impl ComponentKind {
    /// Every component kind.
    pub const ALL: &'static [Self] = &[
        Self::Instruction,
        Self::Skill,
        Self::Mcp,
        Self::Hook,
        Self::Command,
        Self::Agent,
        Self::Plugin,
        Self::Setting,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Instruction => "instruction",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
            Self::Hook => "hook",
            Self::Command => "command",
            Self::Agent => "agent",
            Self::Plugin => "plugin",
            Self::Setting => "setting",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == text)
    }
}

/// How a component is projected into a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProjectionKind {
    /// Registered as a marketplace the product resolves.
    Marketplace,
    /// Installed as a product plugin.
    Plugin,
    /// Written as files the product reads directly.
    NativeFiles,
    /// Installed as a product package.
    Package,
}

impl ProjectionKind {
    /// Every projection kind.
    pub const ALL: &'static [Self] = &[
        Self::Marketplace,
        Self::Plugin,
        Self::NativeFiles,
        Self::Package,
    ];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Marketplace => "marketplace",
            Self::Plugin => "plugin",
            Self::NativeFiles => "native_files",
            Self::Package => "package",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == text)
    }
}

/// A target a projection profile owns, other than the product's own home.
///
/// The kit's schema enumerates exactly one value today, and the global scope is
/// deliberately not among them: the global profile is declared by
/// `projection_profile` itself, and two statements about one fact are a defect
/// even while they agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TargetScope {
    /// A workspace rather than the product's configuration home.
    Project,
    /// A root that belongs to a convention rather than to a product.
    ///
    /// `~/.agents` is the one this exists for: codex reads user-level skills
    /// from `$HOME/.agents/skills`, which is a sibling of every configuration
    /// home rather than a child of one, so no provider targeting a product's
    /// home can reach it. A profile in this scope is relative to that root --
    /// a skill is `skills/<name>`, not `.agents/skills/<name>`, and getting
    /// that wrong is the same sentence this estate has now met eight times.
    UserRoot,
}

impl TargetScope {
    /// Every scope a profile may name.
    pub const ALL: &'static [Self] = &[Self::Project, Self::UserRoot];

    /// The wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::UserRoot => "user_root",
        }
    }

    /// Parse a wire spelling.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|scope| scope.as_str() == text)
    }
}

impl fmt::Display for TargetScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
pub(crate) mod kit {
    //! The vendored conformance kit, read by tests only.
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::path::PathBuf;

    pub fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../provider-kit/v3")
    }

    pub fn json(name: &str) -> serde_json::Value {
        let bytes = std::fs::read(root().join(name)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    pub fn strings(value: &serde_json::Value, key: &str) -> Vec<String> {
        value[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} is not an array"))
            .iter()
            .map(|item| item.as_str().unwrap().to_owned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use kit::{json, strings};

    /// The scopes this build knows are the scopes the kit's schema enumerates.
    ///
    /// Unlike every other closed set here, this one lives in
    /// `provider-info.schema.json` rather than in `manifest.json` -- it is a
    /// property of a declaration rather than a vocabulary of its own. Bound
    /// anyway, because a scope this build could name and the consumer would
    /// refuse is a `provider-info` that does not parse, and a `provider-info`
    /// that does not parse takes `fetch`, `plan`, `apply` and `status` with it.
    #[test]
    fn the_target_scopes_are_the_schema_s() {
        let schema = json("provider-info.schema.json");
        let published = strings(
            &schema["properties"]["scoped_projection_profiles"]["items"]["properties"]["target_scope"],
            "enum",
        );
        assert_eq!(
            sorted(published),
            sorted(spellings(TargetScope::ALL, TargetScope::as_str)),
        );
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    fn spellings<T: Copy>(all: &[T], as_str: fn(T) -> &'static str) -> Vec<String> {
        all.iter().map(|item| as_str(*item).to_owned()).collect()
    }

    #[test]
    fn the_kit_bytes_match_the_digests_it_publishes() {
        // A vocabulary bound to a file nobody verified is bound to nothing.
        let sums = std::fs::read_to_string(kit::root().join("SHA256SUMS")).unwrap();
        for line in sums.lines().filter(|line| !line.trim().is_empty()) {
            let (expected, name) = line.split_once("  ").unwrap();
            let bytes = std::fs::read(kit::root().join(name)).unwrap();
            let actual = setup_core::digest::of_bytes(&bytes);
            assert_eq!(
                actual,
                format!("sha256:{expected}"),
                "{name} does not match SHA256SUMS"
            );
        }
    }

    #[test]
    fn commands_match_the_manifest() {
        let manifest = json("manifest.json");
        assert_eq!(
            sorted(spellings(Command::ALL, Command::as_str)),
            sorted(strings(&manifest, "commands"))
        );
        assert_eq!(
            sorted(spellings(Command::CORE, Command::as_str)),
            sorted(strings(&manifest, "core_commands"))
        );
        assert_eq!(
            sorted(strings(&manifest, "optional_commands")),
            vec![Command::Launch.as_str().to_owned()]
        );
    }

    #[test]
    fn operations_match_the_manifest() {
        let manifest = json("manifest.json");
        let optional = sorted(strings(&manifest, "optional_operations"));
        let core = sorted(strings(&manifest, "core_operations"));
        assert_eq!(sorted(spellings(Operation::CORE, Operation::as_str)), core);

        let mut declared = core;
        declared.extend(optional);
        assert_eq!(
            sorted(spellings(Operation::ALL, Operation::as_str)),
            sorted(declared)
        );
    }

    #[test]
    fn component_and_projection_kinds_match_the_manifest() {
        let manifest = json("manifest.json");
        assert_eq!(
            sorted(spellings(ComponentKind::ALL, ComponentKind::as_str)),
            sorted(strings(&manifest, "component_kinds"))
        );
        assert_eq!(
            sorted(spellings(ProjectionKind::ALL, ProjectionKind::as_str)),
            sorted(strings(&manifest, "projection_kinds"))
        );
    }

    #[test]
    fn the_manifest_protocol_version_is_the_one_this_crate_implements() {
        assert_eq!(json("manifest.json")["protocol_version"], PROTOCOL_VERSION);
    }

    #[test]
    fn only_apply_and_recover_are_allowed_to_change_a_target() {
        let manifest = json("manifest.json");
        let apply = sorted(strings(&manifest, "apply_commands"));
        let mutating = sorted(
            Command::ALL
                .iter()
                .filter(|command| command.mutates())
                .map(|command| command.as_str().to_owned())
                .collect(),
        );
        assert_eq!(mutating, apply);
    }

    #[test]
    fn every_pure_command_the_kit_names_is_one_this_crate_treats_as_pure() {
        let cases = json("conformance-cases.json");
        for name in strings(&cases, "pure_commands") {
            let command = Command::parse(&name).unwrap_or_else(|| panic!("unknown {name}"));
            assert!(!command.mutates(), "{name} must not mutate");
        }
    }

    #[test]
    fn an_unknown_spelling_parses_to_nothing_rather_than_a_neighbour() {
        assert_eq!(Command::parse("plan"), None);
        assert_eq!(Operation::parse("software-install"), None);
        assert_eq!(ComponentKind::parse("instructions"), None);
        assert_eq!(ProjectionKind::parse("native-files"), None);
    }

    #[test]
    fn only_provider_info_is_invoked_without_a_target() {
        for command in Command::ALL {
            assert_eq!(
                command.takes_target(),
                *command != Command::ProviderInfo,
                "{command} target convention is wrong"
            );
        }
    }
}
