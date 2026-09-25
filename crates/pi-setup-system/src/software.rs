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
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-linux-arm64.tar.gz",
        bytes: 42_217_308,
        sha256: "sha256:364b4a9f8491450b27a4857d4e3c780dbaf696790821c176a873e860cbbc3b89",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-linux-x64.tar.gz",
        bytes: 42_120_827,
        sha256: "sha256:80d78dd62d50049a006b981d994c61255bcc10e730b0c278d4ea0a755909764c",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-darwin-arm64.tar.gz",
        bytes: 30_563_988,
        sha256: "sha256:4f8d288b78c9768d3a4ac6f61f06cd34394b82ac17d5b42d1e44a437add401b7",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-darwin-x64.tar.gz",
        bytes: 33_033_993,
        sha256: "sha256:01d8ee28d7114fec4f4eeedbb7561f790853040e9bfbdeebe79437ab66ea51f5",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-windows-arm64.zip",
        bytes: 43_250_108,
        sha256: "sha256:2e0d544999a765018ee5c2ff1a8b1a7e0f5d5b6b1e00b32d8c025d6c1dbcc833",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.87.1/pi-windows-x64.zip",
        bytes: 44_615_504,
        sha256: "sha256:aab2ba67baf8ff97a52d05b62d88e9e65a840c6ea8fa1029a28d62d210d4e5fc",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// The artifacts 0.85.1 was published as, kept so
/// `software_update` has a version to move from and `rollback` a tree to
/// return to. Measured from bytes when it was the current pin.
pub(crate) const PREVIOUS_ARTIFACTS: &[Artifact] = &[
    Artifact {
        platform: "linux/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-linux-arm64.tar.gz",
        bytes: 42_628_180,
        sha256: "sha256:042d20ae885ee4f3b102815f3280b962c377b2e9fb44de4037908cc530eae4d4",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "linux/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-linux-x64.tar.gz",
        bytes: 42_560_927,
        sha256: "sha256:494e498f47d74d21f40b3386f6a5e921a3d49531a169cab55bbdaca0ea1fe25a",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-darwin-arm64.tar.gz",
        bytes: 31_035_676,
        sha256: "sha256:d5f70e3c0cf7398eac239fd0261ee074d98b7ba7f6b43fe3617f052ed5b79d06",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "macos/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-darwin-x64.tar.gz",
        bytes: 33_544_584,
        sha256: "sha256:adb918b845625f184d8bea408d55eacaf21aa87238793c0f5b4f3b9737bce62b",
        shape: Shape::GzipTar,
        member: "pi/pi",
    },
    Artifact {
        platform: "windows/arm64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-windows-arm64.zip",
        bytes: 43_556_369,
        sha256: "sha256:b25e96fe64c9f41f75a924c0d36f395abb98d6c6fec0b78aaa0b86926f938bb4",
        shape: Shape::Zip,
        member: "pi.exe",
    },
    Artifact {
        platform: "windows/x86_64",
        url: "https://github.com/earendil-works/pi/releases/download/v0.85.1/pi-windows-x64.zip",
        bytes: 45_009_021,
        sha256: "sha256:002fa95b90d521245b9985d8f168caebc237ad56e7e30b319807dee1b2e17e1c",
        shape: Shape::Zip,
        member: "pi.exe",
    },
];

/// Pi Coding Agent's program, and where its bytes come from.
pub(crate) const SOFTWARE: Software = Software {
    version: "0.87.1",
    command: "pi",
    delivery: Delivery::Artifacts(ARTIFACTS),
    unsupported: &[],
    previous: Some(Previous {
        version: "0.85.1",
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
