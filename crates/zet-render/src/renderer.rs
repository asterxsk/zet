//! The faces, the atlas, and the device, held together as one object.
//!
//! Each of the three is independently useful and independently testable — the atlas
//! packs rectangles, a font stack rasterises glyphs, the device draws vertices — and none
//! of them knows about the other two. This module is the wiring: it is what turns a
//! character in the grid into a rectangle in the atlas, and the only place where all
//! three are visible at once.
//!
//! # Two faces, one atlas
//!
//! The grid is drawn in the user's terminal font and the chrome in IBM Plex Sans, at
//! different sizes, and both go into the same texture. That is not a compromise:
//! [`Atlas`] is keyed by [`GlyphKey`](zet_font::GlyphKey), which carries a
//! [`FaceKey`](zet_font::FaceKey) — family, weight, italic — so two
//! faces cannot collide in it, and a texture holding both is the case the key was designed
//! for. Two textures would mean two bind groups, a second sampler, and a rule about which
//! batch uses which, all to avoid a collision that cannot happen.
//!
//! The chrome's *bytes* are not here. This crate has never heard of IBM Plex Sans; the
//! caller loads the stack and hands it over, which is what keeps the dependency arrow
//! pointing the way it does.
//!
//! # The cache, and why there is one
//!
//! [`draw_grid`](crate::draw_grid) walks every cell of every row on every frame. That is
//! the right shape — a frame rebuilt from scratch has no way to leave a row out without
//! erasing it — but it means [`GlyphSource::place`] is called once per cell, and a screen
//! of them is mostly the same handful of characters. Rasterising `e` ninety times a frame
//! because it appears in ninety cells is the difference between a terminal that draws in a
//! tenth of a millisecond and one that does not.
//!
//! So the cache is here and not in [`Atlas`], which caches by [`GlyphKey`] — the answer to
//! "have I drawn this glyph before". This one answers "have I drawn this *character*
//! before", and it exists to skip the face lookup and the rasterisation that stand between
//! the two. It is one map per face, because the same character in two faces is two glyphs.

use std::collections::HashMap;
use std::sync::Arc;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use zet_config::FontSettings;
use zet_font::{FontError, FontStack, Glyph, GlyphSpec, Metrics};

use crate::atlas::{Atlas, Placement};
use crate::frame::Frame;
use crate::gpu::{Gpu, GpuError};
use crate::grid::GlyphSource;

/// Something that stopped the renderer being set up or a frame being drawn.
#[derive(Debug, thiserror::Error)]
pub enum RendererError {
    /// A face could not be loaded.
    #[error(transparent)]
    Font(#[from] FontError),

    /// The device could not be created, or the frame could not be presented.
    #[error(transparent)]
    Gpu(#[from] GpuError),
}

/// Where each character a face has been asked for ended up.
///
/// A `None` is remembered as readily as a `Some`. The atlas only ever grows and never
/// frees, so a glyph it could not fit this frame it could not fit on any later one either,
/// and asking again every frame — for the one glyph on screen that cannot be drawn — is
/// the work this map exists to avoid.
#[derive(Default)]
struct Placed(HashMap<GlyphSpec, Option<Placement>>);

/// A face and what it has already been asked to draw.
struct Face {
    /// The faces and their metrics at the current scale.
    stack: FontStack,
    /// Where each character went.
    placed: Placed,
}

/// The atlas entry for a character, rasterising and packing it if it is new.
///
/// `rasterize` is called at most once, and not at all for a character already here. It is
/// a closure rather than a `&mut Face` so that the two maps and the one atlas can be
/// borrowed separately — and so that this, the part with the decisions in it, can be
/// tested against a fake glyph with no font and no device.
fn place(
    placed: &mut Placed,
    atlas: &mut Atlas,
    spec: GlyphSpec,
    rasterize: impl FnOnce() -> Glyph,
) -> Option<Placement> {
    if let Some(placement) = placed.0.get(&spec) {
        return *placement;
    }
    let glyph = rasterize();
    let placement = entry(atlas, &glyph);
    placed.0.insert(spec, placement);
    placement
}

/// The atlas entry for a rasterised glyph, packing it if it is not there yet.
///
/// Two characters a face maps to one glyph id share an entry, which is what lets a font
/// that draws `'` and `’` identically pay for one rectangle.
fn entry(atlas: &mut Atlas, glyph: &Glyph) -> Option<Placement> {
    if let Some(placement) = atlas.get(&glyph.key) {
        return Some(placement);
    }
    atlas.insert(glyph.key, glyph)
}

/// The chrome's face, as a glyph source the chrome's layout can be handed.
///
/// Borrows the whole renderer rather than its parts, because the parts are private and
/// this is the only thing outside the module that needs them. The chrome crate's own
/// `GlyphSource` is a supertrait of [`GlyphSource`] with one method added, so a caller
/// wraps this in a one-field newtype and implements that method — which is cheaper than
/// teaching this crate the chrome's name.
pub struct ChromeGlyphs<'a> {
    renderer: &'a mut Renderer,
}

impl ChromeGlyphs<'_> {
    /// The chrome's metrics: the cell its face was rasterised against.
    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        self.renderer.chrome.stack.metrics()
    }
}

impl GlyphSource for ChromeGlyphs<'_> {
    fn place(&mut self, spec: GlyphSpec) -> Option<Placement> {
        let renderer = &mut *self.renderer;
        place(
            &mut renderer.chrome.placed,
            &mut renderer.atlas,
            spec,
            || renderer.chrome.stack.rasterize(spec),
        )
    }
}

/// Two faces, an atlas they share, and the device that draws from it.
pub struct Renderer {
    /// The terminal font, which the grid is drawn in.
    grid: Face,
    /// The chrome's font, which the titlebar and the settings panel are drawn in.
    chrome: Face,
    /// Where rasterised glyphs from both faces are packed.
    atlas: Atlas,
    /// Where the frame is drawn.
    gpu: Gpu,
    /// The grid's settings, kept so a DPI change can reload the face without the caller
    /// having to hand them over again.
    grid_settings: FontSettings,
}

impl Renderer {
    /// Create a device that draws into `window`, and load both faces at `scale`.
    ///
    /// `width` and `height` are in physical pixels; `scale` is the window's DPI scale,
    /// which both faces are rasterised at and the device only records. `chrome` is passed
    /// in already built, because this crate does not know which face the chrome uses or
    /// where its bytes live.
    ///
    /// # Errors
    ///
    /// [`RendererError::Font`] when a face cannot be loaded, and [`RendererError::Gpu`]
    /// when no device can be created.
    pub fn new<W>(
        window: Arc<W>,
        width: u32,
        height: u32,
        scale: f32,
        grid_settings: &FontSettings,
        chrome: FontStack,
    ) -> Result<Self, RendererError>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let grid = FontStack::load(grid_settings, scale)?;
        let mut gpu = Gpu::new(window, width, height, scale)?;
        let atlas = Atlas::new();
        gpu.upload_atlas(atlas.width(), atlas.height(), atlas.pixels());
        Ok(Self {
            grid: Face {
                stack: grid,
                placed: Placed::default(),
            },
            chrome: Face {
                stack: chrome,
                placed: Placed::default(),
            },
            atlas,
            gpu,
            grid_settings: grid_settings.clone(),
        })
    }

    /// The grid's metrics at the current scale.
    ///
    /// The cell size in here is what the caller divides by to turn a pointer position into
    /// a cell, and what it multiplies by to lay the grid out inside the window.
    #[must_use]
    pub const fn metrics(&self) -> &Metrics {
        self.grid.stack.metrics()
    }

    /// The surface size in physical pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.gpu.size()
    }

    /// The DPI scale the faces were rasterised at.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.gpu.scale()
    }

    /// The grid's settings, as they were when the face was loaded.
    #[must_use]
    pub const fn settings(&self) -> &FontSettings {
        &self.grid_settings
    }

    /// The atlas both faces are packed into.
    #[must_use]
    pub const fn atlas(&self) -> &Atlas {
        &self.atlas
    }

    /// How many distinct characters and styles the grid has looked up.
    #[must_use]
    pub fn placed_glyphs(&self) -> usize {
        self.grid.placed.0.len() + self.chrome.placed.0.len()
    }

    /// The chrome's face, to hand to the chrome's layout.
    pub fn chrome(&mut self) -> ChromeGlyphs<'_> {
        ChromeGlyphs { renderer: self }
    }

    /// Tell the device the window changed size.
    ///
    /// A size change alone does not touch either face: the glyphs are the same size on
    /// screen, there are just more or fewer of them. [`Renderer::set_scale`] is the other
    /// one, and it does.
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        self.gpu.resize(width, height, scale);
    }

    /// Re-rasterise both faces at a new DPI scale.
    ///
    /// The chrome's stack is passed in again rather than remade here, for the same reason
    /// it was passed to [`Renderer::new`]: its bytes are not this crate's. On a scale
    /// change the caller has to rebuild it anyway, since its size is in physical pixels
    /// and the whole point is that those changed.
    ///
    /// # Errors
    ///
    /// [`RendererError::Font`] when the configured family cannot be read at the new size.
    /// Both faces are left as they were when it fails, so a bad scale is a stale frame
    /// rather than an empty window.
    pub fn set_scale(&mut self, scale: f32, chrome: FontStack) -> Result<(), RendererError> {
        let grid = FontStack::load(&self.grid_settings, scale)?;
        self.grid.stack = grid;
        self.chrome.stack = chrome;
        self.discard_glyphs();
        Ok(())
    }

    /// Reload the grid's face from new settings, for a font family or size change.
    ///
    /// # Errors
    ///
    /// [`RendererError::Font`] when the new family cannot be read. The old face stays in
    /// place when it fails, so a family the user typed wrong leaves a working terminal
    /// showing the previous font rather than an empty window.
    pub fn restyle(&mut self, settings: &FontSettings, scale: f32) -> Result<(), RendererError> {
        let grid = FontStack::load(settings, scale)?;
        self.grid.stack = grid;
        self.grid_settings = settings.clone();
        self.discard_glyphs();
        Ok(())
    }

    /// Draw a frame and present it.
    ///
    /// The atlas is re-uploaded first if placing this frame's glyphs grew it, which is why
    /// this takes `&mut self` and not `&self`: a frame is finished, not read, by being
    /// drawn, since drawing it is what places the glyphs it names.
    ///
    /// # Errors
    ///
    /// [`RendererError::Gpu`] when the surface could not be acquired or presented.
    pub fn present(&mut self, frame: &Frame) -> Result<(), RendererError> {
        self.upload_atlas_if_grown();
        self.gpu.draw(frame)?;
        Ok(())
    }

    /// Throw both caches and the atlas away, because every key in them names a face that
    /// no longer exists.
    ///
    /// The atlas is not emptied and refilled — its keys hold [`FaceKey`](zet_font::FaceKey)s
    /// from the stack being replaced, which no lookup in the new stack can ever produce,
    /// so keeping them would only make a texture that grows without being read from again.
    fn discard_glyphs(&mut self) {
        self.atlas = Atlas::new();
        self.grid.placed = Placed::default();
        self.chrome.placed = Placed::default();
        self.upload_atlas_if_grown();
    }

    /// Hand the atlas to the device if anything has been added since it last was.
    fn upload_atlas_if_grown(&mut self) {
        if !self.atlas.changed() {
            return;
        }
        self.gpu
            .upload_atlas(self.atlas.width(), self.atlas.height(), self.atlas.pixels());
        self.atlas.mark_uploaded();
    }
}

impl GlyphSource for Renderer {
    fn place(&mut self, spec: GlyphSpec) -> Option<Placement> {
        let renderer = &mut *self;
        place(&mut renderer.grid.placed, &mut renderer.atlas, spec, || {
            renderer.grid.stack.rasterize(spec)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use zet_font::{FaceKey, FamilyId, GlyphContent, GlyphKey, Weight};

    /// A face whose id is fixed for the whole test module.
    ///
    /// `FamilyId::new()` hands out a fresh id per call, so a helper that built one each
    /// time would be telling the atlas about a different face on every glyph, and every
    /// glyph would be a miss. The atlas's own tests carry the same warning.
    fn shared_face() -> FaceKey {
        static FACE: std::sync::OnceLock<FaceKey> = std::sync::OnceLock::new();
        *FACE.get_or_init(|| FaceKey {
            family: FamilyId::new(),
            weight: Weight::NORMAL,
            italic: false,
        })
    }

    /// A glyph in the shared face, with the id the caller names.
    fn glyph(id: u16) -> Glyph {
        Glyph {
            key: GlyphKey {
                face: shared_face(),
                glyph_id: id,
            },
            left: 0,
            top: 9,
            width: 7,
            height: 9,
            advance: 8.0,
            content: GlyphContent::Alpha,
            data: vec![200; 63],
        }
    }

    #[test]
    fn a_character_is_rasterised_once_however_often_it_is_asked_for() {
        let mut placed = Placed::default();
        let mut atlas = Atlas::new();
        let calls = Cell::new(0);
        // Copied rather than borrowed on each call: the closure's only capture is a
        // shared reference to the counter, so it is `Copy`, and `place` takes it by value.
        let rasterize = || {
            calls.set(calls.get() + 1);
            glyph(7)
        };

        let first = place(&mut placed, &mut atlas, GlyphSpec::new('e'), rasterize);
        let second = place(&mut placed, &mut atlas, GlyphSpec::new('e'), rasterize);
        place(&mut placed, &mut atlas, GlyphSpec::new('e'), rasterize);

        assert_eq!(calls.get(), 1, "the second and third came from the cache");
        assert_eq!(first, second);
    }

    #[test]
    fn two_characters_that_share_a_glyph_share_a_rectangle() {
        // A face that maps two characters to one glyph should be packed once. The cache is
        // per character and the atlas is per glyph, and this is the seam between them.
        let mut placed = Placed::default();
        let mut atlas = Atlas::new();
        let one = place(&mut placed, &mut atlas, GlyphSpec::new('a'), || glyph(7));
        let two = place(&mut placed, &mut atlas, GlyphSpec::new('b'), || glyph(7));

        assert_eq!(one, two, "the same glyph id is the same entry");
        assert_eq!(atlas.len(), 1, "and only one rectangle was used");
        assert_eq!(
            placed.0.len(),
            2,
            "but both characters were recorded, so neither is ever rasterised again"
        );
    }

    #[test]
    fn the_same_character_in_two_styles_is_two_glyphs() {
        let mut placed = Placed::default();
        let mut atlas = Atlas::new();
        let plain = place(&mut placed, &mut atlas, GlyphSpec::new('a'), || glyph(7));
        let bold = place(&mut placed, &mut atlas, GlyphSpec::bold('a'), || glyph(8));
        assert_ne!(plain, bold);
        assert_eq!(atlas.len(), 2);
    }

    #[test]
    fn a_rasteriser_is_not_called_again_for_a_glyph_that_could_not_be_placed() {
        // The one glyph on screen that does not fit is the one that would otherwise be
        // re-rasterised on every frame of the session, to fail again each time.
        let mut placed = Placed::default();
        let mut atlas = Atlas::new();
        let calls = Cell::new(0);
        let rasterize = || {
            calls.set(calls.get() + 1);
            let mut glyph = glyph(7);
            // Wider than the atlas is, so no shelf can ever hold it.
            glyph.width = 4096;
            glyph.data = vec![0; 4096 * 9];
            glyph
        };

        assert_eq!(
            place(&mut placed, &mut atlas, GlyphSpec::new('x'), rasterize),
            None
        );
        assert_eq!(
            place(&mut placed, &mut atlas, GlyphSpec::new('x'), rasterize),
            None
        );
        assert_eq!(calls.get(), 1, "asked once, remembered as unplaceable");
    }

    #[test]
    fn a_new_glyph_marks_the_atlas_as_changed() {
        // `present` re-uploads on this flag, so a placement that did not set it is a glyph
        // that exists on the CPU and nowhere on screen.
        let mut placed = Placed::default();
        let mut atlas = Atlas::new();
        assert!(!atlas.changed(), "nothing has been packed yet");
        place(&mut placed, &mut atlas, GlyphSpec::new('a'), || glyph(7));
        assert!(atlas.changed());

        atlas.mark_uploaded();
        assert!(!atlas.changed());
        place(&mut placed, &mut atlas, GlyphSpec::new('a'), || glyph(7));
        assert!(
            !atlas.changed(),
            "a character already packed does not dirty the texture again"
        );
        place(&mut placed, &mut atlas, GlyphSpec::new('b'), || glyph(8));
        assert!(atlas.changed(), "a new one does");
    }

    #[test]
    fn two_faces_can_share_one_atlas_without_colliding() {
        // This is why `Renderer` puts both faces in one texture, and it is worth a test
        // because the alternative — a texture each — was the first thing the chrome crate
        // reached for, on the belief that the two would overwrite each other.
        let mut grid = Placed::default();
        let mut chrome = Placed::default();
        let mut atlas = Atlas::new();
        let mut chrome_face = shared_face();
        chrome_face.weight = Weight::BOLD;

        place(&mut grid, &mut atlas, GlyphSpec::new('a'), || glyph(7));
        place(&mut chrome, &mut atlas, GlyphSpec::new('a'), || {
            let mut glyph = glyph(7);
            glyph.key.face = chrome_face;
            glyph
        });

        assert_eq!(atlas.len(), 2, "same glyph id, different face, two entries");
        assert_ne!(
            grid.0.get(&GlyphSpec::new('a')),
            chrome.0.get(&GlyphSpec::new('a')),
            "and two different rectangles"
        );
    }
}
