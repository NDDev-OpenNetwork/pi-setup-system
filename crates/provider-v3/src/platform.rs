//! The operating system and architecture names the consumer compares against.
//!
//! The spellings are the consumer's, not Rust's. `ai_stp` normalizes
//! `platform.machine()` with `{"amd64": "x86_64", "aarch64": "arm64"}` and maps
//! `darwin` to `macos`, then compares the result against the plan's `platform`
//! echo. A provider that reported Rust's `aarch64` would produce a plan the
//! consumer rejects as a platform mismatch on the machine that built it.

/// The operating system name in the consumer's vocabulary.
///
/// `std::env::consts::OS` already says `macos` rather than `darwin`, so only the
/// remaining names pass through.
#[must_use]
pub fn os() -> &'static str {
    std::env::consts::OS
}

/// The architecture name in the consumer's vocabulary.
#[must_use]
pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86" => "x86",
        other => other,
    }
}

/// The `platform` object a plan artifact echoes.
#[must_use]
pub fn echo() -> serde_json::Value {
    serde_json::json!({ "os": os(), "arch": arch() })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_echo_has_exactly_the_two_members_the_consumer_compares() {
        let echo = echo();
        let object = echo.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["arch", "os"]);
    }

    #[test]
    fn the_names_are_the_consumers_spellings_not_rusts() {
        // `ai_stp` maps aarch64 to arm64 before comparing. Reporting Rust's
        // spelling would fail the comparison on every Apple Silicon machine.
        assert_ne!(arch(), "aarch64");
        assert_ne!(os(), "darwin");
    }

    #[test]
    fn this_host_reports_one_of_the_three_supported_systems() {
        assert!(
            matches!(os(), "linux" | "macos" | "windows"),
            "unexpected os {}",
            os()
        );
    }
}
