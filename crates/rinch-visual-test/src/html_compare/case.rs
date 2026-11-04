//! A case folder: one document, as written and as cleaned.

use std::path::Path;

use rinch_html::Cleaned;
use serde::{Deserialize, Serialize};

use super::{Result, Setup, frame_css};

/// What the report says about a case.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    /// Its heading in the report.
    pub title: String,
    /// A line under it (a sender, an id).
    pub detail: String,
}

/// The cleaned document, as the probe reads it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanedCase {
    pub html: String,
    pub body_style: String,
}

/// Write a case into `dir`: `original` as written, and `cleaned`.
pub fn write(
    dir: &Path,
    meta: &Meta,
    original: &str,
    cleaned: &Cleaned,
    setup: &Setup,
) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(meta)?)?;
    std::fs::write(dir.join("original.html"), original)?;
    std::fs::write(dir.join("cleaned.html"), cleaned_page(cleaned, setup))?;
    let probe = CleanedCase {
        html: cleaned.html.clone(),
        body_style: cleaned.body_style.clone(),
    };
    std::fs::write(dir.join("cleaned.json"), serde_json::to_vec(&probe)?)?;
    Ok(())
}

/// The cleaned document as a page for Chrome, in the same frame the probe
/// draws (`HtmlView`'s markup inside `.html-compare-host`), so the two get
/// the same input and differ only in how they lay it out.
fn cleaned_page(cleaned: &Cleaned, setup: &Setup) -> String {
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><style>
{chrome}
{frame}
</style></head><body><div class="html-compare-host"><div class="rinch-html-view"><div class="rinch-html-body" style="{style}">{html}</div></div></div></body></html>"#,
        chrome = setup.chrome_css,
        frame = frame_css(setup),
        style = cleaned.body_style.replace('"', "&quot;"),
        html = cleaned.html,
    )
}
