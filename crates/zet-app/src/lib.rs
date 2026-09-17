//! What zet does when you press a key, with no window in sight.
//!
//! This crate is the terminal's behaviour with every platform dependency taken out. It
//! owns the tabs, the selection, the scroll position, the keymap, and the cursor's blink,
//! and it decides what a key press means. It does not own a window, a GPU, a clipboard,
//! or an event loop, and it never learns that any of those exist: when it needs one it
//! returns a [`Command`] and the caller carries it out.
//!
//! # Why the split is here
//!
//! The alternative — an app that owns the window and is called by it — is the shape most
//! terminals have, and it makes the interesting half of the program untestable without a
//! display. Every decision below is a pure function of state that a test can set up:
//! open two tabs, press a key, assert which one is active. That is the whole reason the
//! window is not in this file.
//!
//! # Layers
//!
//! ```text
//! zet-session   one terminal: a child process, a parser, a grid
//! zet-render    one frame: what the grid looks like as vertices
//! zet-app       this crate: what a key does to the grid, and nothing about pixels
//! zet-ui        the chrome around the grid: titlebar, tabs, settings panel
//! zet           the binary: a window, a device, and an event loop
//! ```
//!
//! The dependency arrow never points back down. That is what lets [`App`] be driven by a
//! test, and it is why [`zet_ui`] does not appear in this crate's manifest even though
//! the two are screens of the same window.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
// The grid is a few hundred cells of `usize` coordinates that have to become `u16` for
// the parser and `f32` for the renderer at every step. Each of those conversions is
// proven in range by the grid it came from, so annotating them would be a comment per
// line saying "a terminal is smaller than 65535 columns", which is not a fact anyone
// needs told.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

pub mod action;
pub mod app;
pub mod find;
pub mod settings;
pub mod text;

pub use action::Action;
pub use app::{App, AppError, BLINK_INTERVAL, Command};
pub use find::Find;
pub use settings::{Effect, Id, Kind, Line};
pub use text::selection_text;
