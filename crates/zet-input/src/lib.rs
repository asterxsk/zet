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
//! One flag of the five is deliberately not implemented: `Report alternate keys`
//! asks for the key at the same position on the base layout beside every key, and
//! the host does not carry the physical key that would come from. A terminal that
//! cannot do something is supposed to say so rather than say nothing — the protocol
//! has a program set the flags it wants and then query which it got, exactly so that
//! a partial implementation is discoverable — so the bit is dropped when it arrives
//! and never reported back.

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
