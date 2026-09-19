//! Checking for a newer release, and putting it in place.

use std::path::PathBuf;
use std::time::Duration;

use semver::Version;

use crate::release::{self, Asset, Release};
use crate::version::is_newer;
use crate::{Error, digest, info, install};

/// Where the releases live.
pub const DEFAULT_REPO: &str = "asterxsk/zet";

/// The API root, overridable so that a test can point at a canned response.
pub const DEFAULT_API: &str = "https://api.github.com";

/// The most this will download in one response.
///
/// A release binary is single-digit megabytes. The cap is not there to be tight; it is
/// there so that a server which never stops sending cannot exhaust memory before anything
/// has a chance to notice.
const MAX_RESPONSE: u64 = 128 * 1024 * 1024;

/// How to fetch a URL.
///
/// The updater takes one of these rather than reaching for the network itself, so that the
/// check, the download, and the verification can all be exercised without a server, and so
/// that the one part that does touch the network is a single small type that can be read in
/// full.
pub trait Transport {
    /// Fetch `url` and return its body.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when the server says the resource does not exist; any other
    /// failure as [`Error::Http`].
    fn get(&self, url: &str) -> Result<Vec<u8>, Error>;
}

/// A transport backed by HTTPS.
pub struct Http {
    agent: ureq::Agent,
}

impl Http {
    /// A transport that identifies itself as `user_agent`.
    ///
    /// GitHub rejects a request with no `User-Agent`, so this is required rather than
    /// polite, and naming the version is what lets a release's download counts be read
    /// with any idea of which builds are still in use.
    #[must_use]
    pub fn new(user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .user_agent(user_agent)
            // Every check is a command a person typed and is waiting on, so the whole
            // exchange is bounded rather than each step of it. A step-by-step bound adds
            // up to a wait no one chose; one bound is the wait that was promised.
            .timeout_global(Some(Duration::from_secs(30)))
            .max_redirects(5)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

impl Default for Http {
    fn default() -> Self {
        Self::new(&format!(
            "zet/{} (+https://github.com/{DEFAULT_REPO})",
            env!("CARGO_PKG_VERSION")
        ))
    }
}

impl Transport for Http {
    fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
        let response = self
            .agent
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(|error| match error {
                ureq::Error::StatusCode(404) => Error::NotFound {
                    url: url.to_owned(),
                },
                other => Error::Http(other.to_string()),
            })?;
        response
            .into_body()
            .into_with_config()
            .limit(MAX_RESPONSE)
            .read_to_vec()
            .map_err(|error| Error::Http(error.to_string()))
    }
}

/// A release that is newer than this build and has a download for it.
#[derive(Debug, Clone)]
pub struct Update {
    /// The release.
    pub release: Release,
    /// The asset built for this target.
    pub asset: Asset,
}

impl Update {
    /// The version being offered.
    #[must_use]
    pub fn version(&self) -> &Version {
        &self.release.version
    }

    /// Fetch the asset and check it against the release's published digest.
    ///
    /// Nothing is written to disk until the bytes have been verified, so a truncated
    /// download or a response that is not the file at all never becomes a file that a
    /// later step could mistake for one.
    ///
    /// # Errors
    ///
    /// Fails if the release published no checksums, if either fetch fails, if the size is
    /// not what the release declared, or if the digest does not match.
    pub fn download<T: Transport>(&self, transport: &T) -> Result<Vec<u8>, Error> {
        let sums_asset = self.release.sums_asset().ok_or_else(|| {
            Error::NoChecksum(format!("{} (no {})", self.asset.name, digest::SUMS_ASSET))
        })?;
        let sums = digest::parse_sums(&String::from_utf8_lossy(&transport.get(&sums_asset.url)?))?;

        let bytes = transport.get(&self.asset.url)?;
        // Checked before hashing because it is free and it names the failure precisely: a
        // captive portal answering with a login page has a length, and it is not this one.
        if bytes.len() as u64 != self.asset.size {
            return Err(Error::SizeMismatch {
                name: self.asset.name.clone(),
                expected: self.asset.size,
                got: bytes.len() as u64,
            });
        }
        digest::verify(&self.asset.name, &bytes, &sums)?;
        Ok(bytes)
    }

    /// Write a verified download beside the running executable, ready to be moved into
    /// place.
    ///
    /// # Errors
    ///
    /// Fails if the download cannot be written.
    pub fn stage(&self, bytes: &[u8]) -> Result<PathBuf, Error> {
        let staged = install::staged_path()?;
        std::fs::write(&staged, bytes)?;
        Ok(staged)
    }

    /// Download, verify, stage, and replace the running executable.
    ///
    /// The new version takes effect on the next launch.
    ///
    /// # Errors
    ///
    /// As [`Update::download`], [`Update::stage`], and
    /// [`install::replace_running_exe`].
    pub fn install<T: Transport>(&self, transport: &T) -> Result<PathBuf, Error> {
        let bytes = self.download(transport)?;
        let staged = self.stage(&bytes)?;
        install::replace_running_exe(&staged)?;
        Ok(staged)
    }
}

/// Asks GitHub whether there is anything newer.
pub struct Checker<T> {
    transport: T,
    repo: String,
    api: String,
    target: String,
    current: Version,
}

impl<T: Transport> Checker<T> {
    /// A checker for `repo`, on this binary's target and version.
    pub fn new(transport: T, repo: impl Into<String>) -> Self {
        Self {
            transport,
            repo: repo.into(),
            api: DEFAULT_API.to_owned(),
            target: info::TARGET.to_owned(),
            current: info::version(),
        }
    }

    /// Ask about a target other than the one this binary was built for.
    #[must_use]
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    /// Ask a server other than GitHub's.
    #[must_use]
    pub fn api(mut self, api: impl Into<String>) -> Self {
        self.api = api.into();
        self
    }

    /// Treat `current` as the version to compare against.
    #[must_use]
    pub fn current(mut self, current: Version) -> Self {
        self.current = current;
        self
    }

    /// The URL of the release feed.
    #[must_use]
    pub fn feed_url(&self) -> String {
        format!("{}/repos/{}/releases/latest", self.api, self.repo)
    }

    /// Look for a newer release.
    ///
    /// Returns `None` when this build is current, which includes the case of a repository
    /// that has never published a release: GitHub answers 404 for that, and a project
    /// without releases is not an error condition to report on every launch.
    ///
    /// # Errors
    ///
    /// Fails if the feed cannot be fetched or parsed, or if the latest release has no
    /// build for this target — which is worth saying out loud rather than reporting as
    /// "up to date".
    pub fn check(&self) -> Result<Option<Update>, Error> {
        let body = match self.transport.get(&self.feed_url()) {
            Ok(body) => body,
            Err(Error::NotFound { .. }) => return Ok(None),
            Err(other) => return Err(other),
        };
        let release = release::parse(&String::from_utf8_lossy(&body))?;

        if !is_newer(&release.version, &self.current) {
            return Ok(None);
        }
        let asset = release
            .asset_for(&self.target)
            .ok_or_else(|| Error::NoAsset {
                tag: release.tag.clone(),
                target: self.target.clone(),
            })?
            .clone();
        Ok(Some(Update { release, asset }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A transport that answers from a table and records what it was asked for.
    struct Fake {
        responses: HashMap<String, String>,
        asked: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self {
                responses: pairs
                    .iter()
                    .map(|(url, body)| ((*url).to_owned(), (*body).to_owned()))
                    .collect(),
                asked: RefCell::new(Vec::new()),
            }
        }
    }

    impl Transport for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>, Error> {
            self.asked.borrow_mut().push(url.to_owned());
            self.responses
                .get(url)
                .map(|body| body.clone().into_bytes())
                .ok_or_else(|| Error::NotFound {
                    url: url.to_owned(),
                })
        }
    }

    const FEED: &str = "https://api.github.com/repos/asterxsk/zet/releases/latest";

    fn feed(version: &str, assets: &str) -> String {
        format!(r#"{{"tag_name":"v{version}","name":"zet {version}","assets":[{assets}]}}"#)
    }

    fn asset(name: &str) -> String {
        format!(
            r#"{{"name":"{name}","browser_download_url":"https://example.test/{name}","size":7}}"#
        )
    }

    fn checker(fake: Fake) -> Checker<Fake> {
        Checker::new(fake, "asterxsk/zet")
            .target("x86_64-pc-windows-msvc")
            .current(Version::parse("0.1.0").expect("valid"))
    }

    #[test]
    fn a_repository_with_no_releases_is_not_an_error() {
        // The state this repository is in right now. Reporting it as a failure would mean
        // every launch of every build complains about a release that does not exist yet.
        let fake = Fake::new(&[]);
        assert!(checker(fake).check().expect("no error").is_none());
    }

    #[test]
    fn a_newer_release_is_offered_with_its_asset() {
        let fake = Fake::new(&[(
            FEED,
            &feed("0.2.0", &asset("zet-0.2.0-x86_64-pc-windows-msvc.exe")),
        )]);
        let update = checker(fake).check().expect("no error").expect("an update");
        assert_eq!(update.version(), &Version::parse("0.2.0").expect("valid"));
        assert_eq!(update.asset.name, "zet-0.2.0-x86_64-pc-windows-msvc.exe");
    }

    #[test]
    fn the_same_version_is_not_offered() {
        let fake = Fake::new(&[(
            FEED,
            &feed("0.1.0", &asset("zet-0.1.0-x86_64-pc-windows-msvc.exe")),
        )]);
        assert!(checker(fake).check().expect("no error").is_none());
    }

    #[test]
    fn an_older_release_is_not_offered() {
        let fake = Fake::new(&[(
            FEED,
            &feed("0.0.9", &asset("zet-0.0.9-x86_64-pc-windows-msvc.exe")),
        )]);
        assert!(checker(fake).check().expect("no error").is_none());
    }

    #[test]
    fn a_release_with_no_build_for_this_target_is_reported() {
        // Not "up to date". Those are different facts and only one of them is true.
        let fake = Fake::new(&[(
            FEED,
            &feed("0.2.0", &asset("zet-0.2.0-aarch64-pc-windows-msvc.exe")),
        )]);
        assert!(matches!(
            checker(fake).check(),
            Err(Error::NoAsset { tag, target })
                if tag == "v0.2.0" && target == "x86_64-pc-windows-msvc"
        ));
    }

    #[test]
    fn the_feed_url_is_the_releases_latest_endpoint() {
        let fake = Fake::new(&[]);
        let checker = checker(fake);
        assert_eq!(checker.feed_url(), FEED);
        checker.check().expect("no error");
        assert_eq!(checker.transport.asked.borrow().as_slice(), [FEED]);
    }

    #[test]
    fn a_custom_api_root_replaces_github() {
        let fake = Fake::new(&[]);
        let checker = Checker::new(fake, "someone/else")
            .api("https://mirror.test")
            .target("x86_64-pc-windows-msvc");
        assert_eq!(
            checker.feed_url(),
            "https://mirror.test/repos/someone/else/releases/latest"
        );
    }

    #[test]
    fn a_prerelease_is_not_offered_over_a_stable_build() {
        // End to end through the checker, because this rule is the one that decides whether
        // every stable user is handed a release candidate the day one is tagged.
        let fake = Fake::new(&[(
            FEED,
            &feed(
                "0.2.0-rc.1",
                &asset("zet-0.2.0-rc.1-x86_64-pc-windows-msvc.exe"),
            ),
        )]);
        assert!(checker(fake).check().expect("no error").is_none());
    }

    const BINARY: &str = "https://example.test/zet-0.2.0-x86_64-pc-windows-msvc.exe";
    const SUMS: &str = "https://example.test/SHA256SUMS";
    const NAME: &str = "zet-0.2.0-x86_64-pc-windows-msvc.exe";
    const PAYLOAD: &str = "payload";

    /// An update whose declared size matches `payload`, with the checksum file attached.
    fn update(payload: &str, sums: Option<String>) -> Update {
        let mut assets = vec![Asset {
            name: NAME.to_owned(),
            url: BINARY.to_owned(),
            size: payload.len() as u64,
        }];
        if let Some(sums) = sums {
            assets.push(Asset {
                name: digest::SUMS_ASSET.to_owned(),
                url: SUMS.to_owned(),
                size: sums.len() as u64,
            });
        }
        Update {
            release: Release {
                tag: "v0.2.0".to_owned(),
                version: Version::parse("0.2.0").expect("valid"),
                name: "zet 0.2.0".to_owned(),
                notes: String::new(),
                published_at: String::new(),
                assets,
            },
            asset: Asset {
                name: NAME.to_owned(),
                url: BINARY.to_owned(),
                size: payload.len() as u64,
            },
        }
    }

    fn sums_for(payload: &str) -> String {
        format!("{}  {NAME}\n", digest::sha256_hex(payload.as_bytes()))
    }

    #[test]
    fn a_verified_download_comes_back_as_bytes() {
        let update = update(PAYLOAD, Some(sums_for(PAYLOAD)));
        let fake = Fake::new(&[(BINARY, PAYLOAD), (SUMS, &sums_for(PAYLOAD))]);
        assert_eq!(
            update.download(&fake).expect("verified"),
            PAYLOAD.as_bytes()
        );
    }

    #[test]
    fn the_checksum_file_is_fetched_from_the_release_not_guessed() {
        let update = update(PAYLOAD, Some(sums_for(PAYLOAD)));
        let fake = Fake::new(&[(BINARY, PAYLOAD), (SUMS, &sums_for(PAYLOAD))]);
        update.download(&fake).expect("verified");
        assert_eq!(
            fake.asked.borrow().as_slice(),
            [SUMS, BINARY],
            "the digest has to be in hand before the payload it describes is fetched"
        );
    }

    #[test]
    fn a_payload_that_does_not_match_its_digest_is_refused() {
        let update = update(PAYLOAD, Some(sums_for("something else")));
        let fake = Fake::new(&[(BINARY, PAYLOAD), (SUMS, &sums_for("something else"))]);
        assert!(matches!(
            update.download(&fake),
            Err(Error::ChecksumMismatch(name)) if name == NAME
        ));
    }

    #[test]
    fn a_payload_of_the_wrong_length_is_refused_before_it_is_hashed() {
        // A captive portal or a proxy error page is a 200 response with a body. Its length
        // is the cheapest thing to check and it names the failure precisely.
        let mut update = update(PAYLOAD, Some(sums_for(PAYLOAD)));
        update.asset.size = 999;
        let fake = Fake::new(&[(BINARY, PAYLOAD), (SUMS, &sums_for(PAYLOAD))]);
        assert!(matches!(
            update.download(&fake),
            Err(Error::SizeMismatch {
                expected: 999,
                got: 7,
                ..
            })
        ));
    }

    #[test]
    fn a_release_with_no_checksum_file_cannot_be_installed() {
        let update = update(PAYLOAD, None);
        let fake = Fake::new(&[(BINARY, PAYLOAD)]);
        assert!(matches!(update.download(&fake), Err(Error::NoChecksum(_))));
    }

    #[test]
    fn a_checksum_file_that_omits_this_asset_is_treated_as_missing() {
        let sums = format!("{}  some-other-file.exe\n", digest::sha256_hex(b"x"));
        let update = update(PAYLOAD, Some(sums.clone()));
        let fake = Fake::new(&[(BINARY, PAYLOAD), (SUMS, &sums)]);
        assert!(matches!(
            update.download(&fake),
            Err(Error::NoChecksum(name)) if name == NAME
        ));
    }
}
