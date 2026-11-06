//! Chrome's drawing of a page: headless, at the window's width, in a
//! profile of its own (never the person's), as tall as documents get, then cut
//! below the last row that is not the page's background.

use std::path::Path;
use std::process::Command;

use image::{GenericImageView, RgbaImage};
use serde::{Deserialize, Serialize};

use super::Result;

/// The tallest a screenshot is: past this, a document is cut off.
pub const TALLEST: u32 = 12_000;

/// Screenshot `page` (an HTML file) into `png`, `width` wide. Scripts never
/// run, and nothing is fetched from the network unless `remote`: the page
/// may be mail as it was sent, with its tracking pictures. Chrome is given a
/// copy that starts with a Content-Security-Policy saying so (a `<meta>`
/// before the document goes into its head), and with `remote` off no host
/// name resolves either.
pub fn screenshot(page: &Path, png: &Path, width: u32, profile: &Path, remote: bool) -> Result<()> {
    screenshot_measured(page, png, width, profile, remote).map(|_| ())
}

/// The Content-Security-Policy `<meta>` Chrome's copy of a page starts with.
/// `nonce` allows exactly one inline script (the measuring one), never the
/// page's own.
fn policy(remote: bool, nonce: Option<&str>) -> String {
    let pictures = if remote {
        "data: file: http: https:"
    } else {
        "data: file:"
    };
    let script = match nonce {
        Some(n) => format!("'nonce-{n}'"),
        None => "'none'".to_owned(),
    };
    format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
         script-src {script}; style-src 'unsafe-inline' data: file:; font-src data: file:; \
         img-src {pictures}\">"
    )
}

/// [`screenshot`], and what Chrome says about the page's height
/// ([`PageMeasure`]). The measurement is a second, scripted-by-us-only run;
/// when it fails the screenshot is still taken and the measure has no
/// `page_height`.
pub fn screenshot_measured(
    page: &Path,
    png: &Path,
    width: u32,
    profile: &Path,
    remote: bool,
) -> Result<PageMeasure> {
    let page = std::fs::canonicalize(page)?;
    let policy = policy(remote, None);
    let copy = page.with_extension("chrome.html");
    std::fs::write(&copy, policy + &std::fs::read_to_string(&page)?)?;
    let mut chrome = Command::new(chrome());
    if !remote {
        // Every host name fails to resolve: no request leaves the machine.
        chrome.arg("--host-resolver-rules=MAP * ~NOTFOUND");
    }
    let status = chrome
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
        .arg(format!("file://{}", copy.display()))
        .output()?;
    let _ = std::fs::remove_file(&copy);
    if !status.status.success() || !png.exists() {
        return Err(format!(
            "Chrome did not screenshot {}: {}",
            page.display(),
            String::from_utf8_lossy(&status.stderr)
        )
        .into());
    }
    let heights = measure_page(&page, width, profile, remote);
    let (last_ink, trimmed_height) = trim_measured(png)?;
    Ok(PageMeasure {
        page_height: heights.map(|h| h.0),
        document_height: heights.map(|h| h.1),
        viewport_height: heights.map(|h| h.2),
        trimmed_height,
        last_ink,
    })
}

/// What Chrome says about one page, written to `chrome.json`.
///
/// `b-cleaned.png` is cut 16px below the last row of ink ([`trim`]), so its
/// height is `last_ink + 16`, not the document's: compare `page_height`
/// with rinch's document height, and `last_ink` with rinch's `last_ink`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PageMeasure {
    /// `document.scrollingElement.scrollHeight` at the screenshot's viewport
    /// width, read after the fonts and pictures loaded and two animation
    /// frames. Never less than the measuring window's height
    /// (`viewport_height`). `None` when the measuring run failed.
    pub page_height: Option<f64>,
    /// `document.documentElement`'s border-box height: the content's height,
    /// whatever the viewport. `None` when the measuring run failed.
    pub document_height: Option<f64>,
    /// `window.innerHeight` of the measuring run: the floor of `page_height`.
    pub viewport_height: Option<f64>,
    /// The height of the saved png (last ink row + [`MARGIN`]).
    pub trimmed_height: u32,
    /// The last row (0-based) that differs from the page's background.
    pub last_ink: u32,
}

impl PageMeasure {
    /// Chrome's document height, to compare with rinch's: the scroll height,
    /// unless that is only the viewport's floor, then the document's own.
    pub fn height(&self) -> Option<f64> {
        match (self.page_height, self.document_height, self.viewport_height) {
            (Some(p), Some(d), Some(v)) if p <= v => Some(d.min(p)),
            (Some(p), _, _) => Some(p),
            (None, d, _) => d,
        }
    }
}

/// The script that reads the heights, once fonts and pictures have loaded
/// and two animation frames have passed. Headless Chrome's virtual time does
/// not always deliver animation frames, so a timer reads them anyway after
/// 500ms (virtual) rather than leave the page unmeasured.
const MEASURE: &str = "(()=>{const h=document.documentElement;let done=false;\
const set=()=>{if(done)return;done=true;\
h.setAttribute('data-rinch-page-height',String(document.scrollingElement.scrollHeight));\
h.setAttribute('data-rinch-document-height',String(h.getBoundingClientRect().height));\
h.setAttribute('data-rinch-viewport-height',String(window.innerHeight));};\
const ready=()=>document.fonts.status==='loaded'&&[...document.images].every(i=>i.complete);\
const poll=()=>{if(!ready()){setTimeout(poll,50);return;}\
requestAnimationFrame(()=>requestAnimationFrame(set));setTimeout(set,500);};\
poll();})()";

/// The window height of the measuring run. The scroll height is never less
/// than the viewport's, so this is short: a document taller than it reads as
/// its own height. A document with `100vh` in its CSS lays out against this
/// window, not the screenshot's.
const MEASURE_WINDOW: u32 = 600;

const NONCE: &str = "rinchmeasure";

/// Load `page` in a second Chrome with only our script allowed and read the
/// two heights out of the dumped DOM. `None` if anything goes wrong.
fn measure_page(page: &Path, width: u32, profile: &Path, remote: bool) -> Option<Heights> {
    let copy = page.with_extension("measure.html");
    let text = std::fs::read_to_string(page).ok()?;
    let page_with_script = format!(
        "{}{text}<script nonce=\"{NONCE}\">{MEASURE}</script>",
        policy(remote, Some(NONCE))
    );
    std::fs::write(&copy, page_with_script).ok()?;
    let mut chrome = Command::new(chrome());
    if !remote {
        chrome.arg("--host-resolver-rules=MAP * ~NOTFOUND");
    }
    let out = chrome
        .args([
            "--headless=new",
            "--disable-gpu",
            "--hide-scrollbars",
            "--no-first-run",
            "--no-default-browser-check",
            "--force-device-scale-factor=1",
            "--virtual-time-budget=15000",
            "--dump-dom",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--window-size={width},{MEASURE_WINDOW}"))
        .arg(format!("file://{}", copy.display()))
        .output();
    let _ = std::fs::remove_file(&copy);
    parse_heights(&String::from_utf8_lossy(&out.ok()?.stdout))
}

/// `(scroll height, document height, viewport height)`.
type Heights = (f64, f64, f64);

/// The heights from a dumped DOM.
fn parse_heights(dom: &str) -> Option<Heights> {
    let attr = |name: &str| -> Option<f64> {
        let at = dom.find(&format!("{name}=\""))? + name.len() + 2;
        dom[at..].split('"').next()?.parse().ok()
    };
    Some((
        attr("data-rinch-page-height")?,
        attr("data-rinch-document-height")?,
        attr("data-rinch-viewport-height")?,
    ))
}

/// Screenshot a case's two pages, `a-original.png` (as written, `width`
/// wide) and `b-cleaned.png`; pictures from the web only when `remote`.
/// Also writes `chrome.json` ([`PageMeasure`] of each, as `original` and
/// `cleaned`).
pub fn case(dir: &Path, width: u32, profile: &Path, remote: bool) -> Result<()> {
    let original = screenshot_measured(
        &dir.join("original.html"),
        &dir.join("a-original.png"),
        width,
        profile,
        remote,
    )?;
    let cleaned = screenshot_measured(
        &dir.join("cleaned.html"),
        &dir.join("b-cleaned.png"),
        width,
        profile,
        remote,
    )?;
    std::fs::write(
        dir.join("chrome.json"),
        serde_json::to_vec(&ChromeJson { original, cleaned })?,
    )?;
    Ok(())
}

/// `chrome.json`: Chrome's measure of each of a case's two pages.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChromeJson {
    pub original: PageMeasure,
    pub cleaned: PageMeasure,
}

fn chrome() -> String {
    std::env::var("CHROME").unwrap_or_else(|_| "google-chrome".to_owned())
}

/// The last row of `image` that differs from its bottom row (0 if none).
pub fn last_ink_row(image: &impl GenericImageView<Pixel = image::Rgba<u8>>) -> u32 {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return 0;
    }
    let background = image.get_pixel(w / 2, h - 1);
    (0..h)
        .rev()
        .find(|&y| (0..w).any(|x| differs(image.get_pixel(x, y).0, background.0)))
        .unwrap_or(0)
}

/// How far below the last row of ink [`trim`] keeps.
pub const MARGIN: u32 = 16;

/// Cut the image below its last row that differs from its bottom row.
pub fn trim(png: &Path) -> Result<()> {
    trim_measured(png).map(|_| ())
}

/// [`trim`], returning that row and the new height.
pub fn trim_measured(png: &Path) -> Result<(u32, u32)> {
    let image = image::open(png)?;
    let (w, h) = image.dimensions();
    let last = last_ink_row(&image);
    let keep = (last + MARGIN).min(h);
    let cut: RgbaImage = image.view(0, 0, w, keep).to_image();
    cut.save(png)?;
    Ok((last, keep))
}

/// Whether two pixels differ visibly.
fn differs(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn parses_the_dumped_dom() {
        let dom = r#"<html data-rinch-page-height="1234" data-rinch-document-height="1233.5" data-rinch-viewport-height="513"><body></body></html>"#;
        assert_eq!(parse_heights(dom), Some((1234.0, 1233.5, 513.0)));
        assert_eq!(parse_heights("<html></html>"), None);
    }

    #[test]
    fn height_is_the_documents_when_the_scroll_height_is_the_viewports() {
        let m = |p, d| PageMeasure {
            page_height: p,
            document_height: d,
            viewport_height: Some(513.0),
            trimmed_height: 0,
            last_ink: 0,
        };
        assert_eq!(m(Some(513.0), Some(300.0)).height(), Some(300.0));
        assert_eq!(m(Some(5000.0), Some(5000.0)).height(), Some(5000.0));
        assert_eq!(m(None, Some(7.0)).height(), Some(7.0));
        assert_eq!(m(None, None).height(), None);
    }

    #[test]
    fn json_shape() {
        let m = PageMeasure {
            page_height: Some(1000.0),
            document_height: Some(990.0),
            viewport_height: Some(513.0),
            trimmed_height: 956,
            last_ink: 940,
        };
        let v = serde_json::to_value(ChromeJson {
            original: m,
            cleaned: m,
        })
        .unwrap();
        assert_eq!(v["cleaned"]["page_height"], 1000.0);
        assert_eq!(v["cleaned"]["trimmed_height"], 956);
        assert_eq!(v["original"]["last_ink"], 940);
    }

    #[test]
    fn trim_explains_itself() {
        let mut img = RgbaImage::from_pixel(20, 200, Rgba([255, 255, 255, 255]));
        img.put_pixel(3, 50, Rgba([0, 0, 0, 255]));
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("t.png");
        img.save(&png).unwrap();
        assert_eq!(last_ink_row(&img), 50);
        assert_eq!(trim_measured(&png).unwrap(), (50, 50 + MARGIN));
        assert_eq!(image::open(&png).unwrap().height(), 50 + MARGIN);
    }

    #[test]
    fn the_script_policy_allows_only_the_nonce() {
        assert!(policy(false, None).contains("script-src 'none'"));
        assert!(policy(false, Some("n")).contains("script-src 'nonce-n'"));
    }

    /// Needs Chrome (`CHROME`, or `google-chrome`): `cargo test -- --ignored`.
    #[test]
    #[ignore = "needs Chrome"]
    fn chrome_reports_the_documents_height() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("p.html");
        std::fs::write(
            &page,
            "<!doctype html><body style=\"margin:0\"><div style=\"height:1500px\">x</div>\
             <script>document.documentElement.setAttribute('data-rinch-page-height','1')</script>",
        )
        .unwrap();
        let png = dir.path().join("p.png");
        let m = screenshot_measured(&page, &png, 720, &dir.path().join("profile"), false).unwrap();
        // 1500 tall, the page's own script did not run (it would have set
        // the attribute to 1), and the png is cut 16px below the ink: the
        // two heights are different numbers.
        assert_eq!(m.page_height, Some(1500.0));
        assert_ne!(f64::from(m.trimmed_height), 1500.0);
        assert!(m.trimmed_height <= m.last_ink + MARGIN);
    }
}
