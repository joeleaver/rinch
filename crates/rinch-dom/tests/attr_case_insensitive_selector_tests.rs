//! #682 — `[attr=value i]` must match case-insensitively on desktop.
//!
//! **Already fixed before this file.** `RinchNode::attr_matches` used to
//! destructure `AttrSelectorOperation::WithValue { case_sensitivity: _, .. }`
//! and drop the flag, then compare with plain `==`/`starts_with`/`contains`/
//! `ends_with` — always case-sensitive, whatever the selector asked for.
//! `operation.eval_str(attr_value)` — the selectors crate's own evaluator,
//! the same one `ServoElementSnapshot::attr_matches` uses for invalidation,
//! so an element and its snapshot can never disagree — now threads every
//! case flag through for all six operators, `Exists` included. Re-verified
//! at HEAD (`d25bce36`): the mechanism is already in place, so this file is
//! the pin the issue asked for, not a fix.
//!
//! `* { margin-bottom: 17px }` is the positive control, matching the issue's
//! own measured table exactly.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const VW: f32 = 800.0;
const VH: f32 = 600.0;

fn margin_top(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree.get(id).unwrap().computed_style.margin_top.to_px()
}

fn margin_bottom(doc: &RinchDocument, id: usize) -> f32 {
    doc.tree
        .get(id)
        .unwrap()
        .computed_style
        .margin_bottom
        .to_px()
}

/// The issue's own case: `[data-x="BAR" i]` against `data-x="bar"`.
#[test]
fn case_insensitive_attribute_value_selector_matches() {
    let mut doc = RinchDocument::new();
    doc.load_css(r#"[data-x="BAR" i] { margin-top: 11px; } * { margin-bottom: 17px; }"#);
    let body = doc.body();

    let n = doc.create_element("div");
    doc.set_attribute(n, "data-x", "bar");
    doc.append_child(body, n);

    doc.resolve_layout(VW, VH);

    assert_eq!(margin_bottom(&doc, n.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, n.0),
        11.0,
        "#682: the `i` flag must make the value match case-insensitively"
    );
}

/// The behaviour that must NOT regress: no flag (or the explicit `s` flag)
/// stays case-sensitive.
#[test]
fn case_sensitive_attribute_value_selector_still_refuses_a_different_case() {
    let mut doc = RinchDocument::new();
    doc.load_css(r#"[data-x="BAR"] { margin-top: 11px; } * { margin-bottom: 17px; }"#);
    let body = doc.body();

    let n = doc.create_element("div");
    doc.set_attribute(n, "data-x", "bar");
    doc.append_child(body, n);

    doc.resolve_layout(VW, VH);

    assert_eq!(margin_bottom(&doc, n.0), 17.0, "positive control");
    assert_eq!(
        margin_top(&doc, n.0),
        0.0,
        "no case flag: the selector's exact-case value must not match a \
         different case"
    );
}

/// The other five operators: `~=`, `|=`, `^=`, `$=`, `*=`, each with `i`.
#[test]
fn every_operator_honours_the_case_insensitive_flag() {
    let mut doc = RinchDocument::new();
    doc.load_css(
        r#"
        [data-tokens~="BAR" i] { margin-top: 1px; }
        [data-lang|="EN" i] { margin-top: 2px; }
        [data-starts^="FOO" i] { margin-top: 3px; }
        [data-ends$="BAZ" i] { margin-top: 4px; }
        [data-has*="IDD" i] { margin-top: 5px; }
        * { margin-bottom: 17px; }
        "#,
    );
    let body = doc.body();

    let tokens = doc.create_element("div");
    doc.set_attribute(tokens, "data-tokens", "foo bar baz");
    doc.append_child(body, tokens);

    let lang = doc.create_element("div");
    doc.set_attribute(lang, "data-lang", "en-US");
    doc.append_child(body, lang);

    let starts = doc.create_element("div");
    doc.set_attribute(starts, "data-starts", "foobar");
    doc.append_child(body, starts);

    let ends = doc.create_element("div");
    doc.set_attribute(ends, "data-ends", "foobaz");
    doc.append_child(body, ends);

    let has = doc.create_element("div");
    doc.set_attribute(has, "data-has", "mididdle");
    doc.append_child(body, has);

    doc.resolve_layout(VW, VH);

    assert_eq!(
        (
            margin_bottom(&doc, tokens.0),
            margin_bottom(&doc, lang.0),
            margin_bottom(&doc, starts.0),
            margin_bottom(&doc, ends.0),
            margin_bottom(&doc, has.0),
        ),
        (17.0, 17.0, 17.0, 17.0, 17.0),
        "positive control: the universal rule must land on all five"
    );
    assert_eq!(
        (
            margin_top(&doc, tokens.0),
            margin_top(&doc, lang.0),
            margin_top(&doc, starts.0),
            margin_top(&doc, ends.0),
            margin_top(&doc, has.0),
        ),
        (1.0, 2.0, 3.0, 4.0, 5.0),
        "every attribute-selector operator must honour `i`"
    );
}
