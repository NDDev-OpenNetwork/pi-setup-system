//! The exact argv contract, parsed once and refused closed.
//!
//! The shape comes from the consumer, not from this crate:
//!
//! ```text
//! <executable> provider-info
//! <executable> <command> --target <absolute-resolved-dir> --json [command arguments]
//! ```
//!
//! `provider-info` receives neither `--target` nor `--json`. Every other command
//! receives both, immediately after the command name.
//!
//! # What each command carries
//!
//! | Command | Arguments beyond `--target --json` |
//! | --- | --- |
//! | `validate-bundle` | the five bundle flags |
//! | `plan-operation` | operation, release digest, operation id, expiry; optional backup ref, permission profile, bundle |
//! | `apply-operation` | plan path, plan digest, release digest; optional bundle |
//! | `recover-operation` | none — it reads the journal to know what it is resolving |
//! | `status` | none |
//!
//! `--expected-target-digest` is a v1/v2 flag and is **not** part of v3.
//! In v3 the provider observes the target itself and reports the digest it saw;
//! the consumer compares that against its own observation. Accepting the flag
//! here would invite a caller to assert a snapshot the provider never took.
//!
//! # Why order is not enforced
//!
//! The consumer emits these flags in a fixed order, and this parser accepts any
//! order. Refusing a well-formed call because two flags were swapped would break
//! a caller that did nothing wrong, while accepting a wider set than the consumer
//! emits costs nothing — the required set is still checked exactly, and an
//! unknown flag is still a refusal.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::error::{Error, Result};
use crate::plan::BundleBinding;
use crate::reason::WireReason;
use crate::vocabulary::{Command, Operation};

/// The five flags that bind one bundle.
const BUNDLE_FLAGS: &[&str] = &[
    "--bundle",
    "--bundle-format",
    "--bundle-digest",
    "--artifact-digest",
    "--bundle-size",
];

/// One parsed invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Report capabilities. Carries no target.
    ProviderInfo,
    /// Check a bundle without touching the target.
    ValidateBundle {
        /// The target the caller named.
        target: PathBuf,
        /// The bundle to check.
        bundle: Bundle,
    },
    /// Produce a plan. Pure.
    PlanOperation {
        /// The target the caller named.
        target: PathBuf,
        /// Everything the plan is bound to.
        request: PlanRequest,
    },
    /// Apply an exact plan.
    ApplyOperation {
        /// The target the caller named.
        target: PathBuf,
        /// The approved plan artifact on disk.
        plan_path: PathBuf,
        /// The digest that plan must have.
        plan_digest: String,
        /// The release digest the consumer verified.
        provider_release_digest: String,
        /// The bundle, when the operation carries one.
        bundle: Option<Bundle>,
        /// The program directory, when the operation installs software.
        prefix: Option<PathBuf>,
        /// The downloaded files, one per artifact the plan named, in its order.
        ///
        /// The contract gives software a download phase between planning and
        /// applying and gives the provider no command to run it in -- there is
        /// no `download` among the seven. So the consumer fetches what the plan
        /// named and hands the files back here, which is why this provider never
        /// opens a socket in any phase. The order is how each file is matched to
        /// its entry, so nothing about which is which has to be inferred.
        software_artifacts: Vec<PathBuf>,
    },
    /// Resolve an interrupted operation from its journal.
    RecoverOperation {
        /// The target the caller named.
        target: PathBuf,
    },
    /// Report the target's state without changing it.
    Status {
        /// The target the caller named.
        target: PathBuf,
    },
    /// Start the product. Optional command.
    Launch {
        /// The target the caller named.
        ///
        /// Becomes the product's configuration home, through the environment
        /// variable the product documents for it. A product that documents none
        /// cannot honour a target, which is why this command is not declared
        /// there.
        target: PathBuf,
        /// The program directory holding what a software install placed.
        prefix: Option<PathBuf>,
        /// Everything after a bare `--`, handed to the product verbatim.
        arguments: Vec<String>,
    },
}

/// A bundle as the argv names it: identity plus where the bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    /// Where the literal artifact sits on this machine.
    ///
    /// Not part of identity, and never recorded in a plan.
    pub path: PathBuf,
    /// The identity two parties agree on.
    pub binding: BundleBinding,
}

/// Everything `plan-operation` binds a plan to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRequest {
    /// The operation to plan.
    pub operation: Operation,
    /// The release digest the consumer verified before invoking.
    pub provider_release_digest: String,
    /// The stable operation identifier the consumer minted.
    pub operation_id: String,
    /// When the plan stops being applicable.
    pub expires_at: String,
    /// The backup this operation reads or writes.
    pub backup_ref: Option<String>,
    /// The permission profile to apply.
    pub permission_profile: Option<String>,
    /// The bundle, when the operation carries one.
    pub bundle: Option<Bundle>,
    /// The program directory, when the operation installs software.
    ///
    /// Not the target. The configuration a provider owns and the program it
    /// installs are different paths with different lifetimes, and conflating
    /// them would tie a program to one of the several targets it can serve.
    pub prefix: Option<PathBuf>,
    /// The exact version to install.
    ///
    /// Omitted means the version this build pins. Given means exactly that one,
    /// and anything else is refused rather than quietly installing a neighbour.
    pub software_version: Option<String>,
}

impl Invocation {
    /// The target this invocation names, when it names one.
    #[must_use]
    pub fn target(&self) -> Option<&PathBuf> {
        match self {
            Self::ProviderInfo => None,
            Self::ValidateBundle { target, .. }
            | Self::PlanOperation { target, .. }
            | Self::ApplyOperation { target, .. }
            | Self::RecoverOperation { target }
            | Self::Status { target }
            | Self::Launch { target, .. } => Some(target),
        }
    }

    /// The command this invocation is.
    #[must_use]
    pub const fn command(&self) -> Command {
        match self {
            Self::ProviderInfo => Command::ProviderInfo,
            Self::ValidateBundle { .. } => Command::ValidateBundle,
            Self::PlanOperation { .. } => Command::PlanOperation,
            Self::ApplyOperation { .. } => Command::ApplyOperation,
            Self::RecoverOperation { .. } => Command::RecoverOperation,
            Self::Status { .. } => Command::Status,
            Self::Launch { .. } => Command::Launch,
        }
    }
}

/// Parse one invocation from the arguments after the executable name.
///
/// # Errors
///
/// Refuses an unknown command, a missing or repeated flag, an unknown flag, a
/// bundle size that is not a decimal number, and an operation outside the closed
/// set. Each refusal names a reason; none of them guesses.
pub fn parse<I, S>(arguments: I) -> Result<Invocation>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let tokens: Vec<String> = arguments.into_iter().map(Into::into).collect();
    let Some(name) = tokens.first() else {
        return Err(local("no command was given"));
    };
    let Some(command) = Command::parse(name) else {
        return Err(local(format!(
            "{name:?} is not a provider protocol v3 command"
        )));
    };

    let rest = tokens.get(1..).unwrap_or_default();
    if !command.takes_target() {
        if !rest.is_empty() {
            return Err(local(format!("{command} takes no arguments")));
        }
        return Ok(Invocation::ProviderInfo);
    }

    // Everything after a bare `--` belongs to the product `launch` starts, so
    // it is taken off before this parser sees it. No other command has anything
    // to pass on, and one that finds a `--` gets an empty tail and refuses the
    // leftovers the same way it always would.
    let (mine, passthrough) = Flags::split_passthrough(rest);
    let mut flags = Flags::parse(&mine)?;
    let target = PathBuf::from(flags.take_required("--target")?);
    if !flags.take_switch("--json") {
        return Err(local(format!("{command} requires --json")));
    }

    let invocation = match command {
        Command::ProviderInfo => return Err(local("provider-info never reaches this branch")),
        Command::Status => Invocation::Status { target },
        Command::RecoverOperation => Invocation::RecoverOperation { target },
        Command::Launch => Invocation::Launch {
            target,
            prefix: flags.take_prefix()?,
            arguments: passthrough,
        },
        Command::ValidateBundle => {
            let Some(bundle) = flags.take_bundle()? else {
                return Err(local("validate-bundle requires a bundle"));
            };
            Invocation::ValidateBundle { target, bundle }
        }
        Command::PlanOperation => {
            let operation_name = flags.take_required("--operation")?;
            let Some(operation) = Operation::parse(&operation_name) else {
                return Err(Error::refuse(
                    WireReason::UnsupportedOperation,
                    format!("{operation_name:?} is not an operation this protocol defines"),
                ));
            };
            Invocation::PlanOperation {
                target,
                request: PlanRequest {
                    operation,
                    provider_release_digest: flags.take_required("--provider-release-digest")?,
                    operation_id: flags.take_required("--operation-id")?,
                    expires_at: flags.take_required("--expires-at")?,
                    backup_ref: flags.take_optional("--backup-ref"),
                    permission_profile: flags.take_optional("--permission-profile"),
                    bundle: flags.take_bundle()?,
                    prefix: flags.take_prefix()?,
                    software_version: flags.take_optional("--software-version"),
                },
            }
        }
        Command::ApplyOperation => Invocation::ApplyOperation {
            target,
            plan_path: PathBuf::from(flags.take_required("--plan")?),
            plan_digest: flags.take_required("--plan-digest")?,
            provider_release_digest: flags.take_required("--provider-release-digest")?,
            bundle: flags.take_bundle()?,
            prefix: flags.take_prefix()?,
            software_artifacts: flags
                .take_repeated("--software-artifact")
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        },
    };

    flags.require_exhausted()?;
    Ok(invocation)
}

fn local(detail: impl Into<String>) -> Error {
    Error::refuse(WireReason::ProviderUnavailable, detail)
}

/// Name-to-value pairs, consumed as each command claims what it needs.
///
/// Anything left over at the end is an unknown flag, which is a refusal rather
/// than something to ignore. A provider that silently dropped an argument it did
/// not understand would report success for a request it only partly performed.
struct Flags {
    values: BTreeMap<String, Vec<String>>,
    switches: Vec<String>,
}

/// The flags a caller may give more than once.
///
/// Exactly one: `apply-operation` receives one downloaded file per artifact the
/// plan named, in the plan's order. Every other flag is still refused twice
/// over, because a second value where one is expected is a caller that meant
/// two different things and only one of them would happen.
const REPEATABLE: &[&str] = &["--software-artifact"];

impl Flags {
    /// Split a bare `--` off the end, keeping what follows verbatim.
    ///
    /// Only `launch` has anything to pass on, and what it passes belongs to
    /// another program: `-p`, `--help` and `--version` all mean something to the
    /// product and nothing here. A separator is the one way to say "stop
    /// reading these as mine" without guessing which of them are.
    fn split_passthrough(tokens: &[String]) -> (Vec<String>, Vec<String>) {
        match tokens.iter().position(|token| token == "--") {
            Some(at) => (tokens[..at].to_vec(), tokens[at + 1..].to_vec()),
            None => (tokens.to_vec(), Vec::new()),
        }
    }

    fn parse(tokens: &[String]) -> Result<Self> {
        let mut values: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut switches = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            let Some(token) = tokens.get(index) else {
                break;
            };
            if !token.starts_with("--") {
                return Err(local(format!("{token:?} is not a flag")));
            }
            if token == "--json" {
                if switches.iter().any(|switch| switch == token) {
                    return Err(local("--json was given twice"));
                }
                switches.push(token.clone());
                index += 1;
                continue;
            }
            let Some(value) = tokens.get(index + 1) else {
                return Err(local(format!("{token} has no value")));
            };
            if value.starts_with("--") {
                return Err(local(format!("{token} has no value")));
            }
            let seen = values.entry(token.clone()).or_default();
            if !seen.is_empty() && !REPEATABLE.contains(&token.as_str()) {
                return Err(local(format!("{token} was given twice")));
            }
            seen.push(value.clone());
            index += 2;
        }
        Ok(Self { values, switches })
    }

    fn take_required(&mut self, name: &str) -> Result<String> {
        self.take_optional(name)
            .ok_or_else(|| local(format!("{name} is required")))
    }

    fn take_optional(&mut self, name: &str) -> Option<String> {
        self.values.remove(name)?.into_iter().next()
    }

    /// Every value of a flag a caller may repeat, in the order they were given.
    ///
    /// The order is load-bearing: it is how `apply` knows which file answers
    /// which entry of the plan's `software_artifacts` array.
    fn take_repeated(&mut self, name: &str) -> Vec<String> {
        self.values.remove(name).unwrap_or_default()
    }

    fn take_switch(&mut self, name: &str) -> bool {
        if let Some(position) = self.switches.iter().position(|switch| switch == name) {
            self.switches.remove(position);
            return true;
        }
        false
    }

    /// Take all five bundle flags, or none of them.
    ///
    /// A partial set is refused rather than filled in. Four of the five describe
    /// an identity; guessing the fifth would let a caller bind bytes it never
    /// named.
    fn take_bundle(&mut self) -> Result<Option<Bundle>> {
        let present = BUNDLE_FLAGS
            .iter()
            .filter(|flag| self.values.contains_key(**flag))
            .count();
        if present == 0 {
            return Ok(None);
        }
        if present != BUNDLE_FLAGS.len() {
            return Err(local(
                "a bundle is named by all five of --bundle, --bundle-format, \
                 --bundle-digest, --artifact-digest and --bundle-size",
            ));
        }
        let path = PathBuf::from(self.take_required("--bundle")?);
        let bundle_format = self.take_required("--bundle-format")?;
        let bundle_digest = self.take_required("--bundle-digest")?;
        let artifact_digest = self.take_required("--artifact-digest")?;
        let raw_size = self.take_required("--bundle-size")?;
        let bundle_size: u64 = raw_size.parse().map_err(|_| {
            local(format!(
                "--bundle-size {raw_size:?} is not a decimal byte count"
            ))
        })?;

        Ok(Some(Bundle {
            path,
            binding: BundleBinding {
                bundle_format,
                bundle_digest,
                artifact_digest,
                bundle_size,
            },
        }))
    }

    /// The program directory, checked to be absolute.
    ///
    /// The contract says both `--target` and `--prefix` are absolute. A relative
    /// one would resolve against whatever directory the caller happened to be
    /// in, which is not a property a plan can be bound to.
    fn take_prefix(&mut self) -> Result<Option<PathBuf>> {
        let Some(text) = self.take_optional("--prefix") else {
            return Ok(None);
        };
        let path = PathBuf::from(&text);
        if !path.is_absolute() {
            return Err(local(format!("--prefix {text:?} is not an absolute path")));
        }
        Ok(Some(path))
    }

    fn require_exhausted(&self) -> Result<()> {
        if let Some((name, _)) = self.values.iter().next() {
            return Err(local(format!("{name} is not an argument of this command")));
        }
        if let Some(switch) = self.switches.first() {
            return Err(local(format!(
                "{switch} is not an argument of this command"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    const DIGEST: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    fn bundle_flags() -> Vec<String> {
        [
            "--bundle",
            "/tmp/bundle.zip",
            "--bundle-format",
            "ai-stp-bundle/1",
            "--bundle-digest",
            DIGEST,
            "--artifact-digest",
            DIGEST,
            "--bundle-size",
            "4096",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }

    fn with_target(command: &str, extra: &[&str]) -> Vec<String> {
        let mut tokens = vec![
            command.to_owned(),
            "--target".to_owned(),
            "/tmp/target".to_owned(),
            "--json".to_owned(),
        ];
        tokens.extend(extra.iter().map(|s| (*s).to_owned()));
        tokens
    }

    #[test]
    fn provider_info_takes_neither_target_nor_json() {
        assert_eq!(parse(["provider-info"]).unwrap(), Invocation::ProviderInfo);
        assert!(parse(["provider-info", "--target", "/tmp/x"]).is_err());
        assert!(parse(["provider-info", "--json"]).is_err());
    }

    #[test]
    fn every_other_command_requires_both() {
        for command in Command::ALL.iter().filter(|c| c.takes_target()) {
            let name = command.as_str();
            assert!(parse([name]).is_err(), "{name} accepted no target");
            assert!(
                parse([name, "--target", "/tmp/target"]).is_err(),
                "{name} accepted no --json"
            );
        }
    }

    #[test]
    fn status_and_recover_carry_nothing_else() {
        assert_eq!(
            parse(with_target("status", &[])).unwrap(),
            Invocation::Status {
                target: PathBuf::from("/tmp/target")
            }
        );
        assert_eq!(
            parse(with_target("recover-operation", &[])).unwrap(),
            Invocation::RecoverOperation {
                target: PathBuf::from("/tmp/target")
            }
        );
    }

    #[test]
    fn a_full_plan_request_parses_every_field() {
        let mut tokens = with_target(
            "plan-operation",
            &[
                "--operation",
                "install",
                "--provider-release-digest",
                DIGEST,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                "2026-08-23T15:00:00Z",
                "--backup-ref",
                "slot-000000000001",
                "--permission-profile",
                "default",
            ],
        );
        tokens.extend(bundle_flags());

        let Invocation::PlanOperation { target, request } = parse(tokens).unwrap() else {
            panic!("expected a plan invocation");
        };
        assert_eq!(target, PathBuf::from("/tmp/target"));
        assert_eq!(request.operation, Operation::Install);
        assert_eq!(request.operation_id, "operation_01TEST");
        assert_eq!(request.backup_ref.as_deref(), Some("slot-000000000001"));
        assert_eq!(request.permission_profile.as_deref(), Some("default"));
        let bundle = request.bundle.unwrap();
        assert_eq!(bundle.path, PathBuf::from("/tmp/bundle.zip"));
        assert_eq!(bundle.binding.bundle_size, 4096);
    }

    #[test]
    fn a_plan_without_the_optional_parts_still_parses() {
        let tokens = with_target(
            "plan-operation",
            &[
                "--operation",
                "backup",
                "--provider-release-digest",
                DIGEST,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                "2026-08-23T15:00:00Z",
            ],
        );
        let Invocation::PlanOperation { request, .. } = parse(tokens).unwrap() else {
            panic!("expected a plan invocation");
        };
        assert_eq!(request.operation, Operation::Backup);
        assert!(request.bundle.is_none());
        assert!(request.backup_ref.is_none());
    }

    #[test]
    fn an_operation_outside_the_closed_set_is_refused_by_its_contract_reason() {
        let tokens = with_target(
            "plan-operation",
            &[
                "--operation",
                "reformat",
                "--provider-release-digest",
                DIGEST,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                "2026-08-23T15:00:00Z",
            ],
        );
        let error = parse(tokens).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::UnsupportedOperation));
    }

    #[test]
    fn apply_binds_the_plan_path_and_its_digest() {
        let mut tokens = with_target(
            "apply-operation",
            &[
                "--plan",
                "/tmp/plan.json",
                "--plan-digest",
                DIGEST,
                "--provider-release-digest",
                DIGEST,
            ],
        );
        tokens.extend(bundle_flags());
        let Invocation::ApplyOperation {
            plan_path,
            plan_digest,
            bundle,
            ..
        } = parse(tokens).unwrap()
        else {
            panic!("expected an apply invocation");
        };
        assert_eq!(plan_path, PathBuf::from("/tmp/plan.json"));
        assert_eq!(plan_digest, DIGEST);
        assert!(bundle.is_some());
    }

    #[test]
    fn a_partial_bundle_is_refused_rather_than_completed() {
        let mut partial = bundle_flags();
        partial.truncate(partial.len() - 2); // drop --bundle-size and its value
        let tokens = [with_target("validate-bundle", &[]), partial].concat();
        let error = parse(tokens).unwrap_err();
        assert!(error.detail().contains("all five"));
    }

    #[test]
    fn a_bundle_size_that_is_not_a_number_is_refused() {
        let mut flags = bundle_flags();
        let last = flags.len() - 1;
        flags[last] = "four thousand".to_owned();
        let tokens = [with_target("validate-bundle", &[]), flags].concat();
        assert!(
            parse(tokens)
                .unwrap_err()
                .detail()
                .contains("decimal byte count")
        );
    }

    #[test]
    fn an_unknown_flag_is_refused_rather_than_ignored() {
        // Dropping an argument silently would report success for a request the
        // provider only partly performed.
        let tokens = with_target("status", &["--dry-run", "true"]);
        assert!(parse(tokens).unwrap_err().detail().contains("--dry-run"));
    }

    #[test]
    fn the_v1_expected_target_digest_flag_is_not_a_v3_argument() {
        let tokens = with_target(
            "plan-operation",
            &[
                "--operation",
                "install",
                "--provider-release-digest",
                DIGEST,
                "--operation-id",
                "operation_01TEST",
                "--expires-at",
                "2026-08-23T15:00:00Z",
                "--expected-target-digest",
                DIGEST,
            ],
        );
        assert!(
            parse(tokens)
                .unwrap_err()
                .detail()
                .contains("--expected-target-digest")
        );
    }

    #[test]
    fn a_repeated_flag_is_refused_rather_than_last_one_winning() {
        let tokens = [
            "status", "--target", "/tmp/a", "--target", "/tmp/b", "--json",
        ];
        assert!(parse(tokens).unwrap_err().detail().contains("twice"));
    }

    #[test]
    fn a_flag_with_no_value_is_refused() {
        assert!(
            parse(["status", "--target", "--json"])
                .unwrap_err()
                .detail()
                .contains("no value")
        );
        assert!(
            parse(["status", "--target"])
                .unwrap_err()
                .detail()
                .contains("no value")
        );
    }

    #[test]
    fn order_within_the_accepted_set_does_not_change_the_result() {
        let ordered = parse(with_target("status", &[])).unwrap();
        let swapped = parse(["status", "--json", "--target", "/tmp/target"]).unwrap();
        assert_eq!(ordered, swapped);
    }

    #[test]
    fn an_unknown_command_is_refused_and_never_guessed_at() {
        assert!(
            parse(["plan"])
                .unwrap_err()
                .detail()
                .contains("not a provider protocol v3")
        );
        assert!(
            parse(Vec::<String>::new())
                .unwrap_err()
                .detail()
                .contains("no command")
        );
    }

    #[test]
    fn a_parsed_invocation_reports_the_command_and_target_it_carries() {
        let invocation = parse(with_target("status", &[])).unwrap();
        assert_eq!(invocation.command(), Command::Status);
        assert_eq!(invocation.target(), Some(&PathBuf::from("/tmp/target")));
        assert_eq!(parse(["provider-info"]).unwrap().target(), None);
    }
}
