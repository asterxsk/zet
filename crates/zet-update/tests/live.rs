//! Tests that talk to GitHub.
//!
//! Ignored by default, because a test that needs the network is a test that fails on a
//! train. Run them deliberately:
//!
//! ```text
//! cargo test -p zet-update --test live -- --ignored --nocapture
//! ```
//!
//! What they cover is the part a fake transport cannot: that ureq follows GitHub's
//! redirect from `browser_download_url` to the object store, that the real feed has the
//! shape this crate parses, and that a repository with no releases really does answer 404
//! rather than something this crate would misread as an update.

#![cfg(windows)]

use semver::Version;
use zet_update::{Checker, Http, Transport, release};

/// A repository with a long history of releases, used as a stand-in for the real feed.
const OTHER: &str = "BurntSushi/ripgrep";

/// The state this repository is actually in.
#[test]
#[ignore = "requires network"]
fn this_repository_has_no_releases_yet() {
    let checker = Checker::new(Http::default(), zet_update::DEFAULT_REPO);
    match checker.check() {
        // The ordinary outcome until a release is published. GitHub answers 404 for a
        // repository with no releases, and that has to read as "up to date" rather than as
        // a failure, or every launch complains about a release that does not exist.
        Ok(None) => {}
        Ok(Some(update)) => panic!("there should be no release yet, got {}", update.version()),
        Err(error) => panic!("a repository without releases must not be an error: {error}"),
    }
}

#[test]
#[ignore = "requires network"]
fn a_real_release_feed_has_the_shape_this_parses() {
    let http = Http::default();
    let body = http
        .get(&format!(
            "https://api.github.com/repos/{OTHER}/releases/latest"
        ))
        .expect("the feed should be reachable");
    let release = release::parse(&String::from_utf8_lossy(&body)).expect("the feed should parse");

    assert!(
        release.version > Version::new(0, 0, 0),
        "got {}",
        release.version
    );
    assert!(!release.assets.is_empty(), "a release with no assets");
    assert!(
        release
            .assets
            .iter()
            .any(|asset| asset.url.starts_with("https://")),
        "every asset should carry a download URL"
    );
}

#[test]
#[ignore = "requires network"]
fn a_release_with_no_zet_build_is_reported_rather_than_offered() {
    // The real feed, filtered by this crate's asset rule: a release that exists but was
    // not built for zet must not be handed to the updater as an installable.
    let checker = Checker::new(Http::default(), OTHER)
        .current(Version::new(0, 0, 1))
        .target("x86_64-pc-windows-msvc");
    let error = checker
        .check()
        .expect_err("no zet build should be found in that release");
    assert!(
        matches!(error, zet_update::Error::NoAsset { .. }),
        "got {error:?}"
    );
}

#[test]
#[ignore = "requires network"]
fn a_real_asset_downloads_through_the_redirect() {
    // `browser_download_url` is a redirect to `objects.githubusercontent.com`. A client
    // that does not follow it gets a 302 body and a checksum failure that looks like
    // tampering, so the redirect is worth proving rather than assuming.
    let http = Http::default();
    let body = http
        .get(&format!(
            "https://api.github.com/repos/{OTHER}/releases/latest"
        ))
        .expect("the feed should be reachable");
    let release = release::parse(&String::from_utf8_lossy(&body)).expect("the feed should parse");

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name.contains("x86_64-pc-windows-msvc"))
        .or_else(|| release.assets.first())
        .expect("a downloadable asset");

    let bytes = http.get(&asset.url).expect("the asset should download");
    assert_eq!(
        bytes.len() as u64,
        asset.size,
        "{} arrived as {} bytes, not the {} GitHub declared",
        asset.name,
        bytes.len(),
        asset.size
    );
}
