//! Checksums, and the file that publishes them.
//!
//! The updater replaces the running executable with something it downloaded, so the
//! download has to be checked against a value that did not come from the same place. Both
//! come from the release, which is the honest limit of what this can prove: a checksum
//! establishes that the bytes arrived intact and match what the release published, not who
//! published them. See `SECURITY.md`.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::Error;

/// The name of the file that carries the checksums.
pub const SUMS_ASSET: &str = "SHA256SUMS";

/// The hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Read a `SHA256SUMS` file into a map of file name to hex digest.
///
/// The format is the one `sha256sum` writes: a hex digest, whitespace, then the file name.
/// The separator is two spaces for text mode and ` *` for binary mode, and both appear in
/// the wild, so the parser splits on any run of whitespace rather than on a fixed string.
///
/// # Errors
///
/// Fails if a line is neither blank nor a digest-name pair.
pub fn parse_sums(text: &str) -> Result<BTreeMap<String, String>, Error> {
    let mut sums = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(digest), Some(name)) = (fields.next(), fields.next()) else {
            return Err(Error::MalformedSums {
                line: index + 1,
                text: line.to_owned(),
            });
        };
        sums.insert(
            name.trim_start_matches('*').to_owned(),
            digest.to_lowercase(),
        );
    }
    Ok(sums)
}

/// Check `bytes` against the digest published for `name`.
///
/// # Errors
///
/// Fails if no digest was published for `name`, or if the bytes do not match it.
pub fn verify(name: &str, bytes: &[u8], sums: &BTreeMap<String, String>) -> Result<(), Error> {
    let Some(expected) = sums.get(name) else {
        return Err(Error::NoChecksum(name.to_owned()));
    };
    // Compared in constant time would be better, but an attacker who can time a local
    // string comparison has already won by another route, and the digest is public.
    if &sha256_hex(bytes) != expected {
        return Err(Error::ChecksumMismatch(name.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SHA-256 of the empty input, which is a fixed published constant.
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn an_empty_input_hashes_to_the_published_constant() {
        assert_eq!(sha256_hex(b""), EMPTY);
    }

    #[test]
    fn a_known_input_hashes_to_its_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sums_are_read_in_both_separator_styles() {
        let text = "\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  zet.exe
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *binary.exe
";
        let sums = parse_sums(text).expect("valid");
        assert_eq!(sums.get("zet.exe").map(String::as_str), Some(EMPTY));
        assert_eq!(
            sums.get("binary.exe").map(String::as_str),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        let sums = parse_sums("\n# a comment\n\n  \n").expect("valid");
        assert!(sums.is_empty());
    }

    #[test]
    fn a_digest_is_matched_case_insensitively() {
        // Some tools publish uppercase hex. Refusing those would be a bug that only shows
        // up the day the publishing step changes, which is the worst day for it.
        let sums = parse_sums(&format!("{}  zet.exe", EMPTY.to_uppercase())).expect("valid");
        verify("zet.exe", b"", &sums).expect("an uppercase digest still matches");
    }

    #[test]
    fn a_line_that_is_not_a_pair_is_rejected() {
        assert!(matches!(
            parse_sums("just-a-digest\n"),
            Err(Error::MalformedSums { line: 1, .. })
        ));
        assert!(matches!(
            parse_sums("good  one.exe\nbroken\n"),
            Err(Error::MalformedSums { line: 2, .. })
        ));
    }

    #[test]
    fn a_missing_digest_is_reported_rather_than_assumed() {
        let sums = parse_sums(&format!("{EMPTY}  other.exe\n")).expect("valid");
        assert!(matches!(
            verify("zet.exe", b"", &sums),
            Err(Error::NoChecksum(name)) if name == "zet.exe"
        ));
    }

    #[test]
    fn the_wrong_bytes_do_not_verify() {
        let sums = parse_sums(&format!("{EMPTY}  zet.exe\n")).expect("valid");
        assert!(matches!(
            verify("zet.exe", b"tampered", &sums),
            Err(Error::ChecksumMismatch(name)) if name == "zet.exe"
        ));
    }

    #[test]
    fn the_right_bytes_verify() {
        let sums = parse_sums(&format!("{}  zet.exe\n", sha256_hex(b"payload"))).expect("valid");
        verify("zet.exe", b"payload", &sums).expect("matching bytes verify");
    }
}
