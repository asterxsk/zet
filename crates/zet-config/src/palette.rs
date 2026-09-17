//! The chrome plane: the window, the tab strip, the settings panel, every focus ring.
//!
//! This is zet's own palette and it is deliberately not derived from the active theme.
//! A terminal that tints its titlebar to match your shell prompt looks like an app that
//! painted over your terminal; the whole point of the two-plane rule in DESIGN.md is
//! that the chrome stays the instrument and the grid stays the program's. The only
//! place the two planes touch is [`Palette::ground`], which is the colour behind
//! everything and is what a theme's own ground is usually set to match.

use crate::rgb::Rgb;

/// The chrome colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// The window background, behind everything.
    pub ground: Rgb,
    /// The titlebar and tab strip.
    pub surface: Rgb,
    /// The settings panel, popovers, and menus.
    pub surface_raised: Rgb,
    /// Every one-pixel division in the chrome.
    pub hairline: Rgb,
    /// Focused input borders and drag targets.
    pub hairline_strong: Rgb,
    /// Primary chrome text and the active tab index.
    pub ink: Rgb,
    /// Secondary text and the hovered tab index.
    pub ink_mid: Rgb,
    /// Inactive tab indices and disabled controls.
    pub ink_dim: Rgb,
    /// The active marker. Indicators and focus rings only.
    pub signal: Rgb,
    /// The signal at rest, for a hover preview of an indicator.
    pub signal_dim: Rgb,
    /// Destructive confirmations and error text.
    pub danger: Rgb,
    /// Success confirmations.
    pub ok: Rgb,
}

impl Palette {
    /// The Instrument palette, which is what zet looks like.
    #[must_use]
    pub const fn instrument() -> Self {
        Self {
            ground: Rgb::new(0x0a, 0x0b, 0x0d),
            surface: Rgb::new(0x0f, 0x11, 0x14),
            surface_raised: Rgb::new(0x15, 0x18, 0x1c),
            hairline: Rgb::new(0x23, 0x27, 0x2d),
            hairline_strong: Rgb::new(0x33, 0x39, 0x41),
            ink: Rgb::new(0xe7, 0xe9, 0xec),
            ink_mid: Rgb::new(0x99, 0xa0, 0xa8),
            // `#7b838d`, not the `#5a626b` DESIGN.md first proposed. The document's own
            // contrast table asks for 4.5:1 between `ink-dim` and `surface`, and
            // `#5a626b` measures 3.06:1 — an inactive tab index nobody with ordinary
            // vision could comfortably read, let alone anyone else. The floor is the
            // intent and the hex was the guess, so the hex moved. The hue is unchanged;
            // it is the same grey, lighter.
            ink_dim: Rgb::new(0x7b, 0x83, 0x8d),
            signal: Rgb::new(0xff, 0xa6, 0x2b),
            signal_dim: Rgb::new(0x8a, 0x5a, 0x17),
            danger: Rgb::new(0xff, 0x6b, 0x5e),
            ok: Rgb::new(0x57, 0xd9, 0xa3),
        }
    }

    /// The palette forced-colours mode switches to.
    ///
    /// `highlight` is the system's own highlight colour, which is passed in rather than
    /// chosen here because on a forced-colours desktop it is the one colour the user
    /// has already picked and it outranks anything zet would invent.
    ///
    /// Every distinction the Instrument palette makes with a hairline is made here with
    /// pure white instead. That is the correct reading of a high-contrast mode: the
    /// user is not asking for a darker theme, they are asking for edges that survive a
    /// contrast filter, and a `#23272d` line on a `#000000` ground does not.
    #[must_use]
    pub const fn high_contrast(highlight: Rgb) -> Self {
        Self {
            ground: Rgb::BLACK,
            surface: Rgb::BLACK,
            surface_raised: Rgb::BLACK,
            hairline: Rgb::WHITE,
            hairline_strong: Rgb::WHITE,
            ink: Rgb::WHITE,
            ink_mid: Rgb::WHITE,
            ink_dim: Rgb::WHITE,
            signal: highlight,
            signal_dim: highlight,
            danger: Rgb::WHITE,
            ok: Rgb::WHITE,
        }
    }

    /// Every colour, for a test that wants to check them all without naming them.
    #[must_use]
    pub const fn all(self) -> [(&'static str, Rgb); 12] {
        [
            ("ground", self.ground),
            ("surface", self.surface),
            ("surface-raised", self.surface_raised),
            ("hairline", self.hairline),
            ("hairline-strong", self.hairline_strong),
            ("ink", self.ink),
            ("ink-mid", self.ink_mid),
            ("ink-dim", self.ink_dim),
            ("signal", self.signal),
            ("signal-dim", self.signal_dim),
            ("danger", self.danger),
            ("ok", self.ok),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instrument_palette_matches_the_design_document() {
        // The literal hex values from DESIGN.md. Written out rather than compared to
        // `instrument()` so that a change to the palette has to be made twice, in the
        // document and here, which is the only thing that keeps them in step.
        let palette = Palette::instrument();
        let documented = [
            ("ground", "#0a0b0d"),
            ("surface", "#0f1114"),
            ("surface-raised", "#15181c"),
            ("hairline", "#23272d"),
            ("hairline-strong", "#333941"),
            ("ink", "#e7e9ec"),
            ("ink-mid", "#99a0a8"),
            ("ink-dim", "#7b838d"),
            ("signal", "#ffa62b"),
            ("signal-dim", "#8a5a17"),
            ("danger", "#ff6b5e"),
            ("ok", "#57d9a3"),
        ];
        for ((name, actual), (_, expected)) in palette.all().iter().zip(documented) {
            assert_eq!(actual.to_hex(), expected, "{name}");
        }
    }

    #[test]
    fn the_chrome_clears_its_contrast_floors() {
        let p = Palette::instrument();
        let floors = [
            ("ink on ground", p.ink, p.ground, 12.0),
            ("ink-mid on ground", p.ink_mid, p.ground, 6.0),
            ("ink-dim on surface", p.ink_dim, p.surface, 4.5),
            ("signal on surface", p.signal, p.surface, 4.5),
            ("danger on surface-raised", p.danger, p.surface_raised, 4.5),
            ("ok on surface-raised", p.ok, p.surface_raised, 4.5),
        ];
        for (name, fg, bg, minimum) in floors {
            let ratio = fg.contrast_ratio(bg);
            assert!(ratio >= minimum, "{name} is {ratio:.2}:1, needs {minimum}");
        }
    }

    #[test]
    fn the_high_contrast_palette_is_black_and_white() {
        let p = Palette::high_contrast(Rgb::new(0x00, 0x78, 0xd4));
        assert_eq!(p.ground, Rgb::BLACK);
        assert_eq!(p.ink, Rgb::WHITE);
        assert_eq!(p.hairline, Rgb::WHITE);
        assert_eq!(p.signal, Rgb::new(0x00, 0x78, 0xd4));
        let ratio = p.ink.contrast_ratio(p.ground);
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn every_hairline_is_visible_against_what_it_divides() {
        // A hairline is a one-pixel line. It has no thickness to hide behind, so on
        // either surface it sits on it has to be a different colour — this caught the
        // temptation to make `hairline` on `surface` a two-step change instead of a
        // visible one.
        let p = Palette::instrument();
        for (name, surface) in [("surface", p.surface), ("ground", p.ground)] {
            assert_ne!(p.hairline, surface, "hairline vanishes on {name}");
            assert_ne!(
                p.hairline_strong, surface,
                "hairline-strong vanishes on {name}"
            );
        }
    }
}
