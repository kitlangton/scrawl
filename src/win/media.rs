//! Movies on Windows. The recorder captures with Windows.Graphics.Capture, crops and scales
//! each frame on the GPU (D3D11 video processor, which also converts to NV12) and encodes
//! H.264 into `recording.mp4` through a Media Foundation sink writer (the GPU's encoder when it
//! has one). Small BGRA copies made the same way are the keyframe candidates.
//!
//! Decoding (`frames`, `probe`, `export_audio`) goes through the Media Foundation source reader.

use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext,
    ID3D11Multithread, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView,
    ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFAttributes, IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader,
    MF_E_INVALIDSTREAMNUMBER, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AUDIO_BITS_PER_SAMPLE,
    MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE,
    MF_PD_DURATION, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_ALL_STREAMS,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READER_MEDIASOURCE,
    MF_SOURCE_READERF_ENDOFSTREAM, MF_VERSION, MFAudioFormat_Float, MFCreateAttributes,
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFCreateSinkWriterFromURL,
    MFCreateSourceReaderFromURL, MFMediaType_Audio, MFMediaType_Video, MFSTARTUP_FULL, MFStartup,
    MFVideoArea, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoFormat_RGB32,
    MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{GUID, HSTRING, Interface};

use crate::capture::{
    CANDIDATE_EDGE, CANDIDATE_EDGE_NO_VIDEO, FRAME_SAMPLE_INTERVAL, Sampler, capture_size,
    scrawl_log,
};
use crate::helper::{FrameTimes, Probe};
use crate::model::{CaptureMode, RgbaFrame, TargetInfo};

/// Recordings keep at most this many frames a second (screens only send frames on change).
const MAX_FPS: u32 = 30;
const FRAME_100NS: i64 = 10_000_000 / MAX_FPS as i64;

fn media_startup() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_FULL);
    });
    // Each thread that uses COM joins the multithreaded apartment.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// QueryPerformanceCounter in 100 ns units, the clock `SystemRelativeTime` uses.
fn qpc_100ns() -> i64 {
    let (mut now, mut freq) = (0_i64, 1_i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut now);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    (now as i128 * 10_000_000 / freq.max(1) as i128) as i64
}

// MARK: Recording

struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    winrt: IDirect3DDevice,
}

impl Gpu {
    fn new() -> Result<Self> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .context("D3D11CreateDevice")?;
        let device: ID3D11Device = device.context("no D3D11 device")?;
        let context: ID3D11DeviceContext = context.context("no D3D11 context")?;
        if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
            let _ = unsafe { multithread.SetMultithreadProtected(true) };
        }
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt: IDirect3DDevice =
            unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }?.cast()?;
        Ok(Self {
            video_device: device.cast()?,
            video_context: context.cast()?,
            device,
            context,
            winrt,
        })
    }

    fn texture(
        &self,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        staging: bool,
    ) -> Result<ID3D11Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: if staging {
                D3D11_USAGE_STAGING
            } else {
                D3D11_USAGE_DEFAULT
            },
            BindFlags: if staging {
                0
            } else {
                D3D11_BIND_RENDER_TARGET.0 as u32
            },
            CPUAccessFlags: if staging {
                D3D11_CPU_ACCESS_READ.0 as u32
            } else {
                0
            },
            MiscFlags: 0,
        };
        let mut texture = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
        texture.context("CreateTexture2D")
    }
}

/// Crops, scales and converts the latest frame into one output size and format.
struct Scaler {
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

impl Scaler {
    fn new(
        gpu: &Gpu,
        input: (u32, u32),
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
    ) -> Result<Self> {
        let rate = DXGI_RATIONAL {
            Numerator: MAX_FPS,
            Denominator: 1,
        };
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: input.0,
            InputHeight: input.1,
            OutputFrameRate: rate,
            OutputWidth: width,
            OutputHeight: height,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator = unsafe { gpu.video_device.CreateVideoProcessorEnumerator(&content) }?;
        let processor = unsafe { gpu.video_device.CreateVideoProcessor(&enumerator, 0) }?;
        let output = gpu.texture(width, height, format, false)?;
        let staging = gpu.texture(width, height, format, true)?;
        let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0::default(),
        };
        let mut output_view = None;
        unsafe {
            gpu.video_device.CreateVideoProcessorOutputView(
                &output,
                &enumerator,
                &desc,
                Some(&mut output_view),
            )
        }?;
        let vc = &gpu.video_context;
        unsafe {
            vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            // Input: full-range RGB. Output: BT.709 limited-range YCbCr for video, full-range
            // RGB for candidates.
            let input_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 };
            vc.VideoProcessorSetStreamColorSpace(&processor, 0, &input_space);
            let output_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: if format == DXGI_FORMAT_NV12 {
                    4 | (1 << 4)
                } else {
                    0
                },
            };
            vc.VideoProcessorSetOutputColorSpace(&processor, &output_space);
        }
        Ok(Self {
            enumerator,
            processor,
            output,
            output_view: output_view.context("output view")?,
            staging,
            width,
            height,
            format,
        })
    }

    /// Draws `source` (the `crop` of `input`) into the output, and copies it to the CPU.
    fn run(&self, gpu: &Gpu, input: &ID3D11Texture2D, crop: RECT) -> Result<Vec<u8>> {
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0::default(),
        };
        let mut input_view: Option<ID3D11VideoProcessorInputView> = None;
        unsafe {
            gpu.video_device.CreateVideoProcessorInputView(
                input,
                &self.enumerator,
                &desc,
                Some(&mut input_view),
            )
        }?;
        let out_rect = RECT {
            left: 0,
            top: 0,
            right: self.width as i32,
            bottom: self.height as i32,
        };
        let vc = &gpu.video_context;
        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(input_view),
            ..Default::default()
        };
        unsafe {
            vc.VideoProcessorSetStreamSourceRect(&self.processor, 0, true, Some(&crop));
            vc.VideoProcessorSetStreamDestRect(&self.processor, 0, true, Some(&out_rect));
            vc.VideoProcessorSetOutputTargetRect(&self.processor, true, Some(&out_rect));
        }
        let streams = [stream];
        let result =
            unsafe { vc.VideoProcessorBlt(&self.processor, &self.output_view, 0, &streams) };
        let [stream] = streams;
        drop(ManuallyDrop::into_inner(stream.pInputSurface));
        result?;
        unsafe { gpu.context.CopyResource(&self.staging, &self.output) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            gpu.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }?;
        let pitch = mapped.RowPitch as usize;
        let base = mapped.pData.cast::<u8>();
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out;
        if self.format == DXGI_FORMAT_NV12 {
            // Y rows, then interleaved UV rows (half height), each `pitch` apart.
            out = Vec::with_capacity(w * h * 3 / 2);
            for row in 0..h + h / 2 {
                let line = unsafe { std::slice::from_raw_parts(base.add(row * pitch), w) };
                out.extend_from_slice(line);
            }
        } else {
            out = Vec::with_capacity(w * h * 4);
            for row in 0..h {
                let line = unsafe { std::slice::from_raw_parts(base.add(row * pitch), w * 4) };
                for px in line.chunks_exact(4) {
                    out.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
        }
        unsafe { gpu.context.Unmap(&self.staging, 0) };
        Ok(out)
    }
}

struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    first: Option<i64>,
    last: Option<i64>,
}

impl Encoder {
    fn new(path: &Path, width: u32, height: u32) -> Result<Self> {
        let mut attributes: Option<IMFAttributes> = None;
        unsafe { MFCreateAttributes(&mut attributes, 2) }?;
        let attributes = attributes.context("attributes")?;
        unsafe { attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1) }?;
        let writer = unsafe {
            MFCreateSinkWriterFromURL(&HSTRING::from(path.as_os_str()), None, &attributes)
        }?;
        let bitrate = (f64::from(width) * f64::from(height) * f64::from(MAX_FPS) * 0.08) as u32;
        let output = video_type(&MFVideoFormat_H264, width, height)?;
        unsafe { output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate.clamp(2_000_000, 16_000_000)) }?;
        let stream = unsafe { writer.AddStream(&output) }?;
        let input = video_type(&MFVideoFormat_NV12, width, height)?;
        unsafe {
            input.SetUINT32(&MF_MT_DEFAULT_STRIDE, width)?;
            input.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
            writer.SetInputMediaType(stream, &input, None)?;
            writer.BeginWriting()?;
        }
        Ok(Self {
            writer,
            stream,
            first: None,
            last: None,
        })
    }

    /// Writes one NV12 frame captured at `time` (100 ns, QPC clock).
    fn write(&mut self, nv12: &[u8], time: i64) -> Result<()> {
        let first = *self.first.get_or_insert(time);
        let mut t = time - first;
        if let Some(last) = self.last
            && t <= last
        {
            t = last + 1;
        }
        self.last = Some(t);
        let buffer = unsafe { MFCreateMemoryBuffer(nv12.len() as u32) }?;
        unsafe {
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(nv12.len() as u32)?;
        }
        let sample: IMFSample = unsafe { MFCreateSample() }?;
        unsafe {
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(t)?;
            sample.SetSampleDuration(FRAME_100NS)?;
            self.writer.WriteSample(self.stream, &sample)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        unsafe { self.writer.Finalize() }?;
        Ok(())
    }
}

fn video_type(subtype: &GUID, width: u32, height: u32) -> Result<IMFMediaType> {
    let media = unsafe { MFCreateMediaType() }?;
    unsafe {
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, subtype)?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        media.SetUINT64(&MF_MT_FRAME_RATE, pack(MAX_FPS, 1))?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
    }
    Ok(media)
}

/// The capture item and the pixels of it to record.
struct Source {
    item: GraphicsCaptureItem,
    /// `[x, y, w, h]` of the target inside the item's frames (the whole frame for a window).
    crop: Option<[i32; 4]>,
}

fn capture_source(target: &TargetInfo) -> Result<Source> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    if target.mode == CaptureMode::Window
        && let Some(id) = target.window_id
    {
        let hwnd = super::hwnd(id as i32 as isize);
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd) }?;
        return Ok(Source { item, crop: None });
    }
    let f = target.frame;
    let monitor = super::monitor_at(f.x + f.width / 2.0, f.y + f.height / 2.0)
        .context("No display under the target")?;
    let handle = windows::Win32::Graphics::Gdi::HMONITOR(monitor.handle as *mut _);
    let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(handle) }?;
    let [mx, my, mw, mh] = monitor.rect;
    let crop = [
        (f.x - mx).round().max(0.0) as i32,
        (f.y - my).round().max(0.0) as i32,
        f.width.round().min(mw) as i32,
        f.height.round().min(mh) as i32,
    ];
    Ok(Source {
        item,
        crop: Some(crop),
    })
}

struct Recorder {
    gpu: Gpu,
    pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    source: Source,
    pool_size: SizeInt32,
    /// The newest frame, copied out of the frame pool (which recycles its textures).
    latest: Option<(ID3D11Texture2D, (u32, u32), i64)>,
    encoded: Option<i64>,
    video: Option<(Scaler, Encoder)>,
    candidates: Scaler,
    out: (u32, u32),
}

impl Recorder {
    fn new(target: &TargetInfo, video_path: Option<&Path>, long_edge: f64) -> Result<Self> {
        let gpu = Gpu::new()?;
        let source = capture_source(target)?;
        let pool_size = source.item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &gpu.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            pool_size,
        )?;
        let session = pool.CreateCaptureSession(&source.item)?;
        let _ = session.SetIsCursorCaptureEnabled(true);
        // Windows 11 can drop the yellow capture border.
        let _ = session.SetIsBorderRequired(false);
        let input = (
            pool_size.Width.max(1) as u32,
            pool_size.Height.max(1) as u32,
        );
        // H.264 encoders and decoders work in 16 px macroblocks and pad rows to match; a size
        // off that grid comes back sheared.
        let (w, h) = capture_size(target, long_edge);
        let out = ((w & !15).max(16), (h & !15).max(16));
        let video = match video_path {
            Some(path) => {
                let scaler = Scaler::new(&gpu, input, out.0, out.1, DXGI_FORMAT_NV12)?;
                Some((scaler, Encoder::new(path, out.0, out.1)?))
            }
            None => None,
        };
        let cand = capture_size(
            target,
            if video.is_some() {
                CANDIDATE_EDGE
            } else {
                CANDIDATE_EDGE_NO_VIDEO
            },
        );
        let candidates = Scaler::new(&gpu, input, cand.0, cand.1, DXGI_FORMAT_B8G8R8A8_UNORM)?;
        session.StartCapture()?;
        Ok(Self {
            gpu,
            pool,
            _session: session,
            source,
            pool_size,
            latest: None,
            encoded: None,
            video,
            candidates,
            out,
        })
    }

    fn crop(&self, size: (u32, u32)) -> RECT {
        let [x, y, w, h] = self
            .source
            .crop
            .unwrap_or([0, 0, size.0 as i32, size.1 as i32]);
        RECT {
            left: x.clamp(0, size.0 as i32),
            top: y.clamp(0, size.1 as i32),
            right: (x + w).clamp(1, size.0 as i32),
            bottom: (y + h).clamp(1, size.1 as i32),
        }
    }

    /// Takes every waiting frame, keeping the newest. Returns its capture time if one arrived.
    fn pump(&mut self) -> Result<Option<i64>> {
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            newest = Some(frame);
        }
        let Some(frame) = newest else {
            return Ok(None);
        };
        let content = frame.ContentSize()?;
        let time = frame.SystemRelativeTime()?.Duration;
        let surface = frame.Surface()?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let size = (
            (content.Width.max(1) as u32).min(desc.Width),
            (content.Height.max(1) as u32).min(desc.Height),
        );
        let reuse = self
            .latest
            .as_ref()
            .filter(|(_, s, _)| *s == (desc.Width, desc.Height))
            .map(|(t, _, _)| t.clone());
        let copy = match reuse {
            Some(copy) => copy,
            None => self
                .gpu
                .texture(desc.Width, desc.Height, DXGI_FORMAT_B8G8R8A8_UNORM, false)?,
        };
        unsafe { self.gpu.context.CopyResource(&copy, &texture) };
        // Only the frame's content area is the window (a resized window leaves padding).
        self.latest = Some((copy, size, time));
        drop(frame);
        if content.Width != self.pool_size.Width || content.Height != self.pool_size.Height {
            self.pool_size = content;
            self.pool.Recreate(
                &self.gpu.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                content,
            )?;
        }
        Ok(Some(time))
    }

    /// Encodes the newest frame if it is new and the frame rate allows.
    fn encode(&mut self, force_at: Option<i64>) -> Result<()> {
        let Some((texture, size, time)) = self.latest.clone() else {
            return Ok(());
        };
        let time = force_at.unwrap_or(time);
        if force_at.is_none()
            && self
                .encoded
                .is_some_and(|last| time <= last || time - last < FRAME_100NS)
        {
            return Ok(());
        }
        let crop = self.crop(size);
        let Some((scaler, encoder)) = self.video.as_mut() else {
            return Ok(());
        };
        let nv12 = scaler.run(&self.gpu, &texture, crop)?;
        encoder.write(&nv12, time)?;
        self.encoded = Some(time);
        Ok(())
    }

    fn candidate(&self, t_ms: u64) -> Result<Option<RgbaFrame>> {
        let Some((texture, size, _)) = self.latest.as_ref() else {
            return Ok(None);
        };
        let rgba = self.candidates.run(&self.gpu, texture, self.crop(*size))?;
        Ok(Some(RgbaFrame::new(
            t_ms,
            self.candidates.width,
            self.candidates.height,
            rgba,
        )))
    }
}

/// A movie of solid colours, each `(ms, rgb)` shown until the next, through the recorder's
/// encoder. `width` and `height` must be multiples of 16.
#[cfg(test)]
pub fn write_test_movie(
    path: &Path,
    width: u32,
    height: u32,
    frames: &[(u64, [u8; 3])],
) -> Result<()> {
    media_startup();
    let mut encoder = Encoder::new(path, width, height)?;
    let pixels = (width * height) as usize;
    for &(ms, [r, g, b]) in frames {
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        let y = (16.0 + 0.257 * r + 0.504 * g + 0.098 * b) as u8;
        let u = (128.0 - 0.148 * r - 0.291 * g + 0.439 * b) as u8;
        let v = (128.0 + 0.439 * r - 0.368 * g - 0.071 * b) as u8;
        let mut nv12 = vec![y; pixels];
        nv12.extend([u, v].repeat(pixels / 4));
        encoder.write(&nv12, ms as i64 * 10_000)?;
    }
    encoder.finish()
}

/// The recording thread: see `crate::capture::Sampler`.
pub fn run_sampler(sampler: Sampler) -> Option<PathBuf> {
    media_startup();
    let Sampler {
        target,
        video_output_path,
        started_at,
        stopping,
        urgent,
        frames,
        video_offset_ms,
        ..
    } = sampler;
    let long_edge = if target.mode == CaptureMode::Display {
        2560.0
    } else {
        1280.0
    };
    let mut recorder = match Recorder::new(&target, video_output_path.as_deref(), long_edge) {
        Ok(recorder) => Some(recorder),
        Err(error) => {
            scrawl_log(format!("Screen capture failed to start: {error:#}"));
            None
        }
    };
    if let Some(ref r) = recorder {
        scrawl_log(format!(
            "Recording {}x{} ({:?})",
            r.out.0, r.out.1, target.mode
        ));
    }
    crate::ui::start_metric("sck_ready_ms", started_at);
    let mut last_sample = Instant::now()
        .checked_sub(FRAME_SAMPLE_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut phase = 0_u32;
    let mut failed = false;
    while !stopping.load(Ordering::Acquire) {
        if let Some(r) = recorder.as_mut()
            && !failed
        {
            let step = r.pump().and_then(|arrived| {
                if let Some(time) = arrived
                    && r.encoded.is_none()
                {
                    // The movie starts at its first frame; place that on the capture clock.
                    let lag_ms = (qpc_100ns() - time).max(0) / 10_000;
                    let offset = (started_at.elapsed().as_millis() as i64 - lag_ms).max(0);
                    video_offset_ms.store(offset as u64, Ordering::Release);
                }
                r.encode(None)
            });
            if let Err(error) = step {
                scrawl_log(format!("Recording frame failed: {error:#}"));
                failed = true;
            }
        }
        let is_urgent = urgent.swap(false, Ordering::AcqRel);
        let interval = frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .interval();
        if is_urgent || last_sample.elapsed() >= interval {
            last_sample = Instant::now();
            let t_ms = started_at.elapsed().as_millis() as u64;
            if is_urgent {
                phase = phase.saturating_add(1);
            }
            let frame = recorder
                .as_ref()
                .and_then(|r| r.candidate(t_ms).ok().flatten())
                .unwrap_or_else(|| {
                    crate::render::render_synthetic_demo_frame(1280, 800, t_ms, phase)
                });
            frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(frame, is_urgent);
        }
        std::thread::park_timeout(Duration::from_millis(8));
    }
    let recorder = recorder?;
    let mut recorder = recorder;
    // Hold the last frame until the end, so the movie lasts as long as the capture.
    let _ = recorder.pump();
    let _ = recorder.encode(Some(qpc_100ns()));
    let Recorder { video, .. } = recorder;
    let (_, encoder) = video?;
    encoder.first?;
    match encoder.finish() {
        Ok(()) => video_output_path,
        Err(error) => {
            scrawl_log(format!("Finishing the recording failed: {error:#}"));
            None
        }
    }
}

// MARK: Reading movies

fn reader(movie: &Path, video_processing: bool) -> Result<IMFSourceReader> {
    media_startup();
    let mut attributes: Option<IMFAttributes> = None;
    unsafe { MFCreateAttributes(&mut attributes, 1) }?;
    let attributes = attributes.context("attributes")?;
    if video_processing {
        unsafe { attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1) }?;
    }
    unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(movie.as_os_str()), &attributes) }
        .with_context(|| format!("Could not open {}", movie.display()))
}

fn duration_100ns(reader: &IMFSourceReader) -> u64 {
    unsafe {
        reader.GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
    }
    .ok()
    .and_then(|value: PROPVARIANT| u64::try_from(&value).ok())
    .unwrap_or(0)
}

pub fn probe(movie: &Path) -> Result<Probe> {
    let reader = reader(movie, false)?;
    let duration_ms = duration_100ns(&reader) / 10_000;
    let (mut width, mut height, mut fps, mut audio_tracks) = (0, 0, 0.0, 0);
    for stream in 0.. {
        let media = match unsafe { reader.GetNativeMediaType(stream, 0) } {
            Ok(media) => media,
            Err(error) if error.code() == MF_E_INVALIDSTREAMNUMBER => break,
            Err(_) => break,
        };
        let major = unsafe { media.GetGUID(&MF_MT_MAJOR_TYPE) }.unwrap_or_default();
        if major == MFMediaType_Audio {
            audio_tracks += 1;
        } else if major == MFMediaType_Video && width == 0 {
            if let Ok(size) = unsafe { media.GetUINT64(&MF_MT_FRAME_SIZE) } {
                (width, height) = unpack(size);
            }
            if let Ok(rate) = unsafe { media.GetUINT64(&MF_MT_FRAME_RATE) } {
                let (n, d) = unpack(rate);
                fps = f64::from(n) / f64::from(d.max(1));
            }
        }
    }
    Ok(Probe {
        duration_ms,
        width,
        height,
        audio_tracks,
        fps,
    })
}

/// A decoded video frame as RGBA, and its time in ms.
struct Decoded {
    t_ms: u64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

struct VideoReader {
    reader: IMFSourceReader,
    width: u32,
    height: u32,
    /// The visible area inside each (possibly padded) decoded frame.
    aperture: (u32, u32, u32, u32),
    duration_ms: u64,
}

impl VideoReader {
    fn open(movie: &Path) -> Result<Self> {
        let reader = reader(movie, true)?;
        let stream = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        unsafe {
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(stream, true)?;
        }
        let wanted = unsafe { MFCreateMediaType() }?;
        unsafe {
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(stream, None, &wanted)?;
        }
        let current = unsafe { reader.GetCurrentMediaType(stream) }?;
        let (width, height) = unpack(unsafe { current.GetUINT64(&MF_MT_FRAME_SIZE) }?);
        let mut area = MFVideoArea::default();
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                (&raw mut area).cast::<u8>(),
                std::mem::size_of::<MFVideoArea>(),
            )
        };
        let aperture = if unsafe { current.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, bytes, None) }
            .is_ok()
            && area.Area.cx > 0
        {
            (
                area.OffsetX.value.max(0) as u32,
                area.OffsetY.value.max(0) as u32,
                area.Area.cx as u32,
                area.Area.cy as u32,
            )
        } else {
            (0, 0, width, height)
        };
        Ok(Self {
            duration_ms: duration_100ns(&reader) / 10_000,
            reader,
            width,
            height,
            aperture,
        })
    }

    fn seek(&self, ms: u64) -> Result<()> {
        let position = PROPVARIANT::from(ms as i64 * 10_000);
        unsafe { self.reader.SetCurrentPosition(&GUID::zeroed(), &position) }?;
        Ok(())
    }

    /// The next decoded frame, or `None` at the end.
    fn next(&self) -> Result<Option<(u64, IMFSample)>> {
        let mut flags = 0_u32;
        let mut time = 0_i64;
        let mut sample = None;
        unsafe {
            self.reader.ReadSample(
                MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )
        }?;
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            return Ok(None);
        }
        match sample {
            Some(sample) => Ok(Some(((time.max(0) / 10_000) as u64, sample))),
            None => self.next(),
        }
    }

    fn convert(&self, t_ms: u64, sample: &IMFSample, max_edge: u32) -> Result<Decoded> {
        let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
        let (ax, ay, aw, ah) = self.aperture;
        let mut rgba = vec![0_u8; aw as usize * ah as usize * 4];
        let mut copy = |base: *const u8, pitch: isize| {
            for y in 0..ah as usize {
                let row =
                    unsafe { base.offset((y as isize + ay as isize) * pitch + ax as isize * 4) };
                let src = unsafe { std::slice::from_raw_parts(row, aw as usize * 4) };
                let dst = &mut rgba[y * aw as usize * 4..][..aw as usize * 4];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                }
            }
        };
        if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
            let mut scan0 = std::ptr::null_mut();
            let mut pitch = 0_i32;
            unsafe { two_d.Lock2D(&mut scan0, &mut pitch) }?;
            copy(scan0, pitch as isize);
            unsafe { two_d.Unlock2D() }?;
        } else {
            let mut data = std::ptr::null_mut();
            let mut len = 0_u32;
            unsafe { buffer.Lock(&mut data, None, Some(&mut len)) }?;
            ensure!(
                len as usize >= self.width as usize * self.height as usize * 4,
                "Short video frame"
            );
            copy(data, self.width as isize * 4);
            unsafe { buffer.Unlock() }?;
        }
        Ok(shrink_to_edge(
            Decoded {
                t_ms,
                width: aw,
                height: ah,
                rgba,
            },
            max_edge,
        ))
    }

    /// The frame on screen at each time (the last one at or before it, give or take 40 ms),
    /// for times in increasing order. Returns `(requested, frame)`.
    fn at(&self, times: &[u64], max_edge: u32) -> Result<Vec<(u64, Decoded)>> {
        let mut out = Vec::with_capacity(times.len());
        on_screen_at(
            times,
            self.duration_ms,
            |t| self.seek(t),
            || self.next(),
            |want, t, sample| {
                out.push((want, self.convert(t, sample, max_edge)?));
                Ok(())
            },
        )?;
        Ok(out)
    }
}

/// Walks a movie's samples (`next`, in order; `seek` jumps to the key frame before a time) and
/// hands `emit` the one on screen at each requested time (in increasing order): the last at or
/// before it, give or take 40 ms. A time past the end gets the last sample, and is still
/// answered as the time asked for, so callers can match it.
fn on_screen_at<S>(
    times: &[u64],
    duration_ms: u64,
    mut seek: impl FnMut(u64) -> Result<()>,
    mut next: impl FnMut() -> Result<Option<(u64, S)>>,
    mut emit: impl FnMut(u64, u64, &S) -> Result<()>,
) -> Result<()> {
    let mut previous: Option<(u64, S)> = None;
    let mut pending: Option<(u64, S)> = None;
    let mut position: Option<u64> = None;
    for &want in times {
        let target = want.min(duration_ms.saturating_sub(1));
        // Far ahead (or behind): seek, landing on the key frame before it.
        if position.is_none_or(|p| target + 40 < p || target > p + 2_000) {
            seek(target.saturating_sub(40))?;
            previous = None;
            pending = None;
        }
        loop {
            let sample = match pending.take() {
                Some(sample) => Some(sample),
                None => next()?,
            };
            let Some((t, sample)) = sample else {
                break;
            };
            position = Some(t);
            if t > target + 40 {
                if previous.is_none() {
                    previous = Some((t, sample));
                } else {
                    pending = Some((t, sample));
                }
                break;
            }
            previous = Some((t, sample));
        }
        if let Some((t, sample)) = previous.as_ref() {
            emit(want, *t, sample)?;
        }
    }
    Ok(())
}

/// Box-filters a frame down so its long edge is at most `max_edge` (0 keeps it).
fn shrink_to_edge(frame: Decoded, max_edge: u32) -> Decoded {
    let long = frame.width.max(frame.height);
    if max_edge == 0 || long <= max_edge {
        return frame;
    }
    let scale = f64::from(max_edge) / f64::from(long);
    let w = ((f64::from(frame.width) * scale).round() as u32).max(1);
    let h = ((f64::from(frame.height) * scale).round() as u32).max(1);
    let (sw, sh) = (frame.width as usize, frame.height as usize);
    let mut out = vec![0_u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        let y0 = y * sh / h as usize;
        let y1 = ((y + 1) * sh / h as usize).max(y0 + 1);
        for x in 0..w as usize {
            let x0 = x * sw / w as usize;
            let x1 = ((x + 1) * sw / w as usize).max(x0 + 1);
            let mut acc = [0_u32; 3];
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            for sy in y0..y1 {
                let row = &frame.rgba[(sy * sw + x0) * 4..(sy * sw + x1) * 4];
                for px in row.chunks_exact(4) {
                    acc[0] += u32::from(px[0]);
                    acc[1] += u32::from(px[1]);
                    acc[2] += u32::from(px[2]);
                }
            }
            let o = &mut out[(y * w as usize + x) * 4..][..4];
            o.copy_from_slice(&[
                (acc[0] / n) as u8,
                (acc[1] / n) as u8,
                (acc[2] / n) as u8,
                255,
            ]);
        }
    }
    Decoded {
        t_ms: frame.t_ms,
        width: w,
        height: h,
        rgba: out,
    }
}

pub fn frames(movie: &Path, times: FrameTimes, max_edge: u32) -> Result<Vec<RgbaFrame>> {
    let video = VideoReader::open(movie)?;
    let (requested, sampled): (Vec<u64>, bool) = match times {
        FrameTimes::At(times) => {
            if times.is_empty() {
                return Ok(Vec::new());
            }
            (times.to_vec(), false)
        }
        FrameTimes::Every(ms) => (
            (0..video.duration_ms.max(1))
                .step_by(ms.max(1) as usize)
                .collect(),
            true,
        ),
    };
    let mut order: Vec<usize> = (0..requested.len()).collect();
    order.sort_by_key(|&i| requested[i]);
    let sorted: Vec<u64> = order.iter().map(|&i| requested[i]).collect();
    let decoded = video.at(&sorted, max_edge)?;
    ensure!(
        !decoded.is_empty(),
        "No frames decoded from {}",
        movie.display()
    );
    let mut out: Vec<RgbaFrame> = Vec::with_capacity(decoded.len());
    for (want, frame) in decoded {
        let t = if sampled { frame.t_ms } else { want };
        if sampled && out.last().is_some_and(|f| f.t_ms == t) {
            continue;
        }
        out.push(RgbaFrame::new(t, frame.width, frame.height, frame.rgba));
    }
    if !sampled {
        // Back in the order asked for.
        let mut by_time: Vec<Option<RgbaFrame>> = out.into_iter().map(Some).collect();
        let mut reordered = Vec::with_capacity(by_time.len());
        for &t in &requested {
            if let Some(slot) = by_time
                .iter_mut()
                .find(|f| f.as_ref().is_some_and(|f| f.t_ms == t))
            {
                reordered.push(slot.take().unwrap_or_else(|| unreachable!()));
            }
        }
        return Ok(reordered);
    }
    Ok(out)
}

pub fn shrink(_movie: &Path, _out: &Path, _max_edge: u32) -> Result<crate::retention::Shrunk> {
    bail!("Only cabled-iPhone recordings (macOS) are re-encoded")
}

/// The movie's first audio track as 16 kHz mono WAV, starting at the movie's start.
pub fn export_audio(movie: &Path, wav: &Path) -> Result<()> {
    let reader = reader(movie, false)?;
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    unsafe {
        reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
        reader.SetStreamSelection(stream, true)?;
    }
    let wanted = unsafe { MFCreateMediaType() }?;
    unsafe {
        wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        wanted.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_Float)?;
        reader.SetCurrentMediaType(stream, None, &wanted)?;
    }
    let current = unsafe { reader.GetCurrentMediaType(stream) }?;
    let rate = unsafe { current.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND) }?.max(1);
    let channels = unsafe { current.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS) }?.max(1) as usize;
    ensure!(
        unsafe { current.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }.unwrap_or(32) == 32,
        "Unexpected audio format"
    );
    let mut mono: Vec<f32> = Vec::new();
    loop {
        let mut flags = 0_u32;
        let mut time = 0_i64;
        let mut sample = None;
        unsafe {
            reader.ReadSample(
                stream,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )
        }?;
        if let Some(sample) = sample {
            if mono.is_empty() && time > 0 {
                // Silence before the first sound, so audio time is movie time.
                mono.resize((time as u64 * u64::from(rate) / 10_000_000) as usize, 0.0);
            }
            let buffer = unsafe { sample.ConvertToContiguousBuffer() }?;
            let mut data = std::ptr::null_mut();
            let mut len = 0_u32;
            unsafe { buffer.Lock(&mut data, None, Some(&mut len)) }?;
            let floats =
                unsafe { std::slice::from_raw_parts(data.cast::<f32>(), len as usize / 4) };
            mono.extend(
                floats
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().sum::<f32>() / channels as f32),
            );
            unsafe { buffer.Unlock() }?;
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            break;
        }
    }
    // Area-average down (or hold up) to 16 kHz.
    let out_len = (mono.len() as u64 * 16_000 / u64::from(rate)) as usize;
    let ratio = f64::from(rate) / 16_000.0;
    let resampled: Vec<f32> = (0..out_len)
        .map(|i| {
            let start = (i as f64 * ratio) as usize;
            let end = (((i + 1) as f64 * ratio) as usize).clamp(start + 1, mono.len());
            let slice = &mono[start.min(mono.len() - 1)..end];
            slice.iter().sum::<f32>() / slice.len().max(1) as f32
        })
        .collect();
    std::fs::write(wav, crate::audio::encode_wav_16k(&resampled)?)
        .with_context(|| format!("Could not write {}", wav.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Picks from a movie with a frame every 100 ms up to `last`, seeking to the frame at or
    /// before a time as a key frame every 1 s would.
    fn pick(times: &[u64], duration_ms: u64, last: u64) -> Vec<(u64, u64)> {
        let cursor = std::cell::Cell::new(0_u64);
        let mut out = Vec::new();
        on_screen_at(
            times,
            duration_ms,
            |t| {
                cursor.set(t / 1_000 * 1_000);
                Ok(())
            },
            || {
                let t = cursor.get();
                cursor.set(t + 100);
                Ok((t <= last).then_some((t, t)))
            },
            |want, t, &sample| {
                assert_eq!(t, sample);
                out.push((want, t));
                Ok(())
            },
        )
        .unwrap();
        out
    }

    #[test]
    fn a_time_past_the_end_gets_the_last_frame_under_its_own_time() {
        // A 3 s movie whose last frame is at 2.9 s; the last keyframe is asked for at 3.2 s.
        assert_eq!(
            pick(&[0, 1_250, 3_200], 3_000, 2_900),
            [(0, 0), (1_250, 1_200), (3_200, 2_900)]
        );
    }

    #[test]
    fn frames_are_the_last_at_or_before_each_time_give_or_take_40_ms() {
        assert_eq!(
            pick(&[150, 160, 1_030, 2_510], 3_000, 2_900),
            [(150, 100), (160, 200), (1_030, 1_000), (2_510, 2_500)]
        );
        // Far ahead: a seek, then the walk from the key frame.
        assert_eq!(
            pick(&[150, 2_510], 3_000, 2_900),
            [(150, 100), (2_510, 2_500)]
        );
    }
}
