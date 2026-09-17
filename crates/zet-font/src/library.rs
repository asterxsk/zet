//! The system font database: what is installed, and which face answers for a family.
//!
//! `fontique` does the enumeration. What this module adds is a cache and a policy.
//!
//! The cache matters more than it looks. `Collection::query` walks a family's font list
//! and materialises a blob for each candidate; doing that per character would put a
//! filesystem-backed database lookup inside the acceptance test for every cell on the
//! screen. Every face resolved here is kept, including the failures, so that asking for
//! a family that is not installed costs one lookup over the life of the process rather
//! than one per frame.

use std::collections::HashMap;

use fontique::{
    Attributes, Blob, Collection, CollectionOptions, FallbackKey, FamilyId, FontStyle, FontWeight,
    FontWidth, GenericFamily, QueryStatus, Script, SourceCache, SourceCacheOptions,
};
use swash::{FontRef, Metrics};

/// A typographic weight, 100 to 900.
///
/// A newtype over the CSS numeric scale rather than an enum, because the families that
/// ship real weights use values between the four named ones and rounding them to
/// `Normal` or `Bold` would silently pick the wrong face for a family whose 600 is its
/// actual bold.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Weight(u16);

impl Weight {
    /// Regular.
    pub const NORMAL: Weight = Weight(400);
    /// Medium, the heaviest the chrome is allowed to be.
    pub const MEDIUM: Weight = Weight(500);
    /// Bold.
    pub const BOLD: Weight = Weight(700);

    /// A weight from the CSS numeric scale, clamped to the range that means anything.
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Weight(if value < 100 {
            100
        } else if value > 900 {
            900
        } else {
            value
        })
    }

    /// The numeric value.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl Default for Weight {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Which face a resolve call is asking for.
///
/// Part of the cache key and part of what a caller stores to know which face a glyph
/// came from, which is why it is public and `Copy` rather than an internal detail.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FaceKey {
    /// The family the face belongs to.
    pub family: FamilyId,
    /// The weight that was asked for. Not necessarily the weight the face has: a
    /// family with only a regular is asked for bold, answers with its regular, and is
    /// marked for synthetic emboldening.
    pub weight: Weight,
    /// Whether an italic or oblique face was asked for. As with weight, the face may
    /// have answered with an upright one and be marked for a synthetic skew.
    pub italic: bool,
}

/// One loaded face.
///
/// Cloning is cheap: the blob is reference counted by `fontique`, so a stack holding
/// four fallback faces and an atlas holding keys that refer to them are not four copies
/// of a font file.
#[derive(Clone)]
pub struct Face {
    key: FaceKey,
    blob: Blob<u8>,
    index: u32,
    embolden: bool,
    skew: Option<f32>,
    variations: Vec<([u8; 4], f32)>,
}

impl Face {
    /// What was asked for when this face was resolved.
    #[must_use]
    pub const fn key(&self) -> FaceKey {
        self.key
    }

    /// The family this face belongs to.
    #[must_use]
    pub const fn family(&self) -> FamilyId {
        self.key.family
    }

    /// The raw font file.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        self.blob.as_ref()
    }

    /// The face's index within its file, for a `.ttc` collection.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Whether the face has to be fattened to look like the weight that was asked for.
    ///
    /// A family with a weight axis answers through `variations` instead and this stays
    /// false, because a real 600 drawn by the font beats a 400 with a stroke added.
    #[must_use]
    pub const fn embolden(&self) -> bool {
        self.embolden
    }

    /// The angle in degrees to skew an upright face by, when an italic was asked for
    /// and the family has none.
    #[must_use]
    pub const fn skew(&self) -> Option<f32> {
        self.skew
    }

    /// The variation axis settings the face should be instantiated with.
    ///
    /// Empty for a static font. The tags are raw four-byte values because `fontique`
    /// takes them from `read-fonts`, a crate this one does not depend on, and `swash`
    /// wants them as its own `Tag`, which is a `u32` — the four bytes are the only
    /// representation both crates can be handed.
    #[must_use]
    pub fn variations(&self) -> &[([u8; 4], f32)] {
        &self.variations
    }

    /// A `swash` handle onto the face.
    ///
    /// `None` when the file is not a font the rasteriser understands, which is a real
    /// case for the `.fon` bitmap fonts still present on Windows.
    #[must_use]
    pub fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(self.data(), self.index as usize)
    }

    /// The face's own metrics at `ppem` pixels per em.
    ///
    /// Measured at the face's default instance even when `variations` is not empty:
    /// `swash` only applies variation deltas to metrics through a scaler, and a scaler
    /// needs a glyph to be built for. A variable primary face therefore gets the cell
    /// width of its default instance, which is the one a terminal wants anyway — the
    /// cell must not resize when the user asks for a heavier weight.
    #[must_use]
    pub fn metrics(&self, ppem: f32) -> Option<Metrics> {
        Some(self.font()?.metrics(&[]).scale(ppem))
    }

    /// Whether the face has a glyph for `ch`.
    ///
    /// A character map lookup, not a rasterisation. Not free — it parses the font's
    /// table directory — which is why the stack caches the answer per character.
    #[must_use]
    pub fn covers(&self, ch: char) -> bool {
        self.glyph_id(ch) != 0
    }

    /// The glyph id for `ch`, or zero when the face has no glyph for it.
    ///
    /// Zero is `.notdef`, which is the right answer rather than a failure: a font
    /// draws `.notdef` as a box, and a box is what a terminal shows for a character
    /// nothing on the machine can render.
    #[must_use]
    pub fn glyph_id(&self, ch: char) -> u16 {
        self.font().map_or(0, |font| font.charmap().map(ch))
    }
}

/// The installed fonts, and every face resolved from them so far.
pub struct FontLibrary {
    collection: Collection,
    sources: SourceCache,
    faces: HashMap<FaceKey, Option<Face>>,
    names: Option<Vec<String>>,
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl FontLibrary {
    /// Load the system font database.
    ///
    /// This is the expensive call in the crate — it enumerates every installed family
    /// — and it belongs on a background thread at startup or behind the first frame.
    /// It does not load any font *file*: `fontique` records where each family lives and
    /// materialises the bytes lazily, so constructing this is a registry walk rather
    /// than a few hundred megabytes of reading.
    #[must_use]
    pub fn new() -> Self {
        let mut collection = Collection::new(CollectionOptions::default());
        collection.load_system_fonts();
        Self {
            collection,
            sources: SourceCache::new(SourceCacheOptions::default()),
            faces: HashMap::new(),
            names: None,
        }
    }

    /// Every installed family name, sorted.
    ///
    /// The list the settings panel shows. Sorted case-insensitively first and by the
    /// original text only to break ties, so that `cascadia mono` does not end up below
    /// every capitalised name in the alphabet.
    pub fn families(&mut self) -> &[String] {
        if self.names.is_none() {
            let mut names: Vec<String> =
                self.collection.family_names().map(str::to_owned).collect();
            names.sort_by(|a, b| {
                a.to_lowercase()
                    .cmp(&b.to_lowercase())
                    .then_with(|| a.cmp(b))
            });
            names.dedup();
            self.names = Some(names);
        }
        self.names.as_deref().unwrap_or(&[])
    }

    /// The identifier for a family name, if it is installed.
    #[must_use]
    pub fn family_id(&mut self, family: &str) -> Option<FamilyId> {
        self.collection.family_id(family)
    }

    /// Whether a family is installed.
    #[must_use]
    pub fn has_family(&mut self, family: &str) -> bool {
        self.collection.family_id(family).is_some()
    }

    /// Whether a family is a monospaced face.
    ///
    /// Read from the font's own `post` table rather than by comparing the advance of
    /// `i` and `M`, because the table is the font's own claim and is right for the
    /// families whose `i` and `M` happen to match while `W` does not.
    ///
    /// `false` for a family that is not installed, which is the useful answer: a
    /// settings panel greying out a name does not need to distinguish "absent" from
    /// "proportional".
    #[must_use]
    pub fn is_monospace(&mut self, family: &str) -> bool {
        let Some(id) = self.collection.family_id(family) else {
            return false;
        };
        self.resolve_family(id, Weight::NORMAL, false)
            .and_then(|face| face.metrics(16.0))
            .is_some_and(|metrics| metrics.is_monospace)
    }

    /// Resolve a family name and style to a face.
    #[must_use]
    pub fn resolve(&mut self, family: &str, weight: Weight, italic: bool) -> Option<Face> {
        let id = self.collection.family_id(family)?;
        self.resolve_family(id, weight, italic)
    }

    /// Resolve a family identifier and style to a face.
    #[must_use]
    pub fn resolve_family(
        &mut self,
        family: FamilyId,
        weight: Weight,
        italic: bool,
    ) -> Option<Face> {
        self.lookup(FaceKey {
            family,
            weight,
            italic,
        })
    }

    /// Resolve one of the generic families — `monospace`, `sans-serif`, `emoji`.
    ///
    /// The last resort in a fallback chain, and the reason a fresh install with a
    /// misconfigured font still draws something.
    #[must_use]
    pub fn resolve_generic(
        &mut self,
        generic: GenericFamily,
        weight: Weight,
        italic: bool,
    ) -> Option<Face> {
        let family = self.collection.generic_families(generic).next()?;
        self.lookup(FaceKey {
            family,
            weight,
            italic,
        })
    }

    /// The system's own answer for "what renders this script".
    ///
    /// On Windows this is `IDWriteFontFallback`, which is the same table the rest of
    /// the operating system uses, so a character that renders in Notepad renders here.
    /// The answer is cached by `fontique` per script, so asking once per character in a
    /// run of Chinese text is one lookup and then a hash hit.
    ///
    /// `None` for a script the platform has no sample for, which includes the
    /// `Common` script an emoji is filed under.
    pub fn fallback_family(&mut self, script: Script) -> Option<FamilyId> {
        self.collection
            .fallback_families(FallbackKey::new(script, None))
            .next()
    }

    /// Look a face up, resolving and caching it on the first ask.
    fn lookup(&mut self, key: FaceKey) -> Option<Face> {
        if let Some(cached) = self.faces.get(&key) {
            return cached.clone();
        }
        let resolved = self.query(key);
        // The misses are cached too. A stack asks for the same absent fallback for
        // every character it cannot draw, and without this each of those is a walk
        // through the family list.
        self.faces.insert(key, resolved.clone());
        resolved
    }

    /// Ask `fontique` for the closest face to `key`.
    fn query(&mut self, key: FaceKey) -> Option<Face> {
        let attributes = Attributes::new(
            FontWidth::NORMAL,
            if key.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            FontWeight::new(f32::from(key.weight.value())),
        );
        let mut found: Option<Face> = None;
        {
            let mut query = self.collection.query(&mut self.sources);
            query.set_families([key.family]);
            query.set_attributes(attributes);
            query.matches_with(|font| {
                // `fontique` hands over the nearest face the family has along with a
                // `Synthesis` describing what it could not honour. That is a decision
                // this crate makes itself, at rasterisation, where it can embolden,
                // skew, or drive a variation axis — so the face is taken as it is and
                // the synthesis is read rather than applied.
                let synthesis = &font.synthesis;
                let variations = synthesis
                    .variation_settings()
                    .iter()
                    .map(|(tag, value)| (tag.to_be_bytes(), *value))
                    .collect();
                found = Some(Face {
                    key,
                    blob: font.blob.clone(),
                    index: font.index,
                    embolden: synthesis.embolden(),
                    skew: synthesis.skew(),
                    variations,
                });
                QueryStatus::Stop
            });
        }
        // A family entry can point at a file that has been removed, and a variable
        // font with no default instance is not a face `swash` can open. Both are a
        // miss rather than a panic, and the stack falls through to the next family.
        found.filter(|face| face.font().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A family every Windows install has, and the one the grid falls back to.
    const UBIQUITOUS: &str = "Consolas";

    #[test]
    fn the_system_font_list_is_not_empty() {
        let mut library = FontLibrary::new();
        let families = library.families();
        assert!(
            families.len() > 20,
            "only {} families found, which means enumeration failed",
            families.len()
        );
        let lowered: Vec<String> = families.iter().map(|name| name.to_lowercase()).collect();
        assert!(
            lowered.windows(2).all(|pair| pair[0] <= pair[1]),
            "the list is supposed to be sorted case-insensitively"
        );
    }

    #[test]
    fn a_family_that_ships_with_windows_is_found() {
        let mut library = FontLibrary::new();
        assert!(library.has_family(UBIQUITOUS), "{UBIQUITOUS} not installed");
        let face = library
            .resolve(UBIQUITOUS, Weight::NORMAL, false)
            .expect("a regular face");
        assert!(!face.data().is_empty());
        assert!(face.font().is_some(), "swash could not read the face");
    }

    #[test]
    fn a_family_that_does_not_exist_resolves_to_nothing() {
        let mut library = FontLibrary::new();
        assert!(!library.has_family("Zet Nonexistent Display"));
        assert!(
            library
                .resolve("Zet Nonexistent Display", Weight::NORMAL, false)
                .is_none()
        );
    }

    #[test]
    fn a_family_that_is_not_installed_never_reaches_the_face_table() {
        // The short circuit that keeps a misconfigured fallback cheap. A name that is
        // not in the database is answered by the name index, so the expensive part of
        // a lookup — walking a family's font list and materialising a blob — is never
        // reached at all, let alone once per character.
        let mut library = FontLibrary::new();
        for _ in 0..5 {
            let _ = library.resolve("Zet Nonexistent Display", Weight::NORMAL, false);
        }
        assert!(library.faces.is_empty());
    }

    #[test]
    fn a_replaced_face_comes_back_from_the_cache() {
        let mut library = FontLibrary::new();
        let _ = library.resolve(UBIQUITOUS, Weight::NORMAL, false);
        let before = library.faces.len();
        let _ = library.resolve(UBIQUITOUS, Weight::NORMAL, false);
        assert_eq!(library.faces.len(), before);
    }

    #[test]
    fn a_family_with_no_bold_face_answers_with_an_instruction_to_embolden() {
        // The negative result that has to be cached: a family that has no face for the
        // style asked for still produces a face, and asking again must not re-run the
        // walk that decided it.
        let mut library = FontLibrary::new();
        if !library.has_family("Marlett") {
            // A machine without Marlett cannot test this; it ships with Windows.
            return;
        }
        let first = library
            .resolve("Marlett", Weight::BOLD, false)
            .expect("the regular answers for bold");
        assert!(first.embolden(), "a family with no bold must be fattened");
        let before = library.faces.len();
        let _ = library.resolve("Marlett", Weight::BOLD, false);
        assert_eq!(library.faces.len(), before);
    }

    #[test]
    fn a_monospace_family_reports_itself_as_one() {
        let mut library = FontLibrary::new();
        assert!(
            library.is_monospace(UBIQUITOUS),
            "{UBIQUITOUS} should be monospaced"
        );
    }

    #[test]
    fn a_family_that_is_not_installed_is_not_monospaced() {
        let mut library = FontLibrary::new();
        assert!(!library.is_monospace("Zet Nonexistent Display"));
    }

    #[test]
    fn covering_a_character_is_a_charmap_lookup() {
        let mut library = FontLibrary::new();
        let face = library
            .resolve(UBIQUITOUS, Weight::NORMAL, false)
            .expect("a face");
        assert!(face.covers('A'));
        assert!(face.covers(' '));
        assert_ne!(face.glyph_id('A'), 0);
        // Consolas has no CJK ideographs, which is the whole reason fallback exists.
        assert!(!face.covers('\u{4e2d}'));
        // An uncovered character resolves to `.notdef` rather than to nothing.
        assert_eq!(face.glyph_id('\u{4e2d}'), 0);
    }

    #[test]
    fn metrics_scale_with_the_size() {
        let mut library = FontLibrary::new();
        let face = library
            .resolve(UBIQUITOUS, Weight::NORMAL, false)
            .expect("a face");
        let small = face.metrics(12.0).expect("metrics");
        let large = face.metrics(24.0).expect("metrics");
        assert!(
            large.ascent > small.ascent,
            "{} is not larger than {}",
            large.ascent,
            small.ascent
        );
    }

    #[test]
    fn a_generic_family_resolves_to_something() {
        let mut library = FontLibrary::new();
        assert!(
            library
                .resolve_generic(GenericFamily::Monospace, Weight::NORMAL, false)
                .is_some()
        );
    }

    #[test]
    fn the_system_knows_what_renders_han() {
        // The fallback that makes Chinese text legible instead of a field of boxes.
        // It comes from DirectWrite, so on a machine with no CJK font installed this
        // legitimately returns `None` — but a Windows install that can render a web
        // page can render this.
        let mut library = FontLibrary::new();
        let han = Script::parse("Hani").expect("a valid script tag");
        let family = library.fallback_family(han);
        assert!(family.is_some(), "DirectWrite has no answer for Han");
        let face = library
            .resolve_family(family.expect("checked above"), Weight::NORMAL, false)
            .expect("the fallback family should resolve");
        assert!(
            face.covers('\u{4e2d}'),
            "DirectWrite's Han fallback cannot draw a Han character"
        );
    }

    #[test]
    fn a_script_with_no_sample_has_no_fallback() {
        let mut library = FontLibrary::new();
        assert!(library.fallback_family(Script::UNKNOWN).is_none());
    }

    #[test]
    fn weight_is_clamped_to_the_scale_that_means_anything() {
        assert_eq!(Weight::new(0).value(), 100);
        assert_eq!(Weight::new(50).value(), 100);
        assert_eq!(Weight::new(400).value(), 400);
        assert_eq!(Weight::new(9999).value(), 900);
        assert_eq!(Weight::default(), Weight::NORMAL);
    }

    #[test]
    fn asking_for_bold_on_a_family_that_has_it_does_not_synthesise() {
        let mut library = FontLibrary::new();
        let face = library
            .resolve(UBIQUITOUS, Weight::BOLD, false)
            .expect("a bold face");
        assert!(
            !face.embolden(),
            "Consolas ships a bold and it should have been found"
        );
    }

    #[test]
    fn a_family_that_ships_a_real_italic_is_not_skewed() {
        let mut library = FontLibrary::new();
        let face = library
            .resolve(UBIQUITOUS, Weight::NORMAL, true)
            .expect("an italic face");
        assert!(face.key().italic, "the request did not survive the lookup");
        assert!(
            face.skew().is_none(),
            "Consolas has a real italic, so nothing should be skewed"
        );
    }

    #[test]
    fn a_family_with_no_italic_is_asked_to_be_skewed_instead() {
        // The other half, and the one that is easy to get wrong: `fontique` answers a
        // request for italic on a family that has none with its upright face plus a
        // `Synthesis` saying what to do about it. Reading that field wrongly drops the
        // request silently, and the user sees roman text where they asked for italic.
        let mut library = FontLibrary::new();
        if !library.has_family("Marlett") {
            // Ships with Windows, but a test should not invent a failure on a machine
            // that has had it removed.
            return;
        }
        let face = library
            .resolve("Marlett", Weight::NORMAL, true)
            .expect("the upright face answers for italic");
        assert_eq!(
            face.skew(),
            Some(14.0),
            "a family with no italic must come back with an angle to skew by"
        );
    }
}
