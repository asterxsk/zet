//! The grid plane: a theme is a ground, a default foreground and background, and
//! sixteen ANSI colours.
//!
//! Nothing in this module is used to draw chrome. DESIGN.md's rule is that the window
//! has two planes which never borrow from each other — the chrome has its own palette
//! in [`crate::palette`], and the grid gets exactly what the theme says. The split is
//! enforced by there being no function here that returns a chrome colour and none
//! there that returns an ANSI one.
//!
//! Three themes are zet's own and are held to a contrast floor, asserted in this
//! module's tests. The rest are imported and ship byte-for-byte as their authors
//! published them; they are marked [`Theme::published`] so that the settings panel can
//! say so rather than implying they were checked.

use zet_vt::{Color, ColorSpec};

use crate::rgb::Rgb;

/// How many colours an ANSI palette holds.
pub const ANSI_LEN: usize = 16;

/// The contrast floor zet's own themes are held to, on the body-text colours.
///
/// WCAG AA for normal text. Indices 0 and 8 are exempt because the floor is not
/// satisfiable for them and never was: a palette's "black" is the colour a program
/// paints a *background* with, and requiring it to be legible against a near-black
/// ground would force it to be light, at which point it is no longer black and the
/// programs that use it as a shadow stop working. Every other index is a colour a
/// program prints text in, and those all clear the floor.
pub const ANSI_CONTRAST_FLOOR: f32 = 4.5;

/// The indices exempt from [`ANSI_CONTRAST_FLOOR`].
pub const ANSI_EXEMPT: [usize; 2] = [0, 8];

/// A terminal colour scheme.
///
/// `'static` rather than owned because every theme is compiled in: the settings panel
/// offers a fixed list plus whatever a user drops in a file, and the latter is
/// converted to an owned [`Theme`] at load time through [`Theme::from_toml`]. Keeping
/// the shipped ones as constants means a theme cannot be mutated at runtime, which is
/// what makes it safe to hand a `&'static Theme` to the renderer every frame.
#[derive(Debug)]
pub struct Theme {
    /// The name shown in the settings panel.
    pub name: &'static str,
    /// The value written in the config file.
    pub slug: &'static str,
    /// The colour behind everything, including the chrome's own ground.
    pub ground: Rgb,
    /// `Color::DEFAULT` as a foreground.
    pub foreground: Rgb,
    /// `Color::DEFAULT` as a background.
    pub background: Rgb,
    /// The cursor, which follows the theme rather than the chrome palette because it
    /// sits on a cell.
    pub cursor: Rgb,
    /// The selection highlight behind a selected cell.
    pub selection: Rgb,
    /// The sixteen ANSI colours, in SGR order.
    pub ansi: [Rgb; ANSI_LEN],
    /// Whether this ships exactly as its author published it.
    pub published: bool,
}

impl Theme {
    /// The colour a cell should be painted, given what the grid stored.
    ///
    /// `background` decides which default applies when the cell says
    /// [`ColorSpec::Default`]; a foreground default and a background default are
    /// different colours and the cell does not record which field it is in.
    #[must_use]
    pub fn resolve(&self, color: Color, background: bool) -> Rgb {
        match color.spec() {
            ColorSpec::Default => {
                if background {
                    self.background
                } else {
                    self.foreground
                }
            }
            ColorSpec::Indexed(index) if usize::from(index) < ANSI_LEN => {
                self.ansi[usize::from(index)]
            }
            ColorSpec::Indexed(index) => {
                // The 6x6x6 cube and the grey ramp are fixed by the xterm
                // specification rather than by the theme, so they are computed.
                zet_vt::color::indexed_to_rgb(index)
                    .map_or(self.foreground, |(r, g, b)| Rgb::new(r, g, b))
            }
            ColorSpec::Rgb(r, g, b) => Rgb::new(r, g, b),
        }
    }

    /// The ANSI colours that are held to the contrast floor.
    pub fn audited_colors(&self) -> impl Iterator<Item = (usize, Rgb)> + '_ {
        self.ansi
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, _)| !ANSI_EXEMPT.contains(index))
    }
}

// ---------------------------------------------------------------------------------
// zet's own themes
// ---------------------------------------------------------------------------------

/// The default. Graphite ground, one amber signal, and an ANSI set chosen to be
/// neutral rather than saturated: the brief is an instrument, and an instrument's
/// indicator lamps are not primary colours.
pub static ZET_DARK: Theme = Theme {
    name: "zet dark",
    slug: "zet-dark",
    ground: Rgb::new(0x0a, 0x0b, 0x0d),
    foreground: Rgb::new(0xe7, 0xe9, 0xec),
    background: Rgb::new(0x0a, 0x0b, 0x0d),
    cursor: Rgb::new(0xff, 0xa6, 0x2b),
    selection: Rgb::new(0x23, 0x2a, 0x33),
    ansi: [
        Rgb::new(0x2b, 0x30, 0x38),
        Rgb::new(0xf0, 0x77, 0x6c),
        Rgb::new(0x8f, 0xbf, 0x7f),
        Rgb::new(0xe0, 0xb3, 0x57),
        Rgb::new(0x79, 0xa8, 0xd8),
        Rgb::new(0xc3, 0x9a, 0xc9),
        Rgb::new(0x6f, 0xbf, 0xbf),
        Rgb::new(0xc9, 0xce, 0xd6),
        Rgb::new(0x76, 0x7d, 0x87),
        Rgb::new(0xff, 0x9b, 0x90),
        Rgb::new(0xa8, 0xd6, 0x9a),
        Rgb::new(0xf5, 0xcd, 0x7a),
        Rgb::new(0x9c, 0xc4, 0xea),
        Rgb::new(0xdc, 0xb6, 0xe0),
        Rgb::new(0x8f, 0xd6, 0xd6),
        Rgb::new(0xee, 0xf1, 0xf5),
    ],
    published: false,
};

/// Warm paper. The same palette re-tuned for a light ground: the signal colour is
/// darkened because amber at `#ffa62b` on paper fails the floor, and every ANSI colour
/// is taken down to match.
pub static ZET_LIGHT: Theme = Theme {
    name: "zet light",
    slug: "zet-light",
    ground: Rgb::new(0xfa, 0xf9, 0xf7),
    foreground: Rgb::new(0x14, 0x16, 0x1a),
    background: Rgb::new(0xfa, 0xf9, 0xf7),
    cursor: Rgb::new(0xa3, 0x5f, 0x00),
    selection: Rgb::new(0xdd, 0xda, 0xd4),
    ansi: [
        Rgb::new(0x3a, 0x3d, 0x43),
        Rgb::new(0xb3, 0x2a, 0x1f),
        Rgb::new(0x3f, 0x6b, 0x2a),
        Rgb::new(0x8a, 0x5f, 0x00),
        Rgb::new(0x1f, 0x4f, 0x8a),
        Rgb::new(0x7a, 0x2f, 0x7a),
        Rgb::new(0x1a, 0x62, 0x66),
        Rgb::new(0x5c, 0x61, 0x69),
        Rgb::new(0x8a, 0x8f, 0x97),
        Rgb::new(0xd0, 0x3a, 0x2c),
        Rgb::new(0x46, 0x7a, 0x2c),
        Rgb::new(0x98, 0x68, 0x00),
        Rgb::new(0x27, 0x5f, 0xa8),
        Rgb::new(0x93, 0x3c, 0x93),
        Rgb::new(0x20, 0x75, 0x7a),
        Rgb::new(0x14, 0x16, 0x1a),
    ],
    published: false,
};

/// Pure black, pure white, and an ANSI set with as much headroom as the space allows.
/// Not the default, and not a theme in the same sense as the others: when Windows
/// reports forced colours, zet switches to this and ignores the user's choice, because
/// forced colours are an accessibility requirement rather than a preference.
pub static ZET_CONTRAST: Theme = Theme {
    name: "zet contrast",
    slug: "zet-contrast",
    ground: Rgb::BLACK,
    foreground: Rgb::WHITE,
    background: Rgb::BLACK,
    cursor: Rgb::WHITE,
    selection: Rgb::new(0x40, 0x40, 0x40),
    ansi: [
        Rgb::new(0x40, 0x40, 0x40),
        Rgb::new(0xff, 0x8a, 0x80),
        Rgb::new(0x8a, 0xff, 0x8a),
        Rgb::new(0xff, 0xe0, 0x6a),
        Rgb::new(0x8a, 0xbd, 0xff),
        Rgb::new(0xf0, 0xa0, 0xf0),
        Rgb::new(0x7a, 0xff, 0xff),
        Rgb::new(0xff, 0xff, 0xff),
        Rgb::new(0x9a, 0x9a, 0x9a),
        Rgb::new(0xff, 0xb3, 0xab),
        Rgb::new(0xb3, 0xff, 0xb3),
        Rgb::new(0xff, 0xef, 0xab),
        Rgb::new(0xb3, 0xd6, 0xff),
        Rgb::new(0xf8, 0xc8, 0xf8),
        Rgb::new(0xb3, 0xff, 0xff),
        Rgb::new(0xff, 0xff, 0xff),
    ],
    published: false,
};

/// Every theme zet ships, in the order the settings panel lists them.
///
/// The three zet themes come first because they are the ones held to the contrast
/// floor, and the imported ones follow in the order DESIGN.md names them.
static BUILTIN: [&Theme; 8] = [
    &ZET_DARK,
    &ZET_LIGHT,
    &ZET_CONTRAST,
    &crate::imported::NORD,
    &crate::imported::GRUVBOX_DARK,
    &crate::imported::TOKYO_NIGHT,
    &crate::imported::CATPPUCCIN_MOCHA,
    &crate::imported::SOLARIZED_LIGHT,
];

/// Every theme zet ships, in the order the settings panel lists them.
#[must_use]
pub fn builtin() -> &'static [&'static Theme] {
    &BUILTIN
}

/// The theme a config file gets when it names one that does not exist.
#[must_use]
pub fn default_theme() -> &'static Theme {
    &ZET_DARK
}

/// The theme forced colours switches to.
#[must_use]
pub fn contrast_theme() -> &'static Theme {
    &ZET_CONTRAST
}

/// Look a theme up by the value written in the config file.
#[must_use]
pub fn by_slug(slug: &str) -> Option<&'static Theme> {
    builtin().iter().copied().find(|theme| theme.slug == slug)
}

/// The suggested config value for a theme name, for the settings panel's readout.
#[must_use]
pub fn slug_of(name: &str) -> Option<&'static str> {
    builtin()
        .iter()
        .find(|theme| theme.name == name)
        .map(|theme| theme.slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_has_a_unique_slug_and_name() {
        let mut slugs: Vec<&str> = builtin().iter().map(|t| t.slug).collect();
        let mut names: Vec<&str> = builtin().iter().map(|t| t.name).collect();
        let (slug_count, name_count) = (slugs.len(), names.len());
        slugs.sort_unstable();
        slugs.dedup();
        names.sort_unstable();
        names.dedup();
        assert_eq!(slugs.len(), slug_count, "two themes share a slug");
        assert_eq!(names.len(), name_count, "two themes share a name");
    }

    #[test]
    fn every_theme_is_reachable_by_its_slug() {
        for theme in builtin() {
            assert!(
                by_slug(theme.slug).is_some_and(|found| found.name == theme.name),
                "{} is not reachable",
                theme.slug
            );
        }
    }

    #[test]
    fn zet_own_themes_clear_the_contrast_floor() {
        // PRODUCT.md's seventh success criterion, as a test rather than as a promise.
        for theme in builtin().iter().filter(|t| !t.published) {
            for (index, color) in theme.audited_colors() {
                let ratio = color.contrast_ratio(theme.background);
                assert!(
                    ratio >= ANSI_CONTRAST_FLOOR,
                    "{}: ANSI {index} {color:?} is {ratio:.2}:1 on {:?}",
                    theme.name,
                    theme.background
                );
            }
        }
    }

    #[test]
    fn zet_own_themes_keep_their_own_text_legible() {
        for theme in builtin().iter().filter(|t| !t.published) {
            let ratio = theme.foreground.contrast_ratio(theme.background);
            assert!(ratio >= 7.0, "{}: body text is {ratio:.2}:1", theme.name);
        }
    }

    #[test]
    fn imported_themes_are_marked_as_published() {
        for theme in builtin()
            .iter()
            .filter(|t| t.slug != "zet-dark" && t.slug != "zet-light")
        {
            if theme.slug.starts_with("zet-") {
                assert!(!theme.published, "{} is zet's own", theme.slug);
            } else {
                assert!(theme.published, "{} is imported", theme.slug);
            }
        }
    }

    #[test]
    fn the_exempt_indices_are_the_two_dims() {
        // Not a tautology: the exemption list is a claim about which colours a program
        // paints backgrounds with, and it is the thing a future edit is most likely to
        // widen so that a failing colour passes.
        assert_eq!(ANSI_EXEMPT, [0, 8]);
        assert_eq!(
            ZET_DARK.audited_colors().count(),
            ANSI_LEN - ANSI_EXEMPT.len()
        );
    }

    #[test]
    fn default_resolves_to_the_themes_own_foreground_and_background() {
        let theme = default_theme();
        assert_eq!(theme.resolve(Color::DEFAULT, false), theme.foreground);
        assert_eq!(theme.resolve(Color::DEFAULT, true), theme.background);
    }

    #[test]
    fn an_index_below_sixteen_is_the_theme_and_above_it_is_the_specification() {
        let theme = default_theme();
        assert_eq!(theme.resolve(Color::indexed(3), false), theme.ansi[3]);
        // 196 is pure red in the xterm cube, whatever the theme says about red.
        assert_eq!(
            theme.resolve(Color::indexed(196), false),
            Rgb::new(255, 0, 0)
        );
        assert_eq!(theme.resolve(Color::indexed(232), false), Rgb::new(8, 8, 8));
    }

    #[test]
    fn a_direct_colour_ignores_the_theme_entirely() {
        let theme = default_theme();
        assert_eq!(theme.resolve(Color::rgb(1, 2, 3), false), Rgb::new(1, 2, 3));
        assert_eq!(theme.resolve(Color::rgb(1, 2, 3), true), Rgb::new(1, 2, 3));
    }

    #[test]
    fn every_index_resolves_to_something() {
        // The renderer resolves a colour for every cell it paints, so no index may
        // panic or fall through to a placeholder.
        let theme = default_theme();
        for index in 0..=u8::MAX {
            let _ = theme.resolve(Color::indexed(index), false);
            let _ = theme.resolve(Color::indexed(index), true);
        }
    }
}
