// The window's picture, when the configuration names one.
//
// One instance is one `PictureQuad` and it covers the window. Like the gradient it is drawn
// before every batch and after the clear, so what is under it is the theme's ground and what
// is over it is the terminal.
//
// There is no arithmetic here worth the name: `PictureQuad::new` decided which part of the
// texture lands in the window, and all that is left is to interpolate four numbers across
// two triangles and sample. That is the point of the split — the aspect-ratio decision is
// unit-tested on the CPU, and what runs here is a texture read.

struct Screen {
    // The surface size in physical pixels.
    size: vec2<f32>,
    // A uniform address space rounds a struct up to sixteen bytes. Saying the padding out
    // loud keeps this the same size as the struct on the Rust side.
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var picture_sampler: sampler;
@group(0) @binding(2) var picture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
}

// Physical pixels from the top-left of the surface to clip space, where y is up. A frame's
// coordinates and the framebuffer's both run downward from the top-left, so the y axis is
// the only one that has to be turned over.
fn pixel_to_clip(pixel: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(
        pixel.x / screen.size.x * 2.0 - 1.0,
        1.0 - pixel.y / screen.size.y * 2.0,
        0.0,
        1.0,
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) index: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) opacity: f32,
) -> VertexOutput {
    // 0 -> left-top, 1 -> right-top, 2 -> left-bottom, 3 -> right-bottom: a triangle
    // strip in the order a strip wants its corners. The same corner picks the pixel and
    // the texel, so the two cannot come apart.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let pixel = rect.xy + corner * rect.zw;

    var output: VertexOutput;
    output.position = pixel_to_clip(pixel);
    output.uv = mix(uv.xy, uv.zw, corner);
    output.opacity = opacity;
    return output;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // The texture is sRGB, so this arrives already converted to linear light and already
    // premultiplied — which is the form every colour in a frame is in, and the reason
    // fading a picture is one multiply of all four channels rather than an alpha blend
    // written out by hand.
    return textureSample(picture, picture_sampler, in.uv) * in.opacity;
}
