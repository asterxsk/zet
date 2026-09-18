//! A picture from the disk, decoded into pixels the renderer can upload.
//!
//! `window.background`'s image kind is a path and an opacity, and everything between
//! those two and a texture is here: open the file, ask Windows what is in it, and hand
//! back RGBA bytes in the order a texture wants them.
//!
//! # Why Windows decodes it
//!
//! A PNG decoder is a dependency, a supply chain, and a parser for a format designed for
//! hostile input; GDI+ is already installed on every machine zet runs on, has been the
//! Windows image path since XP, and reads PNG, JPEG, BMP, GIF, and TIFF without being
//! told which is which. Adding a crate to this workspace to do a worse job of it would be
//! the wrong trade, and the four calls below are the whole of the alternative.
//!
//! # What comes back
//!
//! What GDI+ gives for `PixelFormat32bppARGB` is straight alpha, in the order B, G, R, A
//! — the format's name is about the value in a register rather than the bytes in memory.
//! What leaves here is RGBA, premultiplied: the order a texture wants and the form every
//! colour in a frame is in. An image with no alpha — a JPEG, or a PNG without a
//! transparency channel — premultiplies to itself, so the conversion costs one multiply
//! per pixel and removes a case from the renderer.

// Win32 is an unsafe API, and this module is the third of the three that call it.
// Every block carries a SAFETY comment naming the contract it satisfies.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::OnceLock;

use windows_sys::Win32::Graphics::GdiPlus::{
    BitmapData, GdipBitmapLockBits, GdipBitmapUnlockBits, GdipCreateBitmapFromFile,
    GdipCreateBitmapFromScan0, GdipDeleteGraphics, GdipDisposeImage, GdipDrawImageRectI,
    GdipGetImageGraphicsContext, GdipGetImageHeight, GdipGetImageWidth, GdipSetInterpolationMode,
    GdipSetPixelOffsetMode, GdiplusStartup, GdiplusStartupInput, GpBitmap, GpGraphics, GpImage,
    ImageLockModeRead, InterpolationModeHighQualityBicubic, Ok, PixelFormatAlpha,
    PixelFormatCanonical, PixelFormatGDI, PixelOffsetModeHighQuality, Rect,
};

/// A picture, as pixels.
///
/// Owned and top-down: row zero is the top of the picture, which is what both the frame
/// and a texture want, so nothing downstream has to know that the format it came from
/// stores its rows the other way up.
pub struct Picture {
    /// The picture's width in pixels.
    pub width: u32,
    /// Its height.
    pub height: u32,
    /// `width * height * 4` bytes of premultiplied RGBA.
    pub pixels: Vec<u8>,
}

/// The longest side a decoded picture may have.
///
/// It is the largest texture the device will make — `wgpu`'s default
/// `max_texture_dimension_2d` — and going over it is not a picture that draws slowly: the
/// upload fails validation and the process panics. Pictures over it are ordinary, not
/// exotic: a 48-megapixel photograph is eight thousand pixels across and a panorama is
/// wider, and a background is scaled to fill a window whatever size it arrives.
const LONGEST: u32 = 8192;

/// `PixelFormat32bppARGB`, spelled the way the SDK's own macro spells it.
///
/// The individual pixel formats are not in the bindings, only the flag bits, and this is
/// their definition rather than a number copied out of a header: the low byte is the
/// format's index in GDI+'s table and the rest are the flags that say what it can do. It
/// is spelled the way the macro is — decimal constants and shifts — because that is what
/// makes it checkable against the header, which is the only thing it can be checked
/// against.
#[allow(clippy::decimal_bitwise_operands)]
const ARGB: i32 = 10
    | (32 << 8)
    | PixelFormatAlpha.cast_signed()
    | PixelFormatGDI.cast_signed()
    | PixelFormatCanonical.cast_signed();

/// Decode the picture at `path`.
///
/// `None` for every failure there is — no such file, a format GDI+ cannot read, a lock
/// that fails, a picture whose size does not fit the arithmetic below — because they all
/// want the same thing from the caller: no background picture, and the theme's ground
/// where it would have been.
#[must_use]
pub fn load(path: &Path) -> Option<Picture> {
    let file = wide(&path.to_string_lossy());
    // SAFETY: `token` is a live local and `input` is a plain struct whose only field that
    // needs a value is the version, which must be 1 or 2. `output` is documented as
    // optional and nothing here uses it. The call is guarded by a `OnceLock`, so it
    // happens exactly once per process whatever else calls `load` — which matters because
    // GDI+ wants to be started once and is happy to be left running, and stopping it
    // would invalidate pictures that are still in flight.
    let token = STARTUP.get_or_init(|| {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..GdiplusStartupInput::default()
        };
        let mut token = 0usize;
        let status =
            unsafe { GdiplusStartup(&raw mut token, &raw const input, std::ptr::null_mut()) };
        if status == Ok { token } else { 0 }
    });
    if *token == 0 {
        return None;
    }

    let mut bitmap: *mut GpBitmap = std::ptr::null_mut();
    // SAFETY: the path is a null-terminated UTF-16 buffer alive for the call, and
    // `bitmap` is a live local that the API writes a decoded image pointer into on
    // success. Nothing reads it unless the status says the call worked.
    if unsafe { GdipCreateBitmapFromFile(file.as_ptr(), &raw mut bitmap) } != Ok {
        return None;
    }
    // From here the picture has to be disposed whatever happens, which is what the guard
    // is for: an early return that leaked it would leak a decode per attempt.
    let guard = Guard(bitmap);
    let (mut width, mut height) = (0u32, 0u32);
    // SAFETY: `bitmap` is a live decoded image owned by the guard, and both calls write
    // one `u32` into a live local. Neither can fail for an image that exists.
    unsafe {
        GdipGetImageWidth(guard.0.cast::<GpImage>(), &raw mut width);
        GdipGetImageHeight(guard.0.cast::<GpImage>(), &raw mut height);
    }
    if width == 0 || height == 0 {
        return None;
    }

    // Anything the device would refuse is drawn into a bitmap it will take, at the size
    // it would have been shown at anyway. The smaller bitmap is the one read from here on,
    // and the original is still disposed by the guard above it.
    let mut shrunk = None;
    if width > LONGEST || height > LONGEST {
        let (small_width, small_height) = fit(width, height);
        let mut small: *mut GpBitmap = std::ptr::null_mut();
        // SAFETY: a null `scan0` asks GDI+ to allocate the pixels itself, which is what
        // makes the new bitmap an ordinary one this function owns.
        if unsafe {
            GdipCreateBitmapFromScan0(
                i32::try_from(small_width).ok()?,
                i32::try_from(small_height).ok()?,
                0,
                ARGB,
                std::ptr::null(),
                &raw mut small,
            )
        } != Ok
        {
            return None;
        }
        shrunk = Some(Guard(small));
        if !rescale(guard.0, small, small_width, small_height) {
            return None;
        }
        width = small_width;
        height = small_height;
    }
    let image = shrunk.as_ref().map_or(guard.0, |small| small.0);

    let rect = Rect {
        X: 0,
        Y: 0,
        // Both sizes come out of the image itself and an image is no wider than an `i32`
        // by definition of the format; the conversion is the API's, not a narrowing.
        Width: i32::try_from(width).ok()?,
        Height: i32::try_from(height).ok()?,
    };
    let mut data = BitmapData::default();
    // SAFETY: `bitmap` is live, `rect` describes it exactly, and `data` is a live local
    // the API fills in with the address, stride, and format of a locked buffer. The
    // buffer belongs to the image and stays valid until the matching unlock below.
    let locked = unsafe {
        GdipBitmapLockBits(
            image,
            &raw const rect,
            ImageLockModeRead as u32,
            ARGB,
            &raw mut data,
        )
    };
    if locked != Ok {
        return None;
    }

    // SAFETY: the lock above succeeded, so `Scan0` points at `Height` rows of at least
    // `Stride` bytes each and stays valid until the unlock below — which is the only
    // thing that can happen between here and there. Every row read is inside the image:
    // the loop runs `height` times and takes `width` pixels from each row, and a stride
    // is never narrower than a row of the format it is in.
    let mut pixels = unsafe {
        let stride = usize::try_from(data.Stride.unsigned_abs()).ok()?;
        let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
        let source = std::slice::from_raw_parts(data.Scan0.cast::<u8>(), stride * height as usize);
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        for row in 0..height as usize {
            let start = row.checked_mul(stride)?;
            pixels.extend_from_slice(&source[start..start + row_bytes]);
        }
        GdipBitmapUnlockBits(image, &raw mut data);
        pixels
    };

    to_rgba(&mut pixels);
    Some(Picture {
        width,
        height,
        pixels,
    })
}

/// The size a picture of this size is shrunk to so that neither side is over the limit.
///
/// A pure function of two numbers, for the usual reason: the arithmetic is what a wrong
/// answer would be a wrong answer about, and GDI+ is the one part of the shrink that can
/// only be checked by looking at the result.
fn fit(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= LONGEST {
        return (width, height);
    }
    // In integers, and not in floating point: the same multiplication and division is
    // exact either way, and a size that is a whole number of pixels is not something to
    // arrive at through a rounding error. The shift up is what makes the division round to
    // nearest rather than down, and `u64` is what keeps the product from overflowing.
    let side = |length: u32| {
        let scaled =
            (u64::from(length) * u64::from(LONGEST) + u64::from(longest) / 2) / u64::from(longest);
        // Rounded, and never to nothing: everything here is at most `LONGEST`, and the
        // floor is what keeps a thousand-to-one panorama a picture rather than a texture
        // with no height.
        u32::try_from(scaled).unwrap_or(LONGEST).clamp(1, LONGEST)
    };
    (side(width), side(height))
}

/// Draw `from` into `into`, filling it exactly, with the best filter GDI+ has.
///
/// The picture is being made smaller, which is the one direction where a resample can be
/// close to lossless, and a background is stretched across the window afterwards — so a
/// nearest-neighbour shrink would be a visible, permanent loss of detail rather than a
/// saving of anything.
fn rescale(from: *mut GpBitmap, into: *mut GpBitmap, width: u32, height: u32) -> bool {
    let mut graphics: *mut GpGraphics = std::ptr::null_mut();
    // SAFETY: both bitmaps are live and owned by the caller, and `graphics` is a live
    // local the API writes a context into on success. Nothing reads it unless it did.
    if unsafe { GdipGetImageGraphicsContext(into.cast::<GpImage>(), &raw mut graphics) } != Ok {
        return false;
    }
    // Both sides are at most `LONGEST`, so they are already the `i32` the API takes.
    let (across, down) = (width.cast_signed(), height.cast_signed());
    // SAFETY: the context is live and is this function's to use; the modes are GDI+'s own
    // constants; and the source image outlives the call. Every call returns a status that
    // says whether it worked, and the one that does the drawing is the one that is checked.
    let drawn = unsafe {
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
        GdipDrawImageRectI(graphics, from.cast::<GpImage>(), 0, 0, across, down)
    };
    // SAFETY: the context is deleted exactly once, here, and nothing uses it afterwards.
    unsafe {
        GdipDeleteGraphics(graphics);
    }
    drawn == Ok
}

/// Turn a buffer of GDI+'s BGRA into the RGBA a texture wants, premultiplied.
///
/// Two conversions in one pass because they are both per-pixel and both about the same
/// four bytes, and because the order it comes back in is the part of this module most
/// likely to be got wrong quietly: every pixel of every picture is a different colour if
/// the red and blue are left swapped, and the result is still a picture. It is not
/// something a type can say, so it is something a test has to.
///
/// GDI+ hands back straight alpha and every colour in a frame is premultiplied; the
/// conversion is here rather than in the shader because it is once per picture instead of
/// once per pixel per frame. A picture with no transparency is unchanged by the multiply.
fn to_rgba(pixels: &mut [u8]) {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            // Rounded rather than truncated: a fully opaque pixel has to come back as
            // itself, and truncating the multiply would take 255 to 254.
            *channel = u8::try_from((u32::from(*channel) * alpha + 127) / 255).unwrap_or(255);
        }
    }
}

/// Disposes the image it holds, however the function it was made in returns.
struct Guard(*mut GpBitmap);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: the pointer came from `GdipCreateBitmapFromFile`, which succeeded, so it
        // is a live image this function owns and nothing else has disposed. GDI+ is
        // started for the life of the process, so the runtime it was created under is
        // still there to free it.
        unsafe {
            GdipDisposeImage(self.0.cast::<GpImage>());
        }
    }
}

/// The GDI+ token, so that the runtime is started once and never stopped.
static STARTUP: OnceLock<usize> = OnceLock::new();

/// A Rust string as a null-terminated UTF-16 buffer.
///
/// The same conversion `platform` needs, for the same reason: the `W` entry points take
/// `PCWSTR` and the buffer has to outlive the call.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opaque_pixel_arrives_red_where_red_was_asked_for() {
        // 128, 0, 255 in BGRA is the colour the world calls `#FF0080`.
        let mut pixels = vec![128, 0, 255, 255];
        to_rgba(&mut pixels);
        assert_eq!(pixels, vec![255, 0, 128, 255]);
    }

    #[test]
    fn a_transparent_pixel_loses_its_colour_entirely() {
        // What premultiplied alpha means: a pixel with nothing in it contributes nothing
        // to what it is drawn over, whatever colour it claims to be.
        let mut pixels = vec![0, 128, 255, 0];
        to_rgba(&mut pixels);
        assert_eq!(pixels, vec![0, 0, 0, 0]);
    }

    #[test]
    fn half_transparent_white_arrives_as_half_grey() {
        // 255 * 128 / 255 is 128 to the byte, and 254 * 128 / 255 is 127.5 — which rounds
        // to 128 in the first case and 127 in the second, as arithmetic rather than as
        // truncation: a buffer that lost the half every time would darken the edges of
        // every transparent picture by a step.
        let mut pixels = vec![255, 255, 255, 128, 254, 254, 254, 128];
        to_rgba(&mut pixels);
        assert_eq!(pixels, vec![128, 128, 128, 128, 127, 127, 127, 128]);
    }

    #[test]
    fn every_pixel_of_a_buffer_is_converted_and_nothing_between_them_is_touched() {
        // Three pixels back to back: the loop has to be one pixel at a time and four
        // bytes at a time, or the buffer comes out shifted by however much it is not.
        let mut pixels = vec![0, 0, 255, 255, 255, 0, 0, 128, 9, 9, 9, 0];
        to_rgba(&mut pixels);
        assert_eq!(pixels, vec![255, 0, 0, 255, 0, 0, 128, 128, 0, 0, 0, 0]);
    }

    /// The picture the tests decode, which is four pixels and one of each case the
    /// decoder has to get right.
    ///
    /// A file in the repository rather than one written to a temporary directory: this is
    /// the only test here that can fail for a reason outside this process — a GDI+ that
    /// will not start, a lock that comes back in a format nobody asked for — and a fixture
    /// that is the same bytes on every machine is what makes such a failure mean the code
    /// rather than the machine.
    fn fixture() -> Picture {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/tiny.png");
        load(&path).expect("the fixture is a picture")
    }

    #[test]
    fn a_picture_comes_back_at_its_own_size_with_four_bytes_a_pixel() {
        let picture = fixture();
        assert_eq!((picture.width, picture.height), (2, 2));
        assert_eq!(picture.pixels.len(), 2 * 2 * 4);
    }

    #[test]
    fn a_picture_arrives_red_green_and_blue_the_way_round_a_frame_expects_them() {
        // GDI+ is asked for 32bpp ARGB and hands back BGRA in memory, because that is what
        // 32bpp ARGB is when you write it down. Nothing in the type system says so, and
        // every pixel of every picture is wrong in a way that still looks like a picture
        // if this is missed — a red sky and a blue sun.
        let picture = fixture();
        let pixel = |x: usize, y: usize| {
            let at = (y * 2 + x) * 4;
            &picture.pixels[at..at + 4]
        };
        assert_eq!(pixel(0, 0), [255, 0, 0, 255], "the red pixel");
        assert_eq!(pixel(1, 1), [0, 0, 255, 255], "the blue pixel");
    }

    #[test]
    fn a_picture_with_transparency_arrives_premultiplied() {
        // The four cases, and the half-transparent one is the reason the conversion
        // exists: white at half alpha is half-grey in every channel, not white.
        let picture = fixture();
        let pixel = |x: usize, y: usize| {
            let at = (y * 2 + x) * 4;
            &picture.pixels[at..at + 4]
        };
        assert_eq!(
            pixel(1, 0),
            [0, 0, 0, 0],
            "a transparent pixel keeps nothing"
        );
        assert_eq!(pixel(0, 1), [128, 128, 128, 128], "half of white");
    }

    #[test]
    fn a_picture_that_fits_is_left_at_its_own_size() {
        assert_eq!(fit(1920, 1080), (1920, 1080));
        // The limit itself is not over the limit: a texture may be exactly as large as
        // the device says it can be, and shrinking that one would be a resize per launch
        // for the people who chose it.
        assert_eq!(fit(LONGEST, LONGEST), (LONGEST, LONGEST));
        assert_eq!(fit(LONGEST + 1, 100), (LONGEST, 100));
    }

    #[test]
    fn a_picture_that_does_not_fit_is_shrunk_by_its_longest_side() {
        // A panorama: the height is a rounding away from nothing, and one pixel is the
        // floor — a picture zero pixels tall is a texture that cannot be made.
        assert_eq!(fit(10_000, 500), (8192, 410));
        assert_eq!(fit(500, 10_000), (410, 8192));
        assert_eq!(fit(20_000, 4), (8192, 2));
    }

    #[test]
    fn the_shrunk_picture_keeps_the_shape_it_arrived_with() {
        // The crop a window does to a picture assumes the picture has the shape the file
        // says it has, so a resize that rounded the two sides independently would be a
        // picture that is very slightly stretched — the one thing scaling to fill exists
        // to avoid.
        for (width, height) in [(10_000, 5000), (8193, 8191), (30_000, 17_000)] {
            let (across, down) = fit(width, height);
            let before = f64::from(width) / f64::from(height);
            let after = f64::from(across) / f64::from(down);
            assert!(
                (before - after).abs() / before < 0.005,
                "{width}x{height} came back {across}x{down}"
            );
            assert!(across <= LONGEST && down <= LONGEST);
        }
    }

    #[test]
    fn a_file_that_is_not_a_picture_is_not_a_picture() {
        assert!(load(Path::new("no-such-file.png")).is_none());
        // A file that exists and is not an image: GDI+ sniffs the contents rather than
        // the extension, so this is the same answer as the line above.
        assert!(load(Path::new("Cargo.toml")).is_none());
    }
}
