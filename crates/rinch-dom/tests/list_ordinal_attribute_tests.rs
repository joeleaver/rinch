//! `<ol start>` and `<li value>` are read by HTML's rules for parsing integers
//! (#1138, #1153's sibling site).
//!
//! Desktop generates an ordered list's markers itself
//! (`resolve_list_marker` → `compute_list_item_counters`), and read both
//! attributes with `str::parse::<i32>`, which rejects leading whitespace and
//! anything after the digits. HTML skips leading ASCII whitespace, takes an
//! optional sign and the digits up to the first non-digit, and answers an
//! error for no digits or a value past `i32`. Chrome 153's reflected IDL
//! attributes, which are that parse:
//!
//! | attribute | `ol.start` / `li.value` in Chrome 153 | `parse::<i32>` |
//! |---|---|---|
//! | `"3abc"` | 3 | error |
//! | `" -01"` | -1 | error |
//! | `"2.5"` | 2 | error |
//! | `"\t7x"` | 7 | error |
//! | `"99999999999"` | error (`start` 1, `value` 0) | error |
//! | `"abc"` | error | error |
//!
//! An erroneous `start` is 1 and an erroneous `value` is ignored (the item is
//! numbered by its position), so the `99999999999` and `abc` rows pin that the
//! new reader still answers an error where the old one did.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::get_text_content;

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
    let e = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(e, k, v);
    }
    doc.append_child(parent, e);
    e
}

/// The number in an `<li>`'s generated marker (`"3.\u{2002}"` → `"3"`).
fn marker(doc: &RinchDocument, li: NodeId) -> String {
    let node = doc.tree.get(li.0).unwrap();
    let m = node
        .children
        .iter()
        .copied()
        .find(|&c| doc.tree.get(c).unwrap().is_pseudo_element)
        .expect("the <li> generates a marker");
    get_text_content(&doc.tree, m)
        .trim_end_matches(['\u{2002}', '.'])
        .to_string()
}

fn list(doc: &mut RinchDocument, start: Option<&str>, values: &[Option<&str>]) -> Vec<NodeId> {
    let body = doc.body();
    let attrs: Vec<(&str, &str)> = start.map(|s| ("start", s)).into_iter().collect();
    let ol = el(doc, body, "ol", &attrs);
    values
        .iter()
        .map(|v| {
            let attrs: Vec<(&str, &str)> = v.map(|v| ("value", v)).into_iter().collect();
            el(doc, ol, "li", &attrs)
        })
        .collect()
}

#[test]
fn ol_start_and_li_value_parse_like_html() {
    let mut doc = RinchDocument::new();
    let trailing = list(&mut doc, Some("3abc"), &[None, None]);
    let signed = list(&mut doc, Some(" -01"), &[None, None]);
    let overflow = list(&mut doc, Some("99999999999"), &[None]);
    let values = list(
        &mut doc,
        None,
        &[Some("2.5"), None, Some("abc"), Some("\t7x"), None],
    );
    doc.resolve_layout(800.0, 600.0);

    let read = |ids: &[NodeId]| ids.iter().map(|&li| marker(&doc, li)).collect::<Vec<_>>();
    assert_eq!(read(&trailing), ["3", "4"], "start=\"3abc\" is 3");
    assert_eq!(read(&signed), ["-1", "0"], "start=\" -01\" is -1");
    assert_eq!(read(&overflow), ["1"], "a start past i32 is an error: 1");
    assert_eq!(
        read(&values),
        ["2", "3", "4", "7", "8"],
        "value=\"2.5\" is 2, \"abc\" is ignored, \"\\t7x\" is 7"
    );
}

/// review #1167: the ordinal saturates at `i32::MAX` rather than overflowing
/// (a debug-build panic at 0707b561 for any `<li>` after one numbered
/// `i32::MAX`). Chrome 153, `list-style-position: inside` in monospace: every
/// marker after `2147483646` is 12 characters wide ("2147483647. "), never the
/// 13 of a wrapped "-2147483648. ".
#[test]
fn an_ordinal_at_i32_max_saturates_instead_of_overflowing() {
    let mut doc = RinchDocument::new();
    let from_start = list(&mut doc, Some("2147483646"), &[None, None, None]);
    let from_value = list(&mut doc, None, &[Some("2147483647"), None]);
    let from_min = list(&mut doc, Some("-2147483648"), &[None, None]);
    doc.resolve_layout(800.0, 600.0);
    let read = |ids: &[NodeId]| ids.iter().map(|&li| marker(&doc, li)).collect::<Vec<_>>();
    assert_eq!(
        read(&from_start),
        ["2147483646", "2147483647", "2147483647"]
    );
    assert_eq!(read(&from_value), ["2147483647", "2147483647"]);
    assert_eq!(read(&from_min), ["-2147483648", "-2147483647"]);
}
