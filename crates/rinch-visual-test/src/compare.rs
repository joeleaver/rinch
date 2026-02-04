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
/// Uses a simplified SSIM calculation on luminance channel.
fn calculate_ssim(img1: &RgbaImage, img2: &RgbaImage) -> f64 {
    let (_width, _height) = img1.dimensions();

    // Convert to grayscale luminance
    let luma1: Vec<f64> = img1.pixels()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .collect();
    let luma2: Vec<f64> = img2.pixels()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .collect();

    // SSIM constants
    let c1 = 6.5025; // (0.01 * 255)^2
    let c2 = 58.5225; // (0.03 * 255)^2

    // Calculate means
    let n = luma1.len() as f64;
    let mean1: f64 = luma1.iter().sum::<f64>() / n;
    let mean2: f64 = luma2.iter().sum::<f64>() / n;

    // Calculate variances and covariance
    let var1: f64 = luma1.iter().map(|x| (x - mean1).powi(2)).sum::<f64>() / n;
    let var2: f64 = luma2.iter().map(|x| (x - mean2).powi(2)).sum::<f64>() / n;
    let covar: f64 = luma1.iter().zip(luma2.iter())
        .map(|(x, y)| (x - mean1) * (y - mean2))
        .sum::<f64>() / n;

    // SSIM formula
    let ssim = ((2.0 * mean1 * mean2 + c1) * (2.0 * covar + c2)) /
               ((mean1.powi(2) + mean2.powi(2) + c1) * (var1 + var2 + c2));

    ssim.clamp(0.0, 1.0)
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

            let is_different =
                (p1[0] as i16 - p2[0] as i16).abs() > pixel_threshold as i16 ||
                (p1[1] as i16 - p2[1] as i16).abs() > pixel_threshold as i16 ||
                (p1[2] as i16 - p2[2] as i16).abs() > pixel_threshold as i16;

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
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
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
        assert!((ssim - 1.0).abs() < 0.001, "SSIM should be ~1.0 for identical images");
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
        assert!(ssim < 0.95, "SSIM should be low for very different images: {}", ssim);
    }
}
