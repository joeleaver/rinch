//! Tests of [`MockDomDocument`](super::MockDomDocument).

use super::*;
use crate::dom::{DomContent, DomFocus, DomTree};

/// Every structural mutation refuses a **discarded** child, not just
/// `append_child` (issue #184).
///
/// `NodeHandle::insert_after` routes to `insert_before` when the anchor has a
/// next sibling and to `append_child` when it does not, so a mock that guarded
/// only one of them would answer the *same* operation differently depending on
/// where in the list it landed — and would list an id that resolves to nothing,
/// hiding the caller's bug behind a plausible child count.
#[test]
fn a_discarded_child_is_refused_by_every_insertion_path() {
    type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
    let paths: [Relist; 3] = [
        |doc, parent, child, _anchor| doc.append_child(parent, child),
        |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
        |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
    ];
    for relist in paths {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let anchor = doc.create_element("div");
        doc.append_child(body, anchor);
        let child = doc.create_element("div");
        doc.append_child(body, child);

        doc.discard_node(child);
        relist(&mut doc, body, child, anchor);

        assert!(
            !doc.get_children(body).contains(&child),
            "#184: a discarded child must not be relisted under its parent"
        );
        for id in doc.get_children(body) {
            assert!(
                doc.tag_name(id).is_some(),
                "#184: every listed child must still resolve to a node"
            );
        }
    }
}

/// The other half: every one of those paths **accepts** a merely *removed*
/// child, because `remove_node` is a detach and the subtree must come back
/// (issue #719).
///
/// The pair is what makes this mock an oracle for both mistakes rather than
/// one. Sitting on `remove_node` alone — the shape before #719 — a mock that
/// retires everything and a mock that retires nothing are both "consistent";
/// only testing the two verbs against each other tells them apart.
#[test]
fn a_removed_child_is_accepted_by_every_insertion_path() {
    type Relist = fn(&mut MockDomDocument, NodeId, NodeId, NodeId);
    let paths: [Relist; 3] = [
        |doc, parent, child, _anchor| doc.append_child(parent, child),
        |doc, parent, child, anchor| doc.insert_before(parent, child, anchor),
        |doc, parent, child, _anchor| doc.insert_child(parent, child, 0),
    ];
    for relist in paths {
        let mut doc = MockDomDocument::new();
        let body = doc.body();
        let anchor = doc.create_element("div");
        doc.append_child(body, anchor);
        let child = doc.create_element("div");
        let grandchild = doc.create_element("span");
        doc.append_child(child, grandchild);
        doc.append_child(body, child);

        doc.remove_node(child);
        assert!(
            !doc.get_children(body).contains(&child),
            "precondition: remove_node unlinks the child"
        );

        relist(&mut doc, body, child, anchor);

        assert!(
            doc.get_children(body).contains(&child),
            "#719: a removed child must be re-insertable"
        );
        assert_eq!(
            doc.get_children(child),
            vec![grandchild],
            "#719: its subtree must come back with it"
        );
    }
}

/// `replace_node` **detaches** the node it displaced, like both real backends
/// (issue #719): it keeps its identity and its subtree, and can be inserted
/// again.
#[test]
fn replacing_a_node_detaches_it_and_keeps_its_subtree() {
    let mut doc = MockDomDocument::new();
    let body = doc.body();
    let old = doc.create_element("div");
    doc.append_child(body, old);
    let grandchild = doc.create_element("span");
    doc.append_child(old, grandchild);
    let new = doc.create_element("p");

    doc.replace_node(old, new);

    assert_eq!(doc.get_children(body), vec![new]);
    assert!(doc.tag_name(old).is_some(), "#719: `old` must stay live");
    assert_eq!(
        doc.parent_node(old),
        None,
        "#719: but it must be unlinked from its parent"
    );
    assert_eq!(
        doc.get_children(old),
        vec![grandchild],
        "#719: `old` keeps its own subtree"
    );

    doc.append_child(body, old);
    assert_eq!(
        doc.get_children(body),
        vec![new, old],
        "#719: a displaced node can be inserted again"
    );
}

/// `discard_node` is the route that retires: the subtree leaves the table
/// (issue #184).
#[test]
fn discarding_a_node_retires_it_and_its_subtree() {
    let mut doc = MockDomDocument::new();
    let body = doc.body();
    let old = doc.create_element("div");
    doc.append_child(body, old);
    let grandchild = doc.create_element("span");
    doc.append_child(old, grandchild);

    doc.discard_node(old);

    assert!(doc.get_children(body).is_empty());
    assert!(doc.tag_name(old).is_none(), "#184: `old` must be retired");
    assert!(
        doc.tag_name(grandchild).is_none(),
        "#184: `old`'s subtree must be retired with it"
    );
}

/// Replacing a node with itself is a no-op the browser accepts, so it must not
/// unlink a node that is still in the tree (issue #184).
#[test]
fn replacing_a_node_with_itself_does_not_unlink_it() {
    let mut doc = MockDomDocument::new();
    let body = doc.body();
    let node = doc.create_element("div");
    doc.append_child(body, node);

    doc.replace_node(node, node);

    assert_eq!(doc.get_children(body), vec![node]);
    assert_eq!(
        doc.parent_node(node),
        Some(body),
        "#184: a self-replace must leave the node in its parent"
    );
}

/// A retired id must not resurface from `take_dirty_nodes` — a consumer that
/// resolves what it is handed would find nothing there (issue #184).
#[test]
fn a_discarded_node_leaves_the_dirty_list() {
    let mut doc = MockDomDocument::new();
    let body = doc.body();
    let node = doc.create_element("div");
    doc.append_child(body, node);
    doc.set_attribute(node, "class", "x");

    doc.discard_node(node);

    assert!(
        !doc.take_dirty_nodes().contains(&node),
        "#184: a discarded node must not be reported dirty"
    );
}

/// A node marked twice is reported once, in the order first marked, and a
/// node reported once is reported again when marked after the take (#1214:
/// the list is backed by a set now, which a take must empty).
#[test]
fn dirty_nodes_are_reported_once_per_take() {
    let mut doc = MockDomDocument::new();
    let body = doc.body();
    let a = doc.create_element("div");
    let b = doc.create_element("div");
    doc.append_child(body, a);
    doc.append_child(body, b);
    doc.take_dirty_nodes();
    doc.set_attribute(b, "class", "x");
    doc.set_attribute(a, "class", "x");
    doc.set_attribute(b, "class", "y");
    assert_eq!(doc.take_dirty_nodes(), vec![b, a]);
    assert!(doc.take_dirty_nodes().is_empty());
    doc.set_attribute(a, "class", "z");
    assert_eq!(doc.take_dirty_nodes(), vec![a]);
}

// === set_style composes an inline style (#666) ===
//
// The mock is the oracle two other layers are tested against — `StyleProp`
// in this crate, and every component test that renders onto a
// `MockDomDocument` — so its `style` attribute has to be the string a real
// backend would leave. It used to be `push_str("{prop}: {value};")` with no
// separator and no replacement, which is neither parseable CSS nor
// anything `RinchDocument` or a browser would produce.

fn style_of(doc: &MockDomDocument, node: NodeId) -> String {
    doc.get_attribute(node, "style")
        .expect("a style was written")
}

/// A `set_style` onto an attribute somebody else wrote joins it with the
/// separator that makes it a second declaration.
///
/// Appending with none gave `"color: redpadding: 12px;"` — one declaration
/// whose value is `redpadding: 12px`.
#[test]
fn set_style_separates_itself_from_an_existing_declaration() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red");
    doc.set_style(div, "padding", "12px");
    assert_eq!(style_of(&doc, div), "color: red; padding: 12px");
}

/// Setting a property twice replaces it **where it stands**, which is what
/// CSSOM's `setProperty` does and what
/// `rinch-dom`'s `dom_tests::set_style_replaces_a_declaration_in_place`
/// pins for desktop. Appending instead left both declarations in the block.
///
/// The `gap` between them is load-bearing: with only the two `color`
/// declarations, replace-in-place and remove-then-append give the same
/// string, so the fixture would sit on a fixed point and pass either way.
#[test]
fn set_style_replaces_a_declaration_in_place() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_style(div, "color", "red");
    doc.set_style(div, "gap", "4px");
    doc.set_style(div, "color", "blue");
    assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
}

/// `property` is matched ASCII case-insensitively, so the mock composes an
/// inline style the way desktop and a browser do (#711). The `gap` is here
/// for the same reason as above — without it, folding and appending give
/// the same string.
///
/// Kills the mutant that drops `normalize_property_name` from
/// `MockDomDocument::set_style`.
#[test]
fn set_style_matches_a_property_name_case_insensitively() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "color: red; gap: 4px");
    doc.set_style(div, "COLOR", "blue");
    assert_eq!(style_of(&doc, div), "color: blue; gap: 4px");
}

/// …but a **custom** property is compared exactly, so these are two.
/// Chrome 150: `setProperty("--Foo", "3px")` on a block holding
/// `--foo: 2px` gives `cssText === "--foo: 2px; --Foo: 3px;"`.
///
/// Kills the mutant that lowercases unconditionally.
#[test]
fn set_style_keeps_two_custom_properties_that_differ_only_in_case() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "--foo: 2px");
    doc.set_style(div, "--Foo", "3px");
    assert_eq!(style_of(&doc, div), "--foo: 2px; --Foo: 3px");
}

/// And it goes through the workspace's one inline-style parser (#670), so a
/// `;` inside a `url()` already in the attribute is part of that value
/// here too — the mock diverging from desktop on this would make it an
/// oracle for the wrong thing.
#[test]
fn set_style_does_not_split_a_url_already_in_the_attribute() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "background-image: url(data:image/png;base64,AAA=)",
    );
    doc.set_style(div, "padding", "12px");
    assert_eq!(
        style_of(&doc, div),
        "background-image: url(data:image/png;base64,AAA=); padding: 12px"
    );
}

/// The first write onto a node with no style attribute is still just the
/// one declaration — no leading separator, no trailing `;`.
#[test]
fn the_first_set_style_writes_one_bare_declaration() {
    let mut doc = MockDomDocument::new();
    let div = doc.create_element("div");
    doc.set_style(div, "color", "red");
    assert_eq!(style_of(&doc, div), "color: red");
}

/// The mock models the web's split (issue #238): a user's typing moves the
/// live text and leaves the `value` attribute behind, and `live_value` is
/// what reports the field — the attribute is a fossil of the last write.
/// Removing a control's `value` attribute empties its live text, as the
/// web backend's `remove_value_attribute` does — not only the attribute.
#[test]
fn removing_the_value_attribute_empties_the_live_text() {
    let mut doc = MockDomDocument::new();
    let input = doc.create_element("input");
    doc.set_attribute(input, "value", "x");
    doc.__type_into(input, "typed");
    doc.remove_attribute(input, "value");
    assert_eq!(doc.live_value(input).as_deref(), Some(""));
}

#[test]
fn live_value_reports_typed_text_while_the_attribute_lags() {
    let mut doc = MockDomDocument::new();
    let input = doc.create_element("input");
    doc.set_attribute(input, "value", "mount");

    doc.__type_into(input, "typed");

    assert_eq!(doc.live_value(input).as_deref(), Some("typed"));
    assert_eq!(
        doc.get_attribute(input, "value").as_deref(),
        Some("mount"),
        "typing must not reach the attribute, or the split is not modelled"
    );

    // A programmatic write reaches both, as the web's reflected-property
    // mirror makes it.
    doc.set_attribute(input, "value", "written");
    assert_eq!(doc.live_value(input).as_deref(), Some("written"));
    assert_eq!(
        doc.get_attribute(input, "value").as_deref(),
        Some("written")
    );
}

/// A form control always has a live value — `""` before anything set one —
/// and an element that is not a form control answers from its attribute.
#[test]
fn a_pristine_textareas_live_value_is_its_text_children() {
    let mut doc = MockDomDocument::new();
    let textarea = doc.create_element("textarea");
    let a = doc.create_text("one ");
    let b = doc.create_text("two");
    doc.append_child(textarea, a);
    doc.append_child(textarea, b);
    assert_eq!(doc.live_value(textarea).as_deref(), Some("one two"));
    doc.set_attribute(textarea, "value", "");
    assert_eq!(doc.live_value(textarea).as_deref(), Some(""));

    let input = doc.create_element("input");
    let c = doc.create_text("child");
    doc.append_child(input, c);
    assert_eq!(doc.live_value(input).as_deref(), Some(""));
}

#[test]
fn live_value_of_an_empty_control_and_of_a_non_control() {
    let mut doc = MockDomDocument::new();
    let textarea = doc.create_element("textarea");
    let div = doc.create_element("div");
    assert_eq!(doc.live_value(textarea).as_deref(), Some(""));
    assert_eq!(doc.live_value(div), None);

    doc.set_attribute(div, "value", "attr");
    doc.__type_into(div, "ignored");
    assert_eq!(doc.live_value(div).as_deref(), Some("attr"));
}

/// `DomFocus::set_selection_range` records the normalized `(node,
/// start, end, direction)` exactly — issue #552's basic contract, which a
/// mutant swapping `start.min(end)`/`start.max(end)` (or dropping the
/// swap) would still pass if `start <= end` already, so the range here is
/// deliberately given reversed (`5, 1`) to prove the swap runs.
#[test]
fn set_selection_range_records_a_normalized_range() {
    let mut doc = MockDomDocument::new();
    let input = doc.create_element("input");
    doc.set_selection_range(input, 5, 1, SelectionDirection::Backward);
    assert_eq!(
        doc.__selection_range(),
        Some((input, 1, 5, SelectionDirection::Backward))
    );
}

/// `DomFocus::select_text`'s trait default is exactly
/// `set_selection_range(node, 0, <UTF-16 len>, Forward)` (issue #552) —
/// using a string with a surrogate pair (an emoji, 2 UTF-16 units) so a
/// mutant reporting the **byte** length instead would be caught: `"a🙂"`
/// is 2 chars / 5 bytes / 3 UTF-16 units.
#[test]
fn select_text_selects_the_full_utf16_length() {
    let mut doc = MockDomDocument::new();
    let input = doc.create_element("input");
    doc.set_attribute(input, "value", "a🙂");
    doc.select_text(input);
    assert_eq!(
        doc.__selection_range(),
        Some((input, 0, 3, SelectionDirection::Forward))
    );
}
