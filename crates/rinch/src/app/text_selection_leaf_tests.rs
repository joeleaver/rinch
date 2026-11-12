//! Read-only selection in a flex or grid container's own text (#1692,
//! #2244) and around generated content (#2040), against Chrome 157's
//! `getSelection().toString()` of the same markup.
//!
//! Text directly in a flex or grid container is an anonymous item laid out
//! as a text leaf (`cached_text_parley`), in no IFC root. A press there
//! started no selection and a selection across it did not highlight it:
//! the selection's roots were IFC roots only.

use super::hit_testing::painted_element_box;
use super::text_selection_across_tests::{at, drag, highlighted, mount, node, press, release};
use super::text_selection_chrome_tests::SEL;
use super::*;

fn select(app: &mut RinchApp, ends: ((f32, f32), (f32, f32))) -> String {
    let (a, b) = ends;
    press(app, a);
    drag(app, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
    drag(app, b);
    release(app, b);
    app.text_selection_text().unwrap_or_default()
}

/// The text node that is `#id`'s first child: a flex item's text leaf.
fn leaf(app: &RinchApp, id: &str) -> u64 {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let t = d.tree.get(n).unwrap().children[0];
    assert!(
        d.tree.get(t).unwrap().cached_text_parley.is_some(),
        "#{id}'s text is laid out as a text leaf"
    );
    t
}

/// The painted box of `#id`'s text leaf.
fn leaf_box(app: &RinchApp, id: &str) -> (f32, f32, f32, f32) {
    let t = leaf(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    painted_element_box(&d.tree, t)
}

fn leaf_highlight(app: &RinchApp, id: &str) -> Option<(usize, usize)> {
    let t = leaf(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let a = &d.tree.get(t)?.attributes;
    (a.get("data-text-sel").map(String::as_str) == Some("true")).then(|| {
        let get = |k: &str| a.get(k).and_then(|v| v.parse().ok()).unwrap_or(0);
        (get("data-text-sel-start"), get("data-text-sel-end"))
    })
}

/// The point past the end of `#id`'s last line, inside its box.
fn past_end(app: &RinchApp, id: &str) -> (f32, f32) {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, h) = painted_element_box(&d.tree, n);
    (x + w - 1.0, y + h - 5.0)
}

#[test]
fn a_drag_across_a_flex_containers_text_selects_it() {
    // Chrome 157: "hello world hello world", the press in the leaf.
    let mut app = mount(
        r#"<div id="f" style="display:flex">hello world hello world</div>"#,
        SEL,
    );
    let (x, y, _, _) = leaf_box(&app, "f");
    let t = {
        let ends = ((x + 1.0, y + 10.0), past_end(&app, "f"));
        select(&mut app, ends)
    };
    assert_eq!(t, "hello world hello world");
    assert_eq!(leaf_highlight(&app, "f"), Some((0, 23)));
}

#[test]
fn a_drag_across_a_grid_containers_text_selects_it() {
    // The control: a grid container's text is an anonymous grid item laid
    // out in an IFC root of its own (an anonymous box), not a leaf.
    let mut app = mount(
        r#"<div id="f" style="display:grid">hello world hello world</div>"#,
        SEL,
    );
    let (x, y, _, _) =
        painted_element_box(&app.doc.as_ref().unwrap().borrow().tree, node(&app, "f"));
    let t = {
        let ends = ((x + 1.0, y + 10.0), past_end(&app, "f"));
        select(&mut app, ends)
    };
    assert_eq!(t, "hello world hello world");
}

#[test]
fn a_selection_across_a_flex_containers_text_highlights_it() {
    // Chrome 157: "before\n\nflex text here\nafter"; the leaf's text is its
    // white space collapsed, "flex text here".
    let mut app = mount(
        r#"<p id="a" style="margin:0">before</p><div id="f" style="display:flex">flex  text   here</div><p id="b" style="margin:0">after</p>"#,
        SEL,
    );
    let t = {
        let ends = (at(&app, "a", 0.0), past_end(&app, "b"));
        select(&mut app, ends)
    };
    assert_eq!(t, "before\n\nflex text here\nafter");
    assert_eq!(leaf_highlight(&app, "f"), Some((0, 14)));
    assert_eq!(highlighted(&app, "b"), Some((0, 5)));
}

#[test]
fn a_selection_starts_and_ends_in_a_flex_containers_text() {
    let html = r#"<p id="a" style="margin:0">before</p><div id="f" style="display:flex">flex  text   here</div><p id="b" style="margin:0">after</p>"#;
    // From the leaf's start into the paragraph after it.
    let mut app = mount(html, SEL);
    let (x, y, w, _) = leaf_box(&app, "f");
    let t = {
        let ends = ((x + 1.0, y + 10.0), past_end(&app, "b"));
        select(&mut app, ends)
    };
    assert_eq!(t, "flex text here\nafter");
    // Backwards, from the paragraph after it to the leaf's start.
    let mut app = mount(html, SEL);
    let t = {
        let ends = (past_end(&app, "b"), (x + 1.0, y + 10.0));
        select(&mut app, ends)
    };
    assert_eq!(t, "flex text here\nafter");
    // From the paragraph before it to the leaf's end.
    let mut app = mount(html, SEL);
    let t = {
        let ends = (at(&app, "a", 0.0), (x + w - 0.5, y + 10.0));
        select(&mut app, ends)
    };
    assert_eq!(t, "before\n\nflex text here");
    assert_eq!(leaf_highlight(&app, "f"), Some((0, 14)));
}

#[test]
fn a_double_click_in_a_flex_containers_text_selects_its_word() {
    let mut app = mount(
        r#"<div id="f" style="display:flex">flex  text   here</div>"#,
        SEL,
    );
    let (x, y, w, _) = leaf_box(&app, "f");
    let p = (x + w / 2.0, y + 10.0);
    for _ in 0..2 {
        press(&mut app, p);
        release(&mut app, p);
    }
    assert_eq!(app.text_selection_text().as_deref(), Some("text"));
    assert_eq!(leaf_highlight(&app, "f"), Some((5, 9)));
}

#[test]
fn generated_content_is_neither_copied_nor_highlighted() {
    // #2040, Chrome 157: "see link after"; the arrow `a::after` draws is
    // not highlighted.
    let mut app = mount(
        r#"<style>a::after { content: "\2197" }</style><p id="a" style="margin:0">see <a href="https://x.example">link</a> after</p>"#,
        SEL,
    );
    let t = {
        let ends = (at(&app, "a", 0.0), past_end(&app, "a"));
        select(&mut app, ends)
    };
    assert_eq!(t, "see link after");
    let n = node(&app, "a");
    let d = app.doc.as_ref().unwrap().borrow();
    let p = d.tree.get(n).unwrap();
    let text = &p.text_layout.as_ref().unwrap().text_content;
    assert_eq!(text, "see link\u{2197} after", "the arrow is laid out");
    assert_eq!(
        p.attributes.get("data-text-sel-ranges").map(String::as_str),
        Some("0-8,11-17"),
        "the arrow (bytes 8..11) is cut out of the highlight"
    );
}

#[test]
fn a_selection_ending_in_a_flex_containers_text_ends_there() {
    // Chrome 157: "before\n\nflex text": the walk stops at the leaf, not at
    // the end of its container (the `span` after it is a flex item).
    let html = r#"<p id="a" style="margin:0">before</p><div id="f" style="display:flex">flex text<span>more</span></div><p id="b" style="margin:0">after</p>"#;
    let mut app = mount(html, SEL);
    let (x, y, w, _) = leaf_box(&app, "f");
    let t = {
        let ends = (at(&app, "a", 0.0), (x + w - 0.5, y + 10.0));
        select(&mut app, ends)
    };
    assert_eq!(t, "before\n\nflex text");
}
