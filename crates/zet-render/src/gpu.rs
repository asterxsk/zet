//! The graphics device: two pipelines, one texture, one pass.
//!
//! Everything in this module is the part of rendering that talks to `wgpu`. It knows
//! about vertices, bind groups, and surface configuration, and it knows nothing about
//! terminals — it is handed a [`Frame`] and draws it. Keeping the boundary there is what
//! makes the rest of the crate testable without a window, and what makes a rendering bug
//! answerable by looking at the frame instead of at the picture.
//!
//! # The two pipelines
//!
//! * **Quads.** A rectangle of one colour: cell backgrounds, selection, the cursor, the
//!   underline and strikeout bars, every hairline and fill in the chrome.
//! * **Glyphs.** A rectangle of atlas texture, either tinted by the instance colour or
//!   drawn as it is, depending on the instance's `flags`.
//!
//! Both blend premultiplied over what is already there, in the order the frame's batches
//! give them, in a single render pass that ends by presenting.
//!
//! # Colour
//!
//! The surface is configured as `Bgra8UnormSrgb`, so the value written by a fragment
//! shader is linear and the hardware encodes it on the way out. Quad colours in a
//! [`Frame`] are therefore linear, and the atlas is sampled with an sRGB view so that
//! its texels arrive linear too. Blending then happens in linear space, which is the
//! only space in which "fifty percent grey over fifty percent grey" is the right answer.
//!
//! The atlas stores an alpha glyph as white with the coverage in the alpha channel, and
//! a colour glyph as premultiplied RGBA. That single choice is what lets both kinds go
//! through one shader with one blend state: for an alpha glyph the tint multiplies a
//! white texel into a premultiplied result, and for a colour glyph the tint is not read
//! at all.

use std::mem::size_of;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::frame::{BatchKind, Frame, GlyphQuad, Quad};

/// Something went wrong talking to the graphics device.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter could run the pipelines.
    #[error("no graphics adapter could be used: {0}")]
    NoAdapter(String),

    /// The adapter was found but would not give us a device.
    #[error("the graphics device could not be created: {0}")]
    Device(String),

    /// The window's surface could not be created or configured.
    #[error("the window surface is unusable: {0}")]
    Surface(String),

    /// A frame was asked for outside the mode that supports it.
    #[error("the device is not offscreen, so there is nothing to read back")]
    NotOffscreen,
}

/// The result of talking to the graphics device.
pub type GpuResult<T> = Result<T, GpuError>;

/// The uniform both vertex shaders turn pixels into clip space with.
///
/// The padding is load-bearing rather than tidy: a uniform address space lays a struct
/// out in multiples of sixteen bytes, so a Rust struct of one `vec2` would be eight and
/// the shader would read a `screen` of the wrong shape. The two declarations are kept
/// the same size by hand because nothing checks them against each other.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Screen {
    /// The surface size in physical pixels.
    size: [f32; 2],
    /// Never read. Present so the struct is the sixteen bytes the shader expects.
    _pad: [f32; 2],
}

impl Screen {
    /// The uniform value for a surface of `size` physical pixels.
    fn new(size: (u32, u32)) -> Self {
        Self {
            size: [size.0 as f32, size.1 as f32],
            _pad: [0.0; 2],
        }
    }
}

/// How a run of [`Quad`]s is read as instance data.
///
/// One rectangle is one instance: its four corners are made in the vertex shader from
/// the vertex index, so a frame carries one entry per rectangle rather than four and a
/// run of them is a single draw.
const QUAD_VERTICES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<Quad>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4],
};

/// How a run of [`GlyphQuad`]s is read as instance data.
const GLYPH_VERTICES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<GlyphQuad>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &wgpu::vertex_attr_array![
        0 => Float32x4,
        1 => Float32x4,
        2 => Float32x4,
        3 => Uint32,
    ],
};

/// The vertices a rectangle is drawn with: a triangle strip of its four corners.
const CORNERS: std::ops::Range<u32> = 0..4;

/// The size a vertex buffer starts at, and the floor every reallocation is raised to.
///
/// A buffer cannot be zero-sized and be bound, and one page of rectangles covers the
/// first few frames of a session without asking the driver for anything.
const VERTEX_BUFFER_START: u64 = 1 << 12;

/// The bind group the two pipelines share.
///
/// Holding the atlas texture, its sampler, and the screen uniform in one group is what
/// makes the per-batch work one `set_pipeline` and one draw: the quad pipeline is given
/// a layout with two bindings it never reads, which costs nothing and means a frame
/// never has to rebind between a run of rectangles and a run of glyphs.
struct Atlas {
    /// The texture, whenever the CPU-side atlas is uploaded.
    texture: wgpu::Texture,
    /// The group the pass binds. It holds the view it was made from — an sRGB view of an
    /// sRGB texture, which is what makes a texel arrive in the shader as linear light —
    /// and is rebuilt with the texture rather than apart from it.
    group: wgpu::BindGroup,
    /// The texture's size in texels, so that an upload of the same size can skip
    /// rebuilding the texture and the group with it.
    size: (u32, u32),
}

/// What the device draws into.
enum Target {
    /// A window, presented to after every frame.
    Window {
        /// The surface the window's swapchain hangs off. It owns the window handle, so
        /// the window outlives it without the caller having to hold on to anything.
        surface: wgpu::Surface<'static>,
        /// The configuration, kept because a resize reconfigures rather than recreating.
        config: wgpu::SurfaceConfiguration,
    },
    /// An offscreen texture, read back on demand. What the tests use.
    Offscreen {
        /// The texture a frame is drawn into.
        texture: wgpu::Texture,
        /// The view the render pass attaches.
        view: wgpu::TextureView,
        /// Needed to recreate the texture at a new size.
        format: wgpu::TextureFormat,
    },
}

/// A configured graphics device, its pipelines, and its atlas texture.
///
/// The atlas is a single RGBA texture owned here and re-uploaded whenever the CPU-side
/// atlas grows. Keeping the texture on this side of the boundary is what stops the
/// packing code from having to know about `wgpu` at all.
pub struct Gpu {
    /// The device everything here was created from.
    device: wgpu::Device,
    /// The queue frames are submitted to and the atlas is uploaded through.
    queue: wgpu::Queue,
    /// The surface size in physical pixels.
    size: (u32, u32),
    /// The window scale this device was configured at.
    scale: f32,
    /// Where this device draws to.
    target: Target,
    /// The pipeline that draws runs of [`Frame::quads`].
    quads: wgpu::RenderPipeline,
    /// The pipeline that draws runs of [`Frame::glyphs`].
    glyphs: wgpu::RenderPipeline,
    /// The screen-size uniform, written whenever the size changes.
    screen: wgpu::Buffer,
    /// The layout the atlas group is created from, kept for the uploads that rebuild it.
    atlas_layout: wgpu::BindGroupLayout,
    /// Bilinear and clamped, so that a glyph sampled a hair outside its rectangle picks
    /// up its own edge texels rather than the glyph next door.
    atlas_sampler: wgpu::Sampler,
    /// The atlas and the group that binds it.
    atlas: Atlas,
    /// Instance data for the last frame's rectangles. Grows, never shrinks.
    quad_vertices: wgpu::Buffer,
    /// Instance data for the last frame's glyphs. Grows, never shrinks.
    glyph_vertices: wgpu::Buffer,
}

impl Gpu {
    /// Create a device that draws into `window`.
    ///
    /// `width` and `height` are the surface's size in physical pixels and `scale` is the
    /// window's DPI scale, which is only recorded here — a frame is expressed in
    /// physical pixels, so the scale is a fact the caller needs back and not one this
    /// module does arithmetic with.
    ///
    /// # Errors
    ///
    /// [`GpuError::NoAdapter`] when no adapter can run the pipelines, and
    /// [`GpuError::Device`] or [`GpuError::Surface`] when the adapter is found but the
    /// device or the surface cannot be created.
    pub fn new<W>(window: Arc<W>, width: u32, height: u32, scale: f32) -> GpuResult<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| GpuError::Surface(error.to_string()))?;
        let adapter = choose(&instance, Some(&surface))?;
        let (device, queue) = Self::open(&adapter)?;

        let format = surface_format(&surface.get_capabilities(&adapter).formats)
            .ok_or_else(|| GpuError::Surface("the surface offers no usable format".to_owned()))?;
        // A minimised window reports a size of zero, and a surface cannot be configured
        // at zero. One pixel keeps the device drawable until the window comes back.
        let size = (width.max(1), height.max(1));
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            // The standard SDR colour space, which is what an sRGB format is for.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.0,
            height: size.1,
            // Fifo is the one presentation mode every backend has, and a terminal has
            // nothing to gain from tearing.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);

        Ok(Self::assemble(
            device,
            queue,
            format,
            size,
            scale,
            Target::Window { surface, config },
        ))
    }

    /// Create a device that draws into a texture nobody sees.
    ///
    /// This exists so that a frame can be rendered and read back in a test. A terminal's
    /// renderer is the part of it that is hardest to look at and easiest to get subtly
    /// wrong, and "render this frame and assert the pixel is the theme's red" is the
    /// only kind of test that catches a wrong blend state or a flipped `v`.
    ///
    /// # Errors
    ///
    /// As [`Gpu::new`].
    pub fn offscreen(width: u32, height: u32, scale: f32) -> GpuResult<Self> {
        let instance = wgpu::Instance::default();
        let adapter = choose(&instance, None)?;
        Self::offscreen_on(&adapter, width, height, scale)
    }

    /// The half of [`Gpu::offscreen`] that does not choose an adapter.
    ///
    /// Split out so that the software adapter can be handed in: what criterion 8 asks for
    /// is that a machine with no usable GPU still gets a working terminal, and the only
    /// way to test that on a machine that *has* one is to ask for the fallback by name and
    /// push a frame through it. A device made from WARP is a device made from WARP whether
    /// or not it was the machine's first choice.
    fn offscreen_on(
        adapter: &wgpu::Adapter,
        width: u32,
        height: u32,
        scale: f32,
    ) -> GpuResult<Self> {
        let (device, queue) = Self::open(adapter)?;

        // The same format a window would be given, so that what a readback returns is
        // what the screen would have shown: the shader writes linear values, the format
        // encodes them, and the bytes that come back are the encoded ones.
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let size = (width.max(1), height.max(1));
        let texture = render_texture(&device, format, size);

        Ok(Self::assemble(
            device,
            queue,
            format,
            size,
            scale,
            Target::Offscreen {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                texture,
                format,
            },
        ))
    }

    /// Load an adapter and open a device and queue from it.
    ///
    /// The two requests are `async` and every caller of this module is a window event
    /// loop that is not, so they are driven to completion on the spot.
    fn open(adapter: &wgpu::Adapter) -> GpuResult<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|error| GpuError::Device(error.to_string()))
    }

    /// The pipelines, the bind group, and the buffers both constructors need.
    ///
    /// `format` is the format of whatever the device draws into, which is the one thing
    /// the two constructors disagree about.
    fn assemble(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        scale: f32,
        target: Target,
    ) -> Self {
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("zet atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("zet"),
            bind_group_layouts: &[Some(&atlas_layout)],
            immediate_size: 0,
        });

        let quads = pipeline(
            &device,
            &pipeline_layout,
            format,
            &device.create_shader_module(wgpu::include_wgsl!("../shaders/quads.wgsl")),
            QUAD_VERTICES,
            "zet quads",
        );
        let glyphs = pipeline(
            &device,
            &pipeline_layout,
            format,
            &device.create_shader_module(wgpu::include_wgsl!("../shaders/glyphs.wgsl")),
            GLYPH_VERTICES,
            "zet glyphs",
        );

        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("zet atlas"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            // Bilinear, because a glyph asked for at its own size lands on texel centres
            // and one asked for at a fractional one must not look like it was not.
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let screen = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zet screen"),
            size: size_of::<Screen>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&screen, 0, bytemuck::bytes_of(&Screen::new(size)));

        // One transparent texel, replaced by the first upload. A frame of nothing but
        // rectangles is drawable before any glyph has been rasterised, and a draw with
        // an unset binding is a validation error rather than a missing glyph.
        let atlas = Atlas::new(&device, &atlas_layout, &atlas_sampler, &screen, (1, 1));
        atlas.write(&queue, (1, 1), &[0; 4]);
        let quad_vertices = vertex_buffer(&device, "zet quad vertices");
        let glyph_vertices = vertex_buffer(&device, "zet glyph vertices");

        Self {
            device,
            queue,
            size,
            scale,
            target,
            quads,
            glyphs,
            screen,
            atlas_layout,
            atlas_sampler,
            atlas,
            quad_vertices,
            glyph_vertices,
        }
    }

    /// Change the size the device draws at.
    ///
    /// A no-op when the size is unchanged, which matters because a window that is being
    /// dragged resizes every frame and reconfiguring the surface each time is a visible
    /// stutter.
    #[allow(clippy::float_cmp)]
    pub fn resize(&mut self, width: u32, height: u32, scale: f32) {
        let size = (width.max(1), height.max(1));
        // The scale is compared exactly, and on purpose: it is handed over by the window
        // and is the same number until the window really moves to another display, so a
        // tolerance here would be a reconfiguration on every frame of a drag.
        if self.size == size && self.scale == scale {
            return;
        }
        self.size = size;
        self.scale = scale;
        self.queue
            .write_buffer(&self.screen, 0, bytemuck::bytes_of(&Screen::new(size)));

        match &mut self.target {
            Target::Window { surface, config } => {
                config.width = size.0;
                config.height = size.1;
                surface.configure(&self.device, config);
            }
            Target::Offscreen {
                texture,
                view,
                format,
            } => {
                // The readback is `width * height * 4` bytes of this texture, so a device
                // that reported a size its texture did not have would hand back the wrong
                // picture rather than an error.
                *texture = render_texture(&self.device, *format, size);
                *view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            }
        }
    }

    /// Replace the atlas texture's contents.
    ///
    /// `pixels` is `width * height * 4` bytes of RGBA, row-major from the top. The size
    /// is allowed to change, which is how the CPU-side atlas grows; the texture is
    /// recreated when it does.
    pub fn upload_atlas(&mut self, width: u32, height: u32, pixels: &[u8]) {
        if self.atlas.size != (width, height) {
            let atlas = Atlas::new(
                &self.device,
                &self.atlas_layout,
                &self.atlas_sampler,
                &self.screen,
                (width, height),
            );
            self.atlas = atlas;
        }
        self.atlas.write(&self.queue, (width, height), pixels);
    }

    /// Draw one frame and present it.
    ///
    /// A surface that has gone out of date is reconfigured and the frame is skipped
    /// rather than reported: a window being resized produces one or two of those and
    /// they are not errors, they are the window changing size. The caller draws again
    /// next time round the loop.
    ///
    /// # Errors
    ///
    /// [`GpuError::Surface`] when the surface cannot be acquired or its configuration
    /// cannot be produced.
    pub fn draw(&mut self, frame: &Frame) -> GpuResult<()> {
        grow(
            &self.device,
            &mut self.quad_vertices,
            "zet quad vertices",
            bytemuck::cast_slice::<Quad, u8>(&frame.quads).len() as u64,
        );
        grow(
            &self.device,
            &mut self.glyph_vertices,
            "zet glyph vertices",
            bytemuck::cast_slice::<GlyphQuad, u8>(&frame.glyphs).len() as u64,
        );
        self.queue
            .write_buffer(&self.quad_vertices, 0, bytemuck::cast_slice(&frame.quads));
        self.queue
            .write_buffer(&self.glyph_vertices, 0, bytemuck::cast_slice(&frame.glyphs));

        // The view is taken first and the acquisition kept, because the swapchain image
        // has to be given back to the surface after the work on it is submitted.
        let mut acquired = None;
        let view = match &mut self.target {
            Target::Offscreen { view, .. } => view.clone(),
            Target::Window { surface, config } => {
                match surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(texture)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                        let view = texture
                            .texture
                            .create_view(&wgpu::TextureViewDescriptor::default());
                        acquired = Some(texture);
                        view
                    }
                    wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                        surface.configure(&self.device, config);
                        return Ok(());
                    }
                    // Nothing to draw into because nothing is on screen. The next frame
                    // the window is visible again is an ordinary one.
                    wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
                    wgpu::CurrentSurfaceTexture::Timeout => {
                        return Err(GpuError::Surface(
                            "acquiring the frame timed out".to_owned(),
                        ));
                    }
                    wgpu::CurrentSurfaceTexture::Validation => {
                        return Err(GpuError::Surface(
                            "the surface raised a validation error".to_owned(),
                        ));
                    }
                }
            }
        };

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("zet frame"),
            });
        self.encode(&mut encoder, &view, frame);
        self.queue.submit([encoder.finish()]);

        if let Some(texture) = acquired {
            self.queue.present(texture);
        }
        Ok(())
    }

    /// Record the one pass that clears the surface and draws every batch in order.
    fn encode(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, frame: &Frame) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("zet frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: frame.clear[0].into(),
                        g: frame.clear[1].into(),
                        b: frame.clear[2].into(),
                        a: frame.clear[3].into(),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        pass.set_bind_group(0, &self.atlas.group, &[]);
        let mut bound: Option<BatchKind> = None;
        for batch in &frame.batches {
            // A run with nothing in it would be a draw of nothing, and binding an empty
            // vertex buffer to do it is a panic rather than a no-op.
            if batch.range.is_empty() {
                continue;
            }
            if bound != Some(batch.kind) {
                pass.set_pipeline(match batch.kind {
                    BatchKind::Quads => &self.quads,
                    BatchKind::Glyphs => &self.glyphs,
                });
                pass.set_vertex_buffer(
                    0,
                    match batch.kind {
                        BatchKind::Quads => self.quad_vertices.slice(..),
                        BatchKind::Glyphs => self.glyph_vertices.slice(..),
                    },
                );
                bound = Some(batch.kind);
            }
            pass.draw(CORNERS, batch.range.clone());
        }
    }

    /// Read the last drawn frame back as RGBA bytes, row-major from the top.
    ///
    /// # Errors
    ///
    /// [`GpuError::NotOffscreen`] for a device created by [`Gpu::new`], which has no
    /// pixels to give back.
    pub fn read_pixels(&self) -> GpuResult<Vec<u8>> {
        let Target::Offscreen { texture, .. } = &self.target else {
            return Err(GpuError::NotOffscreen);
        };

        // A copy to a buffer has to put every row on a 256-byte boundary, so a row of
        // 64 RGBA pixels arrives in 256 bytes whether or not it needs them and the slack
        // is dropped below.
        let (width, height) = self.size;
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("zet readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("zet readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                // Texture coordinates start at the top-left corner and y increases
                // downward, the same way a frame's do, so the copy is the picture with
                // no flip in it.
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);

        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        // Waits for the copy to finish and the mapping callback to be invoked. A failure
        // to map is reported by the range below, so the callback's result is dropped.
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| GpuError::Device(error.to_string()))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|error| GpuError::Device(error.to_string()))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for line in mapped.chunks_exact(row as usize) {
            pixels.extend_from_slice(&line[..(width * 4) as usize]);
        }
        drop(mapped);
        staging.unmap();

        Ok(pixels)
    }

    /// The surface size in physical pixels.
    #[must_use]
    pub const fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The DPI scale this device was configured at.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }
}

impl Atlas {
    /// A texture of `size` texels and the group that binds it with the screen uniform.
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        screen: &wgpu::Buffer,
        size: (u32, u32),
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zet atlas"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("zet atlas"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: screen.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });

        Self {
            texture,
            group,
            size,
        }
    }

    /// Copy `pixels` into the texture, which is exactly `size` texels across.
    fn write(&self, queue: &wgpu::Queue, size: (u32, u32), pixels: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                // The row stride the CPU-side atlas is packed at. `write_texture` is the
                // one copy path that does not demand 256-byte rows, which is why the
                // upload is not padded.
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// The format to configure a surface with.
///
/// `Bgra8UnormSrgb` is what every backend offers and what a window presents best as, and
/// `Rgba8UnormSrgb` is the same thing with the channels the other way round. Either way
/// the shader's linear value is encoded on the way out. A surface that offers neither is
/// configured with whatever it does offer, and the picture is then written without that
/// encoding — too bright, because a linear value in an unorm format is read as though it
/// were already encoded, but visible, which is the point of a fallback.
fn surface_format(available: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    [
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ]
    .into_iter()
    .find(|format| available.contains(format))
    .or_else(|| available.first().copied())
}

/// A texture a frame can be drawn into and read back out of.
fn render_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("zet offscreen"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// One of the two pipelines: an instance buffer of rectangles, one colour target, and
/// the premultiplied blend both of them use.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    module: &wgpu::ShaderModule,
    vertices: wgpu::VertexBufferLayout<'static>,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(vertices)],
        },
        // Four corners per rectangle, so the strip is the topology that draws exactly
        // them: a triangle list would draw one triangle out of every three vertices and
        // leave half of every rectangle missing.
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                // `One` over `OneMinusSrcAlpha`, for colour and alpha alike: every value
                // in a frame is premultiplied, so the source is added as it stands and
                // only the destination is scaled back.
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// A vertex buffer with no data in it yet, ready to be grown.
fn vertex_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: VERTEX_BUFFER_START,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Pick an adapter for `instance`, and take the software one if the machine has no other.
///
/// PRODUCT.md's eighth criterion is that a machine with no usable GPU adapter still gets a
/// readable, working terminal. The machine this is written on has one, so the first
/// request succeeds and the second is never made — which is exactly why the second is
/// here. A machine in a VM with no graphics acceleration, or over remote desktop with GPU
/// redirection off, enumerates no adapter at all, and until this existed that machine got
/// a dialog box saying zet could not start instead of a terminal.
///
/// An adapter that enumerates but refuses to open a device is not covered: that is a
/// driver problem rather than a missing adapter, and nothing here has seen it happen.
///
/// `force_fallback_adapter` is not a preference among equals: it asks the backend for the
/// software rasteriser by name, which on Windows is WARP and is always present. Asking for
/// it second rather than preferring it means a real GPU is still used when there is one —
/// WARP is a correct terminal at a fraction of the speed, which is the right trade for a
/// fallback and the wrong one for a default.
///
/// The two requests differ only in that flag, so the failed one is not retried and the
/// error reported is the fallback's: that is the request that answered the question the
/// caller actually asked, which is whether *anything* can draw.
fn choose(
    instance: &wgpu::Instance,
    compatible_surface: Option<&wgpu::Surface<'static>>,
) -> GpuResult<wgpu::Adapter> {
    let options = |force_fallback_adapter| wgpu::RequestAdapterOptions {
        force_fallback_adapter,
        compatible_surface,
        ..Default::default()
    };
    if let Ok(adapter) = pollster::block_on(instance.request_adapter(&options(false))) {
        return Ok(adapter);
    }
    pollster::block_on(instance.request_adapter(&options(true)))
        .map_err(|error| GpuError::NoAdapter(error.to_string()))
}

/// Make sure `buffer` holds at least `needed` bytes, replacing it if it does not.
///
/// Buffers grow and never shrink: a frame that needs less than the last one already
/// fits, and a terminal's frames are all about the same size, so the reallocation
/// happens on the first frames of a session and then never again. The new size is a
/// power of two so that a session whose scrollback grows steadily does not reallocate
/// on every single frame.
fn grow(device: &wgpu::Device, buffer: &mut wgpu::Buffer, label: &str, needed: u64) {
    if buffer.size() >= needed {
        return;
    }
    *buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: needed.next_power_of_two(),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 64x64 offscreen device, or `None` on a machine with no adapter at all.
    ///
    /// A machine without one cannot fail these tests in a way that says anything about
    /// the renderer, so the note is printed and the test passes.
    fn device() -> Option<Gpu> {
        match Gpu::offscreen(64, 64, 1.0) {
            Ok(gpu) => Some(gpu),
            Err(GpuError::NoAdapter(message)) => {
                println!("no graphics adapter on this machine, skipping: {message}");
                None
            }
            Err(error) => panic!("the offscreen device could not be created: {error}"),
        }
    }

    /// A 64x64 offscreen device on the software rasteriser, or `None` if asking for one
    /// did not work.
    ///
    /// On Windows that is WARP, which is always present; on a machine where it is not,
    /// the note is printed and the test passes, the same way `device` handles a machine
    /// with no adapter at all.
    fn software_device() -> Option<Gpu> {
        let instance = wgpu::Instance::default();
        let requested =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: true,
                ..Default::default()
            }));
        let adapter = match requested {
            Ok(adapter) => adapter,
            Err(error) => {
                println!("no software adapter on this machine, skipping: {error}");
                return None;
            }
        };
        match Gpu::offscreen_on(&adapter, 64, 64, 1.0) {
            Ok(gpu) => Some(gpu),
            Err(error) => panic!("the software adapter could not open a device: {error}"),
        }
    }

    #[test]
    fn the_software_adapter_draws_the_same_terminal_as_the_chosen_one() {
        // PRODUCT.md's eighth criterion, and the only way to check it on a machine that
        // has a GPU: ask for the fallback by name and push a frame through it. What this
        // catches is a pipeline that needs something WARP does not have, or a device
        // descriptor that asks for limits a software adapter will not grant — both of
        // which would show up on exactly the machines that have no other option, and
        // nowhere else. Without this, the second `request_adapter` would be code that has
        // never run.
        let Some(mut gpu) = software_device() else {
            return;
        };
        let mut frame = Frame::new();
        frame.clear = [0.0, 0.0, 0.0, 1.0];
        frame.begin_quads();
        frame.push_quad(Quad::new(8.0, 8.0, 16.0, 16.0, [0.25, 0.25, 0.25, 1.0]));
        frame.end_quads();

        let pixels = render(&mut gpu, &frame);
        assert_pixel(&pixels, (16, 16), [srgb(0.25), srgb(0.25), srgb(0.25), 255]);
        assert_pixel(&pixels, (48, 48), [0, 0, 0, 255]);
    }

    /// Draw `frame` and hand back its pixels.
    fn render(gpu: &mut Gpu, frame: &Frame) -> Vec<u8> {
        gpu.draw(frame).expect("the frame should draw");
        let pixels = gpu.read_pixels().expect("the frame should be readable");
        assert_eq!(pixels.len(), 64 * 64 * 4);
        pixels
    }

    /// The RGBA of one pixel of a readback.
    fn pixel(pixels: &[u8], at: (u32, u32)) -> [u8; 4] {
        let (x, y) = at;
        let start = ((y * 64 + x) * 4) as usize;
        pixels[start..start + 4]
            .try_into()
            .expect("a readback is a multiple of four bytes")
    }

    /// Assert a pixel, allowing the bit of slack a GPU's rounding is entitled to.
    fn assert_pixel(pixels: &[u8], at: (u32, u32), expected: [u8; 4]) {
        let actual = pixel(pixels, at);
        for (channel, (got, want)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(
                got.abs_diff(*want) <= 2,
                "pixel {at:?} channel {channel}: expected {want}, got {got}"
            );
        }
    }

    /// The byte an sRGB framebuffer stores for a linear value.
    ///
    /// This is the whole of the colour space the renderer promises, in eight lines.
    fn srgb(linear: f32) -> u8 {
        let encoded = if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as u8
    }

    #[test]
    fn a_frame_with_no_batches_is_its_clear_colour() {
        let Some(mut gpu) = device() else {
            return;
        };

        let mut frame = Frame::new();
        // A quarter of the way up the scale in linear light. An unorm surface would come
        // back with 64 here, which is the only test that tells the two apart: everything
        // else in a frame is written by a shader that encodes nothing itself.
        frame.clear = [0.25, 0.25, 0.25, 1.0];
        let pixels = render(&mut gpu, &frame);

        let grey = [srgb(0.25), srgb(0.25), srgb(0.25), 255];
        assert_pixel(&pixels, (32, 32), grey);
        assert_pixel(&pixels, (0, 0), grey);
        assert_pixel(&pixels, (63, 63), grey);
    }

    #[test]
    fn a_quad_covers_the_whole_surface() {
        let Some(mut gpu) = device() else {
            return;
        };

        let mut frame = Frame::new();
        frame.clear = [0.0, 0.0, 0.0, 1.0];
        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 64.0, 64.0, [1.0, 0.0, 0.0, 1.0]));
        frame.end_quads();
        let pixels = render(&mut gpu, &frame);

        let red = [255, 0, 0, 255];
        assert_pixel(&pixels, (32, 32), red);
        assert_pixel(&pixels, (0, 0), red);
        assert_pixel(&pixels, (63, 63), red);
    }

    #[test]
    fn a_quad_covers_only_the_pixels_its_rectangle_names() {
        let Some(mut gpu) = device() else {
            return;
        };

        let mut frame = Frame::new();
        frame.clear = [0.0, 0.0, 0.0, 1.0];
        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 32.0, 64.0, [0.0, 1.0, 0.0, 1.0]));
        frame.end_quads();
        let pixels = render(&mut gpu, &frame);

        let green = [0, 255, 0, 255];
        let black = [0, 0, 0, 255];
        assert_pixel(&pixels, (16, 32), green);
        assert_pixel(&pixels, (48, 32), black);

        // A left-and-right split survives a flipped y, because its rectangle is
        // symmetric about the axis that would be wrong. The same rectangle along the
        // other axis is what pins the direction down, and the y axis is the one most
        // likely to have been turned over by a sign.
        frame.reset();
        frame.begin_quads();
        frame.push_quad(Quad::new(0.0, 0.0, 64.0, 32.0, [0.0, 1.0, 0.0, 1.0]));
        frame.end_quads();
        let pixels = render(&mut gpu, &frame);

        assert_pixel(&pixels, (32, 8), green);
        assert_pixel(&pixels, (32, 56), black);
    }

    #[test]
    fn a_glyph_is_tinted_or_drawn_as_it_is() {
        let Some(mut gpu) = device() else {
            return;
        };

        // Two columns: white with full coverage, which is how an alpha glyph is stored,
        // and an opaque blue, which is how a colour glyph is stored. Both rows are the
        // same so that the middle of the texture, where bilinear sampling mixes them,
        // is not somewhere the assertions look.
        gpu.upload_atlas(
            2,
            2,
            &[
                255, 255, 255, 255, 0, 0, 255, 255, //
                255, 255, 255, 255, 0, 0, 255, 255,
            ],
        );

        // Each glyph covers half the surface, and each is sampled at the middle of its
        // own half of a two-texel atlas: the exact centre of a texel, so nothing is
        // filtered. Those points are where the tint and the flags are the only things
        // deciding the colour. The coordinates are texels, so the left column is
        // `x` from 0 to 1 and the right column from 1 to 2.
        let mut frame = Frame::new();
        frame.begin_glyphs();
        frame.push_glyph(GlyphQuad::alpha(
            [0.0, 0.0, 32.0, 64.0],
            [0.0, 0.0, 1.0, 2.0],
            [1.0, 0.0, 0.0, 1.0],
        ));
        frame.push_glyph(GlyphQuad::color(
            [32.0, 0.0, 32.0, 64.0],
            [1.0, 0.0, 2.0, 2.0],
        ));
        frame.end_glyphs();
        let pixels = render(&mut gpu, &frame);

        // White times a premultiplied red tint, and the texel untouched.
        assert_pixel(&pixels, (16, 32), [255, 0, 0, 255]);
        assert_pixel(&pixels, (48, 32), [0, 0, 255, 255]);

        // A faint tint fades a colour glyph rather than washing it out: half of an
        // opaque blue over a black clear is half the light, which is what a premultiplied
        // blend of a premultiplied source does. A tint that only scaled the alpha would
        // draw this glyph at full strength.
        let mut faded = GlyphQuad::color([32.0, 0.0, 32.0, 64.0], [1.0, 0.0, 2.0, 2.0]);
        faded.color = [0.5, 0.5, 0.5, 0.5];
        frame.reset();
        frame.begin_glyphs();
        frame.push_glyph(faded);
        frame.end_glyphs();
        let pixels = render(&mut gpu, &frame);

        assert_pixel(&pixels, (48, 32), [0, 0, srgb(0.5), 255]);

        // A texel that is half covered draws half the tint, and this is the assertion
        // that says so. It is here rather than only in the end-to-end test because it is
        // the one case the two above cannot reach: both of their alpha texels are fully
        // covered, and a shader that multiplied the tint by the whole texel instead of by
        // its alpha draws those exactly right and every partly covered glyph as a solid
        // rectangle. Half a texel is the smallest thing that tells the two apart.
        gpu.upload_atlas(1, 1, &[255, 255, 255, 128]);

        let mut single = GlyphQuad::alpha(
            [0.0, 0.0, 64.0, 64.0],
            [0.0, 0.0, 1.0, 1.0],
            [1.0, 0.0, 0.0, 1.0],
        );
        single.color = [1.0, 0.0, 0.0, 1.0];
        frame.reset();
        frame.begin_glyphs();
        frame.push_glyph(single);
        frame.end_glyphs();
        let pixels = render(&mut gpu, &frame);

        let half = srgb(128.0 / 255.0);
        assert_pixel(&pixels, (32, 32), [half, 0, 0, 255]);
    }
}
