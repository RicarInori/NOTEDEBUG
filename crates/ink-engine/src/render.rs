//! The wgpu renderer (§3.1): meshed strokes in, pixels out.
//!
//! ## Why the first cut renders to a texture rather than to a surface
//!
//! §3.1's end goal is a `PlatformView`-hosted surface that draws the live stroke with minimal latency.
//! That needs a window system (Android's `ANativeWindow`, a desktop swapchain) and, for the live path,
//! a ring buffer and a present mode. None of that can be verified on this machine, and a renderer
//! nobody can check is a renderer nobody should trust.
//!
//! Rendering into an **offscreen texture and reading it back** exercises everything that decides
//! whether the strokes look right — the pipeline, the transform, the vertex/index buffers, the blend
//! state — and can be asserted pixel by pixel in a test. Attaching a surface later changes only where
//! the colour attachment comes from.
//!
//! ## What is deliberately not here yet
//!
//! Anti-aliasing, the highlighter's multiply blend, the pencil texture, the live stroke's ring buffer
//! and the present path. The mesh already carries the `u` and `half_width` attributes those need.

use crate::mesh::mesh_stroke;
use crate::stroke::Stroke;
use inkweave_api::stylus::PressureCurve;
use wgpu::util::DeviceExt;

/// Blocking wrapper for the two asynchronous setup calls.
///
/// The renderer is created once per process and has no work to overlap, so an executor here is
/// simpler than threading a runtime through every caller — and it keeps `wgpu`'s async-ness out of
/// this crate's public surface.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("no GPU adapter is available: {0}")]
    NoAdapter(String),
    #[error("could not open a GPU device: {0}")]
    NoDevice(String),
    #[error("the readback buffer could not be mapped")]
    Readback,
    #[error("a target of zero pixels cannot be rendered")]
    EmptyTarget,
    #[error("the window system reported no supported surface format")]
    NoSurfaceCapabilities,
    #[error("the GPU surface could not be created: {0}")]
    NoSurface(String),
}

/// One vertex on the GPU. Colour rides here rather than in a uniform so a whole page is one draw call.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuVertex {
    position: [f32; 2],
    color: [f32; 4],
}

/// Target size in pixels, so the vertex stage can map page coordinates onto clip space.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct Viewport {
    size: [f32; 2],
    _pad: [f32; 2],
}

/// Holds the device, the queue and the one pipeline every ink draw uses.
pub struct InkRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    viewport: wgpu::Buffer,
    /// The colour format the pipeline writes. A window surface accepts only the formats its adapter
    /// advertises, so this cannot be a constant once there is a surface involved.
    format: wgpu::TextureFormat,
    /// `max_texture_dimension_2d` as reported by the adapter.
    max_dimension: u32,
}

impl InkRenderer {
    /// Open a device for offscreen rendering.
    ///
    /// High performance is requested because ink is the hot path; wgpu falls back to whatever it can
    /// find, which on a machine with no discrete GPU means an integrated one rather than a failure.
    pub fn new() -> Result<Self, RenderError> {
        let instance = wgpu::Instance::default();
        let (renderer, _) = Self::with_instance(&instance, None)?;
        Ok(renderer)
    }

    /// Open a device that can present to `surface`, and the configuration to give that surface.
    ///
    /// `instance` must be the one that created `surface`: wgpu resolves a surface through the
    /// instance it came from, and asking a second instance's adapter whether it supports a foreign
    /// surface is not a supported query. Passing it in is what keeps the two in step, and the
    /// pairing of renderer and config is returned together because the format choice is entangled —
    /// the pipeline has to target the format the surface was configured with.
    pub(crate) fn for_surface(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface<'_>,
        size: (u32, u32),
    ) -> Result<(Self, wgpu::SurfaceConfiguration), RenderError> {
        Self::with_instance(instance, Some(surface)).and_then(|(renderer, capabilities)| {
            let capabilities = capabilities.ok_or(RenderError::NoSurfaceCapabilities)?;
            let config = surface_config(
                &capabilities,
                renderer.format,
                size,
                renderer.max_surface_dimension(),
            );
            Ok((renderer, config))
        })
    }

    /// The largest square texture the device can make.
    ///
    /// A window can be bigger than this — an emulator reporting a 2048 limit hands out a 2125 px
    /// tall surface, and configuring that is a validation *panic*, not an error. Clamping is what
    /// keeps a tall canvas working: the frame is rendered smaller and the compositor scales it.
    pub(crate) fn max_surface_dimension(&self) -> u32 {
        self.max_dimension
    }

    fn with_instance(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<(Self, Option<wgpu::SurfaceCapabilities>), RenderError> {
        // `initialize_adapter_from_env_or_default` honours `WGPU_ADAPTER_NAME` (which is how a
        // machine with several GPUs is told which one to use) and otherwise takes the preferred one.
        // With a surface in play the adapter must be one that can actually present to it.
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: surface,
            ..Default::default()
        }))
        .map_err(|error| RenderError::NoAdapter(error.to_string()))?;

        // A surface's format has to come from its own capability list; sRGB is preferred because it
        // is the space a shader's colour values are read in. `NoSurfaceCapabilities` is the honest
        // answer for a surface that advertises none, rather than an index that would panic.
        let capabilities = surface.map(|surface| surface.get_capabilities(&adapter));
        let format = capabilities
            .as_ref()
            .map(|caps| {
                caps.formats
                    .iter()
                    .copied()
                    .find(wgpu::TextureFormat::is_srgb)
                    .or_else(|| caps.formats.first().copied())
                    .ok_or(RenderError::NoSurfaceCapabilities)
            })
            .unwrap_or(Ok(TARGET_FORMAT))?;

        let max_dimension = adapter.limits().max_texture_dimension_2d;

        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ink-device"),
            required_features: wgpu::Features::empty(),
            // `downlevel_defaults` rather than the maximum: the same renderer has to run on an
            // Android GLES device and on a desktop discrete GPU, and asking for more than the
            // baseline would make it fail on the weaker of the two.
            //
            // `using_resolution` is not optional, though: the baseline caps a texture at 2048 px, and
            // a *window* is routinely taller than that, so configuring the surface would be a
            // validation error. Taking the size limits from the adapter is what lets a full-screen
            // canvas exist.
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|error| RenderError::NoDevice(error.to_string()))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("ink.wgsl"));

        let viewport = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ink-viewport"),
            contents: bytemuck::bytes_of(&Viewport {
                size: [1.0, 1.0],
                _pad: [0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ink-bind-group-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ink-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: viewport.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ink-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ink-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GpuVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // No culling: a stroke drawn backwards, or a fan swept the other way, would vanish.
                // The mesh's winding is consistent, but culling buys nothing here.
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Straight alpha over: ink is normally opaque, and where it is not, this is the
                    // behaviour a user expects from a drawing app. The highlighter's multiply blend
                    // arrives with §3.5.
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok((
            Self {
                device,
                queue,
                pipeline,
                bind_group,
                viewport,
                format,
                max_dimension,
            },
            capabilities,
        ))
    }

    /// Draw strokes into an offscreen target and read the pixels back as RGBA8, row-major, top-left
    /// first.
    ///
    /// `clear` is the background in straight RGBA8. Returning pixels rather than a texture keeps this
    /// usable for thumbnails and for the export preview as well as for tests.
    pub fn render_to_rgba(
        &self,
        size: (u32, u32),
        strokes: &[Stroke],
        curve: &PressureCurve,
        clear: [u8; 4],
    ) -> Result<Vec<u8>, RenderError> {
        if size.0 == 0 || size.1 == 0 {
            return Err(RenderError::EmptyTarget);
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ink-target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // A readback row must be a multiple of 256 bytes, so the copy is padded and the padding is
        // dropped on the way out.
        let padded_row_bytes = padded_row(size.0 * BYTES_PER_PIXEL);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ink-readback"),
            size: (padded_row_bytes as u64) * (size.1 as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ink-encoder"),
            });

        self.draw(&mut encoder, &view, size, strokes, curve, clear);

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(size.1),
                },
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );

        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // Pump the device until the callback has run. `wait_indefinitely` blocks rather than returning
        // early, which is what makes the `recv` below a wait on work that is already done.
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        receiver
            .recv()
            .map_err(|_| RenderError::Readback)?
            .map_err(|_| RenderError::Readback)?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|_| RenderError::Readback)?;
        let row_bytes = (size.0 * BYTES_PER_PIXEL) as usize;
        let mut pixels = Vec::with_capacity(row_bytes * size.1 as usize);
        for row in 0..size.1 as usize {
            let start = row * padded_row_bytes as usize;
            pixels.extend_from_slice(&mapped[start..start + row_bytes]);
        }
        Ok(pixels)
    }

    /// Hand a rendered frame to the compositor.
    pub(crate) fn present(&self, frame: wgpu::SurfaceTexture) {
        self.queue.present(frame);
    }

    /// Apply a configuration to a surface. Kept here so the device and the chosen format stay private:
    /// a caller that built its own `SurfaceConfiguration` could pick a format the pipeline does not
    /// target, and the failure would be a validation error at draw time rather than at configure time.
    pub(crate) fn configure_surface(
        &self,
        surface: &wgpu::Surface<'_>,
        config: &wgpu::SurfaceConfiguration,
    ) {
        surface.configure(&self.device, config);
    }

    /// Draw one page's strokes into a texture the caller owns — a swap-chain frame, for instance.
    ///
    /// The frame is submitted but not presented: acquiring and presenting belong to whoever owns the
    /// surface, because that is where the platform's lifetime rules live.
    pub(crate) fn render_to_view(
        &self,
        view: &wgpu::TextureView,
        size: (u32, u32),
        strokes: &[Stroke],
        curve: &PressureCurve,
        clear: [u8; 4],
    ) {
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("ink-encoder"),
            });
        self.draw(&mut encoder, view, size, strokes, curve, clear);
        self.queue.submit([encoder.finish()]);
    }

    /// The one place a pass is recorded, shared by the offscreen and the presenting paths so they
    /// cannot drift apart.
    fn draw(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        strokes: &[Stroke],
        curve: &PressureCurve,
        clear: [u8; 4],
    ) {
        self.queue.write_buffer(
            &self.viewport,
            0,
            bytemuck::bytes_of(&Viewport {
                size: [size.0 as f32, size.1 as f32],
                _pad: [0.0, 0.0],
            }),
        );
        let (vertices, indices, index_count) = self.build_buffers(strokes, curve);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ink-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: channel(clear[0]),
                        g: channel(clear[1]),
                        b: channel(clear[2]),
                        a: clear[3] as f64 / 255.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        if index_count > 0 {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            // One draw call for the whole page: every stroke's triangles in one index buffer, with
            // the colour on the vertex.
            pass.draw_indexed(0..index_count, 0, 0..1);
        }
    }

    /// Mesh every stroke into one vertex/index pair, so a page is a single draw call.
    ///
    /// The index count comes back alongside the buffers because a `wgpu::Buffer` does not know how
    /// many indices were written into it.
    fn build_buffers(
        &self,
        strokes: &[Stroke],
        curve: &PressureCurve,
    ) -> (wgpu::Buffer, wgpu::Buffer, u32) {
        let mut vertices: Vec<GpuVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        for stroke in strokes {
            let mesh = mesh_stroke(stroke, curve);
            if mesh.is_empty() {
                continue;
            }
            let base = vertices.len() as u32;
            let color = unpack_argb(stroke.color);
            vertices.extend(mesh.vertices.iter().map(|vertex| GpuVertex {
                position: [vertex.x, vertex.y],
                color,
            }));
            indices.extend(mesh.indices.iter().map(|index| index + base));
        }

        let index_count = indices.len() as u32;

        let vertex_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ink-vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ink-indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        (vertex_buffer, index_buffer, index_count)
    }
}

/// The display a surface is created against.
///
/// A window system normally owns this — winit hands the X11 connection or the Wayland display over —
/// and wgpu refuses to create any surface until the instance carries one. Android has no such owner
/// object: rwh models its display as an empty marker, because there is one display and the platform
/// supplies it implicitly. So the engine synthesises the marker rather than asking every caller for
/// a handle that does not exist.
#[cfg(target_os = "android")]
#[derive(Debug)]
struct AndroidDisplay;

#[cfg(target_os = "android")]
impl wgpu::rwh::HasDisplayHandle for AndroidDisplay {
    fn display_handle(&self) -> Result<wgpu::rwh::DisplayHandle<'_>, wgpu::rwh::HandleError> {
        // SAFETY: an Android display handle is an empty marker, so there is nothing to outlive.
        Ok(unsafe {
            wgpu::rwh::DisplayHandle::borrow_raw(wgpu::rwh::RawDisplayHandle::Android(
                wgpu::rwh::AndroidDisplayHandle::new(),
            ))
        })
    }
}

/// The raw display handle for the instance and the surface, which wgpu requires to be identical when
/// both are present. `None` everywhere the display belongs to a window system object.
fn raw_display_handle() -> Option<wgpu::rwh::RawDisplayHandle> {
    #[cfg(target_os = "android")]
    {
        Some(wgpu::rwh::RawDisplayHandle::Android(
            wgpu::rwh::AndroidDisplayHandle::new(),
        ))
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

fn instance_descriptor() -> wgpu::InstanceDescriptor {
    // `mut` because the display is assigned on Android only; elsewhere the field stays `None` and
    // there is nothing to write.
    #[allow(unused_mut)]
    let mut descriptor = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::all(),
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: Default::default(),
        backend_options: Default::default(),
        // Left `None` here and filled in below; the field's type is a boxed `WgpuHasDisplayHandle`,
        // and the coercion from a concrete marker is what keeps that trait unnameable in this crate.
        display: None,
    };
    #[cfg(target_os = "android")]
    {
        descriptor.display = Some(Box::new(AndroidDisplay));
    }
    descriptor
}

/// A renderer bound to a window surface: the on-screen path.
///
/// This is the only place the crate touches a window system, and it does so through
/// [`wgpu::rwh::HasWindowHandle`] rather than anything platform-named — so Android's `NativeWindow`,
/// a desktop window and a headless test harness all arrive here the same way and the engine needs no
/// `#[cfg]` for any of them.
pub struct SurfaceRenderer {
    /// Held so the surface's instance outlives it: the surface was created from this instance and
    /// wgpu resolves through it.
    #[allow(dead_code)]
    instance: wgpu::Instance,
    renderer: InkRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The size the caller draws in — the platform view's pixels, and the space its touch
    /// coordinates arrive in. `config` may be smaller when the device's texture limit clamps it, and
    /// the viewport has to stay in *this* space or ink would land away from the pen.
    view_size: (u32, u32),
}

impl SurfaceRenderer {
    /// Create a renderer that presents into `window`.
    ///
    /// # Safety
    ///
    /// `window` must outlive the returned renderer. wgpu cannot express that in the type because the
    /// handle arrives as a raw pointer, which is also why this is the one `unsafe` constructor in
    /// the engine.
    pub unsafe fn from_window(
        window: &impl wgpu::rwh::HasWindowHandle,
        size: (u32, u32),
    ) -> Result<Self, RenderError> {
        let instance = wgpu::Instance::new(instance_descriptor());
        let raw_window_handle = window
            .window_handle()
            .map_err(|error| RenderError::NoSurface(error.to_string()))?
            .as_raw();

        // Built by hand rather than through `SurfaceTargetUnsafe::from_window`, which leaves the
        // display handle `None` — and an Android surface cannot be created without one, because the
        // instance has to know which display the surface belongs to before it will hand one out.
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: raw_display_handle(),
            raw_window_handle,
        };
        // SAFETY: the caller guarantees the window outlives this renderer, and the display handle is
        // a marker with nothing behind it.
        let surface = unsafe { instance.create_surface_unsafe(target) }
            .map_err(|error| RenderError::NoSurface(error.to_string()))?;

        let (renderer, config) = InkRenderer::for_surface(&instance, &surface, size)?;
        renderer.configure_surface(&surface, &config);
        Ok(Self {
            instance,
            renderer,
            surface,
            config,
            view_size: size,
        })
    }

    /// The size the caller draws in, in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.view_size
    }

    /// The size of the buffer being rendered into, which a clamped device limit can make smaller
    /// than [`Self::size`].
    pub fn buffer_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Follow a change of the view's size.
    ///
    /// The buffer is re-clamped, and the surface is only reconfigured when that clamped size actually
    /// changed — a resize that stays within the device limit needs no reconfigure.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.view_size = size;
        let max = self.renderer.max_surface_dimension();
        let width = size.0.clamp(1, max);
        let height = size.1.clamp(1, max);
        if (width, height) != (self.config.width, self.config.height) {
            self.config.width = width;
            self.config.height = height;
            self.renderer.configure_surface(&self.surface, &self.config);
        }
    }

    /// Draw one frame and present it.
    ///
    /// Returns `false` when the driver refused the frame — an outdated or lost surface, which is
    /// routine around a resize or a backgrounding. Callers should skip the frame rather than treat it
    /// as fatal, and the next call normally succeeds.
    pub fn render(&mut self, strokes: &[Stroke], curve: &PressureCurve, clear: [u8; 4]) -> bool {
        let frame = match self.surface.get_current_texture() {
            // `Suboptimal` still hands over a usable frame; it only means the surface's
            // configuration is no longer ideal for the window, which the next acquire will fix.
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Lost
            | wgpu::CurrentSurfaceTexture::Timeout => {
                // Reconfiguring is the documented recovery for the first two; a timeout just means
                // the compositor had nothing ready.
                self.renderer.configure_surface(&self.surface, &self.config);
                return false;
            }
            // An occluded canvas has nothing to draw into, and a validation failure is not something
            // a render call can recover from — either way the caller's next frame is unaffected.
            _ => return false,
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render_to_view(
            &view,
            (self.config.width, self.config.height),
            strokes,
            curve,
            clear,
        );
        // Presenting lives on the queue in wgpu 30, so the renderer does it on the caller's behalf:
        // one less handle for a caller to have to hold.
        self.renderer.present(frame);
        true
    }
}

/// The offscreen target's format. Unorm rather than `Srgb` so a readback byte is the byte the shader
/// wrote — an sRGB target would apply a transfer curve to every assertion a test makes.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How many frames the driver may queue ahead. Two is the value a drawing surface wants: more adds
/// latency, fewer stalls the producer.
const MAX_FRAMES_IN_FLIGHT: u32 = 2;

/// The configuration to give a surface given what it will accept.
///
/// `AutoVsync` rather than `AutoNoVsync`: a notebook that tears is worse than one that is one frame
/// behind, and §3.1's budget is written against a vsync-locked present.
fn surface_config(
    capabilities: &wgpu::SurfaceCapabilities,
    format: wgpu::TextureFormat,
    size: (u32, u32),
    max_dimension: u32,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        // Auto is supported for every format a surface advertises, and it is what keeps the ink
        // colours from being reinterpreted as HDR on a display that offers more than one space.
        color_space: wgpu::SurfaceColorSpace::Auto,
        // A surface larger than the device's limit is a validation error, and wgpu reports it by
        // panicking — so the clamp belongs here rather than in a caller that cannot know.
        width: size.0.clamp(1, max_dimension),
        height: size.1.clamp(1, max_dimension),
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: MAX_FRAMES_IN_FLIGHT,
        alpha_mode: capabilities.alpha_modes[0],
        view_formats: vec![],
    }
}

const BYTES_PER_PIXEL: u32 = 4;

/// `copy_texture_to_buffer` requires each row to start on a 256-byte boundary.
fn padded_row(row_bytes: u32) -> u32 {
    row_bytes.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}

/// A clear colour component as wgpu wants it.
///
/// The byte is taken at face value rather than converted from sRGB to linear: for the paper-white
/// background these renders use, 255 is 1.0 either way, and a faithful conversion belongs with the
/// colour management that §3.5's brush work will need.
fn channel(byte: u8) -> f64 {
    byte as f64 / 255.0
}

/// ARGB (the stroke model's packing) to straight-alpha RGBA in 0..1.
fn unpack_argb(color: u32) -> [f32; 4] {
    let channel = |shift: u32| ((color >> shift) & 0xFF) as f32 / 255.0;
    [channel(16), channel(8), channel(0), channel(24)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::{StrokeId, StrokePoint};

    /// Rendering needs a real adapter, which the build machine has but a CI container may not. The
    /// tests skip rather than fail in that case — a missing GPU is not a regression.
    fn renderer() -> Option<InkRenderer> {
        match InkRenderer::new() {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                eprintln!("skipping: {error}");
                None
            }
        }
    }

    fn curve() -> PressureCurve {
        PressureCurve::linear()
    }

    const SIZE: (u32, u32) = (128, 128);
    const BACKGROUND: [u8; 4] = [255, 255, 255, 255];
    const INK: u32 = 0xFF000000;

    fn stroke(points: &[(f32, f32)], width: f32) -> Stroke {
        Stroke::new(
            StrokeId(1),
            points
                .iter()
                .enumerate()
                .map(|(i, (x, y))| StrokePoint::new(*x, *y, 1.0, i as u64 * 2000))
                .collect(),
            INK,
            width,
        )
    }

    fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
        let start = ((y * SIZE.0 + x) * 4) as usize;
        [
            pixels[start],
            pixels[start + 1],
            pixels[start + 2],
            pixels[start + 3],
        ]
    }

    /// How dark a pixel is, 0.0 for the background and 1.0 for solid ink.
    fn darkness(p: [u8; 4]) -> f32 {
        1.0 - p[0] as f32 / 255.0
    }

    #[test]
    fn a_stroke_paints_where_it_was_drawn_and_nowhere_else() {
        let Some(renderer) = renderer() else {
            return;
        };
        // A horizontal run at y = 64, 10 px wide, from x = 20 to x = 108.
        let mut points = Vec::new();
        let mut x = 20.0;
        while x <= 108.0 {
            points.push((x, 64.0));
            x += 4.0;
        }
        let pixels = renderer
            .render_to_rgba(SIZE, &[stroke(&points, 10.0)], &curve(), BACKGROUND)
            .expect("rendered");

        assert!(darkness(pixel(&pixels, 64, 64)) > 0.99, "the centre is ink");
        assert!(
            darkness(pixel(&pixels, 64, 60)) > 0.99,
            "4 px above the centre is still inside a 10 px stroke"
        );
        assert!(darkness(pixel(&pixels, 64, 68)) > 0.99, "and 4 px below");
        assert!(
            darkness(pixel(&pixels, 64, 56)) < 0.01,
            "9 px above is outside"
        );
        assert!(darkness(pixel(&pixels, 64, 72)) < 0.01, "and 9 px below");
        assert!(
            darkness(pixel(&pixels, 2, 64)) < 0.01,
            "the far side of the target stays clear"
        );
        assert!(
            darkness(pixel(&pixels, 126, 64)) < 0.01,
            "and so does the near edge"
        );
    }

    #[test]
    fn the_ends_of_a_stroke_are_round() {
        let Some(renderer) = renderer() else {
            return;
        };
        let points: Vec<(f32, f32)> = (0..=22).map(|i| (20.0 + i as f32 * 4.0, 64.0)).collect();
        let pixels = renderer
            .render_to_rgba(SIZE, &[stroke(&points, 12.0)], &curve(), BACKGROUND)
            .expect("rendered");

        // Half a width behind the first sample is still inside the cap...
        assert!(darkness(pixel(&pixels, 15, 64)) > 0.99);
        // ...and the corner a *square* cap would cover is not, while the circle at the same diagonal
        // is. The sample point of pixel (14, 58) is 7.8 px from the cap's centre, outside a radius of 6.
        assert!(
            darkness(pixel(&pixels, 14, 58)) < 0.01,
            "the square cap's corner must not be painted"
        );
        assert!(darkness(pixel(&pixels, 17, 61)) > 0.99, "on the circle");
    }

    #[test]
    fn the_painted_area_matches_the_mesh_area() {
        let Some(renderer) = renderer() else {
            return;
        };
        let points: Vec<(f32, f32)> = (0..=30).map(|i| (14.0 + i as f32 * 3.0, 64.0)).collect();
        let rendered = stroke(&points, 10.0);
        let expected = mesh_stroke(&rendered, &curve()).area();

        let pixels = renderer
            .render_to_rgba(SIZE, std::slice::from_ref(&rendered), &curve(), BACKGROUND)
            .expect("rendered");
        let painted = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| darkness(**p) > 0.5)
            .count() as f32;

        // The renderer rasterises the same triangles the mesh describes, so the counts agree to
        // within the half-pixel boundary a scanline rule loses.
        let tolerance = expected * 0.05;
        assert!(
            (painted - expected).abs() < tolerance,
            "painted {painted} px but the mesh covers {expected} px²"
        );
    }

    #[test]
    fn strokes_keep_their_own_colour() {
        let Some(renderer) = renderer() else {
            return;
        };
        // `Stroke::color` is ARGB, so red is `0xFFFFRRGGBB` with green and blue clear.
        let red = Stroke::new(
            StrokeId(1),
            (0..=10)
                .map(|i| StrokePoint::new(20.0 + i as f32 * 4.0, 32.0, 1.0, i as u64 * 2000))
                .collect(),
            0xFFFF0000,
            8.0,
        );
        let green = Stroke::new(
            StrokeId(2),
            (0..=10)
                .map(|i| StrokePoint::new(20.0 + i as f32 * 4.0, 96.0, 1.0, i as u64 * 2000))
                .collect(),
            0xFF00FF00,
            8.0,
        );

        let pixels = renderer
            .render_to_rgba(SIZE, &[red, green], &curve(), BACKGROUND)
            .expect("rendered");

        let top = pixel(&pixels, 60, 32);
        assert_eq!((top[0], top[1], top[2]), (255, 0, 0), "red at the top");
        let bottom = pixel(&pixels, 60, 96);
        assert_eq!(
            (bottom[0], bottom[1], bottom[2]),
            (0, 255, 0),
            "green at the bottom"
        );
        // One draw call covered both, so the batch must not have leaked one colour into the other.
        assert_eq!(pixel(&pixels, 60, 64), BACKGROUND);
    }

    #[test]
    fn an_empty_page_is_just_the_background() {
        let Some(renderer) = renderer() else {
            return;
        };
        let pixels = renderer
            .render_to_rgba(SIZE, &[], &curve(), BACKGROUND)
            .expect("rendered");
        assert_eq!(pixels.len(), (SIZE.0 * SIZE.1 * 4) as usize);
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| *p == BACKGROUND));
    }

    #[test]
    fn a_target_with_no_pixels_is_refused_rather_than_rendered() {
        let Some(renderer) = renderer() else {
            return;
        };
        assert!(matches!(
            renderer.render_to_rgba((0, 10), &[], &curve(), BACKGROUND),
            Err(RenderError::EmptyTarget)
        ));
    }
}
