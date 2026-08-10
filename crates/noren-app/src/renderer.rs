//! Minimal, bounded `wgpu` terminal view for the PoC.
//!
//! Per-cell foreground colour recorded by the terminal state reaches drawing
//! here: [`resolve_foreground`] runs a cell's `Color` selection through the
//! default palette (or passes `Rgb` through directly) and the result rides on
//! every vertex the cell's glyph emits, which the fragment shader returns.
//! A cell with no SGR colour resolves to [`DEFAULT_FOREGROUND`] — the exact
//! shade the shader previously returned as a constant — so unstyled output is
//! unchanged.
//!
//! Background colour is not drawn yet: that needs a filled rect behind each
//! glyph rather than colour on the glyph's own vertices. See issue #107.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use wgpu::CurrentSurfaceTexture;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use noren_app::{CellMetrics, MAX_RENDER_COLS, MAX_RENDER_ROWS};
use noren_terminal::{CellAttributes, Color, TerminalSnapshot};
const GLYPH_SCALE: u32 = 2;
const GLYPH_TOP: u32 = 3;
const MAX_VERTICES: usize = (MAX_RENDER_ROWS as usize) * (MAX_RENDER_COLS as usize) * 35 * 6;

/// Width of the left sidebar in cell columns. The terminal occupies the
/// remaining columns to the right, drawn at a pixel offset of
/// `SIDEBAR_COLS * CELL_WIDTH`.
///
/// Exposed as `pub(crate)` so `main.rs` can subtract it from the PTY/terminal
/// grid, and so the frame oracle (`renderer_capture.rs`) can render sidebar
/// content through the same pipeline.
pub(crate) const SIDEBAR_COLS: usize = 16;

/// WGSL source for the PoC glyph pipeline.
///
/// Exposed as `pub(crate)` solely so the offscreen frame-oracle
/// (`renderer_capture.rs`) builds its pipeline from the **same** shader the
/// shipped binary uses, rather than a parallel copy. No behaviour change.
pub(crate) const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec2<f32>,
    @location(1) color: vec3<f32>,
) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.color = color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
"#;

/// One GPU vertex: an NDC position plus the resolved draw colour of the cell
/// whose glyph emitted it.
///
/// The colour is per-vertex rather than per-draw because a frame mixes cells
/// of many colours in one buffer and one `draw` call; the alternative (a draw
/// call per colour run) would reorder glyphs and complicate the vertex budget
/// for no visual gain at this scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vertex {
    pub(crate) position: [f32; 2],
    pub(crate) color: [f32; 3],
}

/// Bytes per [`Vertex`]: two position floats plus three colour floats.
pub(crate) const VERTEX_BYTES: usize = 20;

/// The vertex buffer layout for [`Vertex`], shared by the shipped renderer's
/// pipeline and the offscreen frame-oracle's, so the two cannot drift apart on
/// stride or attribute offsets.
pub(crate) const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 8,
        shader_location: 1,
    },
];

/// Default terminal foreground — the concrete colour behind a `Color::Default`
/// foreground selection, the sidebar, and the status line.
///
/// These are exactly the floats the fragment shader previously returned as a
/// constant for every pixel. Keeping the value here rather than re-deriving it
/// from an 8-bit triple is deliberate: rounding `0.80` through `u8` and back
/// would shift the default shade slightly, and an unstyled prompt must look
/// identical to how it looked before colour existed (issue #107).
pub(crate) const DEFAULT_FOREGROUND: [f32; 3] = [0.80, 0.92, 0.82];

/// The sixteen ANSI colours of the default theme as `(red, green, blue)` in
/// palette-index order: standard colours `0..=7`, bright colours `8..=15`.
///
/// This table is the single theme seam. The values are the xterm defaults; a
/// future configurable theme replaces this table and every palette-derived
/// draw colour follows without touching the resolution logic.
pub(crate) const DEFAULT_ANSI_PALETTE: [[u8; 3]; 16] = [
    [0, 0, 0],
    [205, 0, 0],
    [0, 205, 0],
    [205, 205, 0],
    [0, 0, 238],
    [205, 0, 205],
    [0, 205, 205],
    [229, 229, 229],
    [127, 127, 127],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [92, 92, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

/// One channel of the xterm 6×6×6 colour cube: level zero is zero, and the
/// remaining five levels are `55 + 40 * level`.
const fn cube_channel(level: u8) -> u8 {
    if level == 0 { 0 } else { level * 40 + 55 }
}

/// Derive the full xterm 256-colour palette once, at compile time.
const fn build_default_palette() -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    let mut index = 0_usize;
    while index < 256 {
        palette[index] = if index < 16 {
            DEFAULT_ANSI_PALETTE[index]
        } else if index < 232 {
            let cube = (index - 16) as u32;
            [
                cube_channel((cube / 36) as u8),
                cube_channel(((cube / 6) % 6) as u8),
                cube_channel((cube % 6) as u8),
            ]
        } else {
            let gray = (8 + (index - 232) * 10) as u8;
            [gray, gray, gray]
        };
        index += 1;
    }
    palette
}

/// The xterm 256-colour default palette: entries `0..=15` from
/// [`DEFAULT_ANSI_PALETTE`], `16..=231` the 6×6×6 colour cube, and
/// `232..=255` the 24-step grayscale ramp.
///
/// The 16 ANSI colours and the 256-colour indexes resolve through this one
/// table, so `SGR 31` and `SGR 38;5;1` cannot disagree about what red is.
pub(crate) const DEFAULT_PALETTE: [[u8; 3]; 256] = build_default_palette();

/// Convert an 8-bit-per-channel colour to the shader's `0.0..=1.0` floats.
const fn channels_to_floats([red, green, blue]: [u8; 3]) -> [f32; 3] {
    [
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
    ]
}

/// Resolve one renderer-independent colour selection to a concrete draw colour.
///
/// [`Color::Default`] yields `default` (the caller supplies the contextual
/// default for the slot), [`Color::Ansi`] and [`Color::Indexed`] both resolve
/// through [`DEFAULT_PALETTE`] — one path, so the 16-colour and 256-colour
/// forms of the same colour agree — and [`Color::Rgb`] passes through as
/// direct 24-bit truecolor.
#[must_use]
pub(crate) fn resolve_color(color: Color, default: [f32; 3]) -> [f32; 3] {
    match color {
        Color::Default => default,
        Color::Ansi(ansi) => channels_to_floats(DEFAULT_PALETTE[ansi.palette_index() as usize]),
        Color::Indexed(index) => channels_to_floats(DEFAULT_PALETTE[index as usize]),
        Color::Rgb(red, green, blue) => channels_to_floats([red, green, blue]),
    }
}

/// The colour a cell's glyph is drawn in.
///
/// Background rectangles are deliberately out of scope for this foreground
/// pass. In particular, do not resolve reverse video to the background here:
/// without a rectangle behind it that would draw the glyph in the clear colour
/// and make reversed text disappear. Reverse/background composition belongs in
/// the later background pass.
#[must_use]
pub(crate) fn resolve_foreground(attributes: &CellAttributes) -> [f32; 3] {
    resolve_color(attributes.foreground(), DEFAULT_FOREGROUND)
}

/// Clear colour used by the glyph pipeline's load op.
///
/// Exposed as `pub(crate)` so the offscreen frame-oracle
/// (`renderer_capture.rs`) clears to the exact same colour the shipped binary
/// uses, rather than a parallel literal that could silently drift. No
/// behaviour change.
pub(crate) const CLEAR_COLOR: wgpu::Color = wgpu::Color {
    r: 0.035,
    g: 0.045,
    b: 0.04,
    a: 1.0,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RendererError {
    SurfaceCreation,
    AdapterUnavailable,
    DeviceUnavailable,
    SurfaceUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenderOutcome {
    Presented,
    Reconfigured,
    Skipped,
    DeviceLost,
}

pub(crate) struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    device_lost: Arc<AtomicBool>,
    metrics: CellMetrics,
}

impl Renderer {
    pub(crate) fn new(window: Arc<Window>, metrics: CellMetrics) -> Result<Self, RendererError> {
        let size = window.inner_size();
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = wgpu::Backends::METAL;
        let instance = wgpu::Instance::new(instance_descriptor);
        let surface = instance
            .create_surface(window)
            .map_err(|_| RendererError::SurfaceCreation)?;
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|_| RendererError::AdapterUnavailable)?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("noren-poc-device"),
            ..Default::default()
        }))
        .map_err(|_| RendererError::DeviceUnavailable)?;

        let device_lost = Arc::new(AtomicBool::new(false));
        let device_lost_callback = Arc::clone(&device_lost);
        device.set_device_lost_callback(move |_, _| {
            device_lost_callback.store(true, Ordering::Release);
        });

        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = surface
            .get_default_config(&adapter, width, height)
            .ok_or(RendererError::SurfaceUnsupported)?;
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("noren-poc-glyph-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("noren-poc-pipeline-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("noren-poc-glyph-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 8;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("noren-poc-glyph-vertices"),
            size: vertex_capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            pipeline,
            vertex_buffer,
            vertex_capacity,
            device_lost,
            metrics,
        })
    }

    pub(crate) fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub(crate) fn render(
        &mut self,
        terminal: Option<&TerminalSnapshot>,
        sidebar: Option<&[String]>,
        status: Option<&str>,
    ) -> RenderOutcome {
        if self.device_lost.load(Ordering::Acquire) {
            return RenderOutcome::DeviceLost;
        }

        let vertices = glyph_vertices(
            terminal,
            sidebar,
            status,
            self.config.width,
            self.config.height,
            self.metrics,
        );
        let bytes = vertex_bytes(&vertices);
        let required = u64::try_from(bytes.len()).unwrap_or(u64::MAX).max(8);
        if required > self.vertex_capacity {
            self.vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("noren-poc-glyph-vertices"),
                size: required,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.vertex_capacity = required;
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.vertex_buffer, 0, &bytes);
        }

        let frame = match self.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(frame) => frame,
            CurrentSurfaceTexture::Suboptimal(frame) => frame,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return RenderOutcome::Reconfigured;
            }
            CurrentSurfaceTexture::Timeout | CurrentSurfaceTexture::Occluded => {
                return RenderOutcome::Skipped;
            }
            CurrentSurfaceTexture::Validation => return RenderOutcome::DeviceLost,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("noren-poc-render-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("noren-poc-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(CLEAR_COLOR),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !vertices.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..u32::try_from(vertices.len()).unwrap_or(u32::MAX), 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        RenderOutcome::Presented
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

/// Exposed as `pub(crate)` for the offscreen frame-oracle (see
/// `renderer_capture.rs`); no behaviour change.
///
/// When `sidebar` is `Some`, the first [`SIDEBAR_COLS`] columns are reserved
/// for the sidebar text and the terminal is drawn starting at column
/// `SIDEBAR_COLS`. When `sidebar` is `None`, the terminal occupies the full
/// width starting at column 0 (preserving the pre-sidebar behaviour the frame
/// oracle's existing tests rely on).
pub(crate) fn glyph_vertices(
    terminal: Option<&TerminalSnapshot>,
    sidebar: Option<&[String]>,
    status: Option<&str>,
    width: u32,
    height: u32,
    metrics: CellMetrics,
) -> Vec<Vertex> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let cell_width = metrics.width();
    let cell_height = metrics.height();
    let visible_rows = usize::try_from(height / cell_height)
        .unwrap_or(usize::MAX)
        .clamp(1, usize::from(MAX_RENDER_ROWS));
    let window_cols = usize::try_from(width / cell_width).unwrap_or(usize::MAX);

    let has_sidebar = sidebar.is_some();
    let col_offset = if has_sidebar { SIDEBAR_COLS } else { 0 };
    // Reserve the sidebar, then clamp the terminal to the renderer's drawable
    // budget (`MAX_RENDER_COLS - SIDEBAR_COLS`), floored at one. This is the
    // same formula `main::terminal_cols` applies — kept independent rather than
    // shared so the sidebar geometry test can still pin that the two sites
    // agree (a single shared function would make their agreement structural and
    // the sidebar subtraction itself un-testable). The sidebar lives *inside*
    // the `MAX_RENDER_COLS` ceiling, so the terminal never owns more columns
    // than the renderer can draw beside it.
    let terminal_cols = if has_sidebar {
        let budget = usize::from(MAX_RENDER_COLS)
            .saturating_sub(SIDEBAR_COLS)
            .max(1);
        window_cols.saturating_sub(SIDEBAR_COLS).clamp(1, budget)
    } else {
        // No sidebar (offscreen oracle's pre-sidebar mode): the terminal fills
        // the window, clamped to the renderer's column ceiling.
        window_cols.clamp(1, usize::from(MAX_RENDER_COLS))
    };
    let mut vertices = Vec::new();
    let target = Target {
        width,
        height,
        metrics,
    };

    // The sidebar is chrome, not terminal content: it carries no cell
    // attributes, so it draws in the default foreground.
    if let Some(lines) = sidebar {
        for (row, line) in lines.iter().take(visible_rows).enumerate() {
            for (col, character) in line.chars().take(SIDEBAR_COLS).enumerate() {
                push_glyph(
                    &mut vertices,
                    character,
                    DEFAULT_FOREGROUND,
                    col,
                    row,
                    target,
                );
                if vertices.len() >= MAX_VERTICES {
                    return vertices;
                }
            }
        }
    }

    // `display_cells` is the per-cell parallel of `display_lines`: it selects
    // the same rows and gives a wide character's continuation cell its own
    // column, so the cell index below is the display column for every glyph —
    // the same coordinate model the string path used, now carrying the
    // attributes that path threw away.
    let rows: Vec<&[noren_terminal::Cell]> = terminal
        .map(|snapshot| snapshot.display_cells().collect())
        .unwrap_or_default();
    let total_lines = rows.len() + usize::from(status.is_some());
    let first_line = total_lines.saturating_sub(visible_rows);

    for (row, line_index) in (first_line..total_lines).enumerate() {
        if let Some(cells) = rows.get(line_index) {
            for (col, cell) in cells.iter().take(terminal_cols).enumerate() {
                // A continuation cell draws nothing but still owns its column,
                // exactly as the placeholder space did in `display_lines`.
                if cell.is_continuation() {
                    continue;
                }
                let color = resolve_foreground(cell.attributes());
                for character in cell.text().chars() {
                    push_glyph(
                        &mut vertices,
                        character,
                        color,
                        col_offset + col,
                        row,
                        target,
                    );
                    if vertices.len() >= MAX_VERTICES {
                        return vertices;
                    }
                }
            }
        } else {
            // The status line is renderer chrome with no cell backing.
            for (col, character) in status
                .unwrap_or_default()
                .chars()
                .take(terminal_cols)
                .enumerate()
            {
                push_glyph(
                    &mut vertices,
                    character,
                    DEFAULT_FOREGROUND,
                    col_offset + col,
                    row,
                    target,
                );
                if vertices.len() >= MAX_VERTICES {
                    return vertices;
                }
            }
        }
    }
    vertices
}

/// The draw surface a frame is laid out against: pixel dimensions plus the
/// cell size the grid is drawn at.
///
/// These three travel together through every emit call and are constant for a
/// frame, so passing them as one value keeps the glyph/rect helpers to a
/// readable arity now that each also carries a colour.
#[derive(Clone, Copy, Debug)]
struct Target {
    width: u32,
    height: u32,
    metrics: CellMetrics,
}

/// Emit the 5×7 bitmap glyph for `character` at grid cell `(col, row)`,
/// converting each lit pixel bit to a 2×2 rectangle of vertices in `color`.
fn push_glyph(
    vertices: &mut Vec<Vertex>,
    character: char,
    color: [f32; 3],
    col: usize,
    row: usize,
    target: Target,
) {
    let cell_width = target.metrics.width();
    let cell_height = target.metrics.height();
    let glyph = glyph_rows(character);
    for (glyph_y, bits) in glyph.into_iter().enumerate() {
        for glyph_x in 0..5 {
            if bits & (1 << (4 - glyph_x)) == 0 {
                continue;
            }
            let x = u32::try_from(col).unwrap_or(u32::MAX) * cell_width
                + u32::try_from(glyph_x).unwrap_or(0) * GLYPH_SCALE;
            let y = u32::try_from(row).unwrap_or(u32::MAX) * cell_height
                + GLYPH_TOP
                + u32::try_from(glyph_y).unwrap_or(0) * GLYPH_SCALE;
            push_rect(vertices, x, y, GLYPH_SCALE, color, target);
        }
    }
}

fn push_rect(
    vertices: &mut Vec<Vertex>,
    x: u32,
    y: u32,
    size: u32,
    color: [f32; 3],
    target: Target,
) {
    let (width, height) = (target.width, target.height);
    let left = x as f32 / width as f32 * 2.0 - 1.0;
    let right = x.saturating_add(size) as f32 / width as f32 * 2.0 - 1.0;
    let top = 1.0 - y as f32 / height as f32 * 2.0;
    let bottom = 1.0 - y.saturating_add(size) as f32 / height as f32 * 2.0;
    vertices.extend_from_slice(&[
        Vertex {
            position: [left, top],
            color,
        },
        Vertex {
            position: [left, bottom],
            color,
        },
        Vertex {
            position: [right, bottom],
            color,
        },
        Vertex {
            position: [left, top],
            color,
        },
        Vertex {
            position: [right, bottom],
            color,
        },
        Vertex {
            position: [right, top],
            color,
        },
    ]);
}

/// Exposed as `pub(crate)` for the offscreen frame-oracle (see
/// `renderer_capture.rs`); no behaviour change.
pub(crate) fn vertex_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len().saturating_mul(VERTEX_BYTES));
    for vertex in vertices {
        for value in vertex.position {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
        for value in vertex.color {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

#[rustfmt::skip]
fn glyph_rows(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [1, 1, 1, 1, 17, 17, 14],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 4, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        ';' => [0, 12, 12, 0, 4, 4, 8],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '{' => [2, 4, 4, 8, 4, 4, 2],
        '}' => [8, 4, 4, 2, 4, 4, 8],
        '<' => [2, 4, 8, 16, 8, 4, 2],
        '>' => [8, 4, 2, 1, 2, 4, 8],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '"' => [10, 10, 10, 0, 0, 0, 0],
        '\'' => [4, 4, 8, 0, 0, 0, 0],
        '`' => [8, 4, 2, 0, 0, 0, 0],
        '~' => [0, 0, 9, 22, 0, 0, 0],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '$' => [4, 15, 20, 14, 5, 30, 4],
        '%' => [25, 25, 2, 4, 8, 19, 19],
        '^' => [4, 10, 17, 0, 0, 0, 0],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '*' => [0, 21, 14, 31, 14, 21, 0],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_app::GridGeometry;
    use noren_terminal::TerminalState;

    fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
        let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
        terminal.feed_bytes(bytes);
        terminal.snapshot()
    }

    /// The PoC default cell metrics for tests that exercise the default path.
    fn poc_metrics() -> CellMetrics {
        GridGeometry::poc().cell_metrics()
    }

    // NOTE: the renderer's row/column clamp coverage used to live here as
    // `glyph_input_is_bounded_to_visible_poc_grid`, a count-based test that
    // could not distinguish the clamps from the `MAX_VERTICES` backstop (issue
    // #109). It is replaced by `frame_oracle::glyphs_stay_inside_the_render_clamp_grid`,
    // which reads pixels back from the real pipeline and asserts on *where*
    // glyphs land — the property a vertex-count assertion is structurally
    // unable to pin.

    #[test]
    fn empty_and_zero_sized_inputs_have_no_vertices() {
        let empty = snapshot(1, 1, b"");
        let text = snapshot(1, 8, b"text");
        assert!(glyph_vertices(Some(&empty), None, None, 900, 600, poc_metrics()).is_empty());
        assert!(glyph_vertices(Some(&text), None, None, 0, 600, poc_metrics()).is_empty());
    }

    /// The 16-colour and 256-colour forms of the same colour must resolve
    /// through one path, and truecolor must pass through untouched.
    #[test]
    fn ansi_indexed_and_rgb_resolve_through_one_palette() {
        use noren_terminal::AnsiColor;

        // `SGR 31` (ANSI red) and `SGR 38;5;1` (indexed 1) name the same
        // colour and must produce identical draw colours.
        assert_eq!(
            resolve_color(Color::Ansi(AnsiColor::Red), DEFAULT_FOREGROUND),
            resolve_color(Color::Indexed(1), DEFAULT_FOREGROUND),
        );
        // Truecolor passes through as exact 24-bit channels.
        assert_eq!(
            resolve_color(Color::Rgb(255, 0, 0), DEFAULT_FOREGROUND),
            [1.0, 0.0, 0.0]
        );
        // Default takes the contextual default the caller supplied.
        assert_eq!(
            resolve_color(Color::Default, DEFAULT_FOREGROUND),
            DEFAULT_FOREGROUND
        );
        // Spot-check the xterm cube and grayscale derivations: index 196 is
        // cube (5,0,0) = pure red, and 232 is the darkest gray step.
        assert_eq!(DEFAULT_PALETTE[196], [255, 0, 0]);
        assert_eq!(DEFAULT_PALETTE[232], [8, 8, 8]);
        assert_eq!(DEFAULT_PALETTE[255], [238, 238, 238]);
        // Distinct palette entries must stay distinct.
        assert_ne!(DEFAULT_PALETTE[1], DEFAULT_PALETTE[4]);
    }

    /// A cell's resolved colour must reach the vertices its glyph emits —
    /// this is the wiring issue #107 is about.
    #[test]
    fn sgr_foreground_reaches_the_vertex_colour() {
        // Red 'A' then default-coloured 'B'.
        let terminal = snapshot(1, 4, b"\x1b[31mA\x1b[0mB");
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let red = channels_to_floats(DEFAULT_ANSI_PALETTE[1]);
        assert!(
            vertices.iter().any(|vertex| vertex.color == red),
            "the SGR-31 cell must emit vertices in palette red"
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.color == DEFAULT_FOREGROUND),
            "the unstyled cell must emit vertices in the default foreground"
        );
    }

    #[test]
    fn ascii_glyphs_are_distinct_and_unknown_is_question_mark() {
        assert_ne!(glyph_rows('A'), glyph_rows('B'));
        assert_eq!(glyph_rows('a'), glyph_rows('A'));
        assert_eq!(glyph_rows('界'), glyph_rows('?'));
    }

    /// The encoded vertex stride must match what the pipeline's vertex buffer
    /// layout declares, or the GPU reads position and colour from the wrong
    /// offsets and every glyph is mispositioned or miscoloured.
    #[test]
    fn vertex_encoding_matches_the_declared_buffer_layout() {
        let terminal = snapshot(1, 2, b"A");
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        assert!(!vertices.is_empty(), "'A' emitted no vertices");
        assert_eq!(
            vertex_bytes(&vertices).len(),
            vertices.len() * VERTEX_BYTES,
            "encoded size must equal vertex count times the declared stride"
        );
        // Two position floats plus three colour floats.
        assert_eq!(VERTEX_BYTES, (2 + 3) * size_of::<f32>());
        // The colour attribute must start immediately after the position pair,
        // which is where `vertex_bytes` writes it.
        assert_eq!(VERTEX_ATTRIBUTES[1].offset, 2 * size_of::<f32>() as u64);
    }

    /// The default foreground must be the exact shade the fragment shader
    /// returned as a constant before colour existed, so an unstyled prompt
    /// does not change appearance (issue #107).
    #[test]
    fn unstyled_cells_draw_in_the_previous_constant_foreground() {
        let terminal = snapshot(1, 4, b"AB");
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        assert!(!vertices.is_empty(), "unstyled text emitted no vertices");
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.color == [0.80, 0.92, 0.82]),
            "unstyled cells must draw in the historical constant 0.80/0.92/0.82"
        );
        assert_eq!(DEFAULT_FOREGROUND, [0.80, 0.92, 0.82]);
    }

    const CELL_WIDTH: u32 = noren_app::POC_CELL_WIDTH;

    fn ndc_left(column: u32) -> f32 {
        (column * CELL_WIDTH) as f32 / 900.0 * 2.0 - 1.0
    }

    fn ndc_top_row_zero() -> f32 {
        1.0 - GLYPH_TOP as f32 / 600.0 * 2.0
    }

    fn has_rect_top_left(vertices: &[Vertex], left: f32) -> bool {
        let top = ndc_top_row_zero();
        vertices
            .chunks_exact(6)
            .any(|rect| rect[0].position == [left, top])
    }

    #[test]
    fn wide_characters_place_following_glyphs_at_display_columns() {
        let terminal = snapshot(1, 6, "a日b".as_bytes());
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());

        // a occupies column 0, 日 columns 1-2, so b must start at display
        // column 3 and nothing may draw at column 2's lead edge.
        assert!(has_rect_top_left(&vertices, ndc_left(3)));
        assert!(!has_rect_top_left(&vertices, ndc_left(2)));
    }

    #[test]
    fn wide_output_renders_like_the_equivalent_single_width_layout() {
        let wide = snapshot(1, 6, "a日b".as_bytes());
        let aligned = snapshot(1, 6, b"a? b");
        let m = poc_metrics();
        assert_eq!(
            glyph_vertices(Some(&wide), None, None, 900, 600, m),
            glyph_vertices(Some(&aligned), None, None, 900, 600, m),
            "the wide lead draws in column 1, its continuation column stays empty, and b lands in column 3"
        );
    }

    #[test]
    fn ascii_glyphs_keep_their_character_columns() {
        let terminal = snapshot(1, 4, b"BD");
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        assert!(has_rect_top_left(&vertices, ndc_left(0)));
        assert!(has_rect_top_left(&vertices, ndc_left(1)));
        assert!(!has_rect_top_left(&vertices, ndc_left(2)));
    }

    /// Issue #76 acceptance criterion: a non-default configured cell size must
    /// change what the renderer *draws*, not merely what the geometry computes.
    ///
    /// The shipped bug was that `glyph_vertices` imported the compile-time
    /// `POC_CELL_WIDTH`/`POC_CELL_HEIGHT` constants and drew at 10×20 regardless
    /// of the configured size. This test renders identical terminal content at
    /// two different cell metrics and asserts (a) the vertex arrays differ, and
    /// (b) specific vertex x-positions land at the configured cell-width stride.
    ///
    /// Mutation check: if `push_glyph` is reverted to use `POC_CELL_WIDTH` (the
    /// constant) instead of `metrics.width()`, both vertex arrays become
    /// identical and the `assert_ne!` fails.
    #[test]
    fn non_default_cell_metrics_change_what_the_renderer_draws() {
        let terminal = snapshot(1, 4, b"BBBB");
        let small = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let big = GridGeometry::with_cells(20, 40)
            .expect("valid metrics")
            .cell_metrics();
        let big_verts = glyph_vertices(Some(&terminal), None, None, 900, 600, big);

        assert_ne!(
            small, big_verts,
            "the renderer must produce different vertices at different cell sizes — \
             identical arrays mean the configured metrics were ignored (issue #76)"
        );

        // Column 1 of 'B' has glyph row 1 = 0b10001 which lights glyph column 0,
        // so a rect's left edge sits exactly at `1 * cell_width` pixels.
        // At 10px that is NDC `20/900*2 - 1`; at 20px it is `40/900*2 - 1`.
        let small_edge_1 = 10.0_f32 / 900.0 * 2.0 - 1.0;
        let big_edge_1 = 20.0_f32 / 900.0 * 2.0 - 1.0;
        assert!(
            small
                .chunks_exact(6)
                .any(|rect| (rect[0].position[0] - small_edge_1).abs() < 1e-5),
            "at default metrics, column 1's left edge must be at 10px"
        );
        assert!(
            big_verts
                .chunks_exact(6)
                .any(|rect| (rect[0].position[0] - big_edge_1).abs() < 1e-5),
            "at cell_width=20, column 1's left edge must be at 20px, not 10px"
        );
        // And conversely, the big-vertices must NOT have an edge at the 10px
        // position — that is the shipped bug.
        assert!(
            !big_verts
                .chunks_exact(6)
                .any(|rect| (rect[0].position[0] - small_edge_1).abs() < 1e-5),
            "at cell_width=20, no vertex should land at the 10px column boundary"
        );
    }
}
