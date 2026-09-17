//! zet's terminal emulation core.
//!
//! This crate owns the character pipeline end to end: it turns a byte stream from a
//! pty into a grid of styled cells, and it tracks which of those cells changed so the
//! renderer can repaint the minimum.
//!
//! The layers, bottom up:
//!
//! - [`color`], [`attrs`], [`cell`] — the value types. Sized and laid out for the
//!   render loop rather than for convenience.
//! - [`row`], [`grid`] — storage. Rows know how to trim their own trailing blanks and
//!   whether they are a soft-wrap continuation of the row above.
//! - [`parser`] — a byte-level state machine that emits events. It knows nothing about
//!   screens or cursors.
//! - [`term`] — the state machine that consumes those events: cursor, modes, margins,
//!   tab stops, SGR pen, and the operations that mutate the grid.
//! - [`damage`] — what changed since the last frame.
//!
//! The parser is deliberately ignorant of terminal state. Everything that can be
//! wrong about a terminal, from reflow to erase semantics, lives in [`term`], where
//! it can be tested against a real grid instead of against a byte sequence.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]

pub mod attrs;
pub mod cell;
pub mod color;
pub mod parser;
pub mod row;

pub use attrs::{Attrs, UnderlineStyle};
pub use cell::{Cell, CellFlags};
pub use color::{Color, ColorSpec, NamedColor};
pub use parser::{Params, Perform, Parser, Private};
pub use row::Row;
