//! Minimal, bounded `wgpu` terminal view for the PoC.
//!
//! Cell attributes recorded by the terminal state are resolved into drawing
//! colors before any vertex is emitted: [`resolve_cell_colors`] runs every
//! `Color` selection through the default palette (or passes `Rgb` through
//! directly) and applies reverse video by swapping the resolved foreground
//! and background. Bold widens glyph pixels by one physical pixel; underline
//! draws a bar across the cell bottom in the resolved underline color.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use wgpu::CurrentSurfaceTexture;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use noren_app::{
    MAX_RENDER_COLS, MAX_RENDER_ROWS, POC_CELL_HEIGHT as CELL_HEIGHT, POC_CELL_WIDTH as CELL_WIDTH,
};
use noren_terminal::{Cell, CellAttributes, Color, TerminalSnapshot};

const GLYPH_SCALE: u32 = 2;
const GLYPH_TOP: u32 = 3;
/// Distance in physical pixels from a cell's top edge to its underline bar.
const UNDERLINE_TOP: u32 = CELL_HEIGHT - GLYPH_SCALE;
/// A one-glyph cell emits at most 35 pixel rects (the 5×7 bitmap) plus one
/// background rect and one underline bar. Bold widens pixel rects without
/// adding any, so `35 + 2` rects bounds a normal cell; cells stacking more
/// combining marks than one extra glyph hit the vertex truncation below, as
/// they did before colors were wired.
const MAX_VERTICES: usize = (MAX_RENDER_ROWS as usize) * (MAX_RENDER_COLS as usize) * (35 + 2) * 6;
/// Two position floats plus three color floats per vertex.
const VERTEX_BYTES: usize = 20;

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@location(0) position: vec2<f32>, @location(1) color: vec3<f32>) -> VertexOutput {
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

/// Default terminal foreground: the concrete color behind `Color::Default`
/// foreground selections and the status line.
pub(crate) const DEFAULT_FOREGROUND: [u8; 3] = [204, 235, 209];
/// Default terminal background: the concrete color behind `Color::Default`
/// background selections and the window clear.
pub(crate) const DEFAULT_BACKGROUND: [u8; 3] = [9, 11, 10];

/// The sixteen ANSI colors of the default theme as `(red, green, blue)`, in
/// palette index order (standard colors 0..=7, bright colors 8..=15). This
/// named table is the single theme seam: a future theme replaces it here and
/// every palette-derived draw color follows.
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

/// One channel of the xterm 6×6×6 color cube: zero maps to zero, and the
/// remaining five levels are `55 + 40 * level`.
const fn cube_channel(level: u8) -> u8 {
    if level == 0 { 0 } else { level * 40 + 55 }
}

const fn build_default_palette() -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    let mut index = 0usize;
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

/// The xterm 256-color default palette derived once from
/// [`DEFAULT_ANSI_PALETTE`] (entries 0..=15), the 6×6×6 color cube
/// (entries 16..=231), and the 24-step grayscale ramp (entries 232..=255).
pub(crate) const DEFAULT_PALETTE: [[u8; 3]; 256] = build_default_palette();

/// Resolve one renderer-independent color selection to concrete
/// `(red, green, blue)` against the default palette.
///
/// [`Color::Default`] resolves to `default` (callers pass the default
/// foreground or background according to the slot), [`Color::Ansi`] and
/// [`Color::Indexed`] resolve through [`DEFAULT_PALETTE`], and [`Color::Rgb`]
/// is used directly.
#[must_use]
pub(crate) const fn resolve_color(color: Color, default: [u8; 3]) -> [u8; 3] {
    match color {
        Color::Default => default,
        Color::Ansi(ansi) => DEFAULT_PALETTE[ansi.palette_index() as usize],
        Color::Indexed(index) => DEFAULT_PALETTE[index as usize],
        Color::Rgb(red, green, blue) => [red, green, blue],
    }
}

/// Concrete draw colors for one cell, after palette resolution and the
/// reverse-video swap; the shape returned by [`resolve_cell_colors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedCellColors {
    /// The color drawn glyphs use, swapped with the background under reverse.
    pub(crate) foreground: [u8; 3],
    /// The color the cell's background area uses; cells whose resolved
    /// background equals [`DEFAULT_BACKGROUND`] draw no background rect.
    pub(crate) background: [u8; 3],
    /// The underline bar color: an explicit underline color passes through,
    /// and a default one follows the (possibly swapped) foreground.
    pub(crate) underline: [u8; 3],
}

/// Pure attribute-to-color resolution: the testable seam between terminal
/// state and drawing.
///
/// Foreground and background are resolved against the default palette first;
/// reverse video then swaps the two resolved colors at draw time. An explicit
/// underline color is applied as-is (reverse does not move it), while a
/// default underline color follows the swapped foreground so the bar matches
/// the glyphs it underlines.
#[must_use]
pub(crate) fn resolve_cell_colors(attributes: &CellAttributes) -> ResolvedCellColors {
    let mut foreground = resolve_color(attributes.foreground(), DEFAULT_FOREGROUND);
    let mut background = resolve_color(attributes.background(), DEFAULT_BACKGROUND);
    if attributes.is_reversed() {
        std::mem::swap(&mut foreground, &mut background);
    }
    let underline = resolve_color(attributes.underline_color(), foreground);
    ResolvedCellColors {
        foreground,
        background,
        underline,
    }
}

/// One GPU vertex: NDC position plus the cell-resolved draw color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Vertex {
    pub(crate) position: [f32; 2],
    pub(crate) color: [f32; 3],
}

impl Vertex {
    const fn rgb_floats([red, green, blue]: [u8; 3]) -> [f32; 3] {
        [
            red as f32 / 255.0,
            green as f32 / 255.0,
            blue as f32 / 255.0,
        ]
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
}

impl Renderer {
    pub(crate) fn new(window: Arc<Window>) -> Result<Self, RendererError> {
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
                    attributes: &[
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
                    ],
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
        status: Option<&str>,
    ) -> RenderOutcome {
        if self.device_lost.load(Ordering::Acquire) {
            return RenderOutcome::DeviceLost;
        }

        let vertices = glyph_vertices(terminal, status, self.config.width, self.config.height);
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
            let [clear_red, clear_green, clear_blue] = DEFAULT_BACKGROUND;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("noren-poc-render-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(clear_red) / 255.0,
                            g: f64::from(clear_green) / 255.0,
                            b: f64::from(clear_blue) / 255.0,
                            a: 1.0,
                        }),
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

/// Vertex accumulation bounded by [`MAX_VERTICES`] within one frame.
struct Frame {
    vertices: Vec<Vertex>,
    width: u32,
    height: u32,
}

impl Frame {
    /// Push one NDC rect and report whether the vertex budget is exhausted.
    fn push_rect(&mut self, x: u32, y: u32, rect: (u32, u32), color: [u8; 3]) -> bool {
        let (rect_width, rect_height) = rect;
        let color = Vertex::rgb_floats(color);
        let left = x as f32 / self.width as f32 * 2.0 - 1.0;
        let right = x.saturating_add(rect_width) as f32 / self.width as f32 * 2.0 - 1.0;
        let top = 1.0 - y as f32 / self.height as f32 * 2.0;
        let bottom = 1.0 - y.saturating_add(rect_height) as f32 / self.height as f32 * 2.0;
        self.vertices.extend_from_slice(&[
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
        self.vertices.len() >= MAX_VERTICES
    }

    /// Fill `span_cols` display columns of one cell row, offset `y_offset`
    /// pixels down from the cell top; used for background and underline bars.
    fn fill_cell(
        &mut self,
        col: usize,
        row: usize,
        span_cols: usize,
        y_offset: u32,
        rect_height: u32,
        color: [u8; 3],
    ) -> bool {
        self.push_rect(
            u32::try_from(col).unwrap_or(u32::MAX) * CELL_WIDTH,
            u32::try_from(row).unwrap_or(u32::MAX) * CELL_HEIGHT + y_offset,
            (
                u32::try_from(span_cols).unwrap_or(u32::MAX) * CELL_WIDTH,
                rect_height,
            ),
            color,
        )
    }

    /// One glyph bitmap at display column `col`; bold widens every pixel rect
    /// by one physical pixel. Returns whether the vertex budget is exhausted.
    fn glyph(
        &mut self,
        character: char,
        col: usize,
        row: usize,
        color: [u8; 3],
        bold: bool,
    ) -> bool {
        let pixel_width = GLYPH_SCALE + u32::from(bold);
        for (glyph_y, bits) in glyph_rows(character).into_iter().enumerate() {
            for glyph_x in 0..5 {
                if bits & (1 << (4 - glyph_x)) == 0 {
                    continue;
                }
                let exhausted = self.push_rect(
                    u32::try_from(col).unwrap_or(u32::MAX) * CELL_WIDTH
                        + u32::try_from(glyph_x).unwrap_or(0) * GLYPH_SCALE,
                    u32::try_from(row).unwrap_or(u32::MAX) * CELL_HEIGHT
                        + GLYPH_TOP
                        + u32::try_from(glyph_y).unwrap_or(0) * GLYPH_SCALE,
                    (pixel_width, GLYPH_SCALE),
                    color,
                );
                if exhausted {
                    return true;
                }
            }
        }
        false
    }
}

pub(crate) fn glyph_vertices(
    terminal: Option<&TerminalSnapshot>,
    status: Option<&str>,
    width: u32,
    height: u32,
) -> Vec<Vertex> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let visible_rows = usize::try_from(height / CELL_HEIGHT)
        .unwrap_or(usize::MAX)
        .clamp(1, usize::from(MAX_RENDER_ROWS));
    let visible_cols = usize::try_from(width / CELL_WIDTH)
        .unwrap_or(usize::MAX)
        .clamp(1, usize::from(MAX_RENDER_COLS));
    // display_cells is the cell-facing view of the snapshot, parallel to
    // display_lines: it selects exactly the rows display_lines renders, and
    // yields one cell per display column (a wide lead's continuation keeps
    // its own column and draws nothing). The renderer never re-derives the
    // column rule or reads `&[String]` line content — the walked cell is the
    // display column, with attributes attached.
    let rows: Vec<&[Cell]> = terminal
        .map(|snapshot| snapshot.display_cells().collect())
        .unwrap_or_default();
    let total_lines = rows.len() + usize::from(status.is_some());
    let first_line = total_lines.saturating_sub(visible_rows);
    let mut frame = Frame {
        vertices: Vec::new(),
        width,
        height,
    };

    for (row, line_index) in (first_line..total_lines).enumerate() {
        if line_index >= rows.len() {
            // Status line: plain text in the default foreground, no cell
            // attributes.
            for (col, character) in status
                .unwrap_or_default()
                .chars()
                .take(visible_cols)
                .enumerate()
            {
                if frame.glyph(character, col, row, DEFAULT_FOREGROUND, false) {
                    return frame.vertices;
                }
            }
            continue;
        }
        let row_cells = rows[line_index];

        // Walk cells, not string characters, so every glyph inherits the
        // attributes captured when its cell was written. Each cell in the
        // slice occupies exactly one display column; a wide lead's fills span
        // into its continuation cell's column, so the continuation itself
        // draws nothing.
        let mut col = 0usize;
        for cell in row_cells {
            if col >= visible_cols {
                break;
            }
            if cell.is_continuation() {
                col += 1;
                continue;
            }
            let attributes = cell.attributes();
            let colors = resolve_cell_colors(attributes);
            let span = usize::from(cell.width()).max(1);

            if colors.background != DEFAULT_BACKGROUND
                && frame.fill_cell(col, row, span, 0, CELL_HEIGHT, colors.background)
            {
                return frame.vertices;
            }
            let mut glyph_col = col;
            for character in cell.text().chars() {
                if glyph_col >= visible_cols {
                    break;
                }
                if frame.glyph(
                    character,
                    glyph_col,
                    row,
                    colors.foreground,
                    attributes.is_bold(),
                ) {
                    return frame.vertices;
                }
                glyph_col += 1;
            }
            if attributes.is_underlined()
                && frame.fill_cell(col, row, span, UNDERLINE_TOP, GLYPH_SCALE, colors.underline)
            {
                return frame.vertices;
            }
            // One cell, one display column — the wide lead's width-2 span is
            // covered by its own continuation cell further down the slice —
            // except that combining marks extend the cell's text exactly as
            // they extend `display_lines`, so later cells keep their
            // character-indexed columns.
            col += cell.text().chars().count().max(1);
        }
    }
    frame.vertices
}

pub(crate) fn vertex_bytes(vertices: &[Vertex]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len().saturating_mul(VERTEX_BYTES));
    for vertex in vertices {
        for value in vertex.position.iter().chain(vertex.color.iter()) {
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
    use noren_terminal::TerminalState;

    fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
        let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
        terminal.feed_bytes(bytes);
        terminal.snapshot()
    }

    #[test]
    fn glyph_input_is_bounded_to_visible_poc_grid() {
        let terminal = snapshot(100, 500, &vec![b'A'; 50_000]);
        let vertices = glyph_vertices(Some(&terminal), None, u32::MAX, u32::MAX);
        assert!(vertices.len() <= MAX_VERTICES);
    }

    #[test]
    fn empty_and_zero_sized_inputs_have_no_vertices() {
        let empty = snapshot(1, 1, b"");
        let text = snapshot(1, 8, b"text");
        assert!(glyph_vertices(Some(&empty), None, 900, 600).is_empty());
        assert!(glyph_vertices(Some(&text), None, 0, 600).is_empty());
    }

    #[test]
    fn ascii_glyphs_are_distinct_and_unknown_is_question_mark() {
        assert_ne!(glyph_rows('A'), glyph_rows('B'));
        assert_eq!(glyph_rows('a'), glyph_rows('A'));
        assert_eq!(glyph_rows('界'), glyph_rows('?'));
    }

    #[test]
    fn vertex_encoding_has_two_floats_and_three_color_floats_per_vertex() {
        let terminal = snapshot(1, 2, b"A");
        let vertices = glyph_vertices(Some(&terminal), None, 900, 600);
        assert_eq!(vertex_bytes(&vertices).len(), vertices.len() * VERTEX_BYTES);
    }

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
        let vertices = glyph_vertices(Some(&terminal), None, 900, 600);

        // a occupies column 0, 日 columns 1-2, so b must start at display
        // column 3 and nothing may draw at column 2's lead edge.
        assert!(has_rect_top_left(&vertices, ndc_left(3)));
        assert!(!has_rect_top_left(&vertices, ndc_left(2)));
    }

    #[test]
    fn wide_output_renders_like_the_equivalent_single_width_layout() {
        let wide = snapshot(1, 6, "a日b".as_bytes());
        let aligned = snapshot(1, 6, b"a? b");
        assert_eq!(
            glyph_vertices(Some(&wide), None, 900, 600),
            glyph_vertices(Some(&aligned), None, 900, 600),
            "the wide lead draws in column 1, its continuation column stays empty, and b lands in column 3"
        );
    }

    #[test]
    fn ascii_glyphs_keep_their_character_columns() {
        let terminal = snapshot(1, 4, b"BD");
        let vertices = glyph_vertices(Some(&terminal), None, 900, 600);
        assert!(has_rect_top_left(&vertices, ndc_left(0)));
        assert!(has_rect_top_left(&vertices, ndc_left(1)));
        assert!(!has_rect_top_left(&vertices, ndc_left(2)));
    }
}
