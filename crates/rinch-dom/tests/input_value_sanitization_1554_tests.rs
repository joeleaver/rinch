//! #1554 — an `<input>`'s value is its `value` attribute **sanitized** by
//! HTML's value sanitization algorithm for its type, as a browser's `.value`
//! is. The attribute itself keeps what was written.
//!
//! Chrome 155, measured (`setAttribute("value", v)` then `.value`, headless):
//!
//! | type                         | `v`                     | `.value`        |
//! |------------------------------|-------------------------|-----------------|
//! | text, none, `TEXT`, unknown  | `"p\nq\r\nr"`           | `"pqr"`         |
//! | text                         | `" a \t "`              | `" a \t "`      |
//! | search / tel / password      | `"a\nb"` / `"a\rb"`     | `"ab"`          |
//! | url                          | `"  http://x\n/y  "`    | `"http://x/y"`  |
//! | url                          | `"\t a b \f"`           | `"a b"`         |
//! | url                          | `"a\u{a0}"`             | `"a\u{a0}"`     |
//! | email                        | `"\f a@b,  c@d \t"`     | `"a@b,  c@d"`   |
//! | email multiple               | `" a@b ,\n c@d ,, "`    | `"a@b,c@d,,"`   |
//! | `" email"` (an unknown type) | `" a "`                 | `" a "`         |
//! | hidden / checkbox            | `"a\nb"`                | `"a\nb"`        |
//!
//! `getAttribute("value")` answers `v` unchanged in every row. A `<textarea>`
//! keeps its line breaks.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::form_control::control_value;

fn input(doc: &mut RinchDocument, ty: Option<&str>, value: &str) -> NodeId {
    let body = doc.body();
    let n = doc.create_element("input");
    if let Some(ty) = ty {
        doc.set_attribute(n, "type", ty);
    }
    doc.set_attribute(n, "value", value);
    doc.append_child(body, n);
    n
}

fn value_of(doc: &RinchDocument, n: NodeId) -> Option<String> {
    control_value(&doc.tree.nodes, n.0).map(|v| v.into_owned())
}

#[track_caller]
fn check(ty: Option<&str>, written: &str, sanitized: &str) {
    let mut doc = RinchDocument::new();
    let n = input(&mut doc, ty, written);
    assert_eq!(
        value_of(&doc, n).as_deref(),
        Some(sanitized),
        "control_value of type {ty:?} written {written:?}"
    );
    assert_eq!(
        doc.live_value(n).as_deref(),
        Some(sanitized),
        "live_value of type {ty:?} written {written:?}"
    );
    assert_eq!(
        doc.get_attribute(n, "value").as_deref(),
        Some(written),
        "the attribute keeps what was written"
    );
}

#[test]
fn the_text_states_strip_line_breaks() {
    check(None, "p\nq\r\nr", "pqr");
    check(Some("text"), "p\nq\r\nr", "pqr");
    check(Some("TEXT"), "a\nb", "ab");
    check(Some("search"), "a\nb", "ab");
    check(Some("tel"), "a\rb", "ab");
    check(Some("password"), "a\nb", "ab");
    // An unknown type is the Text state: line breaks go, spaces stay.
    check(Some("bogus"), " a\nb ", " ab ");
    // Not a type a browser trims: `" email"` is an unknown type.
    check(Some(" email"), " a ", " a ");
}

#[test]
fn the_text_state_keeps_every_other_whitespace() {
    check(Some("text"), " a \t ", " a \t ");
    check(Some("text"), "a\tb\u{c}c", "a\tb\u{c}c");
}

#[test]
fn url_and_email_also_trim_ascii_whitespace() {
    check(Some("url"), "  http://x\n/y  ", "http://x/y");
    check(Some("url"), "\t a b \u{c}", "a b");
    check(Some("url"), "a\u{a0}", "a\u{a0}");
    // `type` is ASCII case-insensitive.
    check(Some("URL"), " a ", "a");
    check(Some("email"), " a@b \n", "a@b");
    check(Some("email"), "\u{c} a@b,  c@d \t", "a@b,  c@d");
}

#[test]
fn a_multiple_email_trims_each_address() {
    let mut doc = RinchDocument::new();
    let n = input(&mut doc, Some("email"), " a@b ,\n c@d ,, ");
    assert_eq!(value_of(&doc, n).as_deref(), Some("a@b , c@d ,,"));
    doc.set_attribute(n, "multiple", "");
    assert_eq!(value_of(&doc, n).as_deref(), Some("a@b,c@d,,"));
}

#[test]
fn hidden_checkbox_and_textarea_keep_line_breaks() {
    check(Some("hidden"), "a\nb", "a\nb");
    check(Some("checkbox"), " a\nb", " a\nb");

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let t = doc.create_element("textarea");
    doc.set_attribute(t, "value", "a\r\nb");
    doc.append_child(body, t);
    assert_eq!(value_of(&doc, t).as_deref(), Some("a\r\nb"));
}

/// `:placeholder-shown` reads the value, so a field whose only content is a
/// line break shows its placeholder (Chrome 155: `value="\n"` matches).
#[test]
fn a_value_of_only_line_breaks_shows_the_placeholder() {
    let mut doc = RinchDocument::new();
    doc.load_css("input:placeholder-shown { margin-top: 11px; } * { margin-bottom: 17px; }");
    let n = input(&mut doc, None, "x");
    doc.set_attribute(n, "placeholder", "hi");
    doc.resolve_layout(800.0, 600.0);
    let top = |doc: &RinchDocument| doc.tree.get(n.0).unwrap().computed_style.margin_top.to_px();
    assert_eq!(top(&doc), 0.0, "positive control: a value hides it");
    doc.set_attribute(n, "value", "\r\n");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(top(&doc), 11.0);
}
