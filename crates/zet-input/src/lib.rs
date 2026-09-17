//! Decoded input turned into the bytes a program reads on its stdin.
//!
//! zet's windowing layer produces key and mouse events; the pty takes bytes. This
//! crate is the seam, and it is deliberately independent of `winit`.
//!
//! That independence is not ceremony. A user's keybindings live in the config file
//! as text like `Ctrl+Shift+T`, so the key type has to be namable, parseable and
//! printable on its own — it is a piece of the configuration vocabulary before it
//! is ever a piece of an event. `winit`'s key type cannot be that, because it
//! describes a platform event rather than something a person writes down. So the
//! host maps its own events onto [`KeyEvent`] and [`MouseEvent`] and hands them
//! here, and this crate stays testable without a window.
//!
//! Almost all of this crate is a table, and the table is the legacy VT / xterm one.
//! [`encode_key`] and [`encode_mouse`] translate into the sequences a program that
//! did not negotiate anything newer expects, [`encode_paste`] and [`encode_focus`]
//! are the two that are gated by a mode rather than by a keystroke.
//!
//! # The kitty keyboard protocol is not implemented
//!
//! The kitty keyboard protocol is not here, and what is missing is an input rather
//! than a decision. It is negotiated as a *stack* of flags — `CSI > u` pushes one,
//! `CSI < u` pops one, and every push can change what the keys report — so encoding
//! under it needs the current flag set. [`zet_vt::Modes`] does not track that stack
//! yet, and this crate cannot invent it: the stack is the terminal's state, since
//! the terminal is the only thing that sees the program's escape sequences come in.
//! This crate only ever sees the events the host hands it, so it has no way to learn
//! that a push happened and no way to keep a copy honest.
//!
//! Until `zet-vt` grows the flag stack, [`encode_key`] reports only the legacy
//! encoding, which is exactly what a program that never asked for kitty expects.
//! The visible cost is key release and repeat: legacy VT has no byte for either, so
//! [`encode_key`] answers `None` and the host has nothing to send.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]

pub mod chord;
pub mod encode;
pub mod key;
pub mod mouse;

pub use chord::{Chord, Key, Modifiers, ParseChordError};
pub use encode::{encode_focus, encode_key, encode_mouse, encode_paste};
pub use key::{KeyEvent, KeyKind};
pub use mouse::{MouseAction, MouseButton, MouseEvent};
