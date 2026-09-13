//! What desktop makes of a boolean attribute that is *present but "false"*
//! (issue #551) — the half that says why the writer has to **remove**.
//!
//! `rsx!` used to render a reactive `bool` through `Display`, so a false one
//! wrote `checked="false"` / `selected="false"`. Three desktop readers take
//! presence alone:
//!
//! | reader | site |
//! |---|---|
//! | CSS `:checked` | `stylo_impl.rs`, `NonTSPseudoClass::Checked` |
//! | an `<option>`'s selectedness | `select.rs`, `collect_options` |
//! | an `<option>`'s disabledness | the same |
//!
//! and all three are **right** to: in HTML a present boolean attribute is true
//! whatever it holds, so a browser matches `:checked` on
//! `<input type=checkbox checked="false">` too — measured in Chrome by
//! `html_reads_a_present_boolean_attribute_as_true_whatever_its_value`
//! (`crates/rinch-web/tests/boolean_attributes.rs`). So the cure is a writer that
//! removes the attribute, not a reader that learns a falsey string; these tests
//! exist so that a later "tolerate `false` here too" cannot be mistaken for the
//! fix.
//!
//! This is also the measurement that corrected the issue's own framing. #551 was
//! read off the code and filed as web-only, on the strength of desktop reading
//! `disabled` with a `"false"` escape. That escape was real but **local to the
//! disabled family** — it never reached `:checked` or the `<select>` model, so
//! #551 reproduced on the desktop backend too.
//!
//! It has since been retired from the HTML attributes altogether (issue #612):
//! `disabled` and `readonly` are read by presence alone, so the readers below are
//! no longer the odd ones out — they are the rule, and
//! `computed_style_tests::the_disabled_selector_follows_the_boolean_attribute_rule`
//! is where `:disabled` says the same. Only rinch's own `data-disabled` /
//! `data-nofocus` keep an escape, because both backends implement it on purpose.
//!
//! Each test asserts *both* directions on purpose: at scroll-offset-zero-style
//! fixed points a broken and a fixed reader agree, and "present means on" is only
//! evidence about a reader if "absent means off" is checked beside it.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// `:checked` matches on presence — including the `"false"` the old writer
/// produced, and not once the attribute is gone.
#[test]
fn the_checked_selector_matches_on_presence_alone() {
    let mut doc = RinchDocument::new();
    doc.load_css("input:checked { opacity: 0.5 }");
    let mk = |doc: &mut RinchDocument, attr: Option<&str>| {
        let el = doc.create_element("input");
        doc.set_attribute(el, "style", "width: 80px; height: 30px");
        doc.set_attribute(el, "type", "checkbox");
        if let Some(v) = attr {
            doc.set_attribute(el, "checked", v);
        }
        let body = doc.body();
        doc.append_child(body, el);
        el
    };
    let absent = mk(&mut doc, None);
    let presence = mk(&mut doc, Some(""));
    let truthy = mk(&mut doc, Some("true"));
    let falsey_string = mk(&mut doc, Some("false"));
    doc.resolve_layout(800.0, 600.0);
    let opacity =
        |doc: &RinchDocument, id: NodeId| doc.tree.get(id.0).unwrap().computed_style.opacity;

    assert_eq!(opacity(&doc, absent), 1.0, "no attribute, no match");
    assert_eq!(opacity(&doc, presence), 0.5, "the presence form matches");
    assert_eq!(opacity(&doc, truthy), 0.5, "so does any other value");
    assert_eq!(
        opacity(&doc, falsey_string),
        0.5,
        "`checked=\"false\"` matches too — which is why the writer must remove \
         the attribute rather than write that string (#551)"
    );

    // And the other direction: removing it really does clear the match, so the
    // writer's `remove_attribute` is enough on its own.
    doc.remove_attribute(falsey_string, "checked");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(opacity(&doc, falsey_string), 1.0, "removal clears :checked");
}

fn select_with_options(attrs: &[(&str, Option<(&str, &str)>)]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sel = doc.create_element("select");
    doc.append_child(body, sel);
    for (value, attr) in attrs {
        let o = doc.create_element("option");
        doc.set_attribute(o, "value", value);
        if let Some((k, v)) = attr {
            doc.set_attribute(o, k, v);
        }
        doc.append_child(sel, o);
    }
    (doc, sel)
}

/// An `<option>`'s `selected` is read by presence, `"false"` included.
#[test]
fn an_option_is_selected_by_the_presence_of_the_attribute() {
    let (doc, sel) = select_with_options(&[("v0", None), ("v1", Some(("selected", "false")))]);
    let model = rinch_dom::select::resolve_select_model(&doc.tree, sel.0);
    assert_eq!(
        model.selected_index,
        Some(1),
        "`selected=\"false\"` selects the second option — a reactive `selected:` \
         that wrote that string could never move the selection back (#551)"
    );

    let (mut doc, sel) = select_with_options(&[("v0", None), ("v1", Some(("selected", "false")))]);
    let option_v1 = doc.tree.get(sel.0).unwrap().children[1];
    doc.remove_attribute(NodeId(option_v1), "selected");
    let model = rinch_dom::select::resolve_select_model(&doc.tree, sel.0);
    assert_eq!(
        model.selected_index,
        Some(0),
        "removing it falls back to the first option"
    );
}

/// And an `<option>`'s `disabled`. `collect_options` reads the attribute itself
/// rather than going through `node_is_disabled`, and since #612 the two agree:
/// presence alone, whatever the string.
#[test]
fn an_option_is_disabled_by_the_presence_of_the_attribute() {
    let (doc, sel) = select_with_options(&[("v0", Some(("disabled", "false"))), ("v1", None)]);
    let model = rinch_dom::select::resolve_select_model(&doc.tree, sel.0);
    assert_eq!(
        model.selected_index,
        Some(1),
        "`disabled=\"false\"` skips the first option: presence is the whole \
         value, here and in `node_is_disabled` (#612)"
    );

    let (mut doc, sel) = select_with_options(&[("v0", Some(("disabled", "false"))), ("v1", None)]);
    let option_v0 = doc.tree.get(sel.0).unwrap().children[0];
    doc.remove_attribute(NodeId(option_v0), "disabled");
    let model = rinch_dom::select::resolve_select_model(&doc.tree, sel.0);
    assert_eq!(
        model.selected_index,
        Some(0),
        "removing it makes the first option selectable again"
    );
}
