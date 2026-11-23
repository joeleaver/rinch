//! `prefers-reduced-motion` on desktop (#2209).
//!
//! Stylo's servo build has no `prefers-reduced-motion` media feature (only its
//! Gecko build does), so a query naming it failed to parse and became
//! `not all`: `(prefers-reduced-motion: reduce)` never matched, and neither did
//! `(prefers-reduced-motion: no-preference)` nor `not (… reduce)`, which every
//! browser matches when the OS asks for nothing. rinch now rewrites the feature
//! in each author sheet's `@media` preludes to a query Stylo evaluates to the
//! document's preference, and re-parses those sheets when it changes.
//!
//! Each fixture reads a width a media query sets: 7px when it matches, 100px
//! when it does not. Chrome's answers, OS reduced motion off: `no-preference`
//! and `not (… reduce)` match, `reduce` and the boolean form do not.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

fn doc_with(css: &str) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(&format!(
        "div {{ height: 1px }} .t {{ width: 100px }} {css}"
    ));
    let body = doc.body();
    let t = doc.create_element("div");
    doc.set_attribute(t, "class", "t");
    doc.append_child(body, t);
    doc.resolve_layout(800.0, 600.0);
    (doc, t)
}

fn width(doc: &RinchDocument, t: NodeId) -> f32 {
    doc.tree.get(t.0).unwrap().layout.width
}

/// What each query answers with no reduced-motion preference: Chrome's.
const NO_PREFERENCE: &[(&str, f32)] = &[
    ("(prefers-reduced-motion: no-preference)", 7.0),
    ("not (prefers-reduced-motion: reduce)", 7.0),
    ("(prefers-reduced-motion: reduce)", 100.0),
    ("(prefers-reduced-motion)", 100.0),
    ("not (prefers-reduced-motion: no-preference)", 100.0),
    ("screen and (prefers-reduced-motion: no-preference)", 7.0),
    ("(PREFERS-REDUCED-MOTION:NO-PREFERENCE)", 7.0),
];

#[test]
fn reduced_motion_queries_answer_as_chrome_does_with_no_preference() {
    for (query, want) in NO_PREFERENCE {
        let (doc, t) = doc_with(&format!("@media {query} {{ .t {{ width: 7px }} }}"));
        assert_eq!(
            width(&doc, t),
            *want,
            "#2209: `@media {query}` with no reduced-motion preference — the \
             servo build parsed none of these and answered `not all`"
        );
    }
}

/// The same queries with reduced motion on: every answer flips.
#[test]
fn reduced_motion_queries_answer_the_documents_preference() {
    for (query, no_pref) in NO_PREFERENCE {
        let want = if *no_pref == 7.0 { 100.0 } else { 7.0 };
        let mut doc = RinchDocument::new();
        doc.set_prefers_reduced_motion(true);
        doc.load_css(&format!(
            "div {{ height: 1px }} .t {{ width: 100px }} @media {query} {{ .t {{ width: 7px }} }}"
        ));
        let body = doc.body();
        let t = doc.create_element("div");
        doc.set_attribute(t, "class", "t");
        doc.append_child(body, t);
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(width(&doc, t), want, "`@media {query}`, reduced motion on");
    }
}

/// A change of preference after the sheets were parsed restyles the document,
/// at the same viewport, for every kind of author sheet: `load_css`, the theme
/// slot and a `<style>` element.
#[test]
fn a_change_of_preference_restyles_every_kind_of_sheet() {
    let mut doc = RinchDocument::new();
    doc.load_css("div { height: 1px } .a, .b, .c { width: 100px }");
    doc.load_css("@media (prefers-reduced-motion: reduce) { .a { width: 7px } }");
    // The theme slot cascades before the app's sheets: `!important` to win.
    doc.set_theme_css("@media (prefers-reduced-motion: reduce) { .b { width: 8px !important } }");
    let body = doc.body();
    let style = doc.create_element("style");
    let text = doc.create_text("@media (prefers-reduced-motion: reduce) { .c { width: 9px } }");
    doc.append_child(style, text);
    doc.append_child(body, style);
    let mut els = Vec::new();
    for class in ["a", "b", "c"] {
        let e = doc.create_element("div");
        doc.set_attribute(e, "class", class);
        doc.append_child(body, e);
        els.push(e);
    }
    let widths = |doc: &RinchDocument| els.iter().map(|&e| width(doc, e)).collect::<Vec<_>>();
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(widths(&doc), vec![100.0, 100.0, 100.0]);

    doc.set_prefers_reduced_motion(true);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        widths(&doc),
        vec![7.0, 8.0, 9.0],
        "turned on, every sheet answers it"
    );

    doc.set_prefers_reduced_motion(false);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(widths(&doc), vec![100.0, 100.0, 100.0], "and off again");
}

/// The theme's own rule: transitions collapse to 0.01ms under reduced motion.
#[test]
fn the_theme_rule_shortens_transitions_under_reduced_motion() {
    let css = "div { height: 1px; width: 10px; transition: width 300ms linear }
        div.wide { width: 50px }
        @media (prefers-reduced-motion: reduce) {
            * { animation-duration: 0.01ms !important; transition-duration: 0.01ms !important; }
        }";
    let longest = |reduce: bool| {
        let mut doc = RinchDocument::new();
        doc.set_prefers_reduced_motion(reduce);
        doc.load_css(css);
        let body = doc.body();
        let t = doc.create_element("div");
        doc.append_child(body, t);
        doc.resolve_layout(800.0, 600.0);
        doc.set_attribute(t, "class", "wide");
        doc.resolve_layout(800.0, 600.0);
        doc.tree
            .active_transitions
            .get(&t.0)
            .map(|ts| ts.values().map(|t| t.duration_ms).fold(0.0, f64::max))
            .unwrap_or(0.0)
    };
    assert!(
        (longest(false) - 300.0).abs() < 0.01,
        "no preference: the declared 300ms, got {}",
        longest(false)
    );
    assert!(
        longest(true) < 1.0,
        "reduced motion: the theme's 0.01ms, got {}",
        longest(true)
    );
}
