// Atlas rectangles: every glyph on the screen, whether the atlas stored it as coverage
// or as colour.
//
// One instance is one `GlyphQuad`, so the four corners are made here out of the vertex
// index the same way the quad pipeline makes them.

// A glyph the atlas stored as premultiplied RGBA. The value is `crate::frame::glyph_flags::COLOR`.
const FLAG_COLOR: u32 = 1;

struct Screen {
    // The surface size in physical pixels.
    size: vec2<f32>,
    // A uniform address space rounds a struct up to sixteen bytes. Saying the padding
    // out loud keeps this the same size as the struct on the Rust side, which is the
    // only thing that makes the layout checkable by reading either one.
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var atlas_sampler: sampler;
@group(0) @binding(2) var atlas: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    // Flat, because the flag is a property of the instance: an interpolated one would
    // be a mixture of the two kinds of glyph at every pixel and would mean nothing.
    @location(2) @interpolate(flat) flags: u32,
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
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) flags: u32,
) -> VertexOutput {
    // 0 -> left-top, 1 -> right-top, 2 -> left-bottom, 3 -> right-bottom: a triangle
    // strip in the order a strip wants its corners.
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));

    var output: VertexOutput;
    output.position = pixel_to_clip(rect.xy + corner * rect.zw);
    output.color = color;
    output.uv = uv.xy + corner * (uv.zw - uv.xy);
    output.flags = flags;
    return output;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(atlas, atlas_sampler, in.uv);

    if in.flags == FLAG_COLOR {
        // The atlas stored this glyph as premultiplied RGBA — an emoji is not the
        // theme's foreground, so none of the tint's colour is read. Its alpha scales the
        // whole texel rather than the alpha channel alone, which is what keeps the
        // result premultiplied: scaling only the alpha would leave the colour at full
        // strength and blend the glyph into the background brighter than it was
        // rasterised, where a faded glyph is meant to blend toward the background.
        return texel * in.color.a;
    }

    // The atlas stored this glyph as white with the coverage in its alpha channel, so
    // scaling the premultiplied tint by that coverage scales colour and alpha together
    // and stays premultiplied. It is also why an antialiased edge blends correctly: at
    // half coverage the whole glyph is half there, not half-bright.
    //
    // The coverage is read out of the alpha channel rather than taken as the whole
    // texel, and the difference is the whole picture. The texel's colour is white, so
    // multiplying by all four channels scales the tint's colour by one and leaves it at
    // full strength — which draws every glyph as a solid rectangle of its tint, in the
    // right place and the right size and the right colour, with only the edges dimmed.
    // A pixel that is a quarter covered is a quarter of the tint, and `texel.a` is what
    // says so.
    return in.color * texel.a;
}
