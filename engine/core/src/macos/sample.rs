//! Bounded, owned access to one valid packed ScreenCaptureKit sample.
//!
//! This copies exactly the source pixel buffer's native BGRA rows and raw PTS. It does not map
//! callback arrival to media time, infer an encoded-video origin, convert pixels, or choose color
//! policy. Missing/unsupported color attachments remain unavailable metadata.

use std::ffi::{CStr, c_char, c_void};
use std::slice;

/// Maximum copied pixel bytes per sample.
const MAX_PACKED_BGRA_BYTES: usize = 256 * 1024 * 1024;
/// Maximum serialized ICC profile copied from the exact CoreVideo color-space attachment.
const MAX_SOURCE_ICC_BYTES: usize = 32 * 1024;
const UTF8_ENCODING: u32 = 0x0800_0100;

/// Owned sample planes from a packed 32BGRA `CVPixelBuffer`.
#[derive(Clone)]
pub struct PackedBgraSample {
    pub pts: screencapturekit::cm::CMTime,
    pub width: usize,
    pub height: usize,
    pub pixel_format: u32,
    /// Always false; planar buffers are refused by this accessor.
    pub planar: bool,
    pub bytes_per_row: usize,
    pub data_size: usize,
    pub color: SampleColorAttachments,
    /// Row-major copy including the native per-row padding described by `bytes_per_row`.
    pub bytes: Vec<u8>,
}

/// Color declarations and actual ICC bytes copied from the same sample buffer.
#[derive(Clone, Debug)]
pub struct SampleColorAttachments {
    pub primaries: Option<String>,
    pub transfer_function: Option<String>,
    pub ycbcr_matrix: Option<String>,
    /// Core Foundation description for diagnostics only; not a color-profile authority.
    pub cg_color_space: Option<String>,
    /// ICC serialization of the attached `CGColorSpace`, when available and within bounds.
    pub cg_color_space_icc: Option<Vec<u8>>,
}

/// Copies one packed 32BGRA source buffer with its unmodified PTS and available source color
/// metadata, under a positive caller budget capped at 256 MiB. The caller must first establish that the enclosing CMSampleBuffer is complete,
/// ready, and contains a sample. The CVPixelBuffer remains read-locked until bytes and metadata
/// have been copied. No timestamp conversion or color transform is performed.
pub fn copy_packed_bgra_sample(
    buffer: &screencapturekit::cv::CVPixelBuffer,
    pts: screencapturekit::cm::CMTime,
    maximum_bytes: usize,
) -> Result<PackedBgraSample, String> {
    if !(1..=MAX_PACKED_BGRA_BYTES).contains(&maximum_bytes) {
        return Err("invalid packed BGRA caller budget".into());
    }
    const PIXEL_FORMAT_32_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
    if buffer.is_planar() {
        return Err(
            "source sample is planar; packed BGRA copy does not accept planar buffers".into(),
        );
    }
    let pixel_format = buffer.pixel_format();
    if pixel_format != PIXEL_FORMAT_32_BGRA {
        return Err(format!(
            "unsupported source pixel format {pixel_format:#010x}; expected 32BGRA"
        ));
    }
    let width = buffer.width();
    let height = buffer.height();
    let bytes_per_row = buffer.bytes_per_row();
    let minimum_row = width
        .checked_mul(4)
        .ok_or_else(|| "source pixel row size overflow".to_owned())?;
    if width == 0 || height == 0 || bytes_per_row < minimum_row {
        return Err("invalid source pixel dimensions or stride".into());
    }
    let byte_count = bytes_per_row
        .checked_mul(height)
        .filter(|size| *size <= maximum_bytes && *size <= buffer.data_size())
        .ok_or_else(|| "source pixel sample exceeds bounded copy budget".to_owned())?;
    let lock = buffer
        .lock_read_only()
        .map_err(|error| format!("source pixel lock failed: {error}"))?;
    let address = lock.base_address();
    if address.is_null() {
        return Err("source pixel buffer has no mapped base address".into());
    }
    // SAFETY: CVPixelBuffer is read-locked; checked dimensions/stride bound the copied extent.
    let bytes = unsafe { slice::from_raw_parts(address, byte_count) }.to_vec();
    Ok(PackedBgraSample {
        pts,
        width,
        height,
        pixel_format,
        planar: false,
        bytes_per_row,
        data_size: buffer.data_size(),
        color: source_color_attachments(buffer),
        bytes,
    })
}

/// Retains color declarations and a bounded ICC serialization from the exact callback buffer.
/// The locked SCK crate exposes no safe attachment wrapper, so this uses CoreVideo's retained
/// copy API and checks Core Foundation object type/lifetime before copying.
fn source_color_attachments(
    buffer: &screencapturekit::cv::CVPixelBuffer,
) -> SampleColorAttachments {
    type CfRef = *const c_void;

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static kCVImageBufferColorPrimariesKey: CfRef;
        static kCVImageBufferTransferFunctionKey: CfRef;
        static kCVImageBufferYCbCrMatrixKey: CfRef;
        static kCVImageBufferCGColorSpaceKey: CfRef;
        fn CVBufferCopyAttachment(buffer: *mut c_void, key: CfRef, mode: *mut u32)
        -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFGetTypeID(value: CfRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFCopyDescription(value: CfRef) -> CfRef;
        fn CFStringGetCString(
            value: CfRef,
            buffer: *mut c_char,
            capacity: isize,
            encoding: u32,
        ) -> bool;
        fn CFDataGetLength(data: CfRef) -> isize;
        fn CFDataGetBytePtr(data: CfRef) -> *const u8;
        fn CFRelease(value: CfRef);
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceGetTypeID() -> usize;
        fn CGColorSpaceCopyICCData(color_space: CfRef) -> CfRef;
    }

    unsafe fn read(buffer: *mut c_void, key: CfRef, describe: bool) -> Option<String> {
        let value = unsafe { CVBufferCopyAttachment(buffer, key, std::ptr::null_mut()) };
        if value.is_null() {
            return None;
        }
        if !describe && unsafe { CFGetTypeID(value) != CFStringGetTypeID() } {
            unsafe { CFRelease(value) };
            return None;
        }
        let string = if describe {
            unsafe { CFCopyDescription(value) }
        } else {
            value
        };
        if string.is_null() {
            unsafe { CFRelease(value) };
            return None;
        }
        let mut output = [0i8; 512];
        let copied = unsafe {
            CFStringGetCString(
                string,
                output.as_mut_ptr(),
                output.len() as isize,
                UTF8_ENCODING,
            )
        };
        let result = copied.then(|| {
            // SAFETY: successful CFStringGetCString writes a NUL-terminated UTF-8 buffer.
            unsafe { CStr::from_ptr(output.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        });
        if describe {
            unsafe { CFRelease(string) };
        }
        unsafe { CFRelease(value) };
        result
    }

    unsafe fn read_icc(buffer: *mut c_void, key: CfRef) -> Option<Vec<u8>> {
        let color_space = unsafe { CVBufferCopyAttachment(buffer, key, std::ptr::null_mut()) };
        if color_space.is_null() {
            return None;
        }
        // Accept only a genuine Core Graphics color-space object, not a textual description.
        let is_color_space = unsafe { CFGetTypeID(color_space) == CGColorSpaceGetTypeID() };
        if !is_color_space {
            unsafe { CFRelease(color_space) };
            return None;
        }
        let data = unsafe { CGColorSpaceCopyICCData(color_space) };
        unsafe { CFRelease(color_space) };
        if data.is_null() {
            return None;
        }
        let length = unsafe { CFDataGetLength(data) };
        if length <= 0 || length as usize > MAX_SOURCE_ICC_BYTES {
            unsafe { CFRelease(data) };
            return None;
        }
        let bytes = unsafe { CFDataGetBytePtr(data) };
        if bytes.is_null() {
            unsafe { CFRelease(data) };
            return None;
        }
        // SAFETY: CFData owns a live allocation of the checked positive length until release.
        let copied = unsafe { slice::from_raw_parts(bytes, length as usize) }.to_vec();
        unsafe { CFRelease(data) };
        Some(copied)
    }

    // SAFETY: CoreVideo exports these static CFString keys for process lifetime.
    unsafe {
        let buffer = buffer.as_ptr();
        SampleColorAttachments {
            primaries: read(buffer, kCVImageBufferColorPrimariesKey, false),
            transfer_function: read(buffer, kCVImageBufferTransferFunctionKey, false),
            ycbcr_matrix: read(buffer, kCVImageBufferYCbCrMatrixKey, false),
            cg_color_space: read(buffer, kCVImageBufferCGColorSpaceKey, true),
            cg_color_space_icc: read_icc(buffer, kCVImageBufferCGColorSpaceKey),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use screencapturekit::{cm::CMTime, cv::CVPixelBuffer};

    fn pts() -> CMTime {
        CMTime {
            value: 123,
            timescale: 1000,
            flags: 1,
            epoch: 0,
        }
    }

    #[test]
    fn caller_budget_preserves_full_size_rows_and_raw_pts() {
        let buffer = CVPixelBuffer::create(1024, 768, u32::from_be_bytes(*b"BGRA")).unwrap();
        let length = buffer.bytes_per_row() * buffer.height();
        assert!(length > 2 * 1024 * 1024);
        {
            let mut lock = buffer.lock_read_write().unwrap();
            let address = lock.base_address_mut().unwrap();
            // SAFETY: this fresh test buffer has no aliases, and its writable native lock
            // covers the checked stride/height extent until initialization is finished.
            unsafe { std::ptr::write_bytes(address, 0x5a, length) };
        }
        assert!(copy_packed_bgra_sample(&buffer, pts(), 0).is_err());
        assert!(copy_packed_bgra_sample(&buffer, pts(), MAX_PACKED_BGRA_BYTES + 1).is_err());
        assert!(copy_packed_bgra_sample(&buffer, pts(), 2 * 1024 * 1024).is_err());
        let copied = copy_packed_bgra_sample(&buffer, pts(), 4 * 1024 * 1024).unwrap();
        assert_eq!((copied.width, copied.height), (1024, 768));
        assert_eq!(copied.bytes_per_row, buffer.bytes_per_row());
        assert_eq!(copied.bytes.len(), length);
        assert!(copied.bytes.iter().all(|&value| value == 0x5a));
        assert_eq!(
            (
                copied.pts.value,
                copied.pts.timescale,
                copied.pts.flags,
                copied.pts.epoch
            ),
            (123, 1000, 1, 0)
        );
    }

    #[test]
    fn non_string_color_attachments_remain_unavailable() {
        type CfRef = *const c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFBooleanTrue: CfRef;
            fn CFGetTypeID(value: CfRef) -> usize;
            fn CFRelease(value: CfRef);
        }
        #[link(name = "CoreVideo", kind = "framework")]
        unsafe extern "C" {
            static kCVImageBufferColorPrimariesKey: CfRef;
            static kCVImageBufferTransferFunctionKey: CfRef;
            static kCVImageBufferYCbCrMatrixKey: CfRef;
            fn CVBufferSetAttachment(buffer: *mut c_void, key: CfRef, value: CfRef, mode: u32);
            fn CVBufferCopyAttachment(buffer: *mut c_void, key: CfRef, mode: *mut u32) -> CfRef;
        }
        let buffer = CVPixelBuffer::create(8, 8, u32::from_be_bytes(*b"BGRA")).unwrap();
        // SAFETY: all keys and the Boolean are process-lifetime CF constants. CoreVideo
        // retains its attachment; this intentionally supplies the wrong CF type to the reader.
        unsafe {
            for key in [
                kCVImageBufferColorPrimariesKey,
                kCVImageBufferTransferFunctionKey,
                kCVImageBufferYCbCrMatrixKey,
            ] {
                CVBufferSetAttachment(buffer.as_ptr(), key, kCFBooleanTrue, 0);
                let value = CVBufferCopyAttachment(buffer.as_ptr(), key, std::ptr::null_mut());
                assert!(
                    !value.is_null(),
                    "native fixture must actually install the wrong type"
                );
                assert_eq!(CFGetTypeID(value), CFGetTypeID(kCFBooleanTrue));
                CFRelease(value);
            }
        }
        let color = source_color_attachments(&buffer);
        assert!(color.primaries.is_none());
        assert!(color.transfer_function.is_none());
        assert!(color.ycbcr_matrix.is_none());
    }
}
