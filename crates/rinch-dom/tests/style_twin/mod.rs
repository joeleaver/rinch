//! The twin-document oracle for incremental restyling.
//!
//! An incremental restyle is correct when the document it leaves is the one a
//! fresh document built directly in the final state computes. So each fixture
//! builds the same markup twice — once in its initial state and mutated, once
//! already in the final state — lays both out, and compares **every node**:
//!
//! - every Stylo longhand of the element's primary style, serialized
//!   (`ComputedValues::computed_value_to_string`), plus its custom properties;
//! - rinch's own `ComputedStyle` (what layout and paint actually read), as
//!   JSON;
//! - the generated `::before` / `::after` / list-marker nodes and their text;
//! - the layout box (outside `display: none` subtrees, where it means
//!   nothing).
//!
//! The walk pairs nodes by tree position, so the two documents must have the
//! same shape; a fixture that builds different shapes fails loudly on the
//! child count.
#![allow(dead_code)]

use rinch_dom::RinchDocument;
use style::properties::{ComputedValues, LonghandId, PropertyDeclarationId, ShorthandId};

/// Every longhand this Stylo build knows: `all` covers all of them but the
/// two it deliberately leaves out.
fn longhands() -> Vec<LonghandId> {
    let mut v: Vec<LonghandId> = ShorthandId::All.longhands().collect();
    v.push(LonghandId::Direction);
    v.push(LonghandId::UnicodeBidi);
    v
}

/// One element's Stylo primary style as text, one `name: value` per longhand.
pub fn stylo_fingerprint(cv: &ComputedValues) -> String {
    let mut out = String::new();
    for id in longhands() {
        let v = cv.computed_value_to_string(PropertyDeclarationId::Longhand(id));
        out.push_str(id.name());
        out.push_str(": ");
        out.push_str(&v);
        out.push('\n');
    }
    let custom = cv.custom_properties();
    let mut vars: Vec<String> = custom
        .inherited
        .iter()
        .chain(custom.non_inherited.iter())
        .map(|(k, v)| {
            // The value's own `css` text, out of its Debug form (the type
            // exposes no accessor for it).
            let dbg = format!("{v:?}");
            let css = dbg
                .split_once("css: \"")
                .and_then(|(_, rest)| rest.split_once('"'))
                .map_or(dbg.as_str(), |(css, _)| css)
                .to_string();
            format!("--{k}={css}")
        })
        .collect();
    vars.sort();
    // One line, so the line-by-line diff stays aligned when the sets differ.
    out.push_str("custom: ");
    out.push_str(&vars.join(" "));
    out.push('\n');
    out
}

/// The fingerprint of one node (not its children). `hidden`: the node is in a
/// `display: none` subtree, where the layout box means nothing and is left
/// out.
pub fn node_fingerprint(doc: &RinchDocument, id: usize, hidden: bool) -> String {
    let node = &doc.tree.nodes[id];
    let mut out = String::new();
    if let Some(text) = node.text_content() {
        if node.is_element() {
            out.push_str(&format!("element-with-text {text:?}\n"));
        } else {
            out.push_str(&format!("#text {text:?}\n"));
            return out;
        }
    }
    out.push_str(&format!(
        "<{}> pseudo={}\n",
        node.tag().unwrap_or("?"),
        node.is_pseudo_element
    ));
    if !node.is_pseudo_element {
        let data = node.stylo_element_data.borrow();
        match data.as_ref().and_then(|d| d.styles.primary.clone()) {
            Some(cv) => out.push_str(&stylo_fingerprint(&cv)),
            None => out.push_str("(no stylo style)\n"),
        }
    }
    out.push_str(&serde_json::to_string(&node.computed_style).unwrap());
    out.push('\n');
    if !hidden {
        let l = node.layout;
        out.push_str(&format!(
            "box {:.2} {:.2} {:.2} {:.2}\n",
            l.x, l.y, l.width, l.height
        ));
    }
    out
}

/// Every node under (and including) `root`, in tree order, with its path.
pub fn tree_fingerprint(doc: &RinchDocument, root: usize, boxes: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    walk(doc, root, "0".to_string(), !boxes, &mut out);
    out
}

fn walk(
    doc: &RinchDocument,
    id: usize,
    path: String,
    hidden: bool,
    out: &mut Vec<(String, String)>,
) {
    let hidden = hidden
        || (doc.tree.nodes[id].is_element()
            && doc.tree.nodes[id].computed_style.display
                == rinch_dom::computed_style::DisplayValue::None);
    out.push((path.clone(), node_fingerprint(doc, id, hidden)));
    let children = doc.tree.nodes[id].children.clone();
    out.push((
        format!("{path}#children"),
        format!("{} children", children.len()),
    ));
    for (i, c) in children.into_iter().enumerate() {
        walk(doc, c, format!("{path}/{i}"), hidden, out);
    }
}

/// Assert `incremental` and `fresh` computed the same style for every node
/// under `<html>`. On failure, names the first differing node and the lines
/// that differ.
#[track_caller]
pub fn assert_twin(incremental: &RinchDocument, fresh: &RinchDocument, label: &str) {
    assert_twin_opts(incremental, fresh, label, true);
}

/// [`assert_twin`], comparing layout boxes only when `boxes` is set.
#[track_caller]
pub fn assert_twin_opts(
    incremental: &RinchDocument,
    fresh: &RinchDocument,
    label: &str,
    boxes: bool,
) {
    let a = tree_fingerprint(incremental, incremental.tree.html_id, boxes);
    let b = tree_fingerprint(fresh, fresh.tree.html_id, boxes);
    let mut report = Vec::new();
    for (i, ((pa, fa), (pb, fb))) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            pa,
            pb,
            "{label}: tree shape differs at entry {i}\n{}",
            report.join("\n")
        );
        if fa != fb {
            let diff: Vec<String> = fa
                .lines()
                .zip(fb.lines())
                .filter(|(x, y)| x != y)
                .map(|(x, y)| format!("    incremental: {x}\n    fresh:       {y}"))
                .collect();
            let tag = fa.lines().next().unwrap_or("");
            report.push(format!("node {pa} ({tag}):\n{}", diff.join("\n")));
        }
    }
    assert!(
        report.is_empty(),
        "{label}: {} node(s) differ between the incrementally restyled document \
         and a fresh one built in the final state:\n{}",
        report.len(),
        report.join("\n")
    );
    assert_eq!(a.len(), b.len(), "{label}: node counts differ");
}
