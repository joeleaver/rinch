//! HTML documents drawn three ways, side by side: as written, in Chrome
//! (**A**); cleaned (`rinch_html::clean`), in Chrome (**B**); cleaned, in
//! rinch's `HtmlView` (**C**). A against B is what cleaning loses; B against
//! C is where rinch lays the same HTML out differently from a browser.
//!
//! Each document is a *case*: a folder with `meta.json`, `original.html`,
//! `cleaned.json` (what the probe shows) and `cleaned.html` (the same, as a
//! page for Chrome), written by [`case::write`]. Then:
//!
//! 1. [`chrome::case`] screenshots A and B with headless Chrome (in a
//!    profile of its own; `CHROME` names the binary, `google-chrome` by
//!    default);
//! 2. [`capture::capture`] starts a probe window ([`probe::run`], in a
//!    process the caller names, usually its own executable) and drives it
//!    over rinch's debug server, stitching each document's screenshots;
//! 3. [`report::report`] writes `report.html`, worst first.
//!
//! **Heights.** `b-cleaned.png` is cut 16px below the last row that differs
//! from the background, so its height is "last ink + 16", not the
//! document's. Compare like with like: [`chrome::case`] writes `chrome.json`
//! (Chrome's `page_height`, the content's real extent after fonts, pictures
//! and two frames: the largest of the `scrollHeight`s of the scrolling element,
//! `<html>` and `<body>`, the bottom of every laid-out box, and the content of
//! every `overflow: auto|scroll` element, so a viewport-high scroller does not
//! hide it; `capped` when the screenshot hit [`chrome::TALLEST`]; the `trimmed_height` of the png; the
//! `last_ink` row), [`capture::capture`] writes `rinch.json` with the card's
//! `height` and its `last_ink` row in `c-rinch.png`, and the report sets
//! Chrome's page height against rinch's `height` and the two `last_ink` rows
//! against each other, each labelled. A case folder without these files
//! shows "not measured". Chrome's copy of a page keeps a leading `<!doctype>`
//! first (the policy `<meta>` goes after it), so pages render in standards
//! mode; earlier runs put the meta first, which made every page quirks mode,
//! so their `b-cleaned.png` baselines are not comparable. `chrome::case`'s signature is unchanged;
//! [`chrome::screenshot_measured`] is the same step returning the measure.
//!
//! The `html-compare` binary runs all of it on plain HTML files. An app that
//! shows HTML in its own frame (a mail reader's card, its fonts) passes a
//! [`Setup`] so the comparison draws as the app does.

pub mod capture;
pub mod case;
pub mod chrome;
pub mod probe;
pub mod report;

/// The errors of a comparison step, for a command-line tool to print.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How documents are drawn, in Chrome and in the probe alike.
#[derive(Debug, Clone)]
pub struct Setup {
    /// The width of the element holding a document, in CSS px.
    pub width: u32,
    /// CSS for that element, `.html-compare-host`, and anything inside it:
    /// the app's frame around a document (its padding, background, font).
    pub host_css: String,
    /// CSS only Chrome gets, before `host_css`: `@font-face` rules for the
    /// app's fonts, so Chrome draws in the faces the probe was given.
    pub chrome_css: String,
    /// The probe window's fonts ([`rinch::AppFont`]).
    pub fonts: &'static [rinch::prelude::AppFont],
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            width: 720,
            host_css: ".html-compare-host { padding: 12px; background: #ffffff; color: #222222; font-family: sans-serif; line-height: 1.4; }".into(),
            chrome_css: String::new(),
            fonts: &[],
        }
    }
}

/// The CSS every page and the probe share: the host at its width.
pub(crate) fn frame_css(setup: &Setup) -> String {
    format!(
        "html, body {{ margin: 0; background: #ffffff; }}\n{}\n.html-compare-host {{ width: {}px; box-sizing: border-box; }}",
        setup.host_css, setup.width
    )
}

/// The case folders in `out`, in order.
pub fn case_dirs(out: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut dirs: Vec<std::path::PathBuf> = std::fs::read_dir(out)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("meta.json").exists())
        .collect();
    dirs.sort();
    Ok(dirs)
}
