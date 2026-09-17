//! Font discovery, metrics, and glyph rasterisation.
//!
//! This crate answers three questions and nothing else: which faces does this machine
//! have, how big is a cell, and what does one character look like as pixels.
//!
//! # Why this is not one of the font crates that already exist
//!
//! `cosmic-text` and `swash`'s own shaping layer both solve "lay out a paragraph",
//! which is not the problem a terminal has. A terminal's grid is one glyph per cell at
//! a fixed advance width; text does not wrap around a float, does not need kerning
//! across a cell boundary, and must never ligate two characters into one glyph unless
//! the user asked for it. What a terminal does need, and what general-purpose text
//! stacks are slow at, is resolving thousands of individual characters through a
//! fallback chain and rasterising the ones that are not already in an atlas.
//!
//! So this crate uses `fontique` for the database half and `swash` for the pixels half
//! and supplies the policy itself: a stack, a per-character resolution cache, and cell
//! metrics derived from the primary face rather than from whatever font happened to
//! answer for a given character.
//!
//! # The rule that keeps the grid a grid
//!
//! **Every cell is the width of the primary face's advance for `M`, forever.** A
//! fallback face that is proportional, or monospaced at a different pitch, draws its
//! glyph centred in the cell and is allowed to overflow it. Scaling a fallback to fit
//! would be worse: a CJK ideograph squeezed into a Latin cell is unreadable, and an
//! emoji squeezed into one is a smudge. Overflow is what every terminal does and it is
//! the right answer, because the alternative moves the cell boundaries and a grid whose
//! columns move is not a grid.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]

pub mod glyph;
pub mod library;
pub mod metrics;
pub mod stack;

pub use glyph::{Glyph, GlyphContent, GlyphKey, GlyphSpec};
pub use library::{Face, FaceKey, FontLibrary, Weight};
pub use metrics::Metrics;
pub use stack::FontStack;

/// A font family's identifier within the system database.
///
/// Re-exported rather than referred to through `fontique`, because it is already part
/// of [`FaceKey`] and therefore already part of this crate's public API. A caller that
/// has to name the type to key a cache on a [`GlyphKey`] should not have to depend on
/// the database crate's version to do it.
pub use fontique::FamilyId;

/// Something went wrong finding or loading a font.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    /// The configured family is not installed.
    #[error("no font family named {0:?} is installed")]
    NoSuchFamily(String),

    /// The family exists but has no face the rasteriser can read.
    ///
    /// A real outcome: a family entry in the registry can point at a file that has
    /// been removed, and a variable font with no default instance can have nothing at
    /// the requested weight.
    #[error("{family:?} has no face that could be loaded")]
    Unreadable {
        /// Which family it was.
        family: String,
    },

    /// The font size is not a size.
    #[error("a font size of {0} is not usable")]
    BadSize(f32),
}
