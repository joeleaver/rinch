//! Chrome's drawing of a page: headless, at the window's width, in a
//! profile of its own (never the person's), as tall as documents get, then cut
//! below the last row that is not the page's background.

use std::path::Path;
use std::process::Command;

use image::{GenericImageView, RgbaImage};

use super::Result;

/// The tallest a screenshot is: past this, a document is cut off.
pub const TALLEST: u32 = 12_000;

/// Screenshot `page` (an HTML file) into `png`, `width` wide.
pub fn screenshot(page: &Path, png: &Path, width: u32, profile: &Path) -> Result<()> {
    let page = std::fs::canonicalize(page)?;
    let status = Command::new(chrome())
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            "--no-default-browser-check",
            "--force-device-scale-factor=1",
            // Long enough for pictures from the web to arrive.
            "--virtual-time-budget=15000",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={width},{TALLEST}"))
        .arg(format!("--screenshot={}", png.display()))
        .arg(format!("file://{}", page.display()))
        .output()?;
    if !status.status.success() || !png.exists() {
        return Err(format!(
            "Chrome did not screenshot {}: {}",
            page.display(),
            String::from_utf8_lossy(&status.stderr)
        )
        .into());
    }
    trim(png)
}

/// Screenshot a case's two pages, `a-original.png` (as written, `width`
/// wide) and `b-cleaned.png`.
pub fn case(dir: &Path, width: u32, profile: &Path) -> Result<()> {
    screenshot(
        &dir.join("original.html"),
        &dir.join("a-original.png"),
        width,
        profile,
    )?;
    screenshot(
        &dir.join("cleaned.html"),
        &dir.join("b-cleaned.png"),
        width,
        profile,
    )
}

fn chrome() -> String {
    std::env::var("CHROME").unwrap_or_else(|_| "google-chrome".to_owned())
}

/// Cut the image below its last row that differs from its bottom row.
pub fn trim(png: &Path) -> Result<()> {
    let image = image::open(png)?;
    let (w, h) = image.dimensions();
    let background = image.get_pixel(w / 2, h - 1);
    let mut last = 0;
    for y in (0..h).rev() {
        if (0..w).any(|x| differs(image.get_pixel(x, y).0, background.0)) {
            last = y;
            break;
        }
    }
    let keep = (last + 16).min(h);
    let cut: RgbaImage = image.view(0, 0, w, keep).to_image();
    cut.save(png)?;
    Ok(())
}

/// Whether two pixels differ visibly.
fn differs(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 8)
}
