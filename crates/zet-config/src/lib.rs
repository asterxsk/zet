//! zet's configuration file, its shipped themes, and the chrome palette.
//!
//! This crate is the shared vocabulary for everything above it. It owns three things
//! that would otherwise be redefined in each of them and drift:
//!
//! - [`Rgb`], the colour type, with the sRGB-to-linear conversion the renderer needs
//!   and the WCAG contrast arithmetic the design system is held to.
//! - [`Theme`] and [`Palette`], which are the two planes DESIGN.md keeps apart. A theme
//!   is the grid's; a palette is the chrome's. Nothing here lets one borrow the other.
//! - [`Config`], the file, its defaults, its diagnostics, and the round-trip that
//!   preserves a user's comments.
//!
//! # The file is authoritative
//!
//! PRODUCT.md settles the settings question in one line: the config file is
//! authoritative and the settings panel is a view over it. This crate is therefore the
//! only writer. The panel does not hold state — it edits a [`Config`] and asks this
//! crate to save it, and the hot-reload path reads the same file back. That is what
//! makes "change it in the app" and "change it by hand" the same operation rather than
//! two code paths that have to be kept in agreement.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::all, clippy::pedantic)]
// `struct_excessive_bools` fires on `Config`'s sections, which are groups of switches
// by definition. Splitting a settings section into one type per flag would make the
// file harder to read, not easier, and the lint has no other reading of the word
// "excessive" to offer.
#![allow(clippy::struct_excessive_bools)]

pub mod config;
pub mod imported;
pub mod palette;
pub mod rgb;
pub mod theme;

pub use config::{
    ACTIONS, Appearance, Background, Config, CursorSettings, CursorShape, Diagnostic, FontSettings,
    Loaded, Severity, TabPosition, TabSettings, UpdateSettings, WindowSettings, default_keymap,
    default_path, load, load_default, palette_for, repaired, save,
};
pub use palette::Palette;
pub use rgb::Rgb;
pub use theme::{ANSI_LEN, Theme, builtin, by_slug, contrast_theme, default_theme};

/// Something that went wrong reading or writing the configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The file involved.
        path: std::path::PathBuf,
        /// What the filesystem said.
        #[source]
        source: std::io::Error,
    },

    /// The file is not valid TOML, or holds a value the schema does not accept.
    ///
    /// Carries the rendered `toml_edit` diagnostic, which names the line and the
    /// offending span. That text is the whole value of the variant: a config error
    /// without a line number is a support request.
    #[error("{path}: {message}")]
    Parse {
        /// The file involved.
        path: std::path::PathBuf,
        /// The parser's own rendering of what is wrong and where.
        message: String,
    },

    /// The configuration directory could not be determined.
    #[error("could not determine where to keep the configuration")]
    NoConfigDir,
}
