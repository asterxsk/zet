//! The face chain, and the answer to "what draws this character".
//!
//! A terminal cannot choose a font per string the way a word processor does. It has one
//! grid, one cell size, and a stream of characters arriving one at a time from a program
//! that has no idea what is installed on this machine. So the resolution has to happen
//! per character and it has to be cheap enough to do inline.
//!
//! This module is that resolution, plus the small amount of state that makes it cheap:
//! a memo from a character and a style to the index of the face that draws it, and a
//! flat list of faces that never grows past the handful the machine actually needs.
//!
//! The order is fixed and predictable, because a terminal that picks a different font
//! for the same character depending on what came before it is worse than one that picks
//! an ugly font:
//!
//! 1. The configured family. Nothing else is consulted while it has a glyph.
//! 2. Each configured fallback, in the order the user wrote them.
//! 3. The system's own answer for the character's script, which on Windows is
//!    DirectWrite's font fallback — the same table Notepad and the browser use.
//! 4. The configured family again, whose `.notdef` is a box. A box is the honest
//!    answer to "nothing here can draw this", and it is what the user needs to see.

use std::collections::HashMap;

use fontique::{FamilyId, Script};
use swash::{
    scale::{Render, ScaleContext, Source, StrikeWith, image::Content},
    zeno::{Angle, Format, Transform},
};
use zet_config::FontSettings;

use crate::FontError;
use crate::glyph::{Glyph, GlyphContent, GlyphKey, GlyphSpec};
use crate::library::{Face, FaceKey, FontLibrary, Weight};
use crate::metrics::Metrics;

/// How thick a synthetic bold is, as a fraction of the em.
///
/// A twenty-fourth, which is the faux-bold strength the rest of the industry converged
/// on and which is what makes our synthetic bold match theirs. It is a fraction of the
/// em rather than of the pixel size so that the stroke keeps its proportion as the text
/// grows.
const EMBOLDEN: f32 = 1.0 / 24.0;

/// A face chain resolved against one family, size, and window scale.
pub struct FontStack {
    library: FontLibrary,
    /// The primary family, kept so a character nothing covers can fall back to the
    /// face whose `.notdef` the user already recognises.
    primary: FamilyId,
    /// The families named in the config, in order, already checked to exist.
    configured: Vec<FamilyId>,
    /// Every face resolved so far, `faces[0]` being the primary's regular. Grows to a
    /// handful and then stops.
    faces: Vec<Face>,
    /// Which face draws a given character in a given style.
    resolved: HashMap<(char, Weight, bool), usize>,
    /// The script the system was asked about, so that a run of Chinese text does not
    /// ask DirectWrite once per character.
    script_family: HashMap<Script, Option<FamilyId>>,
    metrics: Metrics,
    context: ScaleContext,
}

impl FontStack {
    /// Resolve a font configuration into a usable stack.
    ///
    /// `scale` is the window's DPI scale — 1.0 at 100%, 1.5 at 150% — which multiplies
    /// the configured point size. The configured size is in points at 100%, so the
    /// stack has to be rebuilt when the window moves to a monitor with a different
    /// scale, and that is the caller's decision to make rather than something to paper
    /// over with a stale cell size.
    ///
    /// # Errors
    ///
    /// [`FontError::NoSuchFamily`] when the configured family is not installed, and
    /// [`FontError::Unreadable`] when it is installed but has no face the rasteriser
    /// can open. Both are worth stopping for rather than silently substituting a font
    /// the user did not ask for.
    pub fn load(settings: &FontSettings, scale: f32) -> Result<Self, FontError> {
        Self::build(FontLibrary::new(), settings, scale)
    }

    /// Resolve a font configuration against the system database plus faces carried in
    /// the binary.
    ///
    /// The chrome's face is one the machine is not expected to have, so it is handed in
    /// here and registered before anything is resolved. Registration and the system
    /// database are the same database on purpose: a family installed on the machine and
    /// the same family shipped in the binary are one family, and the face the user's own
    /// installation supplies is the one they chose.
    ///
    /// # Errors
    ///
    /// The same as [`FontStack::load`]. A family that is neither embedded nor installed
    /// is still [`FontError::NoSuchFamily`].
    pub fn load_embedded(
        settings: &FontSettings,
        scale: f32,
        faces: &[&'static [u8]],
    ) -> Result<Self, FontError> {
        let mut library = FontLibrary::new();
        for face in faces {
            library.register(face);
        }
        Self::build(library, settings, scale)
    }

    /// The body of both constructors, once the database is assembled.
    fn build(
        mut library: FontLibrary,
        settings: &FontSettings,
        scale: f32,
    ) -> Result<Self, FontError> {
        if !settings.size.is_finite() || settings.size <= 0.0 {
            return Err(FontError::BadSize(settings.size));
        }
        let ppem = settings.size * scale;

        let primary = library
            .family_id(&settings.family)
            .ok_or_else(|| FontError::NoSuchFamily(settings.family.clone()))?;
        let primary_face = library
            .resolve_family(primary, Weight::NORMAL, false)
            .ok_or_else(|| FontError::Unreadable {
                family: settings.family.clone(),
            })?;
        let metrics =
            Metrics::from_face(&primary_face, ppem).ok_or_else(|| FontError::Unreadable {
                family: settings.family.clone(),
            })?;

        // A fallback that is not installed is skipped rather than reported. The list is
        // a set of suggestions, the first one that exists wins, and a user who has
        // uninstalled Segoe UI Emoji should not have their terminal refuse to start.
        let mut configured = Vec::with_capacity(settings.fallback.len());
        for name in &settings.fallback {
            let Some(id) = library.family_id(name) else {
                continue;
            };
            if id != primary && !configured.contains(&id) {
                configured.push(id);
            }
        }

        Ok(Self {
            library,
            primary,
            configured,
            faces: vec![primary_face],
            resolved: HashMap::new(),
            script_family: HashMap::new(),
            metrics,
            context: ScaleContext::new(),
        })
    }

    /// The cell this stack measures at.
    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    /// Every family installed on this machine, for a settings panel to offer.
    pub fn families(&mut self) -> &[String] {
        self.library.families()
    }

    /// Whether a family is installed.
    pub fn has_family(&mut self, family: &str) -> bool {
        self.library.has_family(family)
    }

    /// Whether a family is monospaced, which is the only sane choice for the grid.
    pub fn is_monospace(&mut self, family: &str) -> bool {
        self.library.is_monospace(family)
    }

    /// Draw one character.
    ///
    /// This is the only method that does real work, and it does it once per distinct
    /// character and style: the face is resolved through a memo and the pixels come
    /// back to be stored in the renderer's atlas. Nothing here caches the pixels
    /// themselves, because the atlas belongs to whoever owns the GPU and this crate
    /// does not know that a texture exists.
    pub fn rasterize(&mut self, spec: GlyphSpec) -> Glyph {
        let index = self.face_for(spec.ch, spec.weight, spec.italic);
        // Cloned to release the borrow on `self.faces` before `self.context` is used
        // mutably. A `Face` clone copies a reference count and a small vector.
        let face = self.faces[index].clone();
        let glyph_id = face.glyph_id(spec.ch);
        let key = GlyphKey {
            face: face.key(),
            glyph_id,
        };

        let Some(font) = face.font() else {
            return Glyph::blank(key, 0.0);
        };
        let advance = font
            .glyph_metrics(&[])
            .scale(self.metrics.ppem)
            .advance_width(glyph_id);

        let mut scaler = self
            .context
            .builder(font)
            .size(self.metrics.ppem)
            // Hinting snaps stems to the pixel grid. A terminal is small text and this
            // is most of what keeps 13px readable on an LCD; at the sizes a terminal
            // uses, the cost is a rounded stem rather than a distorted letterform.
            .hint(true)
            .variations(face.variations())
            .build();

        let mut render = Render::new(&SOURCES);
        render.format(Format::Alpha);
        if face.embolden() {
            render.embolden(self.metrics.ppem * EMBOLDEN);
        }
        if let Some(degrees) = face.skew() {
            render.transform(Some(Transform::skew(
                Angle::from_degrees(degrees),
                Angle::ZERO,
            )));
        }

        let Some(image) = render.render(&mut scaler, glyph_id) else {
            // A glyph with no outline and no bitmap: a space, or a `.notdef` some
            // fonts draw as nothing at all. It still advances.
            return Glyph::blank(key, advance);
        };

        Glyph {
            key,
            left: image.placement.left,
            // `FontStack` never asks for a subpixel format, so the only two contents
            // swash can return are the coverage mask and a colour bitmap.
            content: match image.content {
                Content::Color => GlyphContent::Color,
                Content::Mask | Content::SubpixelMask => GlyphContent::Alpha,
            },
            top: image.placement.top,
            width: image.placement.width,
            height: image.placement.height,
            advance,
            data: image.data,
        }
    }

    /// Resolve a character to an index into the face list.
    fn face_for(&mut self, ch: char, weight: Weight, italic: bool) -> usize {
        if let Some(&index) = self.resolved.get(&(ch, weight, italic)) {
            return index;
        }
        let index = self.resolve(ch, weight, italic);
        self.resolved.insert((ch, weight, italic), index);
        index
    }

    /// Walk the chain for a character that has not been seen before.
    fn resolve(&mut self, ch: char, weight: Weight, italic: bool) -> usize {
        // 1. The configured family.
        if let Some(index) = self.covering(self.primary, ch, weight, italic) {
            return index;
        }
        // 2. The configured fallbacks, in the user's order. Indexed rather than
        //    iterated by reference so the list does not have to be cloned out of the
        //    way of the mutable borrow `intern` needs.
        for position in 0..self.configured.len() {
            if let Some(index) = self.covering(self.configured[position], ch, weight, italic) {
                return index;
            }
        }
        // 3. The system's answer for this script, which is what makes Chinese, Arabic,
        //    and Thai render instead of turning into boxes.
        if let Some(family) = self.system_family(ch)
            && let Some(index) = self.covering(family, ch, weight, italic)
        {
            return index;
        }
        // 4. Nothing has it. The primary's `.notdef` is the box the user expects, and
        //    `glyph_id` returning zero is what draws it.
        self.intern(self.primary, weight, italic).unwrap_or(0)
    }

    /// The index of a face that draws `ch`, if the family has one.
    fn covering(
        &mut self,
        family: FamilyId,
        ch: char,
        weight: Weight,
        italic: bool,
    ) -> Option<usize> {
        let index = self.intern(family, weight, italic)?;
        self.faces[index].covers(ch).then_some(index)
    }

    /// The index of a family's face in a given style, resolving it on first ask.
    fn intern(&mut self, family: FamilyId, weight: Weight, italic: bool) -> Option<usize> {
        let key = FaceKey {
            family,
            weight,
            italic,
        };
        if let Some(face) = self
            .faces
            .iter()
            .position(|candidate| candidate.key() == key)
        {
            return Some(face);
        }
        let face = self.library.resolve_family(family, weight, italic)?;
        self.faces.push(face);
        Some(self.faces.len() - 1)
    }

    /// Ask the platform what family renders this character's script.
    fn system_family(&mut self, ch: char) -> Option<FamilyId> {
        let script = script_of(ch);
        if let Some(cached) = self.script_family.get(&script) {
            return *cached;
        }
        let family = self.library.fallback_family(script);
        self.script_family.insert(script, family);
        family
    }
}

/// The Unicode script a character belongs to, as an ISO 15924 identifier.
///
/// `fontique`'s fallback is keyed by script rather than by character — Windows exposes
/// no "which font draws this codepoint" call — so this is the translation that makes it
/// usable per character. `FallbackKey::default()` would answer for Latin and nothing
/// else, which is exactly the case that never needs the system's help.
fn script_of(ch: char) -> Script {
    let name = unicode_script::Script::from(ch).short_name();
    // Every short name is a valid four-letter ISO 15924 tag, so a parse failure means
    // the two crates disagree about the standard rather than that this character has
    // no script. Answering `Common` sends the query down the path that returns nothing,
    // which is the same outcome as giving up, and it does not panic on a code path a
    // program's output can reach.
    Script::parse(name).unwrap_or(Script::COMMON)
}

/// The sources a glyph is looked for in, in order of preference.
///
/// A colour emoji font has no outlines and a CJK bitmap font has no scalable glyph at
/// the size asked for, so the list is tried front to back and the first hit wins. A
/// `static` rather than a literal because `Render` borrows it for the whole of a
/// rasterisation.
static SOURCES: [Source; 4] = [
    Source::ColorOutline(0),
    Source::ColorBitmap(StrikeWith::BestFit),
    Source::Bitmap(StrikeWith::BestFit),
    Source::Outline,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn stack() -> FontStack {
        FontStack::load(&FontSettings::default(), 1.0).expect("the default font should load")
    }

    #[test]
    fn the_default_configuration_loads() {
        // DESIGN.md's promise: a fresh install has a working terminal before the user
        // has opened the settings panel.
        let stack = stack();
        assert!(stack.metrics().cell_width > 0.0);
        assert!(stack.metrics().cell_height >= 13.0);
    }

    #[test]
    fn a_size_that_is_not_a_size_is_refused() {
        for size in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let settings = FontSettings {
                size,
                ..FontSettings::default()
            };
            assert!(matches!(
                FontStack::load(&settings, 1.0),
                Err(FontError::BadSize(_))
            ));
        }
    }

    #[test]
    fn a_family_that_is_not_installed_is_refused_by_name() {
        let settings = FontSettings {
            family: "Zet Nonexistent Display".to_owned(),
            ..FontSettings::default()
        };
        match FontStack::load(&settings, 1.0) {
            Err(FontError::NoSuchFamily(name)) => assert_eq!(name, "Zet Nonexistent Display"),
            Err(other) => panic!("expected a missing family, got {other}"),
            Ok(_) => panic!("a family that is not installed should not load"),
        }
    }

    #[test]
    fn a_fallback_that_is_not_installed_does_not_stop_the_stack_loading() {
        let settings = FontSettings {
            fallback: vec!["Zet Nonexistent Display".to_owned(), "Segoe UI".to_owned()],
            ..FontSettings::default()
        };
        let stack = FontStack::load(&settings, 1.0).expect("a bad fallback is not fatal");
        assert!(stack.configured.len() <= 1);
    }

    #[test]
    fn the_scale_multiplies_the_size() {
        let settings = FontSettings::default();
        let plain = FontStack::load(&settings, 1.0).expect("loads");
        let scaled = FontStack::load(&settings, 2.0).expect("loads");
        assert!(
            scaled.metrics().cell_width > plain.metrics().cell_width,
            "a 200% window should have wider cells"
        );
    }

    #[test]
    fn a_latin_character_comes_from_the_configured_family() {
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new('A'));
        assert!(!glyph.is_blank(), "an A should have ink");
        assert_eq!(
            glyph.width as usize * glyph.height as usize,
            glyph.data.len()
        );
        assert_eq!(
            glyph.key.face,
            stack.faces[0].key(),
            "an A should not have gone looking for another font"
        );
    }

    #[test]
    fn a_space_has_no_ink_but_still_advances() {
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new(' '));
        assert!(glyph.is_blank());
        assert!(glyph.data.is_empty());
        assert!(
            (glyph.advance - stack.metrics().cell_width).abs() <= 1.0,
            "a space should advance by a cell"
        );
    }

    #[test]
    fn a_character_nothing_can_draw_is_a_box_and_never_a_hole() {
        // A private-use codepoint, which no font claims. The stack must still answer
        // with a rasterised glyph — `.notdef` — rather than an empty one, because a
        // terminal that draws nothing here shifts every character after it.
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new('\u{e000}'));
        assert!(!glyph.is_blank(), ".notdef should have ink");
    }

    #[test]
    fn a_han_character_is_drawn_by_something() {
        // The end-to-end version of the fallback test in `library`: a character the
        // configured family cannot draw has to reach the system's fallback and come
        // back with ink. On a machine with no CJK font this is a box, which still has
        // ink, so the assertion holds either way and the real check is that the chain
        // did not give up.
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new('\u{4e2d}'));
        assert!(!glyph.is_blank());
    }

    #[test]
    fn an_emoji_is_drawn_in_colour() {
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new('\u{1f600}'));
        if glyph.is_blank() {
            // A machine with no emoji font at all; nothing to assert.
            return;
        }
        assert_eq!(glyph.content, GlyphContent::Color);
        assert_eq!(
            glyph.data.len(),
            glyph.width as usize * glyph.height as usize * 4
        );
    }

    #[test]
    fn bold_and_italic_get_their_own_faces_but_the_same_cell() {
        let mut stack = stack();
        let regular = stack.rasterize(GlyphSpec::new('M'));
        let bold = stack.rasterize(GlyphSpec::bold('M'));
        let italic = stack.rasterize(GlyphSpec::italic('M'));
        assert_ne!(bold.key.face, regular.key.face, "bold is a different face");
        assert!(
            !bold.is_blank() && !italic.is_blank(),
            "weight and style must not cost the glyph its ink"
        );
        // The rule the grid rests on: the style changes the glyph and never the cell.
        // A bold `M` in a monospaced face advances by the same width as a regular one,
        // so turning on bold cannot reflow the text behind it.
        let cell_width = stack.metrics().cell_width;
        for glyph in [&regular, &bold, &italic] {
            assert!(
                (glyph.advance - cell_width).abs() <= 1.0,
                "{:?} advances by {} in a cell of {cell_width}",
                glyph.key.face.weight,
                glyph.advance
            );
        }
    }

    #[test]
    fn a_character_is_resolved_once_and_then_remembered() {
        let mut stack = stack();
        let before = stack.faces.len();
        for _ in 0..20 {
            let _ = stack.rasterize(GlyphSpec::new('A'));
        }
        assert_eq!(stack.faces.len(), before, "the face list should not grow");
        assert_eq!(stack.resolved.len(), 1, "one character, one memo");
    }

    #[test]
    fn asking_about_the_same_script_twice_only_asks_the_system_once() {
        let mut stack = stack();
        let _ = stack.rasterize(GlyphSpec::new('\u{4e2d}'));
        let _ = stack.rasterize(GlyphSpec::new('\u{6587}'));
        assert_eq!(stack.script_family.len(), 1, "Han is one script");
    }

    #[test]
    fn the_script_of_a_character_is_its_unicode_script() {
        assert_eq!(script_of('A'), Script::parse("Latn").expect("a script tag"));
        assert_eq!(
            script_of('\u{4e2d}'),
            Script::parse("Hani").expect("a script tag")
        );
        assert_eq!(
            script_of('\u{0416}'),
            Script::parse("Cyrl").expect("a script tag")
        );
    }

    #[test]
    fn the_face_list_never_grows_past_the_styles_actually_used() {
        let mut stack = stack();
        for ch in "The quick brown fox jumps over the lazy dog.".chars() {
            let _ = stack.rasterize(GlyphSpec::new(ch));
        }
        assert!(
            stack.faces.len() <= 2,
            "an ASCII sentence needed {} faces",
            stack.faces.len()
        );
    }

    #[test]
    fn a_glyphs_bitmap_is_exactly_its_area() {
        let mut stack = stack();
        for ch in ['A', 'g', '_', '\u{2500}'] {
            let glyph = stack.rasterize(GlyphSpec::new(ch));
            let expected = glyph.width as usize * glyph.height as usize;
            match glyph.content {
                GlyphContent::Alpha => assert_eq!(glyph.data.len(), expected, "{ch:?}"),
                GlyphContent::Color => assert_eq!(glyph.data.len(), expected * 4, "{ch:?}"),
            }
        }
    }

    #[test]
    fn the_ink_of_a_capital_sits_above_the_baseline() {
        let mut stack = stack();
        let glyph = stack.rasterize(GlyphSpec::new('M'));
        assert!(glyph.top > 0, "an M has ink above the baseline");
        let (_, y) = glyph.offset_in_cell(stack.metrics().baseline);
        assert!(y >= 0.0, "the ink starts below the top of the cell");
        assert!(
            y <= stack.metrics().cell_height,
            "the ink starts above the bottom of the cell"
        );
    }
}
