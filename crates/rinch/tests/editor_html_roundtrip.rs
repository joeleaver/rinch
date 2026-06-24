//! Rich-text editor HTML round-trip (replaces the old `ce_link_roundtrip`).
//!
//! Loads HTML containing a link and inline marks through the public
//! `EditorHandle::load_html` path, then re-serializes the resulting model and
//! asserts the link `href` and the marks survive — the regression that the old
//! CE link round-trip guarded, now expressed against the model-first editor.
//! (Exhaustive node/mark serialization is covered in `rinch-editor-core`.)
#![cfg(feature = "desktop")]

use rinch::prelude::*;
use rinch_editor_core::serialize::node_to_html;

#[test]
fn load_html_preserves_link_and_marks_round_trip() {
    let editor = create_editor();
    assert!(editor.load_html(
        "<p>see <a href=\"https://example.com\">the link</a> and <strong>bold</strong> text</p>"
    ));

    let html = node_to_html(&editor.doc());

    assert!(
        html.contains("href=\"https://example.com\""),
        "link href must survive the round-trip, got: {html}"
    );
    assert!(
        html.contains("the link"),
        "link text must survive, got: {html}"
    );
    assert!(
        html.contains("<strong>bold</strong>"),
        "bold mark must survive, got: {html}"
    );
}

#[test]
fn load_html_sanitizes_javascript_url() {
    // Schema-whitelisted paste: a javascript: URL must not become a live link.
    let editor = create_editor();
    editor.load_html("<p><a href=\"javascript:alert(1)\">x</a></p>");
    let html = node_to_html(&editor.doc());
    assert!(
        !html.contains("javascript:"),
        "javascript: URLs must be dropped, got: {html}"
    );
}
