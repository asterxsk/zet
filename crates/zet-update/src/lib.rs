//! Version identity, update checks, and self-replacement for zet.
//!
//! # What this crate is for
//!
//! A terminal that has to be reinstalled by hand to get a bug fix is a terminal that stays
//! broken. This crate answers three questions: what am I running, is there something newer,
//! and how do I become it.
//!
//! # The update path
//!
//! ```text
//! Checker::check()   →  GitHub's latest-release endpoint, compared against this build
//! Update::download() →  fetch SHA256SUMS, fetch the binary, verify size and digest
//! Update::stage()    →  write the verified bytes beside the running executable
//! replace_running_exe() →  move the running binary aside, move the new one in
//! ```
//!
//! Nothing is written to disk until the bytes have been verified, and the new version takes
//! effect on the next launch. Windows will not let a running executable be overwritten or
//! deleted, but it will let one be renamed, and that is the entire mechanism.
//!
//! # What a checksum does and does not prove
//!
//! The digest and the payload both come from the release, so verification establishes that
//! the download arrived intact and matches what the release published. It does not
//! establish who published it: zet's archives carry no Authenticode signature. Read
//! `SECURITY.md` in the repository before treating an update as authenticated.
//!
//! # Network
//!
//! The only host this crate contacts is `api.github.com` and the release asset URLs it
//! returns. What that discloses, and how to turn it off, is described in `PRIVACY.md`.

#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::path::PathBuf;

pub mod digest;
pub mod info;
pub mod install;
pub mod release;
pub mod update;
pub mod version;

pub use info::{GIT_SHA, TARGET, short_sha, version_line};
pub use release::{Asset, Release};
pub use update::{Checker, DEFAULT_REPO, Http, Transport, Update};
pub use version::{current, is_newer, parse_tag};

/// Everything that can go wrong on the way to a newer version.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request failed before a response could be read.
    #[error("could not reach the update feed: {0}")]
    Http(String),

    /// The server has no such resource. For the release feed this is the ordinary
    /// "nothing has been released yet" case, not a failure.
    #[error("{url} was not found")]
    NotFound {
        /// What was asked for.
        url: String,
    },

    /// A tag that is not a version.
    #[error("{0:?} is not a version")]
    BadTag(String),

    /// The feed was not the JSON shape this crate reads.
    #[error("the update feed could not be parsed: {0}")]
    Json(#[from] serde_json::Error),

    /// The newest release has no build for this machine.
    #[error("release {tag} has no build for {target}")]
    NoAsset {
        /// The release's tag.
        tag: String,
        /// The target triple it has no build for.
        target: String,
    },

    /// The release published no digest for the file, so nothing can be verified.
    #[error("no checksum was published for {0}")]
    NoChecksum(String),

    /// The download is not what the release published.
    #[error("{0} does not match its published checksum")]
    ChecksumMismatch(String),

    /// The download is not the length the release declared.
    #[error("{name} arrived as {got} bytes, not the {expected} the release declared")]
    SizeMismatch {
        /// The asset's file name.
        name: String,
        /// What the release said it would be.
        expected: u64,
        /// What arrived.
        got: u64,
    },

    /// The checksum file could not be read line by line.
    #[error("line {line} of the checksum file is not a digest and a name: {text:?}")]
    MalformedSums {
        /// One-based line number.
        line: usize,
        /// The offending line.
        text: String,
    },

    /// There is no file at the path the caller asked to install.
    #[error("there is nothing to install at {0}")]
    NothingToInstall(PathBuf),

    /// The install directory refused the write. On Windows this is almost always an
    /// installation under `Program Files` without elevation.
    #[error("{0} cannot be written to; updating an installation there needs administrator rights")]
    NotWritable(PathBuf),

    /// The original binary was moved aside and the new one could not take its place, and
    /// moving the original back failed too.
    #[error("the previous version was left at {backup} and could not be restored: {reason}")]
    Stranded {
        /// Where the working binary now sits.
        backup: PathBuf,
        /// Why it could not be put back.
        reason: String,
    },

    /// A file operation failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),
}
