//! A list item whose text wraps keeps its marker beside the text's first line,
//! and the text wraps inside the item (#1246).
//!
//! The editor's `li` (and its task item) used to be a wrapping flex row of the
//! marker and the item's blocks. A paragraph there kept `flex-basis: auto`, so
//! its hypothetical size was its max-content width — the whole text on one
//! line — and once that was wider than the room beside the marker,
//! `flex-wrap: wrap` moved it onto a flex line of its own: the bullet alone on
//! a line, the text starting under it (Chrome does the same with that CSS).
//! The item is now an ordinary block whose marker hangs outside it, as a
//! browser's `list-item` does: desktop's generated marker span (and the task
//! item's `::before`) is taken out of flow onto the first line.
//!
//! Host fonts: every assertion is a relation between boxes of one document
//! (same line, beside, below, inside), and the text is long enough to wrap at
//! 400px in any face.

use super::*;

const LONG: &str = "the edges of the world are not void or grid: they thin into pencil \
     sketch, then paper, then handwriting, the description of the place before she \
     imagined it, which is the signature look of her mind";

/// Mount one editor 400px wide over `html`, then run `command` (if any).
fn mount(html: &str, command: Option<&'static str>) -> RinchApp {
    let html = html.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(&html);
        if let Some(c) = command {
            handle.set_selection(rinch_editor_core::Selection::cursor(
                rinch_editor_core::Pos(1),
            ));
            assert!(handle.command(c), "{c} runs");
        }
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; font-family: sans-serif",
        );
        root.append_child(&container);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    app
}

#[derive(Debug, Clone, Copy)]
struct Box4 {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn attr<'a>(n: &'a rinch_dom::Node, name: &str) -> Option<&'a str> {
    n.attributes.get(name).map(String::as_str)
}

fn box_of(n: &rinch_dom::Node) -> Box4 {
    Box4 {
        x: n.layout.x,
        y: n.layout.y,
        w: n.layout.width,
        h: n.layout.height,
    }
}

/// `(item, marker, [(name, box)] of the blocks after the marker)`.
type Item = (Box4, Box4, Vec<(String, Box4)>);

/// Every list item (an `li` or a task item) as `(item, marker, children)`:
/// the marker is the item's generated first child, the children are the
/// item's element children after it, all in the item's own coordinates.
fn items(app: &RinchApp) -> Vec<Item> {
    let doc = app.doc.as_ref().unwrap().borrow();
    let nodes = &doc.tree.nodes;
    let mut out = Vec::new();
    for (_, n) in nodes.iter() {
        let is_item = n.tag() == Some("li") || attr(n, "data-pm-type") == Some("task_item");
        if !is_item || n.parent.is_none() {
            continue;
        }
        let mut kids = n.children.iter().map(|&c| &nodes[c]);
        let marker = kids.next().expect("the marker");
        assert!(marker.is_pseudo_element, "the first child is the marker");
        let rest = kids
            .filter(|k| k.tag().is_some() && !k.is_pseudo_element)
            .map(|k| {
                let name = attr(k, "data-pm-type")
                    .map(str::to_string)
                    .unwrap_or_else(|| k.tag().unwrap().to_string());
                (name, box_of(k))
            })
            .collect();
        out.push((box_of(n), box_of(marker), rest));
    }
    out
}

/// The marker sits beside the content's first line, and the content wraps
/// inside the item.
fn assert_beside_and_wrapped(what: &str, item: Box4, marker: Box4, content: Box4) {
    assert!(
        marker.h > 0.0 && marker.w > 0.0,
        "{what}: the marker has a box: {marker:?}"
    );
    assert_eq!(
        content.y, marker.y,
        "{what}: the content's first line is the marker's line \
         (marker {marker:?}, content {content:?})"
    );
    assert!(
        marker.w < 48.0,
        "{what}: the marker is a marker's width, not a share of the line: {marker:?}"
    );
    assert!(
        content.x >= marker.x + marker.w - 1.0 && content.x <= marker.x + marker.w + 12.0,
        "{what}: the content starts right of the marker, to a pixel of edge rounding \
         (marker {marker:?}, content {content:?})"
    );
    assert!(
        content.h >= 2.5 * marker.h,
        "{what}: the content wraps onto several lines \
         (marker {marker:?}, content {content:?})"
    );
    assert!(
        content.x + content.w <= item.w + 0.5,
        "{what}: and stays inside the item (item {item:?}, content {content:?})"
    );
}

#[test]
fn a_wrapping_bullet_item_keeps_its_bullet_beside_the_first_line() {
    let app = mount(&format!("<ul><li><p>{LONG}</p></li></ul>"), None);
    let all = items(&app);
    assert_eq!(all.len(), 1);
    let (item, marker, rest) = &all[0];
    assert_eq!(rest.len(), 1, "{rest:?}");
    assert_beside_and_wrapped("bullet item", *item, *marker, rest[0].1);
}

#[test]
fn a_wrapping_ordered_item_keeps_its_number_beside_the_first_line() {
    let app = mount(&format!("<ol><li><p>{LONG}</p></li></ol>"), None);
    let all = items(&app);
    assert_eq!(all.len(), 1);
    let (item, marker, rest) = &all[0];
    assert_beside_and_wrapped("ordered item", *item, *marker, rest[0].1);
}

#[test]
fn a_wrapping_task_item_keeps_its_checkbox_beside_the_first_line() {
    let app = mount(&format!("<p>{LONG}</p>"), Some("toggleTaskList"));
    let all = items(&app);
    assert_eq!(all.len(), 1, "one task item");
    let (item, marker, rest) = &all[0];
    assert_beside_and_wrapped("task item", *item, *marker, rest[0].1);
}

#[test]
fn a_short_item_is_unchanged_and_a_nested_list_breaks_onto_its_own_line() {
    let app = mount(
        &format!("<ul><li><p>short</p><ul><li><p>{LONG}</p></li></ul></li></ul>"),
        None,
    );
    let all = items(&app);
    assert_eq!(all.len(), 2);
    // The outer item: marker, paragraph, nested list.
    let (outer, marker, rest) = all
        .iter()
        .find(|(_, _, r)| r.len() == 2)
        .expect("the outer item");
    let (_, p) = &rest[0];
    let (name, nested) = &rest[1];
    assert_eq!(name, "bullet_list", "{rest:?}");
    assert_eq!(p.y, marker.y, "a short item: text on the marker's line");
    assert!(
        p.x >= marker.x + marker.w - 1.0 && p.x <= marker.x + marker.w + 12.0 && marker.w < 48.0,
        "right beside it: {p:?} vs {marker:?}"
    );
    assert!(p.h < 2.0 * marker.h, "on one line: {p:?} vs {marker:?}");
    assert!(
        nested.y >= p.y + p.h,
        "the nested list starts below the paragraph: {nested:?} vs {p:?}"
    );
    assert!(
        nested.w >= outer.w - 0.5,
        "and takes the item's full width: {nested:?} vs {outer:?}"
    );
    // The inner item wraps beside its own bullet.
    let (inner, im, ir) = all
        .iter()
        .find(|(_, _, r)| r.len() == 1)
        .expect("the inner item");
    assert_beside_and_wrapped("nested item", *inner, *im, ir[0].1);
}

#[test]
fn a_second_paragraph_in_an_item_starts_its_own_line() {
    let app = mount("<ul><li><p>one</p><p>two</p></li></ul>", None);
    let all = items(&app);
    let (_, marker, rest) = &all[0];
    assert_eq!(rest.len(), 2, "{rest:?}");
    let (p1, p2) = (rest[0].1, rest[1].1);
    assert_eq!(p1.y, marker.y);
    assert_eq!(p2.x, p1.x, "under the text, not under the bullet");
    assert!(
        p2.y >= p1.y + p1.h,
        "the second paragraph is below the first, not beside it: {p1:?} {p2:?}"
    );
}
