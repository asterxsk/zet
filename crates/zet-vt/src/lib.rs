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
// Three pedantic lints do not fit this crate, and the rest are all on.
//
// - `match_same_arms`: the parser and the terminal are state machines transcribed from
//   ECMA-48 and the VT state diagram. Two arms with the same body but different byte
//   ranges are two rows of that table. Merging them is shorter and destroys the mapping
//   back to the specification the code is checked against line by line.
// - `too_many_lines`: a state machine is one function holding one match. Splitting
//   `advance` or `csi_dispatch` by state would put the transitions somewhere other than
//   next to each other, which is the only thing that matters about reading them.
// - `must_use_candidate`: it fires on every two-line accessor over a value type, which
//   here is most of the public surface. Annotating several dozen getters buries the
//   cases where the lint would have caught something.
#![allow(
    clippy::match_same_arms,
    clippy::too_many_lines,
    clippy::must_use_candidate
)]

pub mod attrs;
pub mod cell;
pub mod color;
pub mod damage;
pub mod grid;
pub mod parser;
pub mod row;
pub mod term;

pub use attrs::{Attrs, UnderlineStyle};
pub use cell::{Cell, CellFlags};
pub use color::{Color, ColorSpec, NamedColor};
pub use damage::Damage;
pub use grid::{Grid, Pos};
pub use parser::{Params, Parser, Perform, Private};
pub use row::Row;
pub use term::{Modes, MouseEncoding, MouseMode, Pen, Term};
