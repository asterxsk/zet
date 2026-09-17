//! Themes that ship exactly as their authors published them.
//!
//! These are not audited against zet's contrast floor and are not adjusted to fit the
//! Instrument chrome. A palette that has been recoloured is no longer the palette
//! somebody chose, and the point of shipping Nord is that it is Nord. The settings
//! panel labels them "as published" for that reason, and [`Theme::published`] is what
//! it reads to know.
//!
//! Every value here comes from the palette's own upstream source. That is why they are
//! written out as literal hex rather than derived: there is nothing to derive, and a
//! computation would only be a place for a transcription error to hide.

use crate::rgb::Rgb;
use crate::theme::Theme;

/// Nord, by Arctic Ice Studio and Sven Greb.
///
/// Source: `nordtheme/nord`, `src/nord.scss`. The terminal mapping is the one Nord's
/// own documentation publishes, which reuses the four accent colours across both the
/// normal and bright halves rather than inventing brighter variants.
pub static NORD: Theme = Theme {
    name: "Nord",
    slug: "nord",
    ground: Rgb::new(0x2e, 0x34, 0x40),
    foreground: Rgb::new(0xd8, 0xde, 0xe9),
    background: Rgb::new(0x2e, 0x34, 0x40),
    cursor: Rgb::new(0xd8, 0xde, 0xe9),
    selection: Rgb::new(0x43, 0x4c, 0x5e),
    ansi: [
        Rgb::new(0x3b, 0x42, 0x52),
        Rgb::new(0xbf, 0x61, 0x6a),
        Rgb::new(0xa3, 0xbe, 0x8c),
        Rgb::new(0xeb, 0xcb, 0x8b),
        Rgb::new(0x81, 0xa1, 0xc1),
        Rgb::new(0xb4, 0x8e, 0xad),
        Rgb::new(0x88, 0xc0, 0xd0),
        Rgb::new(0xe5, 0xe9, 0xf0),
        Rgb::new(0x4c, 0x56, 0x6a),
        Rgb::new(0xbf, 0x61, 0x6a),
        Rgb::new(0xa3, 0xbe, 0x8c),
        Rgb::new(0xeb, 0xcb, 0x8b),
        Rgb::new(0x81, 0xa1, 0xc1),
        Rgb::new(0xb4, 0x8e, 0xad),
        Rgb::new(0x8f, 0xc0, 0xd0),
        Rgb::new(0xec, 0xef, 0xf4),
    ],
    published: true,
};

/// Gruvbox dark, by Pavel Pertsev.
///
/// Source: `morhetz/gruvbox`, the `g:terminal_color_*` block in `colors/gruvbox.vim`.
/// The "dark" variant with the *medium* background, which is the one the scheme's own
/// screenshots show.
pub static GRUVBOX_DARK: Theme = Theme {
    name: "Gruvbox dark",
    slug: "gruvbox-dark",
    ground: Rgb::new(0x28, 0x28, 0x28),
    foreground: Rgb::new(0xeb, 0xdb, 0xb2),
    background: Rgb::new(0x28, 0x28, 0x28),
    cursor: Rgb::new(0xeb, 0xdb, 0xb2),
    // `s:bg3`, the literal background of the `Visual` highlight. Gruvbox inverts its
    // selection by default, so the rendered result is this colour as *text* on
    // `#ebdbb2`; zet paints a selection the ordinary way round and takes the token,
    // which is the same colour either way.
    selection: Rgb::new(0x66, 0x5c, 0x54),
    ansi: [
        Rgb::new(0x28, 0x28, 0x28),
        Rgb::new(0xcc, 0x24, 0x1d),
        Rgb::new(0x98, 0x97, 0x1a),
        Rgb::new(0xd7, 0x99, 0x21),
        Rgb::new(0x45, 0x85, 0x88),
        Rgb::new(0xb1, 0x62, 0x86),
        Rgb::new(0x68, 0x9d, 0x6a),
        Rgb::new(0xa8, 0x99, 0x84),
        Rgb::new(0x92, 0x83, 0x74),
        Rgb::new(0xfb, 0x49, 0x34),
        Rgb::new(0xb8, 0xbb, 0x26),
        Rgb::new(0xfa, 0xbd, 0x2f),
        Rgb::new(0x83, 0xa5, 0x98),
        Rgb::new(0xd3, 0x86, 0x9b),
        Rgb::new(0x8e, 0xc0, 0x7c),
        Rgb::new(0xeb, 0xdb, 0xb2),
    ],
    published: true,
};

/// Tokyo Night, by Folke Lemaitre.
///
/// Source: `folke/tokyonight.nvim`, `extras/kitty/tokyonight_night.conf`. The kitty
/// and ghostty extras agree on every one of the twenty values, which matters because
/// the plugin's own `lua/` files do not contain a terminal palette at all — the
/// ANSI table is computed at runtime by blending and brightening the base hues, so
/// there is nothing to read out of them.
///
/// The `night` variant is the one shipped. Storm and moon are different themes that
/// share the name; a picker offering three rows called "Tokyo Night" is worse than one
/// offering the canonical one.
pub static TOKYO_NIGHT: Theme = Theme {
    name: "Tokyo Night",
    slug: "tokyo-night",
    ground: Rgb::new(0x1a, 0x1b, 0x26),
    foreground: Rgb::new(0xc0, 0xca, 0xf5),
    background: Rgb::new(0x1a, 0x1b, 0x26),
    cursor: Rgb::new(0xc0, 0xca, 0xf5),
    selection: Rgb::new(0x28, 0x34, 0x57),
    ansi: [
        Rgb::new(0x15, 0x16, 0x1e),
        Rgb::new(0xf7, 0x76, 0x8e),
        Rgb::new(0x9e, 0xce, 0x6a),
        Rgb::new(0xe0, 0xaf, 0x68),
        Rgb::new(0x7a, 0xa2, 0xf7),
        Rgb::new(0xbb, 0x9a, 0xf7),
        Rgb::new(0x7d, 0xcf, 0xff),
        Rgb::new(0xa9, 0xb1, 0xd6),
        Rgb::new(0x41, 0x48, 0x68),
        // The bright half is genuinely brighter here, computed upstream by an HSLuv
        // lightening pass rather than being the base hues repeated.
        Rgb::new(0xff, 0x89, 0x9d),
        Rgb::new(0x9f, 0xe0, 0x44),
        Rgb::new(0xfa, 0xba, 0x4a),
        Rgb::new(0x8d, 0xb0, 0xff),
        Rgb::new(0xc7, 0xa9, 0xff),
        Rgb::new(0xa4, 0xda, 0xff),
        Rgb::new(0xc0, 0xca, 0xf5),
    ],
    published: true,
};

/// Catppuccin Mocha, by the Catppuccin contributors.
///
/// Source: `catppuccin/kitty`, `themes/mocha.conf`, cross-checked against
/// `catppuccin/alacritty`. Both official ports agree on all twenty values.
///
/// The plugin's own `groups/terminal.lua` disagrees with them, and so does the style
/// guide in one place — it documents white as Subtext 0 and bright white as Subtext 1,
/// which is the reverse of what both ports ship. The ports win: this theme's job is to
/// look like the Catppuccin people already have.
///
/// The four flavours share a name for every colour and differ only in value, so the
/// flavour is part of the theme's identity rather than a setting.
pub static CATPPUCCIN_MOCHA: Theme = Theme {
    name: "Catppuccin Mocha",
    slug: "catppuccin-mocha",
    ground: Rgb::new(0x1e, 0x1e, 0x2e),
    foreground: Rgb::new(0xcd, 0xd6, 0xf4),
    background: Rgb::new(0x1e, 0x1e, 0x2e),
    cursor: Rgb::new(0xf5, 0xe0, 0xdc),
    // Rosewater, which is what both of Catppuccin's own terminal ports ship, and it is
    // startling the first time. The style guide instead specifies overlay2 at 20-30%
    // opacity, which is not a thing a single hex value can express; shipping the
    // ports' opaque value keeps this palette the one people already have rather than a
    // zet-flavoured approximation of it.
    selection: Rgb::new(0xf5, 0xe0, 0xdc),
    ansi: [
        Rgb::new(0x45, 0x47, 0x5a),
        Rgb::new(0xf3, 0x8b, 0xa8),
        Rgb::new(0xa6, 0xe3, 0xa1),
        Rgb::new(0xf9, 0xe2, 0xaf),
        Rgb::new(0x89, 0xb4, 0xfa),
        Rgb::new(0xf5, 0xc2, 0xe7),
        Rgb::new(0x94, 0xe2, 0xd5),
        Rgb::new(0xba, 0xc2, 0xde),
        Rgb::new(0x58, 0x5b, 0x70),
        Rgb::new(0xf3, 0x8b, 0xa8),
        Rgb::new(0xa6, 0xe3, 0xa1),
        Rgb::new(0xf9, 0xe2, 0xaf),
        Rgb::new(0x89, 0xb4, 0xfa),
        Rgb::new(0xf5, 0xc2, 0xe7),
        Rgb::new(0x94, 0xe2, 0xd5),
        Rgb::new(0xa6, 0xad, 0xc8),
    ],
    published: true,
};

/// Solarized Light, by Ethan Schoonover.
///
/// Source: `altercation/solarized`. The accent hexes come from `xresources/solarized`,
/// which is also where every other port gets them; the light variant's slot assignment
/// comes from the 16-colour table in `vim-colors-solarized/colors/solarized.vim`,
/// resolved through that file's own light-mode base swap, because `xresources` only
/// carries the light values in a commented-out block and its live `*colorN` lines are
/// the dark mapping.
///
/// The Apple Terminal and iTerm light profiles in the same repository confirm the
/// result independently, which is how this one ended up as the only imported theme
/// whose values were partly reconstructed rather than read straight out.
///
/// The accent colours are identical between the light and dark variants by design —
/// only the base tones swap — which is why this theme's ANSI 0 is `base02` rather than
/// anything resembling black.
pub static SOLARIZED_LIGHT: Theme = Theme {
    name: "Solarized Light",
    slug: "solarized-light",
    ground: Rgb::new(0xfd, 0xf6, 0xe3),
    foreground: Rgb::new(0x65, 0x7b, 0x83),
    background: Rgb::new(0xfd, 0xf6, 0xe3),
    cursor: Rgb::new(0x65, 0x7b, 0x83),
    selection: Rgb::new(0xee, 0xe8, 0xd5),
    ansi: [
        Rgb::new(0x07, 0x36, 0x42),
        Rgb::new(0xdc, 0x32, 0x2f),
        Rgb::new(0x85, 0x99, 0x00),
        Rgb::new(0xb5, 0x89, 0x00),
        Rgb::new(0x26, 0x8b, 0xd2),
        Rgb::new(0xd3, 0x36, 0x82),
        Rgb::new(0x2a, 0xa1, 0x98),
        Rgb::new(0xee, 0xe8, 0xd5),
        Rgb::new(0x00, 0x2b, 0x36),
        Rgb::new(0xcb, 0x4b, 0x16),
        Rgb::new(0x58, 0x6e, 0x75),
        Rgb::new(0x65, 0x7b, 0x83),
        Rgb::new(0x83, 0x94, 0x96),
        Rgb::new(0x6c, 0x71, 0xc4),
        Rgb::new(0x93, 0xa1, 0xa1),
        Rgb::new(0xfd, 0xf6, 0xe3),
    ],
    published: true,
};
