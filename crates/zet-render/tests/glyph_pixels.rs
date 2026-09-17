//! A character's pixels, from the font all the way to the framebuffer.
//!
//! Every other test of the glyph path hands the atlas a glyph it built by hand:
//! `atlas.rs` writes a rectangle of one coverage value, and `gpu.rs` uploads a texture it
//! wrote as a literal. That is what makes them worth having — they test the packing and
//! the blending against values a reader can check by eye — and it is also why none of
//! them could see the fault this file was written for. A rasteriser that returns a
//! legible mask and a shader that turns that mask into a solid rectangle draws every
//! glyph in the right place, at the right size, in the right colour, and every assertion
//! in both of those modules passes.
//!
//! So this starts where they stop, at a real face rasterising a real character, and ends
//! where the user does: at the bytes a frame leaves behind.

// The three cast lints `zet-render`'s own header explains, for the same two reasons: a
// coverage byte is a fraction of 255 and a glyph is smaller than the 2^24 where `f32`
// starts rounding. Saying so once here is what keeps the arithmetic below readable.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use zet_config::FontSettings;
use zet_font::{FontStack, GlyphContent, GlyphSpec};
use zet_render::atlas::Atlas;
use zet_render::frame::{Frame, GlyphQuad};
use zet_render::gpu::{Gpu, GpuError};

/// The surface every test draws into.
const SIZE: u32 = 64;

/// An offscreen device, or `None` on a machine with no adapter at all.
///
/// The rule `gpu.rs`'s own tests follow: a machine without an adapter cannot fail these
/// assertions in a way that says anything about the renderer, so the note is printed and
/// the test passes.
fn device() -> Option<Gpu> {
    match Gpu::offscreen(SIZE, SIZE, 1.0) {
        Ok(gpu) => Some(gpu),
        Err(GpuError::NoAdapter(message)) => {
            println!("no graphics adapter on this machine, skipping: {message}");
            None
        }
        Err(error) => panic!("the offscreen device could not be created: {error}"),
    }
}

/// The face the default configuration resolves to.
fn stack() -> FontStack {
    FontStack::load(&FontSettings::default(), 1.0).expect("the default font should load")
}

/// The byte an sRGB framebuffer stores for a linear value.
///
/// A mask glyph's coverage lives in a texture's alpha channel, which is linear even in an
/// `Rgba8UnormSrgb` texture — the format's transfer function is not applied to it. The
/// shader emits that coverage as linear light and the surface encodes it on the way out,
/// so the byte that comes back is the coverage put through this.
fn srgb(linear: f32) -> u8 {
    let encoded = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

/// A character's mask, drawn as itself.
///
/// `H` is the character to reach for. Two stems and the counter between them are the
/// smallest shape that cannot be mistaken for a filled box, and it is a shape a terminal
/// draws constantly; an `i` would pass against a solid mask by accident and a `.` would
/// pass against nothing at all.
#[test]
fn a_glyph_reaches_the_frame_with_its_counter_still_in_it() {
    let Some(mut gpu) = device() else {
        return;
    };

    let mut stack = stack();
    let glyph = stack.rasterize(GlyphSpec::new('H'));
    assert_eq!(
        glyph.content,
        GlyphContent::Alpha,
        "an outline glyph should be a coverage mask"
    );
    assert!(glyph.width > 2 && glyph.height > 2, "H should have ink");

    // The mask on its own, before this crate has touched it. Both halves of this matter:
    // an empty mask draws nothing, and a full one is the shape a solid rectangle has, so
    // without this the comparison below could be satisfied by a picture that is wrong in
    // the same way the mask is.
    assert!(
        glyph.data.contains(&0),
        "H rasterised to a solid block: every pixel of its box is covered"
    );
    assert!(
        glyph.data.iter().any(|&value| value != 0),
        "H rasterised to an empty mask"
    );

    let mut atlas = Atlas::new();
    let placement = atlas
        .insert(glyph.key, &glyph)
        .expect("an empty atlas has room for one glyph");
    assert!(!placement.color, "a mask glyph is not a colour glyph");
    gpu.upload_atlas(atlas.width(), atlas.height(), atlas.pixels());

    // The glyph at the surface's own origin, so that a lit pixel's coordinates are the
    // mask's coordinates and the comparison below needs no arithmetic to line them up.
    let mut frame = Frame::new();
    frame.clear = [0.0, 0.0, 0.0, 1.0];
    frame.begin_glyphs();
    frame.push_glyph(GlyphQuad::alpha(
        [0.0, 0.0, placement.width as f32, placement.height as f32],
        placement.uv,
        [1.0, 1.0, 1.0, 1.0],
    ));
    frame.end_glyphs();
    gpu.draw(&frame).expect("the frame should draw");
    let pixels = gpu.read_pixels().expect("the frame should be readable");

    // Every pixel of the glyph's own box, against the mask it came from. The texture
    // coordinates land on texel centres, so nothing is filtered and the two are the same
    // picture; the slack is the rounding a GPU's blending is entitled to.
    for y in 0..placement.height {
        for x in 0..placement.width {
            let coverage = glyph.data[(y * glyph.width + x) as usize];
            let expected = srgb(f32::from(coverage) / 255.0);
            let at = ((y * SIZE + x) * 4) as usize;
            for channel in 0..3 {
                let got = pixels[at + channel];
                assert!(
                    got.abs_diff(expected) <= 2,
                    "pixel ({x}, {y}) channel {channel}: the mask says {coverage} (drawn \
                     as {expected}) and the frame holds {got} — a partly covered pixel is \
                     being drawn at full strength"
                );
            }
        }
    }
}
