//! The GitHub release feed, and picking the right download out of it.

use semver::Version;
use serde::Deserialize;

use crate::Error;
use crate::version::parse_tag;

/// One downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// The file name as published, which is also the key in `SHA256SUMS`.
    pub name: String,
    /// Where to fetch it.
    pub url: String,
    /// The size GitHub reports, used to reject a truncated download before hashing it.
    pub size: u64,
}

/// A published release.
#[derive(Debug, Clone)]
pub struct Release {
    /// The git tag, as published, including any `v` prefix.
    pub tag: String,
    /// The tag read as a version.
    pub version: Version,
    /// The release title.
    pub name: String,
    /// The release notes, as markdown.
    pub notes: String,
    /// When it was published, as an ISO 8601 timestamp.
    pub published_at: String,
    /// Everything attached to it.
    pub assets: Vec<Asset>,
}

impl Release {
    /// The asset for `target`, if the release published one.
    ///
    /// Assets are named `zet-<version>-<target>.exe`. Matching on the suffix alone would
    /// also catch a `.sha256` sidecar, and matching on the prefix alone would hand an
    /// `aarch64` machine an `x86_64` binary, so both ends are checked.
    #[must_use]
    pub fn asset_for(&self, target: &str) -> Option<&Asset> {
        let suffix = format!("-{target}.exe");
        self.assets
            .iter()
            .find(|asset| asset.name.starts_with("zet-") && asset.name.ends_with(&suffix))
    }

    /// The checksum file attached to this release.
    #[must_use]
    pub fn sums_asset(&self) -> Option<&Asset> {
        self.assets
            .iter()
            .find(|asset| asset.name == crate::digest::SUMS_ASSET)
    }
}

/// The subset of GitHub's release object this crate reads.
///
/// Field names are GitHub's, spelled out rather than renamed, so that this stays a
/// faithful copy of the shape it parses. Unknown fields are ignored on purpose: GitHub
/// adds them, and a feed that stops parsing because someone added a key is worse than one
/// that ignores it.
#[derive(Debug, Deserialize)]
pub struct RawRelease {
    #[serde(default)]
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<RawAsset>,
}

/// The subset of GitHub's asset object this crate reads.
#[derive(Debug, Deserialize)]
pub struct RawAsset {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
}

impl RawRelease {
    /// Turn the wire form into a [`Release`].
    ///
    /// # Errors
    ///
    /// Fails if the tag is not a version.
    pub fn into_release(self) -> Result<Release, Error> {
        let version = parse_tag(&self.tag_name)?;
        let name = self
            .name
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.tag_name.clone());
        Ok(Release {
            tag: self.tag_name,
            version,
            name,
            notes: self.body.unwrap_or_default(),
            published_at: self.published_at.unwrap_or_default(),
            assets: self
                .assets
                .into_iter()
                .map(|asset| Asset {
                    name: asset.name,
                    url: asset.browser_download_url,
                    size: asset.size,
                })
                .collect(),
        })
    }
}

/// Parse a release feed response.
///
/// # Errors
///
/// Fails if the body is not a release object, or its tag is not a version.
pub fn parse(json: &str) -> Result<Release, Error> {
    serde_json::from_str::<RawRelease>(json)?.into_release()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real payload shape, trimmed to the fields this crate reads.
    const FEED: &str = r#"
{
  "url": "https://api.github.com/repos/asterxsk/zet/releases/1",
  "html_url": "https://github.com/asterxsk/zet/releases/tag/v0.2.0",
  "id": 1,
  "tag_name": "v0.2.0",
  "name": "zet 0.2.0",
  "body": "Faster reflow.",
  "draft": false,
  "prerelease": false,
  "created_at": "2026-09-01T10:00:00Z",
  "published_at": "2026-09-01T10:05:00Z",
  "author": { "login": "asterxsk", "id": 1 },
  "assets": [
    {
      "name": "zet-0.2.0-x86_64-pc-windows-msvc.exe",
      "browser_download_url": "https://github.com/asterxsk/zet/releases/download/v0.2.0/zet-0.2.0-x86_64-pc-windows-msvc.exe",
      "size": 8123456,
      "download_count": 12,
      "content_type": "application/octet-stream"
    },
    {
      "name": "zet-0.2.0-aarch64-pc-windows-msvc.exe",
      "browser_download_url": "https://github.com/asterxsk/zet/releases/download/v0.2.0/zet-0.2.0-aarch64-pc-windows-msvc.exe",
      "size": 7900000
    },
    {
      "name": "SHA256SUMS",
      "browser_download_url": "https://github.com/asterxsk/zet/releases/download/v0.2.0/SHA256SUMS",
      "size": 190
    }
  ]
}
"#;

    #[test]
    fn a_release_is_read_from_the_wire_form() {
        let release = parse(FEED).expect("valid");
        assert_eq!(release.tag, "v0.2.0");
        assert_eq!(release.version, Version::parse("0.2.0").expect("valid"));
        assert_eq!(release.name, "zet 0.2.0");
        assert_eq!(release.notes, "Faster reflow.");
        assert_eq!(release.published_at, "2026-09-01T10:05:00Z");
        assert_eq!(release.assets.len(), 3);
    }

    #[test]
    fn unknown_fields_do_not_break_the_feed() {
        // GitHub adds fields. A parser that fails on a new key breaks on a day nobody
        // changed this repository.
        let with_extras = r#"{"tag_name":"v1.0.0","immutable":true,"reactions":{"total_count":3}}"#;
        let release = parse(with_extras).expect("valid");
        assert_eq!(release.version, Version::parse("1.0.0").expect("valid"));
    }

    #[test]
    fn absent_optional_fields_fall_back_rather_than_fail() {
        let release = parse(r#"{"tag_name":"v1.0.0"}"#).expect("valid");
        assert_eq!(
            release.name, "v1.0.0",
            "the tag stands in for a missing title"
        );
        assert!(release.notes.is_empty());
        assert!(release.assets.is_empty());
    }

    #[test]
    fn the_asset_for_a_target_is_the_one_built_for_it() {
        let release = parse(FEED).expect("valid");
        let asset = release
            .asset_for("x86_64-pc-windows-msvc")
            .expect("a matching asset");
        assert_eq!(asset.name, "zet-0.2.0-x86_64-pc-windows-msvc.exe");
        assert_eq!(asset.size, 8_123_456);
    }

    #[test]
    fn a_target_prefix_alone_does_not_match() {
        // `x86_64-pc-windows-msvc` is a prefix of nothing here, but the reverse mistake is
        // the dangerous one: a suffix match with no prefix check would let `notzet-....exe`
        // through, and a prefix match with no suffix check would hand this machine the
        // `aarch64` build the moment one exists.
        let release = parse(FEED).expect("valid");
        assert!(release.asset_for("aarch64-pc-windows-msvc").is_some());
        assert!(release.asset_for("i686-pc-windows-msvc").is_none());
        assert!(release.asset_for("").is_none());
    }

    #[test]
    fn a_checksum_sidecar_is_never_mistaken_for_the_binary() {
        let release = parse(FEED).expect("valid");
        let picked = release
            .asset_for("x86_64-pc-windows-msvc")
            .expect("an asset");
        assert!(picked.name.ends_with(".exe"));
        assert_ne!(picked.name, "SHA256SUMS");
    }

    #[test]
    fn the_checksum_file_is_found() {
        let release = parse(FEED).expect("valid");
        assert_eq!(
            release.sums_asset().map(|asset| asset.name.as_str()),
            Some("SHA256SUMS")
        );
    }

    #[test]
    fn a_release_without_checksums_reports_none() {
        let release = parse(r#"{"tag_name":"v1.0.0","assets":[]}"#).expect("valid");
        assert!(release.sums_asset().is_none());
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_rejected() {
        assert!(matches!(
            parse(r#"{"tag_name":"nightly"}"#),
            Err(Error::BadTag(_))
        ));
    }

    #[test]
    fn a_body_that_is_not_a_release_is_rejected() {
        assert!(matches!(parse("not json"), Err(Error::Json(_))));
    }
}
