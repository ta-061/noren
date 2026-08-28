//! Offscreen rendered-frame capture for the FR-005 oracle.
//!
//! ## Why this exists
//!
//! `docs/requirements/v0.1.md` FR-005 requires that "PTY bytes update terminal
//! state and the resulting visible cells are drawn in the window", passing only
//! when "state snapshots **and a macOS rendered-frame capture** match their
//! oracles." The state-snapshot half already exists; this module supplies the
//! rendered-frame half: it drives the real `wgpu` pipeline offscreen, reads the
//! pixels back to the CPU, and hands them to the oracle test for structural
//! assertion.
//!
//! ## Faithfulness — same code path as the shipped binary
//!
//! A test that renders through a *different* path proves nothing about the real
//! renderer. This module therefore re-compiles the shipped renderer source and
//! draws with the **same** WGSL [`SHADER`](renderer_source::SHADER) and the
//! **same** [`glyph_vertices`](renderer_source::glyph_vertices) /
//! [`vertex_bytes`](renderer_source::vertex_bytes) the binary uses — it does not
//! keep a parallel copy that could silently drift. The only deliberate
//! divergence is the *target*: where the binary presents to a window surface,
//! this path renders to an offscreen `Rgba8Unorm` texture and copies it to a
//! readback buffer. That divergence cannot mask any vertex/glyph/grid defect
//! (which is exactly what the oracle checks); it only selects the output
//! encoding, so the oracle can reason about exact byte values.
//!
//! The `renderer_source` module is the whole `renderer.rs` re-included here via
//! `#[path]`; only `SHADER` / `glyph_vertices` / `vertex_bytes` are consumed, so
//! the rest (the window-bound `Renderer`) is dead weight in this context and
//! `#[allow(dead_code)]` silences it.
//!
//! ## No `unsafe`
//!
//! The workspace denies hand-written `unsafe`. Readback uses wgpu's safe
//! `map_async` + `get_mapped_range` API; the 256-byte
//! `COPY_BYTES_PER_ROW_ALIGNMENT` padding is un-padded row by row with safe
//! indexing.

// The re-included renderer source carries the window-bound `Renderer` and its
// error/outcome enums, which the oracle never drives here. Allow the weight.
#[allow(dead_code)]
#[path = "renderer.rs"]
pub(crate) mod renderer_source;

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;

use noren_app::sidebar_text::SidebarTextRow;
use noren_terminal::TerminalSnapshot;
use renderer_source::{
    FrameChrome, SHADER, Target, VERTEX_ATTRIBUTES, VERTEX_BYTES, Vertex, glyph_vertices_for,
    glyph_vertices_for_chrome, glyph_vertices_for_sidebar_rows, theme_clear_color, vertex_bytes,
};

/// Linear RGBA8 (non-sRGB) offscreen target.
///
/// A linear target lets the oracle compare bytes directly against the fragment
/// shader's constant `0.80/0.92/0.82` output and the `0.035/0.045/0.04` clear
/// colour, with no gamma curve in the way. The shipped binary presents to a
/// surface (sRGB or not, driver-chosen); encoding never changes *which* pixels
/// light up, so it does not affect any structural oracle assertion.
const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Why an offscreen capture could not be produced.
#[derive(Debug)]
pub(crate) enum CaptureError {
    /// No Metal adapter was available headlessly.
    AdapterUnavailable,
    /// A device/queue could not be requested from the adapter.
    DeviceUnavailable,
}

/// A captured RGBA8 frame plus its pixel width (the row stride `pixel` indexes
/// by). The frame's height is not stored: no oracle assertion reads it, and
/// `render()` sets it from the same `rows` the caller already knows, so a
/// stored copy would only invite the tautological `height == rows*CH` checks
/// the grid-dimension test was rewritten to avoid.
pub(crate) struct CapturedFrame {
    pub(crate) width: u32,
    pub(crate) rgba: Vec<u8>,
}

impl CapturedFrame {
    /// RGBA bytes for the pixel at `(x, y)`. Panics for out-of-range coords.
    #[must_use]
    pub(crate) fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let index = (usize::try_from(y).unwrap_or(0) * usize::try_from(self.width).unwrap_or(0)
            + usize::try_from(x).unwrap_or(0))
            * 4;
        [
            self.rgba[index],
            self.rgba[index + 1],
            self.rgba[index + 2],
            self.rgba[index + 3],
        ]
    }
}

/// Environment variable selecting how [`OffscreenRenderer::new`] experiences
/// adapter availability, so the oracle's adapter-absence behaviour can be
/// exercised (and asserted) on machines that have a working adapter.
///
/// Values:
///
/// - unset or `real` — the real machine state (`Backends::METAL`);
/// - `absent` — the backend set is emptied, so wgpu genuinely enumerates no
///   adapter and `new()` fails with the exact `AdapterUnavailable` an
///   adapter-less machine produces (the real wgpu path, not a fake branch);
/// - `device-fails` — `new()` returns `DeviceUnavailable` directly, standing
///   in for an adapter that exists but cannot produce a device.
///
/// Neither forced value can substitute a working device or fake a frame: they
/// only *remove* success paths, so they cannot make a broken renderer pass.
const ORACLE_ADAPTER_ENV: &str = "NOREN_FRAME_ORACLE_ADAPTER";

/// How [`OffscreenRenderer::new`] should experience adapter availability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OracleAdapterMode {
    /// The machine's real adapters.
    Real,
    /// No adapter at all: wgpu enumerates over an empty backend set.
    Absent,
    /// An adapter exists, but no device can be requested from it.
    DeviceFails,
}

/// The [`OracleAdapterMode`] the environment asks for (see [`ORACLE_ADAPTER_ENV`]).
fn oracle_adapter_mode() -> OracleAdapterMode {
    match std::env::var(ORACLE_ADAPTER_ENV).as_deref() {
        Ok("absent") => OracleAdapterMode::Absent,
        Ok("device-fails") => OracleAdapterMode::DeviceFails,
        _ => OracleAdapterMode::Real,
    }
}

/// The backend set the oracle may request adapters from for the current mode.
fn oracle_backends() -> wgpu::Backends {
    match oracle_adapter_mode() {
        OracleAdapterMode::Absent => wgpu::Backends::empty(),
        OracleAdapterMode::Real | OracleAdapterMode::DeviceFails => wgpu::Backends::METAL,
    }
}

/// Owns the offscreen device/queue and the glyph pipeline (built once).
///
/// Construction is the single point where headless wgpu initialisation can fail;
/// if it does, [`OffscreenRenderer::new`] returns the exact
/// [`CaptureError`] so the oracle can report `offscreen=blocked` honestly
/// instead of substituting a mock.
pub(crate) struct OffscreenRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
}

impl OffscreenRenderer {
    /// Create the Metal device and build the glyph pipeline from the real
    /// `SHADER`. Uses `Backends::METAL` and the same adapter options as the
    /// shipped renderer, but with `compatible_surface: None` so it needs no
    /// display.
    ///
    /// [`ORACLE_ADAPTER_ENV`] can force either headless failure mode so the
    /// oracle's skip policy is verifiable on machines that HAVE an adapter.
    pub(crate) fn new() -> Result<Self, CaptureError> {
        if oracle_adapter_mode() == OracleAdapterMode::DeviceFails {
            // wgpu has no knob for making `request_device` fail on a working
            // adapter, so this mode stands in for "an adapter exists but no
            // device can be had from it" directly.
            return Err(CaptureError::DeviceUnavailable);
        }
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        instance_descriptor.backends = oracle_backends();
        let instance = wgpu::Instance::new(instance_descriptor);
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|_| CaptureError::AdapterUnavailable)?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("noren-frame-oracle-device"),
            ..Default::default()
        }))
        .map_err(|_| CaptureError::DeviceUnavailable)?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("noren-frame-oracle-shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(SHADER)),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("noren-frame-oracle-pipeline-layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("noren-frame-oracle-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // The shipped renderer's own layout constants, not a parallel
                // copy: a stride or attribute-offset change in `renderer.rs`
                // reaches the oracle's pipeline automatically, so the oracle
                // cannot keep passing against a layout the binary no longer
                // uses.
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
                    format: OFFSCREEN_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
        })
    }

    /// Render the given terminal snapshot to an offscreen `target`-sized
    /// texture and return its RGBA pixels.
    ///
    /// The target should be exact multiples of the PoC cell size so the
    /// rendered grid is exactly the state grid. The pipeline, vertex
    /// generation, and clear colour are the shipped renderer's, all read from
    /// the target's [`Theme`] — the same value the binary's `Renderer` holds —
    /// so the oracle exercises the exact colour path configuration selects.
    ///
    /// [`Theme`]: noren_app::theme::Theme
    pub(crate) fn capture(
        &self,
        target: Target,
        terminal: Option<&TerminalSnapshot>,
        sidebar: Option<&[String]>,
        status: Option<&str>,
    ) -> CapturedFrame {
        let vertices = glyph_vertices_for(target, terminal, sidebar, status);
        self.capture_vertices(target, vertices)
    }

    /// Render sidebar rows whose domain kinds are retained by the production
    /// text projection, including the session-only lifecycle colour gate.
    pub(crate) fn capture_sidebar_rows(
        &self,
        target: Target,
        terminal: Option<&TerminalSnapshot>,
        sidebar: Option<&[SidebarTextRow]>,
        status: Option<&str>,
    ) -> CapturedFrame {
        let vertices = glyph_vertices_for_sidebar_rows(target, terminal, sidebar, status);
        self.capture_vertices(target, vertices)
    }

    /// Render through the same complete chrome seam as the live window.
    ///
    /// Palette-hint tests use this path so their assertions are on read-back
    /// pixels emitted by production vertex generation, not on configuration
    /// values or pre-rendered strings.
    pub(crate) fn capture_chrome(
        &self,
        target: Target,
        terminal: Option<&TerminalSnapshot>,
        chrome: FrameChrome<'_>,
    ) -> CapturedFrame {
        let vertices = glyph_vertices_for_chrome(target, terminal, chrome);
        self.capture_vertices(target, vertices)
    }

    fn capture_vertices(&self, target: Target, vertices: Vec<Vertex>) -> CapturedFrame {
        assert!(
            target.width > 0 && target.height > 0,
            "capture target must be non-zero"
        );

        let bytes = vertex_bytes(&vertices);
        let (width, height) = (target.width, target.height);

        let vertex_buffer = if bytes.is_empty() {
            None
        } else {
            let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("noren-frame-oracle-vertices"),
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&buffer, 0, &bytes);
            Some(buffer)
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("noren-frame-oracle-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let padded_rows = padded_bytes_per_row(width);
        let readback_size = u64::from(padded_rows) * u64::from(height);
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("noren-frame-oracle-readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("noren-frame-oracle-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("noren-frame-oracle-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(theme_clear_color(&target.theme)),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(buffer) = &vertex_buffer {
                pass.set_pipeline(&self.pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                pass.draw(0..u32::try_from(vertices.len()).unwrap_or(u32::MAX), 0..1);
            }
        }
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
                    bytes_per_row: Some(padded_rows),
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

        // Map and read back. The map callback fires during `poll(Wait)`; the
        // bounded `recv_timeout` guards against a silent hang if it ever does
        // not, surfacing the failure instead of stalling CI.
        let (sender, receiver) = std::sync::mpsc::channel::<Result<(), wgpu::BufferAsyncError>>();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("wgpu device poll failed during frame readback");
        let map_result = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("frame readback map callback never fired");
        map_result.expect("wgpu reported a buffer map failure");

        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .expect("wgpu get_mapped_range failed");
        let unpadded_row = usize::try_from(width).unwrap_or(0) * 4;
        let padded_row = usize::try_from(padded_rows).unwrap_or(0);
        let height_usize = usize::try_from(height).unwrap_or(0);
        let mut rgba = Vec::with_capacity(unpadded_row * height_usize);
        for row in 0..height_usize {
            let start = row * padded_row;
            rgba.extend_from_slice(&mapped[start..start + unpadded_row]);
        }
        drop(mapped);
        readback.unmap();

        CapturedFrame { width, rgba }
    }
}

/// Padded bytes-per-row wgpu requires for `copy_texture_to_buffer`.
///
/// wgpu mandates `COPY_BYTES_PER_ROW_ALIGNMENT` (256) alignment on the
/// destination row pitch; for an RGBA8 image the natural pitch is `width * 4`,
/// rounded up to the next multiple of 256. The caller un-pads each row.
fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded.div_ceil(alignment) * alignment
}

/// Block on a wgpu adapter/device request future from a synchronous caller.
///
/// Mirrors the shipped renderer's `block_on`: a thread-parking waker drives the
/// future on the current thread.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_row_rounds_width_up_to_256_bytes() {
        // 10px * 4 = 40 bytes -> 256.
        assert_eq!(padded_bytes_per_row(10), 256);
        // 90px * 4 = 360 bytes -> 512.
        assert_eq!(padded_bytes_per_row(90), 512);
        // 64px * 4 = 256 bytes -> already aligned.
        assert_eq!(padded_bytes_per_row(64), 256);
    }
}
