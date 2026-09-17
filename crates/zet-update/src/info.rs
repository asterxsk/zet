//! What this binary is.
//!
//! A version on its own is not enough to identify a build. Two binaries can both call
//! themselves `0.1.0` and differ, and the one a bug report comes from is the one that
//! matters. The commit and the target triple come from the build script; see `build.rs`.

/// The target triple this binary was compiled for, such as `x86_64-pc-windows-msvc`.
///
/// This is what selects a release asset, so it has to be the compiler's answer rather
/// than a guess from `std::env::consts`, which cannot tell an MSVC build from a GNU one.
pub const TARGET: &str = env!("ZET_TARGET");

/// The commit this binary was built from, or `unknown` for a build without a repository.
pub const GIT_SHA: &str = env!("ZET_GIT_SHA");

/// The version, as a parseable value.
#[must_use]
pub fn version() -> semver::Version {
    crate::version::current()
}

/// The short form of [`GIT_SHA`] — the seven characters everyone actually reads.
#[must_use]
pub fn short_sha() -> &'static str {
    GIT_SHA.get(..7).unwrap_or(GIT_SHA)
}

/// The line `zet --version` prints.
///
/// One line, because it ends up pasted into issue reports, and because a `--version` that
/// wraps is a `--version` that gets truncated by whoever is reading it.
#[must_use]
pub fn version_line() -> String {
    format!(
        "zet {} ({}, {})",
        env!("CARGO_PKG_VERSION"),
        short_sha(),
        TARGET
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_script_stamped_a_target() {
        // A missing stamp would silently break asset selection on every platform, and the
        // symptom would look like a release that has no download for you.
        assert_ne!(TARGET, "unknown");
        assert_ne!(TARGET, "");
        assert!(TARGET.contains('-'), "expected a triple, got {TARGET:?}");
    }

    #[test]
    fn the_version_line_names_all_three_things() {
        let line = version_line();
        assert!(line.starts_with("zet "));
        assert!(line.contains(env!("CARGO_PKG_VERSION")), "{line}");
        assert!(line.contains(TARGET), "{line}");
        assert_eq!(line.lines().count(), 1);
    }

    #[test]
    fn a_short_sha_is_seven_characters_or_whatever_there_is() {
        // `unknown`, from a source tarball with no repository behind it, is shorter than
        // seven characters and must not panic on the slice.
        assert!(short_sha().len() <= 7);
        assert!(GIT_SHA.starts_with(short_sha()));
    }
}
