//! Image comparison using SSIM (Structural Similarity Index).

use image::{DynamicImage, GenericImageView, ImageBuffer, Rgba, RgbaImage};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CompareError {
    #[error("Failed to decode image: {0}")]
    DecodeError(String),

    #[error("Image dimensions don't match: {0}x{1} vs {2}x{3}")]
    DimensionMismatch(u32, u32, u32, u32),
}

/// Result of comparing two images.
#[derive(Debug)]
pub struct ComparisonResult {
    /// SSIM score (0.0 to 1.0, higher = more similar)
    pub ssim_score: f64,
    /// Whether the comparison passed the threshold
    pub passed: bool,
    /// Diff image highlighting differences (if comparison failed)
    pub diff_image: Option<DynamicImage>,
    /// Number of pixels that differ significantly
    pub diff_pixel_count: u32,
    /// Total pixels compared
    pub total_pixels: u32,
}

/// Compare two images and return similarity metrics.
pub fn compare_images(
    actual_png: &[u8],
    expected_png: &[u8],
    threshold: f64,
) -> Result<ComparisonResult, CompareError> {
    let actual = image::load_from_memory(actual_png)
        .map_err(|e| CompareError::DecodeError(format!("actual: {}", e)))?;
    let expected = image::load_from_memory(expected_png)
        .map_err(|e| CompareError::DecodeError(format!("expected: {}", e)))?;

    compare_images_decoded(&actual, &expected, threshold)
}

/// Compare two decoded images.
pub fn compare_images_decoded(
    actual: &DynamicImage,
    expected: &DynamicImage,
    threshold: f64,
) -> Result<ComparisonResult, CompareError> {
    let (w1, h1) = actual.dimensions();
    let (w2, h2) = expected.dimensions();

    if w1 != w2 || h1 != h2 {
        return Err(CompareError::DimensionMismatch(w1, h1, w2, h2));
    }

    let actual_rgba = actual.to_rgba8();
    let expected_rgba = expected.to_rgba8();

    // Calculate SSIM
    let ssim_score = calculate_ssim(&actual_rgba, &expected_rgba);
    let passed = ssim_score >= threshold;

    // Count different pixels and generate diff image if failed
    let (diff_count, diff_image) = if !passed {
        let (count, diff) = generate_diff(&actual_rgba, &expected_rgba);
        (count, Some(DynamicImage::ImageRgba8(diff)))
    } else {
        (0, None)
    };

    Ok(ComparisonResult {
        ssim_score,
        passed,
        diff_image,
        diff_pixel_count: diff_count,
        total_pixels: w1 * h1,
    })
}

/// Calculate SSIM between two images.
///
/// SSIM is a *local* statistic: it is defined per window and averaged over the
/// image. Computing one mean/variance/covariance over every pixel instead —
/// which this used to do — collapses it into a global correlation that is
/// dominated by overall brightness and total contrast, and is almost blind to
/// localized structural change. Two renders that a human reads as ~91% alike
/// scored 0.45 that way, which made every threshold in `tests.json`
/// uninterpretable.
///
/// Windows are 8x8 and non-overlapping (a cheap stand-in for the 11x11 Gaussian
/// of the original paper — enough to make the score track what the eye sees).
/// Partial windows at the right/bottom edge are dropped.
fn calculate_ssim(img1: &RgbaImage, img2: &RgbaImage) -> f64 {
    const WINDOW: usize = 8;
    // (0.01 * 255)^2 and (0.03 * 255)^2 — the stabilizing constants.
    const C1: f64 = 6.5025;
    const C2: f64 = 58.5225;

    let (width, height) = img1.dimensions();
    let luma = |img: &RgbaImage| -> Vec<f64> {
        img.pixels()
            .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
            .collect()
    };
    let luma1 = luma(img1);
    let luma2 = luma(img2);

    let (w, h) = (width as usize, height as usize);
    let (windows_x, windows_y) = (w / WINDOW, h / WINDOW);
    if windows_x == 0 || windows_y == 0 {
        // Too small to window: fall back to treating the image as one window.
        return ssim_window(&luma1, &luma2, C1, C2).clamp(0.0, 1.0);
    }

    let mut total = 0.0;
    for wy in 0..windows_y {
        for wx in 0..windows_x {
            let mut a = Vec::with_capacity(WINDOW * WINDOW);
            let mut b = Vec::with_capacity(WINDOW * WINDOW);
            for dy in 0..WINDOW {
                let row = (wy * WINDOW + dy) * w + wx * WINDOW;
                a.extend_from_slice(&luma1[row..row + WINDOW]);
                b.extend_from_slice(&luma2[row..row + WINDOW]);
            }
            total += ssim_window(&a, &b, C1, C2);
        }
    }

    (total / (windows_x * windows_y) as f64).clamp(0.0, 1.0)
}

/// SSIM over a single window of luminance samples.
fn ssim_window(a: &[f64], b: &[f64], c1: f64, c2: f64) -> f64 {
    let n = a.len() as f64;
    let mean_a: f64 = a.iter().sum::<f64>() / n;
    let mean_b: f64 = b.iter().sum::<f64>() / n;
    let var_a: f64 = a.iter().map(|x| (x - mean_a).powi(2)).sum::<f64>() / n;
    let var_b: f64 = b.iter().map(|x| (x - mean_b).powi(2)).sum::<f64>() / n;
    let covar: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - mean_a) * (y - mean_b))
        .sum::<f64>()
        / n;

    ((2.0 * mean_a * mean_b + c1) * (2.0 * covar + c2))
        / ((mean_a.powi(2) + mean_b.powi(2) + c1) * (var_a + var_b + c2))
}

/// Generate a diff image highlighting differences.
///
/// Returns (diff_pixel_count, diff_image).
fn generate_diff(img1: &RgbaImage, img2: &RgbaImage) -> (u32, RgbaImage) {
    let (width, height) = img1.dimensions();
    let mut diff = ImageBuffer::new(width, height);
    let mut diff_count = 0u32;

    // Pixel difference threshold (allow small anti-aliasing differences)
    let pixel_threshold = 10u8;

    for y in 0..height {
        for x in 0..width {
            let p1 = img1.get_pixel(x, y);
            let p2 = img2.get_pixel(x, y);

            let is_different = (p1[0] as i16 - p2[0] as i16).abs() > pixel_threshold as i16
                || (p1[1] as i16 - p2[1] as i16).abs() > pixel_threshold as i16
                || (p1[2] as i16 - p2[2] as i16).abs() > pixel_threshold as i16;

            if is_different {
                diff_count += 1;
                // Red overlay for differences
                diff.put_pixel(x, y, Rgba([255, 0, 0, 200]));
            } else {
                // Dimmed original pixel
                let gray = ((p1[0] as u16 + p1[1] as u16 + p1[2] as u16) / 3) as u8;
                diff.put_pixel(x, y, Rgba([gray, gray, gray, 128]));
            }
        }
    }

    (diff_count, diff)
}

/// Save a comparison result's diff image to a file.
pub fn save_diff_image(result: &ComparisonResult, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(diff) = &result.diff_image {
        diff.save(path)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_images() {
        // Create two identical 10x10 red images
        let mut img = RgbaImage::new(10, 10);
        for pixel in img.pixels_mut() {
            *pixel = Rgba([255, 0, 0, 255]);
        }

        let ssim = calculate_ssim(&img, &img);
        assert!(
            (ssim - 1.0).abs() < 0.001,
            "SSIM should be ~1.0 for identical images"
        );
    }

    /// A localized defect on a *busy* background must move the score.
    ///
    /// This is the case the old whole-image SSIM could not see. On a detailed
    /// background the global variance is already large, so blanking a 1.6%
    /// patch barely perturbs it: the old statistic scored 0.9921 here, while
    /// windowing scores 0.9844. That gap is the whole point — a real screen is
    /// busy, and a real rendering defect is local.
    #[test]
    fn test_localized_difference_on_busy_background() {
        // 4px checkerboard: plenty of structure everywhere, like a page of text.
        let mut img1 = RgbaImage::new(128, 128);
        for (x, y, pixel) in img1.enumerate_pixels_mut() {
            let v = if ((x / 4) + (y / 4)) % 2 == 0 { 255 } else { 0 };
            *pixel = Rgba([v, v, v, 255]);
        }
        // Flatten a 16x16 corner (1.6% of the image) to mid-grey.
        let mut img2 = img1.clone();
        for y in 0..16 {
            for x in 0..16 {
                img2.put_pixel(x, y, Rgba([128, 128, 128, 255]));
            }
        }

        let ssim = calculate_ssim(&img1, &img2);
        assert!(
            ssim < 0.99,
            "a localized defect on a busy background must lower SSIM below 0.99, got {ssim}"
        );
    }

    #[test]
    fn test_different_images() {
        let mut img1 = RgbaImage::new(10, 10);
        let mut img2 = RgbaImage::new(10, 10);

        for pixel in img1.pixels_mut() {
            *pixel = Rgba([255, 0, 0, 255]); // Red
        }
        for pixel in img2.pixels_mut() {
            *pixel = Rgba([0, 0, 255, 255]); // Blue
        }

        let ssim = calculate_ssim(&img1, &img2);
        assert!(
            ssim < 0.95,
            "SSIM should be low for very different images: {}",
            ssim
        );
    }
}
