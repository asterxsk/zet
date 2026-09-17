//! Turning a terminal and an app into a frame.
//!
//! Two things end up on the screen and they never borrow from each other. The **grid**
//! is the program's: character cells, their foregrounds and backgrounds, the selection,
//! the cursor, all drawn from the active theme's ANSI palette. The **chrome** is zet's:
//! the tab strip, the settings panel, the scrollbar, every hairline, all drawn from the
//! app palette. [DESIGN.md] calls this the rule that keeps zet from looking like an app
//! that painted over your terminal, and it is enforced before anything gets here —
//! [`zet_config::Theme`] cannot return a chrome colour and [`zet_config::Palette`]
//! cannot return a theme colour.
//!
//! What this crate does is turn both of those into the same two arrays of rectangles.
//! It owns the atlas, the glyph cache, and the device; it does not own the layout, which
//! is [`zet_ui`]'s job, or the state, which is [`zet_app`]'s.
//!
//! [DESIGN.md]: https://github.com/asterxsk/zet/blob/main/DESIGN.md
//! [`zet_ui`]: https://github.com/asterxsk/zet
//! [`zet_app`]: https://github.com/asterxsk/zet

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
// Every integer in this crate is a coordinate or a texture offset and every float is a
// distance in physical pixels, so the four cast lints fire on nearly every line that
// turns one into the other. They are worth having almost everywhere else and worth
// nothing here: a cell index is bounded by the window's size, a glyph is bounded by the
// atlas, and a pixel count is far below the 2^24 where `f32` starts rounding. Annotating
// each one would be dozens of attributes saying "a terminal is smaller than 16 million
// pixels wide", which is not a fact anyone needs told.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

pub mod atlas;
pub mod frame;
pub mod gpu;
pub mod grid;
pub mod renderer;

pub use atlas::{Atlas, Placement};
pub use frame::{Batch, BatchKind, Frame, GlyphQuad, Quad, glyph_flags};
pub use gpu::{Gpu, GpuError, GpuResult};
pub use grid::{Cursor, GlyphSource, Selection, View, draw_grid};
pub use renderer::{ChromeGlyphs, Renderer, RendererError};
