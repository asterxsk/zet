//! `ConPTY` sessions and shell profile discovery.
//!
//! This crate owns one thing: a running child process attached to a pseudoconsole, and
//! the bytes flowing in and out of it. It knows nothing about terminals, grids, or
//! rendering, and it never will — the byte stream it hands out is the same one
//! [`zet_vt`] parses, and the two crates never meet.
//!
//! # Why this is written against `windows-sys` rather than a pty crate
//!
//! Every wrapper in the ecosystem was eliminated for a specific reason. `portable-pty`
//! 0.9.0 exposes no creation-flags parameter at all and the published artifact predates
//! its own winapi-to-windows-sys migration. The fork that does expose the flags has an
//! open bug inverting `TerminateProcess` success on Windows, which is exactly the
//! teardown path this crate has to get right. `conpty` pins a two-year-old `windows`
//! tree, and `xpty` has never implemented its own async support. What is left is a few
//! hundred lines of documented FFI against the same crate `winit` already links.
//!
//! # The things that hang, and how they are avoided
//!
//! `ConPTY` has four behaviours that turn a straightforward read loop into a deadlock,
//! and none of them are in its documentation:
//!
//! - [`ClosePseudoConsole`] **blocks until the pseudoconsole's output has been drained**.
//!   Closing it while nothing is reading it hangs the thread forever. So the reader
//!   thread is left running while the close happens, and it is the reader that lets the
//!   close return.
//! - Closing the **input** half is how the console tells the child its window went away,
//!   and the child dies with `0xC000013A` instead of seeing end of input. The write end
//!   is therefore kept alive until after the pseudoconsole is closed.
//! - A read on the output pipe never returns zero while the pseudoconsole is open, so
//!   the reader cannot use end-of-file to notice that the child exited.
//! - `ConPTY`'s own tree kill misses detached grandchildren. A job object with
//!   `KILL_ON_JOB_CLOSE` is the only thing that reliably reaps them, and the process has
//!   to be created suspended and assigned to the job before it is allowed to run, or a
//!   grandchild spawned in the gap escapes it.
//!
//! [`zet_vt`]: https://docs.rs/zet-vt

pub mod discovery;

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{DEFAULT_BUFFER, Pty, SpawnConfig};

use std::path::PathBuf;

/// Something that went wrong starting or driving a child process.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    /// The pseudoconsole could not be created.
    #[error("could not create the pseudoconsole: {0}")]
    ConPty(String),

    /// The child process could not be started.
    #[error("could not start {program}: {message}")]
    Spawn {
        /// The program that was asked for.
        program: String,
        /// What Windows said.
        message: String,
    },

    /// A handle operation failed.
    #[error("{what} failed: {message}")]
    Io {
        /// Which operation.
        what: &'static str,
        /// What Windows said.
        message: String,
    },

    /// A terminal size of zero, which `ConPTY` rejects.
    #[error("a terminal must be at least 1x1, not {cols}x{rows}")]
    EmptySize {
        /// The width asked for.
        cols: u16,
        /// The height asked for.
        rows: u16,
    },

    /// The program to run does not exist.
    #[error("no such program: {0}")]
    NotFound(PathBuf),
}
