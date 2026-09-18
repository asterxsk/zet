// The window's background, when it is not one flat colour.
//
// One instance is one `Gradient` and it covers the window. It is drawn before every batch
// in the frame, so what it is drawn over is the clear colour and what is drawn over it is
// the terminal: the grid's own ground is skipped when a backdrop is set, which is what
// lets a gradient show through the cells that have no background of their own.
//
// The arithmetic that turns an angle into the axis below is on the Rust side, in
// `Gradient::new`, and is unit-tested there. What is left here is the interpolation.

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
    // The pixel this fragment is at, in the frame's coordinates. Passed rather than
    // derived from the clip position, because the division that would undo the projection
    // is three lines of arithmetic to recover a number that was already known.
    @location(0) pixel: vec2<f32>,
    // `first` and `second` rather than `from` and `to`: `from` is a reserved word in
    // WGSL, which the parser says so plainly about and which nothing else in this file
    // would have hinted at.
    @location(1) first: vec4<f32>,
    @location(2) second: vec4<f32>,
    @location(3) axis: vec4<f32>,
    @location(4) origin: vec2<f32>,
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
    @location(1) first: vec4<f32>,
    @location(2) second: vec4<f32>,
    @location(3) axis: vec4<f32>,
) -> VertexOutput {
    // 0 -> left-top, 1 -> right-top, 2 -> left-bottom, 3 -> right-bottom: a triangle
    // strip in the order a strip wants its corners.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let pixel = rect.xy + corner * rect.zw;

    var output: VertexOutput;
    output.position = pixel_to_clip(pixel);
    output.pixel = pixel;
    output.first = first;
    output.second = second;
    output.axis = axis;
    output.origin = rect.xy;
    return output;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // How far along the interval the rectangle projects onto this pixel is. `axis.w` is
    // where that interval starts, which is a corner and not the origin for every angle
    // that is not a multiple of ninety degrees.
    let along = dot(in.pixel - in.origin, in.axis.xy) - in.axis.w;
    let t = clamp(along / in.axis.z, 0.0, 1.0);
    // Both stops are linear and premultiplied, so the mix is of the colours themselves.
    return mix(in.first, in.second, t);
}
