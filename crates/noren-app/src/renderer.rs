//! Minimal, bounded `wgpu` terminal view for the PoC.

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
use noren_terminal::TerminalSnapshot;
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
};

@vertex
fn vs_main(@location(0) position: vec2<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    return output;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.80, 0.92, 0.82, 1.0);
}
"#;

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
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
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
) -> Vec<[f32; 2]> {
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

    if let Some(lines) = sidebar {
        for (row, line) in lines.iter().take(visible_rows).enumerate() {
            for (col, character) in line.chars().take(SIDEBAR_COLS).enumerate() {
                push_glyph(&mut vertices, character, col, row, width, height, metrics);
                if vertices.len() >= MAX_VERTICES {
                    return vertices;
                }
            }
        }
    }

    // display_lines preserves wide-character continuation columns, so the
    // character index below is the display column for every glyph.
    let lines = terminal
        .map(TerminalSnapshot::display_lines)
        .unwrap_or_default();
    let total_lines = lines.len() + usize::from(status.is_some());
    let first_line = total_lines.saturating_sub(visible_rows);

    for (row, line_index) in (first_line..total_lines).enumerate() {
        let line = if line_index < lines.len() {
            lines[line_index].as_str()
        } else {
            status.unwrap_or_default()
        };
        for (col, character) in line.chars().take(terminal_cols).enumerate() {
            push_glyph(
                &mut vertices,
                character,
                col_offset + col,
                row,
                width,
                height,
                metrics,
            );
            if vertices.len() >= MAX_VERTICES {
                return vertices;
            }
        }
    }
    vertices
}

/// Emit the 5×7 bitmap glyph for `character` at grid cell `(col, row)`,
/// converting each lit pixel bit to a 2×2 rectangle of vertices.
fn push_glyph(
    vertices: &mut Vec<[f32; 2]>,
    character: char,
    col: usize,
    row: usize,
    width: u32,
    height: u32,
    metrics: CellMetrics,
) {
    let cell_width = metrics.width();
    let cell_height = metrics.height();
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
            push_rect(vertices, x, y, GLYPH_SCALE, width, height);
        }
    }
}

fn push_rect(vertices: &mut Vec<[f32; 2]>, x: u32, y: u32, size: u32, width: u32, height: u32) {
    let left = x as f32 / width as f32 * 2.0 - 1.0;
    let right = x.saturating_add(size) as f32 / width as f32 * 2.0 - 1.0;
    let top = 1.0 - y as f32 / height as f32 * 2.0;
    let bottom = 1.0 - y.saturating_add(size) as f32 / height as f32 * 2.0;
    vertices.extend_from_slice(&[
        [left, top],
        [left, bottom],
        [right, bottom],
        [left, top],
        [right, bottom],
        [right, top],
    ]);
}

/// Exposed as `pub(crate)` for the offscreen frame-oracle (see
/// `renderer_capture.rs`); no behaviour change.
pub(crate) fn vertex_bytes(vertices: &[[f32; 2]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vertices.len().saturating_mul(8));
    for vertex in vertices {
        bytes.extend_from_slice(&vertex[0].to_ne_bytes());
        bytes.extend_from_slice(&vertex[1].to_ne_bytes());
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

    #[test]
    fn glyph_input_is_bounded_to_visible_poc_grid() {
        // Grid dimensions one past the renderer limits exercise the same
        // dimension clamps as u32::MAX: visible_rows clamps to
        // MAX_RENDER_ROWS and terminal_cols clamps to MAX_RENDER_COLS.
        // No overflow path exists — all dimension arithmetic uses
        // saturating_add and clamp — so u32::MAX adds no coverage beyond
        // these values.
        let rows = MAX_RENDER_ROWS + 1;
        let cols = MAX_RENDER_COLS + 1;
        let bytes = vec![b'A'; usize::from(rows) * usize::from(cols)];
        let terminal = snapshot(rows, cols, &bytes);
        let width = u32::from(cols) * CELL_WIDTH + CELL_WIDTH;
        let height = u32::from(rows) * CELL_HEIGHT + CELL_HEIGHT;
        let vertices = glyph_vertices(Some(&terminal), None, None, width, height, poc_metrics());
        assert!(vertices.len() <= MAX_VERTICES);
    }

    #[test]
    fn empty_and_zero_sized_inputs_have_no_vertices() {
        let empty = snapshot(1, 1, b"");
        let text = snapshot(1, 8, b"text");
        assert!(glyph_vertices(Some(&empty), None, None, 900, 600, poc_metrics()).is_empty());
        assert!(glyph_vertices(Some(&text), None, None, 0, 600, poc_metrics()).is_empty());
    }

    #[test]
    fn ascii_glyphs_are_distinct_and_unknown_is_question_mark() {
        assert_ne!(glyph_rows('A'), glyph_rows('B'));
        assert_eq!(glyph_rows('a'), glyph_rows('A'));
        assert_eq!(glyph_rows('界'), glyph_rows('?'));
    }

    #[test]
    fn vertex_encoding_has_two_floats_per_vertex() {
        let terminal = snapshot(1, 2, b"A");
        let vertices = glyph_vertices(Some(&terminal), None, None, 900, 600, poc_metrics());
        assert_eq!(vertex_bytes(&vertices).len(), vertices.len() * 8);
    }

    const CELL_WIDTH: u32 = noren_app::POC_CELL_WIDTH;
    const CELL_HEIGHT: u32 = noren_app::POC_CELL_HEIGHT;

    fn ndc_left(column: u32) -> f32 {
        (column * CELL_WIDTH) as f32 / 900.0 * 2.0 - 1.0
    }

    fn ndc_top_row_zero() -> f32 {
        1.0 - GLYPH_TOP as f32 / 600.0 * 2.0
    }

    fn has_rect_top_left(vertices: &[[f32; 2]], left: f32) -> bool {
        let top = ndc_top_row_zero();
        vertices.chunks_exact(6).any(|rect| rect[0] == [left, top])
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
                .any(|rect| (rect[0][0] - small_edge_1).abs() < 1e-5),
            "at default metrics, column 1's left edge must be at 10px"
        );
        assert!(
            big_verts
                .chunks_exact(6)
                .any(|rect| (rect[0][0] - big_edge_1).abs() < 1e-5),
            "at cell_width=20, column 1's left edge must be at 20px, not 10px"
        );
        // And conversely, the big-vertices must NOT have an edge at the 10px
        // position — that is the shipped bug.
        assert!(
            !big_verts
                .chunks_exact(6)
                .any(|rect| (rect[0][0] - small_edge_1).abs() < 1e-5),
            "at cell_width=20, no vertex should land at the 10px column boundary"
        );
    }
}
