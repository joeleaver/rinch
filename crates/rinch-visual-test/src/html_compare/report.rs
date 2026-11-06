//! The side-by-side report: per case, the original in Chrome (A),
//! the cleaned HTML in Chrome (B) and in rinch (C), with how far
//! apart each pair is, worst first. A against B is what cleaning loses (our
//! CSS); B against C is how rinch lays out the same HTML differently.

use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView};

use super::Result;
use super::case::Meta;

/// How different two drawings are: the share of differing pixels where
/// they overlap, and how their heights differ.
#[derive(Debug, Clone, Copy, Default)]
pub struct Distance {
    pub pixels: f64,
    pub height: f64,
}

impl Distance {
    /// One number to sort by, 0 for alike.
    pub fn score(self) -> f64 {
        self.pixels + self.height
    }
}

/// Compare two screenshots.
pub fn distance(a: &DynamicImage, b: &DynamicImage) -> Distance {
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    if w == 0 || h == 0 {
        return Distance {
            pixels: 1.0,
            height: 1.0,
        };
    }
    let mut differing = 0u64;
    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
            let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            if p.iter()
                .zip(q.iter())
                .take(3)
                .any(|(m, n)| m.abs_diff(*n) > 40)
            {
                differing += 1;
            }
        }
    }
    let sampled = u64::from(w.div_ceil(2)) * u64::from(h.div_ceil(2));
    let (ha, hb) = (f64::from(a.height()), f64::from(b.height()));
    Distance {
        pixels: differing as f64 / sampled as f64,
        height: (ha - hb).abs() / ha.max(hb),
    }
}

/// Chrome's and rinch's heights for one case, like with like.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Heights {
    /// Chrome's page height (`chrome.json`), rinch's card height (`rinch.json`).
    pub chrome_page: Option<f64>,
    pub rinch_page: Option<f64>,
    /// The last row of ink on each side.
    pub chrome_ink: Option<u32>,
    pub rinch_ink: Option<u32>,
    /// Chrome's screenshot hit [`TALLEST`](super::chrome::TALLEST): its ink
    /// row is the window's, not a height.
    pub chrome_capped: bool,
}

impl Heights {
    /// Read them from a case folder; a side that was not measured is `None`.
    pub fn read(dir: &Path) -> Heights {
        let json = |name: &str| {
            std::fs::read(dir.join(name))
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        };
        let chrome = json("chrome.json")
            .and_then(|v| serde_json::from_value::<super::chrome::ChromeJson>(v).ok());
        let rinch = json("rinch.json");
        Heights {
            chrome_page: chrome.and_then(|c| c.cleaned.height()),
            rinch_page: rinch.as_ref().and_then(|v| v["height"].as_f64()),
            chrome_ink: chrome.map(|c| c.cleaned.last_ink),
            chrome_capped: chrome.is_some_and(|c| c.cleaned.capped),
            rinch_ink: rinch
                .as_ref()
                .and_then(|v| v["last_ink"].as_u64())
                .map(|n| n as u32),
        }
    }

    /// rinch's height over Chrome's, as a signed share (+0.1: rinch is 10%
    /// taller), for the pair that was measured.
    pub fn page_diff(&self) -> Option<f64> {
        relative(self.rinch_page?, self.chrome_page?)
    }

    /// The same for the last rows of ink.
    pub fn ink_diff(&self) -> Option<f64> {
        if self.chrome_capped {
            return None;
        }
        relative(f64::from(self.rinch_ink?), f64::from(self.chrome_ink?))
    }

    /// How far apart the last ink rows are, as `distance`'s height term.
    pub fn ink_height_distance(&self) -> Option<f64> {
        if self.chrome_capped {
            return None;
        }
        let (a, b) = (f64::from(self.chrome_ink?), f64::from(self.rinch_ink?));
        Some(if a.max(b) > 0.0 {
            (a - b).abs() / a.max(b)
        } else {
            0.0
        })
    }

    /// The line the report shows: both comparisons, each labelled.
    pub fn describe(&self) -> String {
        let pair = |a: Option<f64>, b: Option<f64>, d: Option<f64>, what: &str| match (a, b, d) {
            (Some(a), Some(b), Some(d)) => {
                format!(
                    "{what}: Chrome {a:.0}px, rinch {b:.0}px ({:+.1}%)",
                    d * 100.0
                )
            }
            _ => format!("{what}: not measured"),
        };
        let ink = if self.chrome_capped {
            format!(
                "last ink row: Chrome's screenshot is capped at {}px, so not compared",
                super::chrome::TALLEST
            )
        } else {
            pair(
                self.chrome_ink.map(f64::from),
                self.rinch_ink.map(f64::from),
                self.ink_diff(),
                "last ink row",
            )
        };
        format!(
            "{} · {ink}",
            pair(
                self.chrome_page,
                self.rinch_page,
                self.page_diff(),
                "document height"
            ),
        )
    }
}

fn relative(rinch: f64, chrome: f64) -> Option<f64> {
    (chrome > 0.0).then(|| (rinch - chrome) / chrome)
}

struct Row {
    heights: Heights,
    dir: PathBuf,
    meta: Meta,
    cleaning: Distance,
    layout: Distance,
    cut_off: bool,
}

/// Write `out/report.html` over the case folders `dirs`.
pub fn report(out: &Path, dirs: &[PathBuf]) -> Result<PathBuf> {
    let mut rows = Vec::new();
    for dir in dirs {
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join("meta.json"))?)?;
        let open = |name: &str| image::open(dir.join(name)).ok();
        let (a, b, c) = (
            open("a-original.png"),
            open("b-cleaned.png"),
            open("c-rinch.png"),
        );
        let pair = |x: &Option<DynamicImage>, y: &Option<DynamicImage>| match (x, y) {
            (Some(x), Some(y)) => distance(x, y),
            _ => Distance {
                pixels: 1.0,
                height: 1.0,
            },
        };
        let cut_off = std::fs::read(dir.join("rinch.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| v["cut_off"].as_bool().unwrap_or(false));
        let heights = Heights::read(dir);
        let mut layout = pair(&b, &c);
        // b-cleaned.png is cut below its last ink; c-rinch.png is the whole
        // card. Their image heights are not alike, the last ink rows are.
        if let Some(d) = heights.ink_height_distance() {
            layout.height = d;
        }
        rows.push(Row {
            heights,
            dir: dir.clone(),
            meta,
            cleaning: pair(&a, &b),
            layout,
            cut_off,
        });
    }
    rows.sort_by(|x, y| {
        let worst = |r: &Row| r.cleaning.score().max(r.layout.score());
        worst(y).total_cmp(&worst(x))
    });
    let mut html = String::from(HEAD);
    for row in &rows {
        let name = row
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        html.push_str(&format!(
            r#"<section><h2>{name} · {title}</h2>
<p class="meta">{detail} · cleaning differs {cp:.0}% of pixels, height {ch:.0}% · rinch differs {lp:.0}% of pixels, height {lh:.0}% (last ink rows){cut}<br>{heights}</p>
<div class="cols">
<figure><figcaption>A. As written, in Chrome</figcaption><img src="{name}/a-original.png"></figure>
<figure><figcaption>B. Cleaned, in Chrome</figcaption><img src="{name}/b-cleaned.png"></figure>
<figure><figcaption>C. Cleaned, in rinch</figcaption><img src="{name}/c-rinch.png"></figure>
</div></section>
"#,
            title = escape(&row.meta.title),
            detail = escape(&row.meta.detail),
            cp = row.cleaning.pixels * 100.0,
            ch = row.cleaning.height * 100.0,
            lp = row.layout.pixels * 100.0,
            lh = row.layout.height * 100.0,
            heights = row.heights.describe(),
            cut = if row.cut_off {
                " · rinch's drawing is cut off at the probe's height"
            } else {
                ""
            },
        ));
    }
    html.push_str("</body></html>\n");
    let path = out.join("report.html");
    std::fs::write(&path, html)?;
    Ok(path)
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const HEAD: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>Rendering comparison</title>
<style>
body { margin: 16px; font: 14px/1.4 system-ui, sans-serif; background: #f4f5f7; color: #16181d; }
section { margin: 0 0 40px; }
h2 { font-size: 16px; margin: 0 0 4px; }
.meta { margin: 0 0 8px; color: #5b6170; font-size: 13px; }
.cols { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; align-items: start; }
figure { margin: 0; background: #ffffff; border: 1px solid #dfe2e7; }
figcaption { padding: 6px 8px; font-size: 12px; color: #3a3f4a; border-bottom: 1px solid #dfe2e7; }
img { display: block; width: 100%; height: auto; }
</style></head><body>
<h1>Rendering comparison</h1>
<p>Each document three ways, worst first. A against B is what cleaning loses; B against C is how rinch lays out the same HTML differently from Chrome.</p>
"#;

/// Write `out/index.html`, linking each width's report (`w<width>/report.html`).
pub fn index(out: &Path, widths: &[u32]) -> Result<PathBuf> {
    let mut html = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Rendering comparison</title>\
         <style>body { margin: 24px; font: 15px/1.5 system-ui, sans-serif; }</style></head><body>\
         <h1>Rendering comparison</h1><p>The same documents drawn at each width:</p><ul>",
    );
    for w in widths {
        html.push_str(&format!(
            "<li><a href=\"w{w}/report.html\">{w}px wide</a></li>"
        ));
    }
    html.push_str("</ul></body></html>\n");
    let path = out.join("index.html");
    std::fs::write(&path, html)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_with_like() {
        // Chrome: 1000px page, ink ends at 940 (so the png is 956 tall);
        // rinch: the same 1000px card, same ink row. The old comparison of
        // 956 against 1000 called rinch 4.4% taller; now it is 0.
        let h = Heights {
            chrome_page: Some(1000.0),
            rinch_page: Some(1000.0),
            chrome_ink: Some(940),
            rinch_ink: Some(940),
            chrome_capped: false,
        };
        assert_eq!(h.page_diff(), Some(0.0));
        assert_eq!(h.ink_diff(), Some(0.0));
        assert_eq!(h.ink_height_distance(), Some(0.0));
        let taller = Heights {
            rinch_page: Some(1100.0),
            rinch_ink: Some(1034),
            ..h
        };
        assert!((taller.page_diff().unwrap() - 0.1).abs() < 1e-9);
        assert!((taller.ink_diff().unwrap() - 94.0 / 940.0).abs() < 1e-9);
        let text = taller.describe();
        assert!(text.contains("document height: Chrome 1000px, rinch 1100px (+10.0%)"));
        assert!(text.contains("last ink row: Chrome 940px, rinch 1034px (+10.0%)"));
    }

    #[test]
    fn a_capped_screenshot_is_not_a_height() {
        let h = Heights {
            chrome_page: Some(5341.0),
            rinch_page: Some(5400.0),
            chrome_ink: Some(11999),
            rinch_ink: Some(5380),
            chrome_capped: true,
        };
        assert_eq!(h.ink_diff(), None);
        assert_eq!(h.ink_height_distance(), None);
        assert!(h.describe().contains("capped at 12000px"));
        assert!(h.page_diff().is_some());
    }

    #[test]
    fn unmeasured_side_is_said() {
        let h = Heights {
            rinch_page: Some(500.0),
            ..Heights::default()
        };
        assert_eq!(h.page_diff(), None);
        assert_eq!(h.ink_height_distance(), None);
        assert!(h.describe().contains("document height: not measured"));
    }

    #[test]
    fn reads_the_case_folder() {
        let dir = tempfile::tempdir().unwrap();
        let m = |page: f64, ink: u32| super::super::chrome::PageMeasure {
            page_height: Some(page),
            document_height: Some(page),
            viewport_height: Some(513.0),
            scroll_height: None,
            capped: false,
            trimmed_height: ink + 16,
            last_ink: ink,
        };
        std::fs::write(
            dir.path().join("chrome.json"),
            serde_json::to_vec(&super::super::chrome::ChromeJson {
                original: m(900.0, 800),
                cleaned: m(1000.0, 940),
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("rinch.json"),
            br#"{"height":1010.5,"last_ink":945,"cut_off":false}"#,
        )
        .unwrap();
        let h = Heights::read(dir.path());
        assert_eq!(h.chrome_page, Some(1000.0));
        assert_eq!(h.rinch_page, Some(1010.5));
        assert_eq!(h.chrome_ink, Some(940));
        assert_eq!(h.rinch_ink, Some(945));
        // An older case folder, without chrome.json or last_ink, still reads.
        let old = tempfile::tempdir().unwrap();
        std::fs::write(old.path().join("rinch.json"), br#"{"height":10.0}"#).unwrap();
        let h = Heights::read(old.path());
        assert_eq!((h.chrome_page, h.rinch_ink), (None, None));
    }
}
