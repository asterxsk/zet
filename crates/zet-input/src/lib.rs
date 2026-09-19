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
//! # The kitty keyboard protocol
//!
//! [`encode_key`] speaks the kitty keyboard protocol as well as the legacy one, and
//! which it speaks is not its decision: the flags arrive on [`zet_vt::Modes`], which
//! is where the terminal keeps the stack a program pushes and pops. A program that
//! asked for nothing gets the legacy bytes, which is every program that existed
//! before this protocol did.
//!
//! What the protocol buys is the four things the legacy encoding cannot say: that a
//! key went up, that it is repeating rather than being pressed again, that an Escape
//! is a key and not the start of a sequence, and what text a key produced alongside
//! the key itself. [`crate::encode`]'s `kitty_key` is where that lives and it is
//! written against the specification's own tables rather than against what other
//! terminals do.
//!
//! All five of the flags are implemented. `Report alternate keys` is the one that
//! needs the host's help: the key at the same position on the base layout is a fact
//! about the physical switch, so [`KeyEvent::base`] carries it and
//! [`KeyEvent::unshifted`] carries the key the current layout produces with no
//! modifiers. Both are written as sub-fields of the key's own sequence — see
//! `crate::encode`'s `alternate_keys` — and a terminal that could not honour a flag is
//! meant to drop it from what it reports back rather than claim it, which is the
//! handshake the protocol is built around.

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
