//! The Pi Coding Agent setup system.
//!
//! This file is the harness's *facts*. Every command over them lives in
//! [`harness_runtime`], shared with every other setup system, so a change to
//! behaviour lands once and a change to Pi Coding Agent's surface lands here.
//!
//! The owner assigned this harness the program lifecycle as well. It is not
//! declared yet, for the same reason as Grok: this runtime does not install the
//! product.

use std::process::ExitCode;

use harness_runtime::Harness;
use provider_v3::{ComponentKind, ProjectionKind};

/// Everything specific to Pi Coding Agent, verified against `pi-baseline.json`.
pub const PI: Harness = Harness {
    harness_id: "pi",
    provider_id: "pi-setup-system",
    version: env!("CARGO_PKG_VERSION"),
    product: "Pi Coding Agent",
    vendor: "Earendil Works",
    documented_config_home: "~/.pi/agent",
    config_home_env: "PI_CODING_AGENT_DIR",
    control_directory: ".pi-setup-system",
    state_file: "NDDEV-PI-PROVIDER.json",
    profile_id: "pi/native-files/1",
    // Everything outside this list is a sibling overlay preserved verbatim.
    // `extensions` is where Pi's plugins live. It was missing here while the
    // consumer's own two descriptions of that route disagreed -- the composition
    // rule said `packages`, the catalog layout said `extensions` -- and claiming
    // a route while the product's own sources contradicted each other would have
    // been guessing at someone else's directory. Settled on their side by the
    // layout 1.1 correction, and the canonical compiler now answers `extensions`
    // for `plugin`, so it is declared.
    native_namespaces: &["AGENTS.md", "settings.json", "skills", "extensions"],
    // The product's own: credentials, session history and runtime caches. Never
    // read, never written, and never copied into a backup slot.
    never_touch: &["trust.json", "sessions"],
    permission_profiles: &["default"],
    // Exactly what the canonical compiler routes for Pi. It answers `None` for
    // mcp, hook, command and agent, so declaring any of those would promise a
    // destination the product does not have.
    component_kinds: &[
        ComponentKind::Instruction,
        ComponentKind::Skill,
        ComponentKind::Setting,
        ComponentKind::Plugin,
    ],
    projection_kinds: &[ProjectionKind::NativeFiles, ProjectionKind::Package],
    max_files: 8192,
    max_bytes: 64 * 1024 * 1024,
    kit_identity: include_str!("../../../provider-kit/v3/KIT-IDENTITY.json"),
};

fn main() -> ExitCode {
    harness_runtime::run(&PI, std::env::args().skip(1).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_declaration_is_valid_and_names_this_host() {
        let info = PI.provider_info().unwrap();
        assert_eq!(info.provider_id, env!("CARGO_PKG_NAME"));
        assert_eq!(info.harness_id, "pi");
        assert_eq!(info.protocol_version, 3);
        assert!(info.supports_this_host());
    }

    #[test]
    fn no_namespace_is_both_owned_and_disclaimed() {
        for name in PI.never_touch {
            assert!(
                !PI.native_namespaces.contains(name),
                "{name} is claimed and disclaimed"
            );
        }
    }

    #[test]
    fn the_baseline_this_harness_cites_is_present_and_readable() {
        // The facts above are transcribed from it; a build whose baseline is
        // missing has no evidence for what it claims to own.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references/pi-baseline.json");
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(value.is_object());
    }

    #[test]
    fn the_control_directory_and_state_file_are_provider_owned_not_product_owned() {
        assert!(PI.control_directory.contains("setup-system"));
        assert!(PI.state_file.starts_with("NDDEV-"));
        assert!(!PI.native_namespaces.contains(&PI.state_file));
    }
}
