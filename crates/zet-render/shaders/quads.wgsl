// Solid rectangles: cell backgrounds, the selection, the cursor, the underline and
// strikeout bars, every hairline and fill in the chrome.
//
// One instance is one `Quad`, so the frame carries one entry per rectangle and the four
// corners are made here out of the vertex index. That is what lets a whole run of
// rectangles be one buffer and one draw call.

struct Screen {
    // The surface size in physical pixels.
    size: vec2<f32>,
    // A uniform address space rounds a struct up to sixteen bytes. Saying the padding
    // out loud keeps this the same size as the struct on the Rust side, which is the
    // only thing that makes the layout checkable by reading either one.
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> screen: Screen;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// Physical pixels from the top-left of the surface to clip space, where y is up. A
// frame's coordinates and the framebuffer's both run downward from the top-left, so the
// y axis is the only one that has to be turned over.
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
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    // 0 -> left-top, 1 -> right-top, 2 -> left-bottom, 3 -> right-bottom: a triangle
    // strip in the order a strip wants its corners.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));

    var output: VertexOutput;
    output.position = pixel_to_clip(rect.xy + corner * rect.zw);
    output.color = color;
    return output;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Already linear and premultiplied: the two conversions happen once, where the frame
    // is built, rather than per pixel.
    return in.color;
}
