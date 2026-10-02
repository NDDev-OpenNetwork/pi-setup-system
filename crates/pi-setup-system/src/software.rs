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
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-linux-arm64.tar.gz",
        bytes: 42_646_353,
        sha256: "sha256:b60b3fda830a43c1dc3f5edb5fc7b681f22a0a930b48cdac13b97570c6045819",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-linux-x64.tar.gz",
        bytes: 42_549_511,
        sha256: "sha256:8fd5543a52a889d60ad57ccbf6c969e73c75c5240aae18ac40b506947a63dc38",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-darwin-arm64.tar.gz",
        bytes: 31_004_152,
        sha256: "sha256:97291e7d2eb2d7d95ab1f67d26de7902302201bc8786c132bbbc9e53fa8526cc",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-darwin-x64.tar.gz",
        bytes: 33_464_767,
        sha256: "sha256:62fb78fcdbc7c0dbd21044dd44bfb3af20df447285bb55e9debf86ff4838776d",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-windows-arm64.zip",
        bytes: 43_674_914,
        sha256: "sha256:122d3a1825eac4aa17081c769188c1d058febf579a9407091ae829626997ea96",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v1.0.0/pi-windows-x64.zip",
        bytes: 45_041_072,
        sha256: "sha256:f7dbd39814bf6763f01e7f688ad089615de1d0eb55fc8915a49acdc2088f4404",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// The artifacts 0.99.1 was published as, kept so
/// `software_update` has a version to move from and `rollback` a tree to
/// return to. Measured from bytes when it was the current pin.
pub(crate) const PREVIOUS_ARTIFACTS: &[Artifact] = &[
    Artifact {
        platform: "linux/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-linux-arm64.tar.gz",
        bytes: 42_712_503,
        sha256: "sha256:e632a9e55bc86525ffd0f71d09185d630883f64b9cfc858f731d1edc92716954",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-linux-x64.tar.gz",
        bytes: 42_608_802,
        sha256: "sha256:c81b9a367bb2985fa45a2c0d4f12b147acc43655683910a5abf937fe22208425",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-darwin-arm64.tar.gz",
        bytes: 31_069_587,
        sha256: "sha256:4692aba1dcd48219b61edb4ecebc3c33f6199ebe65299228fce5ba69c31a23a6",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-darwin-x64.tar.gz",
        bytes: 33_524_290,
        sha256: "sha256:9ad6fc356f4d08b9d10e8a6f929ac1ba4908dd533544ba989c54c73b92b53e13",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-windows-arm64.zip",
        bytes: 43_737_495,
        sha256: "sha256:a16cfd489dbbf9d6c60033cae3d15b035e7b4a5c1260f833f84c4b37f58706fc",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.99.1/pi-windows-x64.zip",
        bytes: 45_102_770,
        sha256: "sha256:7e5c2971c0be6019a8b89edc5e3f0efb4cbdeb0ac236fd3a3307c1f62057528f",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// Pi Coding Agent's program, and where its bytes come from.
pub(crate) const SOFTWARE: Software = Software {
    version: "1.0.0",
    command: "pi",
    delivery: Delivery::Artifacts(ARTIFACTS),
    unsupported: &[],
    previous: Some(Previous {
        version: "0.99.1",
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
