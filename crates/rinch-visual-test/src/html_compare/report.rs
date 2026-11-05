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

struct Row {
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
        rows.push(Row {
            dir: dir.clone(),
            meta,
            cleaning: pair(&a, &b),
            layout: pair(&b, &c),
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
<p class="meta">{detail} · cleaning differs {cp:.0}% of pixels, height {ch:.0}% · rinch differs {lp:.0}% of pixels, height {lh:.0}%{cut}</p>
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
