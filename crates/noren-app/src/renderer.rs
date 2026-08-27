//! Minimal, bounded `wgpu` terminal view for the PoC.
//!
//! Per-cell foreground colour recorded by the terminal state reaches drawing
//! here: [`resolve_foreground`] runs a cell's `Color` selection through the
//! selected theme's palette (or passes `Rgb` through directly) and the result
//! rides on every vertex the cell's glyph emits, which the fragment shader
//! returns. A cell with no SGR colour resolves to the theme's default
//! foreground — for the default `dark` theme, the exact shade the shader
//! previously returned as a constant — so unstyled output is unchanged.
//!
//! Explicit SGR backgrounds emit one filled cell rectangle immediately before
//! that cell's glyph, so the glyph remains legible over the background.
//!
//! The theme is selected through the `[theme]` configuration section and
//! owned by the [`Renderer`]; every colour decision below — palette
//! resolution, default foreground, clear colour — reads from it, so a theme
//! that exists in configuration changes what is drawn.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use wgpu::CurrentSurfaceTexture;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use noren_app::theme::Theme;
use noren_app::{CellMetrics, MAX_RENDER_COLS, MAX_RENDER_ROWS};
use noren_terminal::{CellAttributes, Color, TerminalSnapshot};
const GLYPH_SCALE: u32 = 2;
const GLYPH_TOP: u32 = 3;
const MAX_GLYPH_PIXELS: usize = 35;
const VERTICES_PER_RECT: usize = 6;
/// One cell can emit one background rectangle plus the largest 5x7 glyph.
/// The bound is `MAX_RENDER_ROWS * MAX_RENDER_COLS * (1 + 35) * 6`.
const MAX_VERTICES: usize = (MAX_RENDER_ROWS as usize)
    * (MAX_RENDER_COLS as usize)
    * (1 + MAX_GLYPH_PIXELS)
    * VERTICES_PER_RECT;

/// Width of the left sidebar in cell columns. The terminal occupies the
/// remaining columns to the right, drawn at a pixel offset of
/// `SIDEBAR_COLS * CELL_WIDTH`.
///
/// Exposed as `pub(crate)` so `main.rs` can subtract it from the PTY/terminal
/// grid, and so the frame oracle (`renderer_capture.rs`) can render sidebar
/// content through the same pipeline.
pub(crate) const SIDEBAR_COLS: usize = 16;

/// What owns one row in a rendered terminal frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameRow {
    Terminal(usize),
    Status,
}

/// Shared vertical layout for terminal/status rendering and terminal hit testing.
///
/// Underfilled frames retain the renderer's established top alignment: the
/// first content row is drawn at frame row zero, an optional status follows
/// the content, and unused rows remain below them. When the frame is
/// overfilled, the earliest terminal rows are clipped from the top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameRowLayout {
    first_terminal_line: usize,
    terminal_row_count: usize,
    status_frame_row: Option<usize>,
}

impl FrameRowLayout {
    pub(crate) fn new(
        height: u32,
        metrics: CellMetrics,
        content_rows: usize,
        status_row_present: bool,
    ) -> Option<Self> {
        if height == 0 {
            return None;
        }
        // Preserve the renderer's historical behavior for a non-zero surface
        // shorter than one cell: row zero is emitted and clipped by the frame.
        let visible_rows = fully_drawable_rows(height, metrics).max(1);
        // A status line owns the last available row. Deriving the terminal
        // range from that reservation avoids ever adding one to `content_rows`,
        // which is important when the caller supplies `usize::MAX` rows.
        let terminal_capacity = visible_rows - usize::from(status_row_present);
        let terminal_row_count = content_rows.min(terminal_capacity);
        let first_terminal_line = content_rows - terminal_row_count;
        let status_frame_row = status_row_present.then_some(terminal_row_count);
        Some(Self {
            first_terminal_line,
            terminal_row_count,
            status_frame_row,
        })
    }

    pub(crate) const fn rendered_rows(self) -> usize {
        match self.status_frame_row {
            Some(row) => row + 1,
            None => self.terminal_row_count,
        }
    }

    pub(crate) fn row_at(self, frame_row: usize) -> Option<FrameRow> {
        if frame_row < self.terminal_row_count {
            Some(FrameRow::Terminal(self.first_terminal_line + frame_row))
        } else if self.status_frame_row == Some(frame_row) {
            Some(FrameRow::Status)
        } else {
            None
        }
    }

    pub(crate) fn content_line_at(self, frame_row: usize) -> Option<usize> {
        match self.row_at(frame_row) {
            Some(FrameRow::Terminal(line)) => Some(line),
            Some(FrameRow::Status) | None => None,
        }
    }
}

/// Fully drawable cell rows within a frame, excluding any partial bottom row.
/// The renderer's row ceiling applies to sidebar drawing and hit testing alike.
pub(crate) fn fully_drawable_rows(height: u32, metrics: CellMetrics) -> usize {
    usize::try_from(height / metrics.height())
        .unwrap_or(usize::MAX)
        .min(usize::from(MAX_RENDER_ROWS))
}

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
/// The dark theme's value is exactly the floats the fragment shader
/// previously returned as a constant for every pixel; see
/// [`noren_app::theme::DARK`] (re-exported here as [`DARK`]) for the value
/// and its history. Rounding `0.80` through `u8` and back would shift the
/// default shade slightly, and an unstyled prompt must look identical to how
/// it looked before colour existed (issue #107).
///
/// Convert an 8-bit-per-channel colour to the shader's `0.0..=1.0` floats.
const fn channels_to_floats([red, green, blue]: [u8; 3]) -> [f32; 3] {
    [
        red as f32 / 255.0,
        green as f32 / 255.0,
        blue as f32 / 255.0,
    ]
}

/// Resolve one renderer-independent colour selection to a concrete draw colour
/// **through the selected theme**.
///
/// [`Color::Default`] yields `default` (the caller supplies the contextual
/// default for the slot), [`Color::Ansi`] and [`Color::Indexed`] both resolve
/// through the theme's 256-colour table — one path, so the 16-colour and
/// 256-colour forms of the same colour agree — and [`Color::Rgb`] passes
/// through as direct 24-bit truecolor.
#[must_use]
pub(crate) fn resolve_color(theme: &Theme, color: Color, default: [f32; 3]) -> [f32; 3] {
    match color {
        Color::Default => default,
        Color::Ansi(ansi) => {
            channels_to_floats(theme.indexed_palette()[ansi.palette_index() as usize])
        }
        Color::Indexed(index) => channels_to_floats(theme.indexed_palette()[index as usize]),
        Color::Rgb(red, green, blue) => channels_to_floats([red, green, blue]),
    }
}

/// The colour a cell's glyph is drawn in, under the selected theme.
#[must_use]
pub(crate) fn resolve_foreground(theme: &Theme, attributes: &CellAttributes) -> [f32; 3] {
    resolve_color(theme, attributes.foreground(), theme.foreground())
}

/// Resolve an explicit cell background through the same theme palette /
/// truecolor path as foreground. `None` is intentional: an unstyled cell must
/// remain exactly the clear colour, with no rectangle changing its
/// rasterisation.
#[must_use]
pub(crate) fn resolve_background(theme: &Theme, attributes: &CellAttributes) -> Option<[f32; 3]> {
    match attributes.background() {
        Color::Default => None,
        background => Some(resolve_color(theme, background, theme.background())),
    }
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

/// The clear colour a theme's background becomes at the render-pass load op.
///
/// The dark theme clears with the exact historical [`CLEAR_COLOR`] constants
/// (f64 literals, not f32-widened values) so the no-`[theme]` default keeps
/// its pre-theme clear bit-for-bit; other themes clear to their own
/// background.
pub(crate) fn theme_clear_color(theme: &Theme) -> wgpu::Color {
    if *theme == noren_app::theme::DARK {
        return CLEAR_COLOR;
    }
    let [red, green, blue] = theme.background();
    wgpu::Color {
        r: red as f64,
        g: green as f64,
        b: blue as f64,
        a: 1.0,
    }
}

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
    theme: Theme,
}

impl Renderer {
    pub(crate) fn new(
        window: Arc<Window>,
        metrics: CellMetrics,
        theme: Theme,
    ) -> Result<Self, RendererError> {
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
            theme,
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

        let target = Target::new(
            &self.theme,
            self.config.width,
            self.config.height,
            self.metrics,
        );
        let vertices = glyph_vertices_for(target, terminal, sidebar, status);
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
                        load: wgpu::LoadOp::Clear(theme_clear_color(&self.theme)),
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
/// oracle's existing tests rely on). Every colour decision — palette
/// resolution, the sidebar/status default foreground, the clear colour the
/// caller loads — reads from the target's theme, which is how a configured
/// theme changes what is drawn.
pub(crate) fn glyph_vertices_for(
    target: Target,
    terminal: Option<&TerminalSnapshot>,
    sidebar: Option<&[String]>,
    status: Option<&str>,
) -> Vec<Vertex> {
    glyph_vertices_with_budget(target, terminal, sidebar, status, MAX_VERTICES)
}

/// Internal seam for exercising the production vertex-budget backstop without
/// allocating a clamp-sized frame. The shipped path above always supplies
/// [`MAX_VERTICES`]; tests use a smaller budget but traverse this same emission
/// code and the same post-primitive guards.
fn glyph_vertices_with_budget(
    target: Target,
    terminal: Option<&TerminalSnapshot>,
    sidebar: Option<&[String]>,
    status: Option<&str>,
    vertex_budget: usize,
) -> Vec<Vertex> {
    let (width, height, metrics, theme) =
        (target.width, target.height, target.metrics, target.theme);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let cell_width = metrics.width();
    let cell_height = metrics.height();
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
    // `display_cells` is the per-cell parallel of `display_lines`: it selects
    // the same rows and gives a wide character's continuation cell its own
    // column, so the cell index below is the display column for every glyph —
    // the same coordinate model the string path used, now carrying the
    // attributes that path threw away.
    let rows: Vec<&[noren_terminal::Cell]> = terminal
        .map(|snapshot| snapshot.display_cells().collect())
        .unwrap_or_default();
    let layout = FrameRowLayout::new(height, metrics, rows.len(), status.is_some())
        .expect("non-zero frame height has a row layout");
    let mut vertices = Vec::new();

    // The sidebar is chrome, not terminal content: it carries no cell
    // attributes, so it draws in the theme's default foreground. Unlike
    // terminal and status rows, sidebar rows are interactive chrome and are
    // only drawn when the whole cell is visible; sidebar hit testing uses
    // this same count.
    if let Some(lines) = sidebar {
        for (row, line) in lines
            .iter()
            .take(fully_drawable_rows(height, metrics))
            .enumerate()
        {
            for (col, character) in line.chars().take(SIDEBAR_COLS).enumerate() {
                push_glyph(
                    &mut vertices,
                    character,
                    theme.foreground(),
                    col,
                    row,
                    target,
                );
                if vertices.len() >= vertex_budget {
                    return vertices;
                }
            }
        }
    }

    for row in 0..layout.rendered_rows() {
        match layout
            .row_at(row)
            .expect("rendered row count only includes owned rows")
        {
            FrameRow::Terminal(line_index) => {
                let cells = rows
                    .get(line_index)
                    .expect("terminal layout only names display rows");
                for (col, cell) in cells.iter().take(terminal_cols).enumerate() {
                    if let Some(color) = resolve_background(&theme, cell.attributes()) {
                        push_rect(
                            &mut vertices,
                            u32::try_from(col_offset + col).unwrap_or(u32::MAX) * cell_width,
                            u32::try_from(row).unwrap_or(u32::MAX) * cell_height,
                            cell_width,
                            cell_height,
                            color,
                            target,
                        );
                    }
                    if vertices.len() >= vertex_budget {
                        return vertices;
                    }
                    // A continuation cell draws no glyph but still owns its
                    // column, exactly as the placeholder space did in
                    // `display_lines`.
                    if cell.is_continuation() {
                        continue;
                    }
                    let color = resolve_foreground(&theme, cell.attributes());
                    for character in cell.text().chars() {
                        push_glyph(
                            &mut vertices,
                            character,
                            color,
                            col_offset + col,
                            row,
                            target,
                        );
                        if vertices.len() >= vertex_budget {
                            return vertices;
                        }
                    }
                }
            }
            FrameRow::Status => {
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
                        theme.foreground(),
                        col_offset + col,
                        row,
                        target,
                    );
                    if vertices.len() >= vertex_budget {
                        return vertices;
                    }
                }
            }
        }
    }
    vertices
}

/// The draw surface a frame is laid out against: the selected theme plus the
/// pixel dimensions and cell size the grid is drawn at.
///
/// These four travel together through every emit call and are constant for a
/// frame, so passing them as one value keeps the glyph/rect helpers to a
/// readable arity; bundling the theme here is also what keeps the public
/// vertex and capture seams under the argument-count lint without scattering
/// the theme through every signature.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Target {
    pub(crate) theme: Theme,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) metrics: CellMetrics,
}

impl Target {
    /// Assemble one frame's draw surface.
    pub(crate) fn new(theme: &Theme, width: u32, height: u32, metrics: CellMetrics) -> Self {
        Self {
            theme: *theme,
            width,
            height,
            metrics,
        }
    }
}

/// Emit the 5×7 bitmap glyph for `character` at grid cell `(col, row)`.
///
/// Text glyphs keep the historical 2×2 pixels and top inset. Box-drawing
/// glyphs instead scale their bitmap across the whole cell: a line ending at a
/// cell edge must meet the next frame cell without the text glyph's padding.
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
    let cell_x = u32::try_from(col).unwrap_or(u32::MAX) * cell_width;
    let cell_y = u32::try_from(row).unwrap_or(u32::MAX) * cell_height;
    let fills_cell = is_box_drawing(character);
    let glyph = glyph_rows(character);
    for (glyph_y, bits) in glyph.into_iter().enumerate() {
        for glyph_x in 0..5 {
            if bits & (1 << (4 - glyph_x)) == 0 {
                continue;
            }
            let (x, y, width, height) = if fills_cell {
                let (x, width) = scaled_bitmap_segment(glyph_x, 5, cell_width);
                let (y, height) = scaled_bitmap_segment(glyph_y, 7, cell_height);
                (cell_x + x, cell_y + y, width, height)
            } else {
                (
                    cell_x + u32::try_from(glyph_x).unwrap_or(0) * GLYPH_SCALE,
                    cell_y + GLYPH_TOP + u32::try_from(glyph_y).unwrap_or(0) * GLYPH_SCALE,
                    GLYPH_SCALE,
                    GLYPH_SCALE,
                )
            };
            push_rect(vertices, x, y, width, height, color, target);
        }
    }
}

/// Return one proportional bitmap segment without overflowing at `u32::MAX`.
fn scaled_bitmap_segment(index: usize, segments: u32, length: u32) -> (u32, u32) {
    let boundary = |position: usize| {
        (u64::try_from(position).unwrap_or(u64::MAX) * u64::from(length) / u64::from(segments))
            as u32
    };
    let start = boundary(index);
    let end = boundary(index + 1);
    (start, end - start)
}

fn push_rect(
    vertices: &mut Vec<Vertex>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: [f32; 3],
    target: Target,
) {
    let (target_width, target_height) = (target.width, target.height);
    let left = x as f32 / target_width as f32 * 2.0 - 1.0;
    let right = x.saturating_add(width) as f32 / target_width as f32 * 2.0 - 1.0;
    let top = 1.0 - y as f32 / target_height as f32 * 2.0;
    let bottom = 1.0 - y.saturating_add(height) as f32 / target_height as f32 * 2.0;
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

const QUESTION_MARK_GLYPH: [u8; 7] = [14, 17, 1, 2, 4, 0, 4];
const UNICODE_REPLACEMENT_GLYPH: [u8; 7] = [4, 10, 17, 21, 17, 10, 4];

/// Add one visible diacritic row to an ASCII base glyph.
///
/// Uppercase glyphs already consume all seven rows, so their repeated lower
/// body row is compressed out while the final baseline row is retained.
/// Lowercase accented letters have room above their x-height; replacing row
/// zero also replaces the dot on `i` with the requested accent.
fn top_marked_ascii(base: char, mark: u8) -> [u8; 7] {
    let rows = glyph_rows(base);
    if base.is_ascii_uppercase() {
        [mark, rows[0], rows[1], rows[2], rows[3], rows[4], rows[6]]
    } else {
        [mark, rows[1], rows[2], rows[3], rows[4], rows[5], rows[6]]
    }
}

fn bottom_marked_ascii(base: char, mark: u8) -> [u8; 7] {
    let rows = glyph_rows(base);
    [rows[0], rows[1], rows[2], rows[3], rows[4], rows[6], mark]
}

/// Bitmap coverage for the complete Latin-1 Supplement block
/// (`U+00A0..=U+00FF`). Precomposed letters retain their ASCII base shape and
/// encode their accent in the spare top or bottom row; Latin-1 punctuation and
/// symbols use compact 5×7 approximations.
#[rustfmt::skip]
fn latin1_rows(character: char) -> Option<[u8; 7]> {
    const GRAVE: u8 = 0b01000;
    const ACUTE: u8 = 0b00010;
    const CIRCUMFLEX: u8 = 0b01010;
    const TILDE: u8 = 0b01011;
    const DIAERESIS: u8 = 0b10001;
    const RING: u8 = 0b00100;

    let rows = match character {
        '\u{00a0}' => [0, 0, 0, 0, 0, 0, 0],
        '\u{00a1}' => [4, 0, 4, 4, 4, 4, 4],
        '\u{00a2}' => [4, 14, 20, 20, 21, 14, 4],
        '\u{00a3}' => [6, 8, 8, 30, 8, 8, 31],
        '\u{00a4}' => [0, 17, 14, 10, 14, 17, 0],
        '\u{00a5}' => [17, 10, 4, 31, 4, 31, 4],
        '\u{00a6}' => [4, 4, 4, 0, 4, 4, 4],
        '\u{00a7}' => [14, 16, 12, 18, 6, 1, 14],
        '\u{00a8}' => [17, 0, 0, 0, 0, 0, 0],
        '\u{00a9}' => [14, 17, 23, 20, 23, 17, 14],
        '\u{00aa}' => [4, 10, 14, 10, 0, 14, 0],
        '\u{00ab}' => [0, 5, 10, 20, 10, 5, 0],
        '\u{00ac}' => [0, 0, 31, 1, 1, 0, 0],
        '\u{00ad}' => [0, 0, 0, 14, 0, 0, 0],
        '\u{00ae}' => [14, 17, 30, 21, 18, 17, 14],
        '\u{00af}' => [31, 0, 0, 0, 0, 0, 0],
        '\u{00b0}' => [6, 9, 9, 6, 0, 0, 0],
        '\u{00b1}' => [4, 4, 31, 4, 4, 0, 31],
        '\u{00b2}' => [6, 9, 2, 4, 15, 0, 0],
        '\u{00b3}' => [14, 1, 6, 1, 14, 0, 0],
        '\u{00b4}' => [2, 4, 0, 0, 0, 0, 0],
        '\u{00b5}' => [0, 0, 17, 17, 19, 29, 16],
        '\u{00b6}' => [15, 29, 29, 13, 5, 5, 5],
        '\u{00b7}' => [0, 0, 0, 4, 0, 0, 0],
        '\u{00b8}' => [0, 0, 0, 0, 0, 4, 8],
        '\u{00b9}' => [4, 12, 4, 4, 14, 0, 0],
        '\u{00ba}' => [4, 10, 4, 10, 0, 14, 0],
        '\u{00bb}' => [0, 20, 10, 5, 10, 20, 0],
        '\u{00bc}' => [8, 24, 9, 2, 5, 7, 1],
        '\u{00bd}' => [8, 24, 9, 2, 7, 1, 7],
        '\u{00be}' => [24, 8, 25, 2, 5, 7, 1],
        '\u{00bf}' => [4, 0, 4, 8, 16, 17, 14],
        '\u{00c0}' => top_marked_ascii('A', GRAVE),
        '\u{00c1}' => top_marked_ascii('A', ACUTE),
        '\u{00c2}' => top_marked_ascii('A', CIRCUMFLEX),
        '\u{00c3}' => top_marked_ascii('A', TILDE),
        '\u{00c4}' => top_marked_ascii('A', DIAERESIS),
        '\u{00c5}' => top_marked_ascii('A', RING),
        '\u{00c6}' => [14, 21, 20, 30, 20, 21, 23],
        '\u{00c7}' => bottom_marked_ascii('C', 4),
        '\u{00c8}' => top_marked_ascii('E', GRAVE),
        '\u{00c9}' => top_marked_ascii('E', ACUTE),
        '\u{00ca}' => top_marked_ascii('E', CIRCUMFLEX),
        '\u{00cb}' => top_marked_ascii('E', DIAERESIS),
        '\u{00cc}' => top_marked_ascii('I', GRAVE),
        '\u{00cd}' => top_marked_ascii('I', ACUTE),
        '\u{00ce}' => top_marked_ascii('I', CIRCUMFLEX),
        '\u{00cf}' => top_marked_ascii('I', DIAERESIS),
        '\u{00d0}' => [30, 9, 9, 29, 9, 9, 30],
        '\u{00d1}' => top_marked_ascii('N', TILDE),
        '\u{00d2}' => top_marked_ascii('O', GRAVE),
        '\u{00d3}' => top_marked_ascii('O', ACUTE),
        '\u{00d4}' => top_marked_ascii('O', CIRCUMFLEX),
        '\u{00d5}' => top_marked_ascii('O', TILDE),
        '\u{00d6}' => top_marked_ascii('O', DIAERESIS),
        '\u{00d7}' => [0, 17, 10, 4, 10, 17, 0],
        '\u{00d8}' => [15, 19, 21, 21, 21, 25, 30],
        '\u{00d9}' => top_marked_ascii('U', GRAVE),
        '\u{00da}' => top_marked_ascii('U', ACUTE),
        '\u{00db}' => top_marked_ascii('U', CIRCUMFLEX),
        // `top_marked_ascii` cannot express an umlauted U: every U body row
        // is 0b10001 — the same bits as DIAERESIS — so the compressed glyph
        // would be pixel-identical to plain U and the diaeresis invisible.
        // The U is therefore shortened by one row and a blank separator row
        // keeps the two dots readable as a floating diaeresis.
        '\u{00dc}' => [DIAERESIS, 0, 17, 17, 17, 17, 14],
        '\u{00dd}' => top_marked_ascii('Y', ACUTE),
        // Uppercase thorn keeps a full-height stem but drops its bowl one
        // row below cap height, staying distinct from both `P` and the
        // x-height-bowled lowercase `þ` below.
        '\u{00de}' => [16, 30, 17, 17, 30, 16, 16],
        '\u{00df}' => [12, 18, 18, 28, 18, 18, 29],
        '\u{00e0}' => top_marked_ascii('a', GRAVE),
        '\u{00e1}' => top_marked_ascii('a', ACUTE),
        '\u{00e2}' => top_marked_ascii('a', CIRCUMFLEX),
        '\u{00e3}' => top_marked_ascii('a', TILDE),
        '\u{00e4}' => top_marked_ascii('a', DIAERESIS),
        '\u{00e5}' => top_marked_ascii('a', RING),
        '\u{00e6}' => [0, 0, 14, 5, 15, 20, 15],
        '\u{00e7}' => bottom_marked_ascii('c', 4),
        '\u{00e8}' => top_marked_ascii('e', GRAVE),
        '\u{00e9}' => top_marked_ascii('e', ACUTE),
        '\u{00ea}' => top_marked_ascii('e', CIRCUMFLEX),
        '\u{00eb}' => top_marked_ascii('e', DIAERESIS),
        '\u{00ec}' => top_marked_ascii('i', GRAVE),
        '\u{00ed}' => top_marked_ascii('i', ACUTE),
        '\u{00ee}' => top_marked_ascii('i', CIRCUMFLEX),
        '\u{00ef}' => top_marked_ascii('i', DIAERESIS),
        '\u{00f0}' => [2, 4, 14, 3, 15, 17, 14],
        '\u{00f1}' => top_marked_ascii('n', TILDE),
        '\u{00f2}' => top_marked_ascii('o', GRAVE),
        '\u{00f3}' => top_marked_ascii('o', ACUTE),
        '\u{00f4}' => top_marked_ascii('o', CIRCUMFLEX),
        '\u{00f5}' => top_marked_ascii('o', TILDE),
        '\u{00f6}' => top_marked_ascii('o', DIAERESIS),
        '\u{00f7}' => [0, 4, 0, 31, 0, 4, 0],
        '\u{00f8}' => [0, 0, 15, 19, 21, 25, 30],
        '\u{00f9}' => top_marked_ascii('u', GRAVE),
        '\u{00fa}' => top_marked_ascii('u', ACUTE),
        '\u{00fb}' => top_marked_ascii('u', CIRCUMFLEX),
        '\u{00fc}' => top_marked_ascii('u', DIAERESIS),
        '\u{00fd}' => top_marked_ascii('y', ACUTE),
        '\u{00fe}' => [16, 16, 30, 17, 30, 16, 16],
        '\u{00ff}' => top_marked_ascii('y', DIAERESIS),
        _ => return None,
    };
    Some(rows)
}

fn is_box_drawing(character: char) -> bool {
    matches!(character, '\u{2500}'..='\u{257f}')
}

#[derive(Clone, Copy)]
enum BoxStroke {
    None,
    Light,
    Heavy,
    Double,
}

/// Rasterize up/right/down/left box-drawing arms around the bitmap centre.
///
/// Light strokes occupy one pixel, heavy strokes occupy three, and double
/// strokes occupy the two pixels either side of the centre. The bitmap is
/// stretched across the whole terminal cell by [`push_glyph`], so every arm
/// reaches and joins the corresponding edge of an adjacent cell.
fn box_stroke_rows(up: BoxStroke, right: BoxStroke, down: BoxStroke, left: BoxStroke) -> [u8; 7] {
    fn vertical_arm(rows: &mut [u8; 7], stroke: BoxStroke, start: usize, end: usize) {
        let mask = match stroke {
            BoxStroke::None => 0,
            BoxStroke::Light => 0b00100,
            BoxStroke::Heavy => 0b01110,
            BoxStroke::Double => 0b01010,
        };
        for row in &mut rows[start..=end] {
            *row |= mask;
        }
    }

    fn horizontal_arm(rows: &mut [u8; 7], stroke: BoxStroke, start: usize, end: usize) {
        let mut mask = 0;
        for column in start..=end {
            mask |= 1 << (4 - column);
        }
        match stroke {
            BoxStroke::None => {}
            BoxStroke::Light => rows[3] |= mask,
            BoxStroke::Heavy => {
                for row in &mut rows[2..=4] {
                    *row |= mask;
                }
            }
            BoxStroke::Double => {
                rows[2] |= mask;
                rows[4] |= mask;
            }
        }
    }

    let mut rows = [0; 7];
    vertical_arm(&mut rows, up, 0, 3);
    horizontal_arm(&mut rows, right, 2, 4);
    vertical_arm(&mut rows, down, 3, 6);
    horizontal_arm(&mut rows, left, 0, 2);
    rows
}

/// Bitmap coverage for the complete Unicode Box Drawing block
/// (`U+2500..=U+257F`). Weight variants share a topology rasterizer; dashed,
/// rounded, and diagonal forms have explicit bitmaps where arm weights are not
/// enough to describe their appearance.
#[rustfmt::skip]
fn box_drawing_rows(character: char) -> Option<[u8; 7]> {
    use BoxStroke::{Double as D, Heavy as H, Light as L, None as N};

    let rows = match character {
        '\u{2500}' => box_stroke_rows(N, L, N, L),
        '\u{2501}' => box_stroke_rows(N, H, N, H),
        '\u{2502}' => box_stroke_rows(L, N, L, N),
        '\u{2503}' => box_stroke_rows(H, N, H, N),
        '\u{2504}' => [0, 0, 0, 0b11011, 0, 0, 0],
        '\u{2505}' => [0, 0, 0b11011, 0b11011, 0b11011, 0, 0],
        '\u{2506}' => [4, 4, 0, 4, 4, 0, 4],
        '\u{2507}' => [14, 14, 0, 14, 14, 0, 14],
        '\u{2508}' => [0, 0, 0, 0b10101, 0, 0, 0],
        '\u{2509}' => [0, 0, 0b10101, 0b10101, 0b10101, 0, 0],
        '\u{250a}' => [4, 0, 4, 0, 4, 0, 4],
        '\u{250b}' => [14, 0, 14, 0, 14, 0, 14],
        '\u{250c}' => box_stroke_rows(N, L, L, N),
        '\u{250d}' => box_stroke_rows(N, H, L, N),
        '\u{250e}' => box_stroke_rows(N, L, H, N),
        '\u{250f}' => box_stroke_rows(N, H, H, N),
        '\u{2510}' => box_stroke_rows(N, N, L, L),
        '\u{2511}' => box_stroke_rows(N, N, L, H),
        '\u{2512}' => box_stroke_rows(N, N, H, L),
        '\u{2513}' => box_stroke_rows(N, N, H, H),
        '\u{2514}' => box_stroke_rows(L, L, N, N),
        '\u{2515}' => box_stroke_rows(L, H, N, N),
        '\u{2516}' => box_stroke_rows(H, L, N, N),
        '\u{2517}' => box_stroke_rows(H, H, N, N),
        '\u{2518}' => box_stroke_rows(L, N, N, L),
        '\u{2519}' => box_stroke_rows(L, N, N, H),
        '\u{251a}' => box_stroke_rows(H, N, N, L),
        '\u{251b}' => box_stroke_rows(H, N, N, H),
        '\u{251c}' => box_stroke_rows(L, L, L, N),
        '\u{251d}' => box_stroke_rows(L, H, L, N),
        '\u{251e}' => box_stroke_rows(H, L, L, N),
        '\u{251f}' => box_stroke_rows(L, L, H, N),
        '\u{2520}' => box_stroke_rows(H, L, H, N),
        '\u{2521}' => box_stroke_rows(H, H, L, N),
        '\u{2522}' => box_stroke_rows(L, H, H, N),
        '\u{2523}' => box_stroke_rows(H, H, H, N),
        '\u{2524}' => box_stroke_rows(L, N, L, L),
        '\u{2525}' => box_stroke_rows(L, N, L, H),
        '\u{2526}' => box_stroke_rows(H, N, L, L),
        '\u{2527}' => box_stroke_rows(L, N, H, L),
        '\u{2528}' => box_stroke_rows(H, N, H, L),
        '\u{2529}' => box_stroke_rows(H, N, L, H),
        '\u{252a}' => box_stroke_rows(L, N, H, H),
        '\u{252b}' => box_stroke_rows(H, N, H, H),
        '\u{252c}' => box_stroke_rows(N, L, L, L),
        '\u{252d}' => box_stroke_rows(N, L, L, H),
        '\u{252e}' => box_stroke_rows(N, H, L, L),
        '\u{252f}' => box_stroke_rows(N, H, L, H),
        '\u{2530}' => box_stroke_rows(N, L, H, L),
        '\u{2531}' => box_stroke_rows(N, L, H, H),
        '\u{2532}' => box_stroke_rows(N, H, H, L),
        '\u{2533}' => box_stroke_rows(N, H, H, H),
        '\u{2534}' => box_stroke_rows(L, L, N, L),
        '\u{2535}' => box_stroke_rows(L, L, N, H),
        '\u{2536}' => box_stroke_rows(L, H, N, L),
        '\u{2537}' => box_stroke_rows(L, H, N, H),
        '\u{2538}' => box_stroke_rows(H, L, N, L),
        '\u{2539}' => box_stroke_rows(H, L, N, H),
        '\u{253a}' => box_stroke_rows(H, H, N, L),
        '\u{253b}' => box_stroke_rows(H, H, N, H),
        '\u{253c}' => box_stroke_rows(L, L, L, L),
        '\u{253d}' => box_stroke_rows(L, L, L, H),
        '\u{253e}' => box_stroke_rows(L, H, L, L),
        '\u{253f}' => box_stroke_rows(L, H, L, H),
        '\u{2540}' => box_stroke_rows(H, L, L, L),
        '\u{2541}' => box_stroke_rows(L, L, H, L),
        '\u{2542}' => box_stroke_rows(H, L, H, L),
        '\u{2543}' => box_stroke_rows(H, L, L, H),
        '\u{2544}' => box_stroke_rows(H, H, L, L),
        '\u{2545}' => box_stroke_rows(L, L, H, H),
        '\u{2546}' => box_stroke_rows(L, H, H, L),
        '\u{2547}' => box_stroke_rows(H, H, L, H),
        '\u{2548}' => box_stroke_rows(L, H, H, H),
        '\u{2549}' => box_stroke_rows(H, L, H, H),
        '\u{254a}' => box_stroke_rows(H, H, H, L),
        '\u{254b}' => box_stroke_rows(H, H, H, H),
        '\u{254c}' => [0, 0, 0, 0b11101, 0, 0, 0],
        '\u{254d}' => [0, 0, 0b11101, 0b11101, 0b11101, 0, 0],
        '\u{254e}' => [4, 4, 4, 0, 4, 4, 4],
        '\u{254f}' => [14, 14, 14, 0, 14, 14, 14],
        '\u{2550}' => box_stroke_rows(N, D, N, D),
        '\u{2551}' => box_stroke_rows(D, N, D, N),
        '\u{2552}' => box_stroke_rows(N, D, L, N),
        '\u{2553}' => box_stroke_rows(N, L, D, N),
        '\u{2554}' => box_stroke_rows(N, D, D, N),
        '\u{2555}' => box_stroke_rows(N, N, L, D),
        '\u{2556}' => box_stroke_rows(N, N, D, L),
        '\u{2557}' => box_stroke_rows(N, N, D, D),
        '\u{2558}' => box_stroke_rows(L, D, N, N),
        '\u{2559}' => box_stroke_rows(D, L, N, N),
        '\u{255a}' => box_stroke_rows(D, D, N, N),
        '\u{255b}' => box_stroke_rows(L, N, N, D),
        '\u{255c}' => box_stroke_rows(D, N, N, L),
        '\u{255d}' => box_stroke_rows(D, N, N, D),
        '\u{255e}' => box_stroke_rows(L, D, L, N),
        '\u{255f}' => box_stroke_rows(D, L, D, N),
        '\u{2560}' => box_stroke_rows(D, D, D, N),
        '\u{2561}' => box_stroke_rows(L, N, L, D),
        '\u{2562}' => box_stroke_rows(D, N, D, L),
        '\u{2563}' => box_stroke_rows(D, N, D, D),
        '\u{2564}' => box_stroke_rows(N, D, L, D),
        '\u{2565}' => box_stroke_rows(N, L, D, L),
        '\u{2566}' => box_stroke_rows(N, D, D, D),
        '\u{2567}' => box_stroke_rows(L, D, N, D),
        '\u{2568}' => box_stroke_rows(D, L, N, L),
        '\u{2569}' => box_stroke_rows(D, D, N, D),
        '\u{256a}' => box_stroke_rows(L, D, L, D),
        '\u{256b}' => box_stroke_rows(D, L, D, L),
        '\u{256c}' => box_stroke_rows(D, D, D, D),
        '\u{256d}' => [0, 0, 2, 7, 4, 4, 4],
        '\u{256e}' => [0, 0, 8, 28, 4, 4, 4],
        '\u{256f}' => [4, 4, 4, 28, 8, 0, 0],
        '\u{2570}' => [4, 4, 4, 7, 2, 0, 0],
        '\u{2571}' => [1, 2, 2, 4, 8, 8, 16],
        '\u{2572}' => [16, 8, 8, 4, 2, 2, 1],
        '\u{2573}' => [17, 10, 10, 4, 10, 10, 17],
        '\u{2574}' => box_stroke_rows(N, N, N, L),
        '\u{2575}' => box_stroke_rows(L, N, N, N),
        '\u{2576}' => box_stroke_rows(N, L, N, N),
        '\u{2577}' => box_stroke_rows(N, N, L, N),
        '\u{2578}' => box_stroke_rows(N, N, N, H),
        '\u{2579}' => box_stroke_rows(H, N, N, N),
        '\u{257a}' => box_stroke_rows(N, H, N, N),
        '\u{257b}' => box_stroke_rows(N, N, H, N),
        '\u{257c}' => box_stroke_rows(N, H, N, L),
        '\u{257d}' => box_stroke_rows(L, N, H, N),
        '\u{257e}' => box_stroke_rows(N, L, N, H),
        '\u{257f}' => box_stroke_rows(H, N, L, N),
        _ => return None,
    };
    Some(rows)
}

#[rustfmt::skip]
fn glyph_rows(character: char) -> [u8; 7] {
    match character {
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
        'a' => [0, 0, 14, 1, 15, 17, 15],
        'b' => [16, 16, 22, 25, 17, 17, 30],
        'c' => [0, 0, 14, 17, 16, 17, 14],
        'd' => [1, 1, 13, 19, 17, 17, 15],
        'e' => [0, 0, 14, 17, 31, 16, 14],
        'f' => [6, 9, 8, 28, 8, 8, 8],
        'g' => [0, 0, 15, 17, 15, 1, 14],
        'h' => [16, 16, 22, 25, 17, 17, 17],
        'i' => [4, 0, 12, 4, 4, 4, 14],
        'j' => [2, 0, 6, 2, 2, 18, 12],
        'k' => [16, 16, 18, 20, 24, 20, 18],
        'l' => [12, 4, 4, 4, 4, 4, 14],
        'm' => [0, 0, 26, 21, 21, 17, 17],
        'n' => [0, 0, 30, 17, 17, 17, 17],
        'o' => [0, 0, 14, 17, 17, 17, 14],
        'p' => [0, 0, 30, 17, 30, 16, 16],
        'q' => [0, 0, 15, 17, 15, 1, 1],
        'r' => [0, 0, 22, 25, 16, 16, 16],
        's' => [0, 0, 15, 16, 14, 1, 30],
        't' => [8, 8, 28, 8, 8, 9, 6],
        'u' => [0, 0, 17, 17, 17, 19, 13],
        'v' => [0, 0, 17, 17, 17, 10, 4],
        'w' => [0, 0, 17, 17, 21, 21, 10],
        'x' => [0, 0, 17, 10, 4, 10, 17],
        'y' => [0, 0, 17, 17, 15, 1, 14],
        'z' => [0, 0, 31, 2, 4, 8, 31],
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
        '?' => QUESTION_MARK_GLYPH,
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
        _ => latin1_rows(character)
            .or_else(|| box_drawing_rows(character))
            .unwrap_or(UNICODE_REPLACEMENT_GLYPH),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noren_app::GridGeometry;
    use noren_app::theme::DARK;
    use noren_terminal::TerminalState;

    /// Default-theme vertex emission for test call sites: the shape the
    /// pre-theme tests used, now entering through the [`Target`] seam.
    fn frame_vertices(
        terminal: Option<&TerminalSnapshot>,
        sidebar: Option<&[String]>,
        status: Option<&str>,
        width: u32,
        height: u32,
        metrics: CellMetrics,
    ) -> Vec<Vertex> {
        glyph_vertices_for(
            Target::new(&Theme::default(), width, height, metrics),
            terminal,
            sidebar,
            status,
        )
    }

    fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
        let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
        terminal.feed_bytes(bytes);
        terminal.snapshot()
    }

    /// The PoC default cell metrics for tests that exercise the default path.
    fn poc_metrics() -> CellMetrics {
        GridGeometry::poc().cell_metrics()
    }

    /// The dark theme's clear colour is the exact historical constant — f64
    /// literals, not f32-widened values — so the no-`[theme]` default keeps
    /// its pre-theme clear bit-for-bit (`theme_clear_color` special-cases
    /// dark for exactly this reason).
    #[test]
    fn dark_theme_clear_colour_is_the_historical_constant() {
        let dark = theme_clear_color(&Theme::default());
        assert_eq!(
            (dark.r, dark.g, dark.b, dark.a),
            (CLEAR_COLOR.r, CLEAR_COLOR.g, CLEAR_COLOR.b, CLEAR_COLOR.a)
        );
    }

    // NOTE: the renderer's row/column clamp coverage used to live here as
    // `glyph_input_is_bounded_to_visible_poc_grid`, a count-based test that
    // could not distinguish the clamps from the `MAX_VERTICES` backstop (issue
    // #109). It is replaced by `frame_oracle::glyphs_stay_inside_the_render_clamp_grid`,
    // which reads pixels back from the real pipeline and asserts on *where*
    // glyphs land — the property a vertex-count assertion is structurally
    // unable to pin. The separate test below exercises the vertex-budget
    // backstop itself through a deliberately smaller budget (issue #118).

    #[test]
    fn vertex_budget_backstop_truncates_every_emission_path() {
        let metrics = poc_metrics();
        let height = metrics.height();
        let glyph_budget = glyph_rows('A')
            .iter()
            .map(|row| row.count_ones() as usize)
            .sum::<usize>()
            * VERTICES_PER_RECT;
        assert!(glyph_budget > 0, "the detector glyph must emit vertices");

        // Each fixture has a known first primitive or glyph and additional
        // drawable input after it. At a budget equal to that first emission,
        // the limited result must be its exact prefix. A `<=` ceiling assertion
        // would not prove which guard stopped emission; these exact prefix and
        // positive-control checks fail if the relevant early return is removed.
        let assert_truncated =
            |path: &str, full: Vec<Vertex>, limited: Vec<Vertex>, expected_len: usize| {
                assert!(
                    full.len() > expected_len,
                    "{path} fixture has no post-budget emission to detect a missing guard"
                );
                assert_eq!(
                    limited.len(),
                    expected_len,
                    "{path} did not stop at the first budget-reaching emission"
                );
                assert_eq!(
                    limited.as_slice(),
                    &full[..expected_len],
                    "{path} did not return the exact pre-backstop prefix"
                );
            };

        let sidebar = vec!["AA".to_owned()];
        let sidebar_width = u32::try_from(SIDEBAR_COLS).unwrap_or(u32::MAX) * metrics.width();
        assert_truncated(
            "sidebar glyph",
            glyph_vertices_for(
                Target::new(&Theme::default(), sidebar_width, height, metrics),
                None,
                Some(sidebar.as_slice()),
                None,
            ),
            glyph_vertices_with_budget(
                Target::new(&Theme::default(), sidebar_width, height, metrics),
                None,
                Some(sidebar.as_slice()),
                None,
                glyph_budget,
            ),
            glyph_budget,
        );

        let background_terminal = snapshot(1, 1, b"\x1b[48;2;12;98;201mA");
        assert_truncated(
            "terminal background rectangle",
            glyph_vertices_for(
                Target::new(&Theme::default(), metrics.width(), height, metrics),
                Some(&background_terminal),
                None,
                None,
            ),
            glyph_vertices_with_budget(
                Target::new(&Theme::default(), metrics.width(), height, metrics),
                Some(&background_terminal),
                None,
                None,
                VERTICES_PER_RECT,
            ),
            VERTICES_PER_RECT,
        );

        // Two ordinary cells would be a weak detector for the terminal-glyph
        // guard: the background-path check runs before the second cell and
        // could stop it instead. A combining mark shares the first cell, so
        // only the guard inside this cell's character loop can suppress it.
        let terminal_glyphs = snapshot(1, 1, "A\u{0301}".as_bytes());
        let rows: Vec<_> = terminal_glyphs.display_cells().collect();
        assert_eq!(
            rows[0][0].text(),
            "A\u{0301}",
            "terminal-glyph fixture must keep both code points in one cell"
        );
        assert_truncated(
            "terminal cell glyph",
            glyph_vertices_for(
                Target::new(&Theme::default(), metrics.width(), height, metrics),
                Some(&terminal_glyphs),
                None,
                None,
            ),
            glyph_vertices_with_budget(
                Target::new(&Theme::default(), metrics.width(), height, metrics),
                Some(&terminal_glyphs),
                None,
                None,
                glyph_budget,
            ),
            glyph_budget,
        );

        let status_width = 2 * metrics.width();
        assert_truncated(
            "status glyph",
            glyph_vertices_for(
                Target::new(&Theme::default(), status_width, height, metrics),
                None,
                None,
                Some("AA"),
            ),
            glyph_vertices_with_budget(
                Target::new(&Theme::default(), status_width, height, metrics),
                None,
                None,
                Some("AA"),
                glyph_budget,
            ),
            glyph_budget,
        );
    }

    #[test]
    fn shared_frame_row_layout_pins_all_alignment_regimes() {
        let metrics = poc_metrics();
        let height = 30 * metrics.height();

        let underfilled = FrameRowLayout::new(height, metrics, 1, true).expect("non-zero frame");
        assert_eq!(underfilled.rendered_rows(), 2);
        assert_eq!(underfilled.row_at(0), Some(FrameRow::Terminal(0)));
        assert_eq!(underfilled.row_at(1), Some(FrameRow::Status));
        assert_eq!(underfilled.row_at(2), None);
        assert_eq!(underfilled.row_at(29), None);

        let exact = FrameRowLayout::new(height, metrics, 30, false).expect("non-zero frame");
        assert_eq!(exact.row_at(0), Some(FrameRow::Terminal(0)));
        assert_eq!(exact.row_at(29), Some(FrameRow::Terminal(29)));
        assert_eq!(exact.row_at(30), None);

        let status_only = FrameRowLayout::new(height, metrics, 0, true).expect("non-zero frame");
        assert_eq!(status_only.row_at(0), Some(FrameRow::Status));
        assert_eq!(status_only.row_at(1), None);

        let clipped = FrameRowLayout::new(height, metrics, 30, true).expect("non-zero frame");
        assert_eq!(clipped.row_at(0), Some(FrameRow::Terminal(1)));
        assert_eq!(clipped.row_at(28), Some(FrameRow::Terminal(29)));
        assert_eq!(clipped.row_at(29), Some(FrameRow::Status));
        assert_eq!(clipped.row_at(30), None);
    }

    #[test]
    fn nonzero_subcell_frame_preserves_the_renderer_row_zero_clip() {
        let metrics = poc_metrics();
        let layout = FrameRowLayout::new(1, metrics, 1, false).expect("non-zero frame");

        assert_eq!(fully_drawable_rows(1, metrics), 0);
        assert_eq!(layout.rendered_rows(), 1);
        assert_eq!(layout.row_at(0), Some(FrameRow::Terminal(0)));
        assert!(FrameRowLayout::new(0, metrics, 1, false).is_none());
    }

    #[test]
    fn max_content_rows_keep_the_status_last_without_overflow() {
        let metrics = poc_metrics();
        let visible_rows = usize::from(MAX_RENDER_ROWS);
        let height = (u32::from(MAX_RENDER_ROWS) + 1) * metrics.height();

        let with_status =
            FrameRowLayout::new(height, metrics, usize::MAX, true).expect("non-zero frame");
        assert_eq!(fully_drawable_rows(height, metrics), visible_rows);
        assert_eq!(with_status.rendered_rows(), visible_rows);
        assert_eq!(
            with_status.row_at(0),
            Some(FrameRow::Terminal(usize::MAX - (visible_rows - 1)))
        );
        assert_eq!(
            with_status.row_at(visible_rows - 2),
            Some(FrameRow::Terminal(usize::MAX - 1))
        );
        assert_eq!(with_status.row_at(visible_rows - 1), Some(FrameRow::Status));
        assert_eq!(with_status.row_at(visible_rows), None);

        let without_status =
            FrameRowLayout::new(height, metrics, usize::MAX, false).expect("non-zero frame");
        assert_eq!(
            without_status.row_at(0),
            Some(FrameRow::Terminal(usize::MAX - visible_rows))
        );
        assert_eq!(
            without_status.row_at(visible_rows - 1),
            Some(FrameRow::Terminal(usize::MAX - 1))
        );

        let subcell_status =
            FrameRowLayout::new(1, metrics, usize::MAX, true).expect("non-zero frame");
        assert_eq!(subcell_status.row_at(0), Some(FrameRow::Status));
        assert_eq!(subcell_status.row_at(1), None);
    }

    #[test]
    fn sidebar_draws_only_fully_visible_cell_rows() {
        let metrics = poc_metrics();
        let width = (SIDEBAR_COLS as u32) * metrics.width();
        let lines = vec!["A".to_owned(), "B".to_owned()];
        let draw =
            |height| frame_vertices(None, Some(lines.as_slice()), None, width, height, metrics);
        let first_row_vertices = glyph_rows('A')
            .iter()
            .map(|row| row.count_ones() as usize)
            .sum::<usize>()
            * VERTICES_PER_RECT;

        assert!(draw(0).is_empty(), "a zero-height frame draws no sidebar");
        assert!(
            draw(metrics.height() - 1).is_empty(),
            "a sub-cell frame draws no partial sidebar row"
        );
        assert_eq!(
            draw(metrics.height()).len(),
            first_row_vertices,
            "exactly one cell of height draws exactly the first sidebar row"
        );
        assert_eq!(
            draw(metrics.height() + metrics.height() / 2).len(),
            first_row_vertices,
            "a partial second cell must not draw the second sidebar row"
        );
    }

    #[test]
    fn empty_and_zero_sized_inputs_have_no_vertices() {
        let empty = snapshot(1, 1, b"");
        let text = snapshot(1, 8, b"text");
        assert!(frame_vertices(Some(&empty), None, None, 900, 600, poc_metrics()).is_empty());
        assert!(frame_vertices(Some(&text), None, None, 0, 600, poc_metrics()).is_empty());
    }

    /// The 16-colour and 256-colour forms of the same colour must resolve
    /// through one path, and truecolor must pass through untouched.
    #[test]
    fn ansi_indexed_and_rgb_resolve_through_one_palette() {
        use noren_terminal::AnsiColor;

        // `SGR 31` (ANSI red) and `SGR 38;5;1` (indexed 1) name the same
        // colour and must produce identical draw colours.
        assert_eq!(
            resolve_color(
                &Theme::default(),
                Color::Ansi(AnsiColor::Red),
                DARK.foreground()
            ),
            resolve_color(&Theme::default(), Color::Indexed(1), DARK.foreground()),
        );
        // Truecolor passes through as exact 24-bit channels.
        assert_eq!(
            resolve_color(&Theme::default(), Color::Rgb(255, 0, 0), DARK.foreground()),
            [1.0, 0.0, 0.0]
        );
        // Default takes the contextual default the caller supplied.
        assert_eq!(
            resolve_color(&Theme::default(), Color::Default, DARK.foreground()),
            DARK.foreground()
        );
        // Spot-check the xterm cube and grayscale derivations: index 196 is
        // cube (5,0,0) = pure red, and 232 is the darkest gray step.
        assert_eq!(DARK.indexed_palette()[196], [255, 0, 0]);
        assert_eq!(DARK.indexed_palette()[232], [8, 8, 8]);
        assert_eq!(DARK.indexed_palette()[255], [238, 238, 238]);
        // Distinct palette entries must stay distinct.
        assert_ne!(DARK.indexed_palette()[1], DARK.indexed_palette()[4]);
    }

    /// A cell's resolved colour must reach the vertices its glyph emits —
    /// this is the wiring issue #107 is about.
    #[test]
    fn sgr_foreground_reaches_the_vertex_colour() {
        // Red 'A' then default-coloured 'B'.
        let terminal = snapshot(1, 4, b"\x1b[31mA\x1b[0mB");
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let red = channels_to_floats(DARK.ansi()[1]);
        assert!(
            vertices.iter().any(|vertex| vertex.color == red),
            "the SGR-31 cell must emit vertices in palette red"
        );
        assert!(
            vertices
                .iter()
                .any(|vertex| vertex.color == DARK.foreground()),
            "the unstyled cell must emit vertices in the default foreground"
        );
    }

    #[test]
    fn explicit_background_emits_a_full_rect_before_the_glyph() {
        let terminal = snapshot(1, 1, b"\x1b[38;2;241;207;33;48;2;12;98;201mA");
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let background = channels_to_floats([12, 98, 201]);
        let foreground = channels_to_floats([241, 207, 33]);

        assert!(
            vertices.len() > VERTICES_PER_RECT,
            "background and glyph missing"
        );
        assert!(
            vertices[..VERTICES_PER_RECT]
                .iter()
                .all(|vertex| vertex.color == background),
            "the first primitive must be the cell background"
        );
        assert_eq!(
            vertices[VERTICES_PER_RECT].color, foreground,
            "the glyph must follow its background"
        );
        assert_eq!(
            vertices[0].position,
            [-1.0, 1.0],
            "the background must start at the cell's top-left"
        );
        assert_eq!(
            vertices[2].position,
            [(-1.0 + 20.0 / 900.0), 1.0 - 40.0 / 600.0],
            "the background must cover one configured cell"
        );
    }

    #[test]
    fn default_background_emits_no_extra_vertices() {
        let terminal = snapshot(1, 1, b"A");
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let glyph_pixels = glyph_rows('A')
            .iter()
            .map(|row| row.count_ones() as usize)
            .sum::<usize>();

        assert_eq!(
            vertices.len(),
            glyph_pixels * VERTICES_PER_RECT,
            "a default-background cell must keep the historical glyph vertex count"
        );
    }

    #[test]
    fn clamp_coordinates_keep_markers_inside_the_render_grid() {
        let terminal = snapshot(
            MAX_RENDER_ROWS + 1,
            MAX_RENDER_COLS + 1,
            b"\x1b[61;1HA\x1b[31;161HA\x1b[31;31HA",
        );
        let width = u32::from(MAX_RENDER_COLS + 1) * 10;
        let height = u32::from(MAX_RENDER_ROWS + 1) * 20;
        let vertices = frame_vertices(Some(&terminal), None, None, width, height, poc_metrics());

        let contains = |row: usize, col: usize| {
            let left = col as f32 * 10.0 / width as f32 * 2.0 - 1.0;
            let right = (col as f32 + 1.0) * 10.0 / width as f32 * 2.0 - 1.0;
            let top = 1.0 - row as f32 * 20.0 / height as f32 * 2.0;
            let bottom = 1.0 - (row as f32 + 1.0) * 20.0 / height as f32 * 2.0;
            vertices.iter().any(|vertex| {
                vertex.position[0] >= left
                    && vertex.position[0] < right
                    && vertex.position[1] <= top
                    && vertex.position[1] > bottom
            })
        };

        assert!(
            contains(29, 30) || contains(30, 30),
            "interior marker vanished"
        );
        assert!(!contains(usize::from(MAX_RENDER_ROWS), 0));
        assert!(!contains(29, usize::from(MAX_RENDER_COLS)));
    }

    #[test]
    fn ascii_glyphs_are_distinct_and_unknown_unicode_uses_replacement() {
        assert_ne!(glyph_rows('A'), glyph_rows('B'));
        assert_ne!(glyph_rows('a'), glyph_rows('A'));
        assert_eq!(glyph_rows('界'), UNICODE_REPLACEMENT_GLYPH);
        assert_eq!(glyph_rows('日'), UNICODE_REPLACEMENT_GLYPH);
        assert_ne!(glyph_rows('界'), glyph_rows('?'));
    }

    #[test]
    fn every_printable_ascii_glyph_is_pairwise_distinct() {
        let mut seen = std::collections::HashMap::new();

        for byte in b' '..=b'~' {
            let character = char::from(byte);
            let rows = glyph_rows(character);
            assert!(
                seen.insert(rows, character).is_none(),
                "printable ASCII glyph {character:?} collides with an earlier glyph"
            );
        }

        assert_eq!(seen.len(), 95, "printable ASCII must contain 95 glyphs");
    }

    /// Distinct covered characters may share a bitmap only where a 5×7 grid
    /// genuinely cannot tell them apart. Every acceptable pair is hardcoded
    /// below, so a future glyph edit that reintroduces a case-blind or
    /// diacritic-losing collision — or invents a new box-drawing alias —
    /// fails here instead of passing silently.
    #[test]
    fn covered_range_glyph_collisions_match_the_hardcoded_allowlist() {
        // A space and a non-breaking space are both blank cells.
        // At 5×7 an ASCII hyphen, slash, equals sign, backslash, and bar are
        // pixel-identical to their box-drawing equivalents, and a broken bar
        // matches the dashed U+254E column.
        let allowlist: [(char, char); 7] = [
            (' ', '\u{00a0}'),
            ('-', '\u{2500}'),
            ('/', '\u{2571}'),
            ('=', '\u{2550}'),
            ('\\', '\u{2572}'),
            ('|', '\u{2502}'),
            ('\u{00a6}', '\u{254e}'),
        ];

        let mut covered: Vec<char> = (0x20u8..=0x7e).map(char::from).collect();
        covered.extend((0x00a0..=0x00ff).filter_map(char::from_u32));
        covered.extend((0x2500..=0x257f).filter_map(char::from_u32));

        let mut collisions = Vec::new();
        for (index, &first) in covered.iter().enumerate() {
            for &second in &covered[index + 1..] {
                if glyph_rows(first) == glyph_rows(second) {
                    collisions.push((first, second));
                }
            }
        }

        assert_eq!(
            collisions, allowlist,
            "covered-range glyph collisions must equal the accepted list exactly"
        );
    }

    #[test]
    fn complete_latin1_supplement_is_reachable_without_unicode_fallback() {
        for scalar in 0x00a0..=0x00ff {
            let character = char::from_u32(scalar).expect("Latin-1 scalar is valid");
            let rows = latin1_rows(character)
                .unwrap_or_else(|| panic!("missing Latin-1 glyph U+{scalar:04X}"));
            assert_eq!(
                glyph_rows(character),
                rows,
                "U+{scalar:04X} is not reachable through the production lookup"
            );
            assert_ne!(
                rows, UNICODE_REPLACEMENT_GLYPH,
                "U+{scalar:04X} aliases the unsupported-Unicode fallback"
            );
            assert!(
                rows.iter().all(|row| *row <= 0b1_1111),
                "U+{scalar:04X} escapes the five-bit bitmap width"
            );
        }

        assert_eq!(glyph_rows('\u{00a0}'), glyph_rows(' '));
        assert_ne!(glyph_rows('É'), glyph_rows('E'));
        assert_ne!(glyph_rows('é'), glyph_rows('e'));
        assert_ne!(glyph_rows('ø'), glyph_rows('o'));
        assert!(
            latin1_rows('\u{0100}').is_none(),
            "coverage must stop at the documented U+00FF boundary"
        );
    }

    #[test]
    fn complete_box_drawing_block_avoids_the_question_mark_fallback() {
        for scalar in 0x2500..=0x257f {
            let character = char::from_u32(scalar).expect("box-drawing scalar is valid");
            let rows = box_drawing_rows(character)
                .unwrap_or_else(|| panic!("missing box-drawing glyph U+{scalar:04X}"));
            assert_eq!(
                glyph_rows(character),
                rows,
                "U+{scalar:04X} is not reachable through the production lookup"
            );
            assert_ne!(
                rows, QUESTION_MARK_GLYPH,
                "U+{scalar:04X} aliases the unsupported-character fallback"
            );
        }
        assert!(
            box_drawing_rows('\u{2580}').is_none(),
            "coverage must stop at the documented U+257F boundary"
        );
    }

    #[test]
    fn common_box_drawing_glyphs_preserve_topology_and_weight() {
        assert_eq!(glyph_rows('─'), [0, 0, 0, 31, 0, 0, 0]);
        assert_eq!(glyph_rows('│'), [4, 4, 4, 4, 4, 4, 4]);
        assert_eq!(glyph_rows('┌'), [0, 0, 0, 7, 4, 4, 4]);
        assert_eq!(glyph_rows('┼'), [4, 4, 4, 31, 4, 4, 4]);
        assert_eq!(glyph_rows('═'), [0, 0, 31, 0, 31, 0, 0]);
        assert_eq!(glyph_rows('║'), [10, 10, 10, 10, 10, 10, 10]);
        assert_ne!(glyph_rows('─'), glyph_rows('━'));
        assert_ne!(glyph_rows('│'), glyph_rows('┃'));
        assert_ne!(glyph_rows('┌'), glyph_rows('╭'));
        assert_ne!(glyph_rows('╱'), glyph_rows('╲'));
    }

    #[test]
    fn box_drawing_strokes_reach_cell_edges_through_the_vertex_path() {
        let metrics = poc_metrics();
        let width = metrics.width();
        let height = metrics.height();
        let vertices_for = |text: &str| {
            let terminal = snapshot(1, 1, text.as_bytes());
            frame_vertices(Some(&terminal), None, None, width, height, metrics)
        };

        let horizontal = vertices_for("─");
        assert!(
            horizontal.iter().any(|vertex| vertex.position[0] == -1.0)
                && horizontal.iter().any(|vertex| vertex.position[0] == 1.0),
            "horizontal frame stroke must span both cell edges"
        );

        let vertical = vertices_for("│");
        assert!(
            vertical.iter().any(|vertex| vertex.position[1] == 1.0)
                && vertical.iter().any(|vertex| vertex.position[1] == -1.0),
            "vertical frame stroke must span both cell edges"
        );
    }

    /// The encoded vertex stride must match what the pipeline's vertex buffer
    /// layout declares, or the GPU reads position and colour from the wrong
    /// offsets and every glyph is mispositioned or miscoloured.
    #[test]
    fn vertex_encoding_matches_the_declared_buffer_layout() {
        let terminal = snapshot(1, 2, b"A");
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
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
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        assert!(!vertices.is_empty(), "unstyled text emitted no vertices");
        assert!(
            vertices
                .iter()
                .all(|vertex| vertex.color == [0.80, 0.92, 0.82]),
            "unstyled cells must draw in the historical constant 0.80/0.92/0.82"
        );
        assert_eq!(DARK.foreground(), [0.80, 0.92, 0.82]);
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
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());

        // a occupies column 0, 日 columns 1-2, so b must start at display
        // column 3 and nothing may draw at column 2's lead edge.
        assert!(has_rect_top_left(&vertices, ndc_left(3)));
        assert!(!has_rect_top_left(&vertices, ndc_left(2)));
    }

    #[test]
    fn wide_output_renders_like_the_equivalent_single_width_layout() {
        let wide = snapshot(1, 6, "a日b".as_bytes());
        // U+FFFD is a narrow scalar that intentionally uses the same explicit
        // replacement bitmap as unsupported wide U+65E5. The intervening
        // space models the wide character's continuation column without
        // reviving the old (and now invalid) assumption that fallback is '?'.
        let aligned = snapshot(1, 6, "a\u{fffd} b".as_bytes());
        let m = poc_metrics();
        assert_eq!(
            frame_vertices(Some(&wide), None, None, 900, 600, m),
            frame_vertices(Some(&aligned), None, None, 900, 600, m),
            "the wide lead draws in column 1, its continuation column stays empty, and b lands in column 3"
        );
    }

    #[test]
    fn ascii_glyphs_keep_their_character_columns() {
        let terminal = snapshot(1, 4, b"BD");
        let vertices = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
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
        let small = frame_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        let big = GridGeometry::with_cells(20, 40)
            .expect("valid metrics")
            .cell_metrics();
        let big_verts = frame_vertices(Some(&terminal), None, None, 900, 600, big);

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
