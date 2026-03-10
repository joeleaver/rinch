/// Raw pixel formats commonly produced by cameras.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// RGBA 8-bit per channel (already decoded).
    Rgba,
    /// YUYV 4:2:2 packed (2 pixels per 4 bytes).
    Yuyv,
    /// NV12 semi-planar (Y plane + interleaved UV plane).
    Nv12,
    /// Motion JPEG compressed frames.
    Mjpeg,
}

/// A decoded video frame in RGBA8 format.
#[derive(Clone)]
pub struct VideoFrame {
    /// RGBA8 pixel data, length = width * height * 4.
    pub data: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Microseconds since capture start.
    pub timestamp_us: u64,
}

/// Convert raw camera pixels to RGBA8.
///
/// For `PixelFormat::Rgba`, returns a copy of the source data.
/// For `PixelFormat::Mjpeg`, returns an error (use nokhwa's built-in decoder instead).
pub fn convert_to_rgba(
    src: &[u8],
    format: PixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, crate::AvError> {
    match format {
        PixelFormat::Rgba => Ok(src.to_vec()),
        PixelFormat::Yuyv => convert_yuyv_to_rgba(src, width, height),
        PixelFormat::Nv12 => convert_nv12_to_rgba(src, width, height),
        PixelFormat::Mjpeg => Err(crate::AvError::FormatUnsupported(
            "MJPEG software decode not implemented; use nokhwa's built-in decoder".into(),
        )),
    }
}

/// YUYV 4:2:2 → RGBA8.
///
/// YUYV packs two pixels into 4 bytes: [Y0, U, Y1, V].
/// Each pair shares U and V chroma values.
fn convert_yuyv_to_rgba(src: &[u8], width: u32, height: u32) -> Result<Vec<u8>, crate::AvError> {
    let pixel_count = (width * height) as usize;
    let expected_len = pixel_count * 2; // 2 bytes per pixel in YUYV
    if src.len() < expected_len {
        return Err(crate::AvError::FormatUnsupported(format!(
            "YUYV buffer too small: got {}, expected {}",
            src.len(),
            expected_len
        )));
    }

    let mut rgba = vec![255u8; pixel_count * 4];

    for i in (0..pixel_count).step_by(2) {
        let base = i * 2;
        let y0 = src[base] as f32;
        let u = src[base + 1] as f32 - 128.0;
        let y1 = src[base + 2] as f32;
        let v = src[base + 3] as f32 - 128.0;

        let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g0 = (y0 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
        let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

        let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g1 = (y1 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
        let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

        let out0 = i * 4;
        rgba[out0] = r0;
        rgba[out0 + 1] = g0;
        rgba[out0 + 2] = b0;
        // alpha already 255

        let out1 = (i + 1) * 4;
        rgba[out1] = r1;
        rgba[out1 + 1] = g1;
        rgba[out1 + 2] = b1;
    }

    Ok(rgba)
}

/// NV12 semi-planar → RGBA8.
///
/// NV12 layout: full Y plane (width*height bytes) followed by interleaved UV plane
/// (width*height/2 bytes, with U and V alternating).
fn convert_nv12_to_rgba(src: &[u8], width: u32, height: u32) -> Result<Vec<u8>, crate::AvError> {
    let w = width as usize;
    let h = height as usize;
    let y_plane_size = w * h;
    let expected_len = y_plane_size + y_plane_size / 2;
    if src.len() < expected_len {
        return Err(crate::AvError::FormatUnsupported(format!(
            "NV12 buffer too small: got {}, expected {}",
            src.len(),
            expected_len
        )));
    }

    let mut rgba = vec![255u8; y_plane_size * 4];
    let y_plane = &src[..y_plane_size];
    let uv_plane = &src[y_plane_size..];

    for row in 0..h {
        for col in 0..w {
            let y = y_plane[row * w + col] as f32;
            let uv_idx = (row / 2) * w + (col & !1);
            let u = uv_plane[uv_idx] as f32 - 128.0;
            let v = uv_plane[uv_idx + 1] as f32 - 128.0;

            let r = (y + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g = (y - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
            let b = (y + 1.772 * u).clamp(0.0, 255.0) as u8;

            let out = (row * w + col) * 4;
            rgba[out] = r;
            rgba[out + 1] = g;
            rgba[out + 2] = b;
        }
    }

    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgba_passthrough() {
        let src = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let result = convert_to_rgba(&src, PixelFormat::Rgba, 2, 1).unwrap();
        assert_eq!(result, src);
    }

    #[test]
    fn test_yuyv_basic() {
        // Two black pixels: Y=0, U=128, V=128 (neutral chroma)
        let src = vec![0, 128, 0, 128];
        let result = convert_to_rgba(&src, PixelFormat::Yuyv, 2, 1).unwrap();
        assert_eq!(result.len(), 8);
        // Both pixels should be black with alpha 255
        assert_eq!(result[0], 0); // R
        assert_eq!(result[1], 0); // G
        assert_eq!(result[2], 0); // B
        assert_eq!(result[3], 255); // A
    }

    #[test]
    fn test_nv12_basic() {
        // 2x2 black: Y plane = [0,0,0,0], UV plane = [128,128]
        let src = vec![0, 0, 0, 0, 128, 128];
        let result = convert_to_rgba(&src, PixelFormat::Nv12, 2, 2).unwrap();
        assert_eq!(result.len(), 16);
        for i in 0..4 {
            assert_eq!(result[i * 4], 0); // R
            assert_eq!(result[i * 4 + 1], 0); // G
            assert_eq!(result[i * 4 + 2], 0); // B
            assert_eq!(result[i * 4 + 3], 255); // A
        }
    }

    #[test]
    fn test_yuyv_buffer_too_small() {
        let src = vec![0, 128];
        let result = convert_to_rgba(&src, PixelFormat::Yuyv, 2, 1);
        assert!(result.is_err());
    }
}
