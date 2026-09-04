//! Pi Coding Agent's own program, as measured rather than as described.
//!
//! Generated from the `software_artifacts` block of
//! `references/pi-baseline.json`. Every member path below was read out
//! of the archive it names, not assumed: codex's carries the target triple and
//! so genuinely differs per platform.
//!
//! Where a `previous_software_artifacts` block is present, it is transcribed
//! too. It is not a second choice: the outgoing current pin is stored there on
//! a bump, so the pair is always two consecutive real releases and there is
//! still exactly one value to keep fresh.
//!
//! Do not edit. The test at the bottom re-reads that baseline and compares it
//! field by field, so an edit here fails rather than silently installing bytes
//! nobody measured.

use harness_runtime::{Artifact, Delivery, Previous, Shape, Software};

/// The artifacts pi is published as.
pub(crate) const ARTIFACTS: &[Artifact] = &[
    Artifact {
        platform: "linux/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-linux-arm64.tar.gz",
        bytes: 42_774_976,
        sha256: "sha256:821750e0ac6bf6e10c35b93ddab88a44f2d0ef8411af9ea4e8ffe620f62130df",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-linux-x64.tar.gz",
        bytes: 42_708_859,
        sha256: "sha256:a7e7c65f1dc528d2e17e7d946ad2b61df0e2b0f9952faee77807c2484b464d6e",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-darwin-arm64.tar.gz",
        bytes: 31_183_100,
        sha256: "sha256:b0a1a3ab9708047e31b76a27911e8b445b3e4a38e2f46a08b6635df75f3499c0",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-darwin-x64.tar.gz",
        bytes: 33_687_389,
        sha256: "sha256:611290e032a47f1546bd30e12c14a59a600a24662d5239c0c159ef3c7a0ca3b0",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-windows-arm64.zip",
        bytes: 43_703_991,
        sha256: "sha256:c10fb6f30f188b1eba61608e0d33453456ee9805ff543fdd99bcaf85f2d949df",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.0/pi-windows-x64.zip",
        bytes: 45_158_082,
        sha256: "sha256:526085e0206acb8e8f9997efcd4e3654fb8a47a04318e09e7324ed5abe549586",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// The artifacts 0.84.4 was published as, kept so
/// `software_update` has a version to move from and `rollback` a tree to
/// return to. Measured from bytes when it was the current pin.
pub(crate) const PREVIOUS_ARTIFACTS: &[Artifact] = &[
    Artifact {
        platform: "linux/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-linux-arm64.tar.gz",
        bytes: 42_529_658,
        sha256: "sha256:135580f6b942151646e67b8b866d987d28ce3cff5a497030775ddd29659f943d",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-linux-x64.tar.gz",
        bytes: 42_464_648,
        sha256: "sha256:c2f3c3e6a1850bd87654cc3ca8811013272397c3d042a4e2a64c43ee1b423972",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-darwin-arm64.tar.gz",
        bytes: 30_928_407,
        sha256: "sha256:c68e3ac4d05b4e282aaab2e6c76f161d3e9e68f19a22e38913cbfaadb6c800f0",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-darwin-x64.tar.gz",
        bytes: 33_440_191,
        sha256: "sha256:7a042d6413065421387001a4986190a1a03186c95a695f4dee0bdc76e60de8f7",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-windows-arm64.zip",
        bytes: 43_455_125,
        sha256: "sha256:6b2726efc34a9158ab06bf7b981f7bcccf15de9ea236a3f4ef7a894a78aa386e",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.84.4/pi-windows-x64.zip",
        bytes: 44_907_374,
        sha256: "sha256:03b2318774f18721e959d9f8f3340a9f942e7aa516fb7030d3007a12a40a4a97",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// Pi Coding Agent's program, and where its bytes come from.
pub(crate) const SOFTWARE: Software = Software {
    version: "0.85.0",
    command: "pi",
    delivery: Delivery::Artifacts(ARTIFACTS),
    unsupported: &[],
    previous: Some(Previous {
        version: "0.84.4",
        artifacts: PREVIOUS_ARTIFACTS,
    }),
};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    // Named rather than glob-imported: a product delivered by a package manager
    // has no `Artifact` in scope, and the test is the same text for all seven.
    use harness_runtime::{Delivery, Shape};

    use super::SOFTWARE;

    fn measured() -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../references/pi-baseline.json");
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn every_artifact_compiled_in_is_the_one_the_baseline_measured() {
        let block = &measured()["software_artifacts"];
        assert_eq!(block["version"], SOFTWARE.version);
        assert_eq!(block["command"], SOFTWARE.command);

        let Delivery::Artifacts(compiled) = SOFTWARE.delivery else {
            // A product delivered by a package manager has no artifacts, and
            // the baseline must agree that it has none.
            assert_eq!(block["shape"], "manager");
            assert!(block["platforms"].as_object().unwrap().is_empty());
            return;
        };
        let published = block["platforms"].as_object().unwrap();
        assert_eq!(
            compiled.len(),
            published.len(),
            "the table and the baseline disagree on how many platforms exist"
        );
        for artifact in compiled {
            let entry = &published[artifact.platform];
            assert_eq!(entry["url"], artifact.url, "{}", artifact.platform);
            assert_eq!(entry["bytes"], artifact.bytes, "{}", artifact.platform);
            assert_eq!(entry["sha256"], artifact.sha256, "{}", artifact.platform);
            let member = entry.get("member").and_then(serde_json::Value::as_str);
            assert_eq!(
                member.unwrap_or(""),
                artifact.member,
                "{} names a different member",
                artifact.platform
            );
            assert_eq!(
                artifact.shape == Shape::Raw,
                member.is_none(),
                "{} disagrees about whether the bytes are the program",
                artifact.platform
            );
        }
    }

    /// The second pin is the baseline's, or it is absent in both places.
    ///
    /// Asserted from either side rather than only where it exists: a harness
    /// that has never been bumped must compile in `None`, and a build that
    /// dropped the block while the baseline still carried it would otherwise
    /// pass by having nothing to compare.
    #[test]
    fn the_version_this_build_can_move_between_is_the_one_measured_before_it() {
        let baseline = measured();
        let recorded = baseline.get("previous_software_artifacts");
        let Some(earlier) = SOFTWARE.previous else {
            assert!(
                recorded.is_none(),
                "the baseline records a previous release and this build names none"
            );
            return;
        };
        let block = recorded.unwrap_or_else(|| {
            panic!("this build names a previous release the baseline does not record")
        });
        assert_eq!(block["version"], earlier.version);
        assert_ne!(
            earlier.version, SOFTWARE.version,
            "a second pin equal to the first is one version wearing two names"
        );
        let published = block["platforms"].as_object().unwrap();
        assert_eq!(
            earlier.artifacts.len(),
            published.len(),
            "the previous table and the baseline disagree on how many platforms exist"
        );
        for artifact in earlier.artifacts {
            let entry = &published[artifact.platform];
            assert_eq!(entry["url"], artifact.url, "{}", artifact.platform);
            assert_eq!(entry["bytes"], artifact.bytes, "{}", artifact.platform);
            assert_eq!(entry["sha256"], artifact.sha256, "{}", artifact.platform);
        }
    }

    #[test]
    fn a_platform_the_vendor_does_not_publish_is_listed_rather_than_missing() {
        let block = &measured()["software_artifacts"];
        let unpublished: Vec<&str> = block
            .get("unpublished")
            .and_then(serde_json::Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(unpublished, SOFTWARE.unsupported);
    }

    #[test]
    fn no_release_calls_a_platform_both_published_and_unpublished() {
        let baseline = measured();
        for name in ["software_artifacts", "previous_software_artifacts"] {
            let Some(block) = baseline.get(name) else {
                continue;
            };
            let published = block["platforms"].as_object().unwrap();
            let unpublished = block
                .get("unpublished")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str);
            for platform in unpublished {
                assert!(
                    !published.contains_key(platform),
                    "{name}: {platform} is both published and unpublished"
                );
            }
        }
    }
}
