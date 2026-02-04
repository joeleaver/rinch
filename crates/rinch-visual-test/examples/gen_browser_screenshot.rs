//! Generate a browser screenshot from a Rinch DOM dump.
//!
//! Usage:
//!   cargo run -p rinch-visual-test --example gen_browser_screenshot
//!
//! Reads a DOM tree from `/tmp/rinch_dom.json`, converts it to HTML/CSS,
//! renders it in Chromium via Playwright, and saves the screenshot.

use std::path::PathBuf;

fn main() {
    let dom_str = std::fs::read_to_string("/tmp/rinch_dom.json").expect("read dom");
    let dom: serde_json::Value = serde_json::from_str(&dom_str).expect("parse");

    let config = rinch_visual_test::html_serializer::HtmlConfig {
        viewport_width: 1200,
        viewport_height: 800,
        background_color: "#1a1b1e".to_string(),
    };

    let html = rinch_visual_test::html_serializer::serialize_to_html(&dom, &config);
    std::fs::write("/tmp/rinch_exported.html", &html).expect("write html");
    eprintln!("HTML written to /tmp/rinch_exported.html");

    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
    let browser = rinch_visual_test::browser::BrowserCapture::new(&scripts).expect("browser");
    let png = browser.capture_html(&html, 1200, 800).expect("capture");
    std::fs::write("/tmp/browser_screenshot.png", &png).expect("write png");
    eprintln!("Browser screenshot written to /tmp/browser_screenshot.png");
}
