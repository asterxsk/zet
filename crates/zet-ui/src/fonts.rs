//! The chrome's typeface, and the seam between the layout and whatever draws glyphs.
//!
//! # The face is self-hosted
//!
//! DESIGN.md fixes the chrome on IBM Plex Sans at two weights, and self-hosted means
//! exactly that: the files below are in this crate, registered into the font database
//! before anything is resolved, and the system is never asked whether it has Plex. A
//! machine that has never heard of the family draws the same titlebar as the machine
//! zet was designed on. It is also why the chrome does not scale with the terminal
//! font: the two are different faces with different jobs.
//!
//! # The seam
//!
//! [`zet_render::GlyphSource`] is the whole of "here is a character, tell me where its
//! pixels are", and the renderer's atlas already answers it. The chrome needs one
//! answer that trait does not carry — how big the face was rasterised — because every
//! text role in DESIGN.md is written as a size while a placement is in the source's own
//! pixels, and a bar of chrome text has to be centred on a line, which needs a baseline.
//! So [`GlyphSource`] here adds that single method and inherits the rest: one trait, one
//! implementation for the renderer to write, and no second vocabulary for glyphs.
//!
//! # One atlas, two faces
//!
//! The chrome does not own an atlas. The renderer's is the only one, and it holds the
//! grid's face and the chrome's side by side. That is not a compromise: [`zet_render::Atlas`]
//! is keyed by [`zet_font::GlyphKey`], which is a [`FaceKey`] and a glyph id, and a
//! `FaceKey` carries the family, the weight, and the italic flag. Two faces cannot
//! collide in it, which is exactly what the key's face field is for, so a shelf of Plex
//! and a shelf of Cascadia in one texture are two sets of unrelated coordinates rather
//! than one set that has to be told apart. Sharing is also the only thing that works: the
//! device binds one atlas texture for the whole pass, so a second own atlas would leave
//! the chrome's coordinates pointing into the grid's shelves.
//!
//! [`FaceKey`]: zet_font::FaceKey

use zet_config::FontSettings;
use zet_font::{FontError, FontStack, Metrics};

use crate::geometry::TAB_SIZE;

/// IBM Plex Sans Regular, weight 400.
pub const REGULAR: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");

/// IBM Plex Sans Medium, weight 500.
///
/// The heaviest the chrome is allowed to be. Nothing in it is bold.
pub const MEDIUM: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");

/// The family both of those belong to.
pub const FAMILY: &str = "IBM Plex Sans";

/// The size the chrome's text is rasterised at.
///
/// The largest size in DESIGN.md's type table, and therefore the one every other role
/// is a fraction of. Building the source at the biggest size keeps the fractions at or
/// below one, which is the direction a rasteriser degrades gracefully in.
pub const BASE_SIZE: f32 = TAB_SIZE;

/// What the layout needs from whatever draws glyphs.
///
/// `place` is the renderer's, and is the only way this crate gets pixels. This adds the
/// cell the face was built at, which is what turns a design size like "13px" into a
/// rectangle: the placement carries raster pixels, and where the top of a line and its
/// baseline are is not in it.
pub trait GlyphSource: zet_render::GlyphSource {
    /// The cell the source was rasterised against, in physical pixels.
    fn metrics(&self) -> &Metrics;
}

/// The settings the chrome's face is loaded from.
///
/// No fallbacks, for the reason this module already gives: chrome text is digits,
/// Latin letters and four punctuation marks, and a fallback would only be a way for a
/// system font to appear in the titlebar.
#[must_use]
pub fn settings() -> FontSettings {
    FontSettings {
        family: FAMILY.to_owned(),
        size: BASE_SIZE,
        fallback: Vec::new(),
    }
}

/// The chrome's typeface at a window scale.
///
/// The bytes are registered into the stack's own database rather than looked up in the
/// system's, which is what "self-hosted" means in practice: the machine does not have to
/// have Plex installed, and patching the family on disk cannot change the titlebar.
///
/// The scale is the window's DPI scale and belongs to the caller: a window that moves to
/// another monitor builds a new stack rather than papering over a stale cell size.
///
/// # Errors
///
/// [`FontError::NoSuchFamily`] cannot happen with the faces this crate ships, and
/// [`FontError::Unreadable`] would mean the binary's own font data is corrupt. Both are
/// returned rather than panicked on because this is called once at startup, where there
/// is a window to put a message in.
pub fn stack(scale: f32) -> Result<FontStack, FontError> {
    FontStack::load_embedded(&settings(), scale, &[REGULAR, MEDIUM])
}
