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
    doc.load_css(&format!("div {{ height: 1px }} .t {{ width: 100px }} {css}"));
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
