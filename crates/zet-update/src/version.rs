//! Version numbers and the tags that carry them.

use semver::Version;

use crate::Error;

/// Read a release tag as a version.
///
/// Tags are `v`-prefixed: the release workflow derives them from the workspace version, and
/// `v0.2.0` is what a person reads in a release URL. The prefix is optional here so that a
/// hand-made tag without one still resolves, and so does `0.2` — which is a valid release
/// tag a human might type even though it is not a valid version.
///
/// # Errors
///
/// Fails if the tag is not a version at all.
pub fn parse_tag(tag: &str) -> Result<Version, Error> {
    let trimmed = tag.trim().trim_start_matches('v');
    let mut parsed = Version::parse(trimmed).map_err(|_| Error::BadTag(tag.to_owned()))?;

    // Build metadata is dropped, and this is load-bearing rather than cosmetic. The SemVer
    // specification says metadata is ignored when determining precedence, but `semver`'s
    // `Version` derives `Ord` over its fields with `build` last, so `1.2.3+anything` sorts
    // strictly above `1.2.3`. A tag carrying metadata would therefore compare newer than
    // the very version it names: the update would be offered on every launch, forever, and
    // installing it would change nothing.
    parsed.build = semver::BuildMetadata::EMPTY;
    Ok(parsed)
}

/// Whether `candidate` is a version the running build should offer to install.
///
/// Newer is not the same as greater. A build on the stable channel must not be moved onto a
/// prerelease by an automatic update — `1.0.0-alpha` sorts above `0.9.0`, so a plain `>`
/// would hand every stable user an alpha the moment one is tagged. A build already on a
/// prerelease has opted into that channel and does move between prereleases.
#[must_use]
pub fn is_newer(candidate: &Version, current: &Version) -> bool {
    if candidate <= current {
        return false;
    }
    candidate.pre.is_empty() || !current.pre.is_empty()
}

/// The version a `--version` line reports, without the tag prefix.
#[must_use]
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("CARGO_PKG_VERSION is semver for every crate cargo builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).expect("valid")
    }

    #[test]
    fn a_tag_is_read_with_or_without_its_prefix() {
        assert_eq!(parse_tag("v1.2.3").expect("valid"), v("1.2.3"));
        assert_eq!(parse_tag("1.2.3").expect("valid"), v("1.2.3"));
        assert_eq!(parse_tag(" v1.2.3 ").expect("valid"), v("1.2.3"));
    }

    #[test]
    fn a_prerelease_tag_keeps_its_prerelease() {
        assert_eq!(parse_tag("v1.0.0-rc.1").expect("valid"), v("1.0.0-rc.1"));
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_rejected() {
        assert!(matches!(parse_tag("nightly"), Err(Error::BadTag(_))));
        assert!(matches!(parse_tag("v"), Err(Error::BadTag(_))));
        assert!(matches!(parse_tag("v1.2"), Err(Error::BadTag(_))));
        assert!(matches!(parse_tag(""), Err(Error::BadTag(_))));
    }

    #[test]
    fn build_metadata_does_not_make_a_tag_look_newer() {
        // `1.2.3+abc` and `1.2.3` are the same version. If the metadata survived, the tag
        // would compare greater than the version it names and the update would be offered
        // forever, every launch, without ever changing the binary.
        let tagged = parse_tag("v1.2.3+built.7").expect("valid");
        assert!(!is_newer(&tagged, &v("1.2.3")));
    }

    #[test]
    fn a_greater_version_is_newer() {
        assert!(is_newer(&v("0.2.0"), &v("0.1.0")));
        assert!(is_newer(&v("1.0.0"), &v("0.9.9")));
        assert!(!is_newer(&v("0.1.0"), &v("0.1.0")));
        assert!(!is_newer(&v("0.1.0"), &v("0.2.0")));
    }

    #[test]
    fn a_prerelease_is_never_offered_to_a_stable_build() {
        // `1.0.0-rc.1` sorts above `0.9.0`, so an automatic update would otherwise hand
        // every stable user a release candidate the moment one is tagged.
        assert!(!is_newer(&v("1.0.0-rc.1"), &v("0.9.0")));
        assert!(!is_newer(&v("0.2.0-beta.1"), &v("0.1.0")));
    }

    #[test]
    fn a_stable_release_is_offered_to_a_prerelease_build() {
        // The path off a prerelease channel: someone running `0.2.0-beta.1` is offered the
        // `0.2.0` that supersedes it.
        assert!(is_newer(&v("0.2.0"), &v("0.2.0-beta.1")));
    }

    #[test]
    fn a_build_on_a_prerelease_moves_between_prereleases() {
        assert!(is_newer(&v("0.2.0-beta.2"), &v("0.2.0-beta.1")));
        assert!(!is_newer(&v("0.2.0-beta.1"), &v("0.2.0-beta.2")));
    }

    #[test]
    fn the_current_version_parses() {
        // The workspace version is the one thing every release depends on; if it is not
        // semver the updater cannot compare anything.
        assert_eq!(current(), v(env!("CARGO_PKG_VERSION")));
    }
}
