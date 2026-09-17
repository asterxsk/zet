//! One running terminal: a child process on a pseudoconsole, the parser chewing its
//! output, and the grid that comes out.
//!
//! [`zet_pty`] owns a process and hands out the bytes it writes; [`zet_vt`] owns the
//! parser and the grid and has never heard of a process. This crate is the join between
//! them: it owns one [`zet_vt::Parser`], the [`zet_vt::Term`] that parser feeds, and the
//! scrollback view over that grid, for one tab. [`Sessions`] owns several of those and
//! hands out the tab numbers.
//!
//! It knows nothing about windows, rendering, or configuration, and it never will. What
//! it produces is a grid and a title; deciding what those look like is somebody else's
//! problem, and that boundary is what keeps a rendering change from being a terminal
//! change.
//!
//! # The reader is not here
//!
//! `zet-pty` already runs the thread that reads the pseudoconsole, and its output arrives
//! on a channel through `Pty::next_chunk`. A second reader would only race that one, so
//! this crate does not add one. It adds a *pump* instead: a thread parked in `next_chunk`
//! with a timeout, appending whatever comes back to a buffer this crate owns and calling
//! the host's [`Waker`].
//!
//! The pump exists because nothing else can wait. A `ConPTY` output pipe never reports
//! end of file while the pseudoconsole is open, so there is no future to await and no
//! readiness to poll; somebody has to sit on the channel, and it must not be the thread
//! that draws.
//!
//! # What the pump makes possible, and what it costs
//!
//! [`Session::drain`] takes those bytes without blocking and feeds them to the parser,
//! which is what makes draining cheap enough to do on every frame. The price is that the
//! buffer is unbounded while the host is not draining: a program that prints faster than
//! the host drains will grow it. The pty's own channel is bounded, so the backpressure
//! the child feels is real — it is the gap between the pump and the host that is not.
//!
//! # Teardown
//!
//! [`Session::close`] and `Drop` do the same thing, in an order that matters: stop the
//! pump, join it, and only then hand the pty to `Pty::shutdown`. Joining first is what
//! lets teardown be the last owner of the pseudoconsole, and a pump still parked in
//! `next_chunk` is what would keep a child alive after its tab was closed.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]

pub mod session;
pub mod sessions;

pub use session::{Drained, NoopWaker, Session, SessionError, Waker};
pub use sessions::Sessions;
