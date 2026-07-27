//! Browser-driven tests for `<style>` injection and theme updates (#155).
//!
//! These need a real `document.head`, which means a real browser. Run them with
//! a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant under test: theme updates rewrite *only* the theme element.
//! App CSS injected through `inject_style` must survive any number of theme
//! updates, because a theme update replaces the matched element's text wholesale.
#![cfg(target_arch = "wasm32")]

use rinch_web::WebDocument;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Remove every rinch-owned `<style>` so each test starts from a known head.
///
/// Both the theme element and any style this module injected are removed; the
/// harness's own page furniture is left alone.
fn reset_styles() {
    let doc = document();
    for selector in ["[data-rinch-theme]", "[data-test-app-css]"] {
        while let Ok(Some(el)) = doc.query_selector(selector) {
            el.remove();
        }
    }
}

/// The CSS text of every theme-marked `<style>` currently in the document.
fn theme_texts() -> Vec<String> {
    let doc = document();
    let list = doc.query_selector_all("[data-rinch-theme]").unwrap();
    (0..list.length())
        .filter_map(|i| list.item(i))
        .map(|n| n.text_content().unwrap_or_default())
        .collect()
}

/// Mark a style element so `reset_styles` can find it again.
fn tag_last_injected_style() -> web_sys::Element {
    let doc = document();
    let styles = doc.query_selector_all("head style").unwrap();
    let el: web_sys::Element = styles
        .item(styles.length() - 1)
        .unwrap()
        .dyn_into()
        .unwrap();
    el.set_attribute("data-test-app-css", "true").unwrap();
    el
}

/// The #155 regression: app CSS injected before any theme exists must not be
/// eaten by the first theme update.
///
/// Pre-fix this fails — `inject_style` stamped `data-rinch-theme` on the app
/// style, so it was the first (and only) match the theme upsert rewrote.
#[wasm_bindgen_test]
fn theme_update_does_not_clobber_injected_app_css() {
    reset_styles();
    let doc = WebDocument::new(document());

    doc.inject_style(".app-sentinel { color: rebeccapurple; }");
    let app_style = tag_last_injected_style();

    doc.update_theme_style(":root { --rinch-primary-color: blue; }");

    assert_eq!(
        app_style.text_content().unwrap_or_default(),
        ".app-sentinel { color: rebeccapurple; }",
        "theme update overwrote app CSS injected via inject_style"
    );
    assert_eq!(
        theme_texts(),
        vec![":root { --rinch-primary-color: blue; }".to_string()],
        "theme CSS should live in its own element"
    );

    reset_styles();
}

/// A dark-mode toggle is a *sequence* of theme updates; app CSS survives all of
/// them, and the theme element is updated in place rather than duplicated.
#[wasm_bindgen_test]
fn repeated_theme_updates_upsert_in_place() {
    reset_styles();
    let doc = WebDocument::new(document());

    doc.update_theme_style(":root { --mode: light; }");
    doc.inject_style(".app-sentinel { color: teal; }");
    let app_style = tag_last_injected_style();

    doc.update_theme_style(":root { --mode: dark; }");
    doc.update_theme_style(":root { --mode: light; }");

    assert_eq!(
        theme_texts(),
        vec![":root { --mode: light; }".to_string()],
        "theme updates must upsert one element, not append new ones"
    );
    assert_eq!(
        app_style.text_content().unwrap_or_default(),
        ".app-sentinel { color: teal; }",
        "app CSS must survive repeated theme updates"
    );

    reset_styles();
}

/// `inject_style` appends: two app stylesheets coexist, and neither is a theme.
#[wasm_bindgen_test]
fn inject_style_appends_unmarked_elements() {
    reset_styles();
    let doc = WebDocument::new(document());

    doc.inject_style(".first { color: red; }");
    let first = tag_last_injected_style();
    doc.inject_style(".second { color: green; }");
    let second = tag_last_injected_style();

    assert_eq!(
        first.text_content().unwrap_or_default(),
        ".first { color: red; }"
    );
    assert_eq!(
        second.text_content().unwrap_or_default(),
        ".second { color: green; }"
    );
    assert!(
        theme_texts().is_empty(),
        "inject_style must not mark app CSS as theme CSS"
    );

    reset_styles();
}
