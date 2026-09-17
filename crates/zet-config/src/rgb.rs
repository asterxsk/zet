//! An eight-bit-per-channel colour, and the contrast arithmetic the design system is
//! held to.
//!
//! Everything in zet that is not a terminal cell goes through this type: the chrome
//! palette, the themes' own accent colours, the settings panel's swatches. It exists
//! rather than a bare `[u8; 3]` because three things have to be true of every colour
//! and all three are easy to get wrong at a call site: it must be readable from the
//! hex form the design document is written in, it must be convertible to the linear
//! space the GPU blends in, and it must be able to report its contrast ratio so that
//! the accessibility floor can be asserted in a test rather than eyeballed.

use core::fmt;

/// A colour in the sRGB space, eight bits per channel.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// Black, which is not the same as "unset" anywhere in this crate.
    pub const BLACK: Rgb = Rgb::new(0, 0, 0);

    /// White.
    pub const WHITE: Rgb = Rgb::new(0xff, 0xff, 0xff);

    /// Build a colour from its three channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse `#rrggbb` or `rrggbb`.
    ///
    /// The leading `#` is optional because the design document writes colours with it
    /// and every theme file in the wild is split about whether it uses one.
    ///
    /// Returns `None` for anything else: three- or four-digit shorthand, eight-digit
    /// alpha forms, and named colours are all deliberately unsupported, since accepting
    /// them here would mean a config file could contain a colour that means something
    /// different from what the design system specifies.
    #[must_use]
    pub const fn from_hex(hex: &str) -> Option<Self> {
        let bytes = hex.as_bytes();
        let digits = if let [b'#', rest @ ..] = bytes {
            rest
        } else {
            bytes
        };
        if digits.len() != 6 {
            return None;
        }
        // Written out rather than built in a loop because a `const fn` cannot index a
        // slice without proving the index in range at every step, and three pairs of
        // digits are cheaper to read than the proof.
        let r = match (nibble(digits[0]), nibble(digits[1])) {
            (Some(hi), Some(lo)) => (hi << 4) | lo,
            _ => return None,
        };
        let g = match (nibble(digits[2]), nibble(digits[3])) {
            (Some(hi), Some(lo)) => (hi << 4) | lo,
            _ => return None,
        };
        let b = match (nibble(digits[4]), nibble(digits[5])) {
            (Some(hi), Some(lo)) => (hi << 4) | lo,
            _ => return None,
        };
        Some(Self { r, g, b })
    }

    /// The three channels, in order.
    #[must_use]
    pub const fn to_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    /// The `#rrggbb` form, for the settings panel and for writing a theme back out.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// The colour in linear space, which is what a shader has to blend in.
    ///
    /// A surface configured as `Bgra8UnormSrgb` converts on the way in and on the way
    /// out, so a constant handed to a shader is read as linear. Passing sRGB numbers
    /// straight through and blending them there is the bug that makes a 50% white
    /// overlay come out at the wrong grey, and it gets worse the more transparent the
    /// window is, so the conversion belongs here rather than at each use.
    #[must_use]
    pub fn to_linear(self) -> [f32; 4] {
        [
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
            1.0,
        ]
    }

    /// The colour in linear space at the given alpha.
    #[must_use]
    pub fn to_linear_alpha(self, alpha: f32) -> [f32; 4] {
        let [r, g, b, _] = self.to_linear();
        [r, g, b, alpha]
    }

    /// WCAG relative luminance.
    #[must_use]
    pub fn relative_luminance(self) -> f32 {
        let [r, g, b] = [self.r, self.g, self.b].map(channel_luminance);
        0.2126f32.mul_add(r, 0.7152f32.mul_add(g, 0.0722 * b))
    }

    /// The WCAG contrast ratio against `other`, from 1.0 to 21.0.
    ///
    /// Always at least 1.0 and always reported against the lighter of the two, so the
    /// order of the arguments never matters.
    #[must_use]
    pub fn contrast_ratio(self, other: Rgb) -> f32 {
        let (a, b) = (self.relative_luminance(), other.relative_luminance());
        let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }

    /// Whether this colour clears `minimum` against `other`.
    #[must_use]
    pub fn contrasts_with(self, other: Rgb, minimum: f32) -> bool {
        self.contrast_ratio(other) >= minimum
    }
}

impl fmt::Debug for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl serde::Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> serde::Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Reads the colour without asking the deserializer to lend a `&str`.
        ///
        /// A `&str` would be cheaper and does not work: a TOML deserializer builds its
        /// value as it walks the source and cannot always hand out a borrow, so
        /// deserializing into one fails with "expected a borrowed string" on exactly
        /// the config files that have a colour in them.
        struct Hex;

        impl serde::de::Visitor<'_> for Hex {
            type Value = Rgb;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a colour as six hex digits, like \"#0a0b0d\"")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Rgb, E> {
                Rgb::from_hex(value).ok_or_else(|| {
                    E::custom(format!(
                        "{value:?} is not a colour; expected six hex digits like \"#0a0b0d\""
                    ))
                })
            }
        }

        deserializer.deserialize_str(Hex)
    }
}

impl From<[u8; 3]> for Rgb {
    fn from([r, g, b]: [u8; 3]) -> Self {
        Self { r, g, b }
    }
}

impl From<Rgb> for [u8; 3] {
    fn from(rgb: Rgb) -> Self {
        rgb.to_array()
    }
}

/// One hex digit, or `None` if the byte is not one.
const fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// One channel, from sRGB's transfer function to linear light.
#[must_use]
pub fn srgb_to_linear(value: u8) -> f32 {
    let c = f32::from(value) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// One channel, from linear light back to sRGB's transfer function.
///
/// The inverse is not used to draw anything — the GPU does that on the way to the
/// swapchain. It is here because the theme contrast tests need to round-trip a colour
/// through linear space and check it came back, which is the only way to catch a wrong
/// exponent in [`srgb_to_linear`] that would otherwise show up as a subtly wrong
/// screenshot.
#[must_use]
pub fn linear_to_srgb(value: f32) -> u8 {
    let c = value.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055f32.mul_add(c.powf(1.0 / 2.4), -0.055)
    };
    // The clamp above bounds this to `0.0..=1.0`, so the product is within `0..=255`
    // and the cast has nothing left to truncate or to lose a sign from.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = (s * 255.0).round() as u8;
    byte
}

/// One channel's contribution to relative luminance.
fn channel_luminance(value: u8) -> f32 {
    let c = f32::from(value) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrips_through_the_display_form() {
        for rgb in [
            Rgb::BLACK,
            Rgb::WHITE,
            Rgb::new(0x0a, 0x0b, 0x0d),
            Rgb::new(0xff, 0xa6, 0x2b),
            Rgb::new(1, 2, 3),
        ] {
            assert_eq!(Rgb::from_hex(&rgb.to_hex()), Some(rgb), "{rgb:?}");
        }
    }

    #[test]
    fn hex_accepts_both_cases_and_an_optional_hash() {
        let expected = Rgb::new(0xab, 0xcd, 0xef);
        for written in ["#abcdef", "abcdef", "#ABCDEF", "ABCDEF", "#AbCdEf"] {
            assert_eq!(Rgb::from_hex(written), Some(expected), "{written}");
        }
    }

    #[test]
    fn hex_refuses_everything_that_is_not_six_digits() {
        // Each of these means something in some other notation, and silently accepting
        // one would put a colour in the palette that is not the one written down.
        for rejected in [
            "",
            "#",
            "#fff",
            "fff",
            "#ffff",
            "#ffffffff",
            "#gggggg",
            "#12345",
            "#1234567",
            " #123456",
            "#123456 ",
            "rgb(1,2,3)",
        ] {
            assert_eq!(Rgb::from_hex(rejected), None, "{rejected:?} was accepted");
        }
    }

    #[test]
    fn black_and_white_are_the_contrast_extremes() {
        let ratio = Rgb::BLACK.contrast_ratio(Rgb::WHITE);
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
        assert!((Rgb::WHITE.contrast_ratio(Rgb::WHITE) - 1.0).abs() < 1e-6);
        assert!((Rgb::BLACK.contrast_ratio(Rgb::BLACK) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn contrast_does_not_depend_on_argument_order() {
        let a = Rgb::new(0x0a, 0x0b, 0x0d);
        let b = Rgb::new(0xe7, 0xe9, 0xec);
        assert!((a.contrast_ratio(b) - b.contrast_ratio(a)).abs() < 1e-6);
    }

    #[test]
    fn the_ink_on_ground_pair_clears_the_design_floors() {
        // The two numbers in DESIGN.md's contrast table. Asserting them here means a
        // palette edit that dims the chrome text fails a test rather than a review.
        let ground = Rgb::new(0x0a, 0x0b, 0x0d);
        let ink = Rgb::new(0xe7, 0xe9, 0xec);
        let ink_mid = Rgb::new(0x99, 0xa0, 0xa8);
        let surface = Rgb::new(0x0f, 0x11, 0x14);
        let ink_dim = Rgb::new(0x7b, 0x83, 0x8d);
        let signal = Rgb::new(0xff, 0xa6, 0x2b);

        assert!(
            ink.contrasts_with(ground, 12.0),
            "{}",
            ink.contrast_ratio(ground)
        );
        assert!(
            ink_mid.contrasts_with(ground, 6.0),
            "{}",
            ink_mid.contrast_ratio(ground)
        );
        assert!(
            ink_dim.contrasts_with(surface, 4.5),
            "{}",
            ink_dim.contrast_ratio(surface)
        );
        assert!(
            signal.contrasts_with(surface, 4.5),
            "{}",
            signal.contrast_ratio(surface)
        );
    }

    #[test]
    fn linear_conversion_roundtrips_within_a_rounding_step() {
        for value in 0..=u8::MAX {
            let linear = srgb_to_linear(value);
            let back = linear_to_srgb(linear);
            assert!(
                back.abs_diff(value) <= 1,
                "{value} became {back} through {linear}"
            );
        }
    }

    #[test]
    fn linear_space_darkens_the_midtones() {
        // The property that makes the conversion worth doing: mid grey in sRGB is much
        // darker than 0.5 in linear light, so blending without converting lifts it.
        assert!(srgb_to_linear(128) < 0.25, "{}", srgb_to_linear(128));
        assert!((srgb_to_linear(0) - 0.0).abs() < 1e-6);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn alpha_is_carried_through_the_linear_form() {
        let [r, g, b, a] = Rgb::new(0xff, 0x00, 0x00).to_linear_alpha(0.5);
        assert!((r - 1.0).abs() < 1e-6);
        assert!((g).abs() < 1e-6);
        assert!((b).abs() < 1e-6);
        assert!((a - 0.5).abs() < 1e-6);
    }
}
