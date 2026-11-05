//! Read-only selection that follows its container's `user-select` and runs
//! across paragraphs, as in a browser: a press in one, a drag into another,
//! every paragraph between them selected, and copy gives their text a line
//! each.

use super::hit_testing::painted_element_box;
use super::*;

const VP: (u32, u32) = (800, 600);
const FONT: &str = "font: 16px/20px sans-serif; width: 600px; margin: 0";

/// An app whose root holds `build`'s elements; returns it and the node ids by
/// their `id` attribute.
pub(super) fn mount(html: &'static str, root_style: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", root_style);
        root.set_inner_html(html);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    app
}

pub(super) fn node(app: &RinchApp, id: &str) -> u64 {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id))
        .map(|(n, _)| n)
        .unwrap_or_else(|| panic!("no #{id}"))
}

/// A point `fraction` of the way across the first line of `#id`.
pub(super) fn at(app: &RinchApp, id: &str, fraction: f32) -> (f32, f32) {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let (x, y, w, _) = painted_element_box(&d.tree, n);
    (x + w * fraction, y + 10.0)
}

pub(super) fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
}

pub(super) fn drag(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
}

pub(super) fn release(app: &mut RinchApp, (x, y): (f32, f32)) {
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
}

/// The part of `#id` that carries the highlight, if any.
pub(super) fn highlighted(app: &RinchApp, id: &str) -> Option<(usize, usize)> {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let a = &d.tree.get(n)?.attributes;
    (a.get("data-text-sel").map(String::as_str) == Some("true")).then(|| {
        let get = |k: &str| a.get(k).and_then(|v| v.parse().ok()).unwrap_or(0);
        (get("data-text-sel-start"), get("data-text-sel-end"))
    })
}

pub(super) const THREE: &str = r#"<p id="a">first paragraph</p><p id="b">second paragraph</p><div><p id="c">third paragraph</p></div>"#;

#[test]
fn a_containers_user_select_reaches_the_paragraphs_inside_it() {
    // `user-select: auto` takes its parent's value (CSS UI 4): the paragraphs
    // declare nothing, the container `text`.
    let mut app = mount(
        THREE,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    {
        let p = at(&app, "a", 0.0);
        press(&mut app, p);
    }
    let s = app
        .text_selection
        .as_ref()
        .expect("a press in #a starts a selection");
    assert_eq!(s.ifc_node_id, node(&app, "a"));
}

#[test]
fn text_stays_unselectable_where_nothing_says_otherwise() {
    // rinch's default, for an app's own controls.
    let mut app = mount(THREE, FONT);
    {
        let p = at(&app, "a", 0.0);
        press(&mut app, p);
    }
    assert!(app.text_selection.is_none());
}

#[test]
fn a_drag_selects_every_paragraph_it_passes_and_copies_them_a_line_each() {
    let mut app = mount(
        THREE,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    let start = at(&app, "a", 0.0);
    press(&mut app, start);
    {
        let p = at(&app, "b", 0.5);
        drag(&mut app, p);
    }
    let end = at(&app, "c", 0.0);
    // The end of "third" (5 letters in): a point inside the word.
    drag(&mut app, end);
    let to = {
        let d = app.doc.as_ref().unwrap().borrow();
        let c = node(&app, "c");
        RinchApp::compute_ifc_offset_from_click(&d.tree, c, end.0 + 39.0, end.1)
    };
    let end = (end.0 + 39.0, end.1);
    drag(&mut app, end);
    release(&mut app, end);
    assert_eq!(highlighted(&app, "a"), Some((0, "first paragraph".len())));
    assert_eq!(highlighted(&app, "b"), Some((0, "second paragraph".len())));
    assert_eq!(highlighted(&app, "c"), Some((0, to)));
    let text = app.text_selection_text().unwrap();
    assert_eq!(
        text,
        format!(
            "first paragraph\nsecond paragraph\n{}",
            &"third paragraph"[..to]
        )
    );
}

#[test]
fn dragging_back_gives_up_the_paragraphs_it_leaves() {
    let mut app = mount(
        THREE,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    {
        let p = at(&app, "a", 0.0);
        press(&mut app, p);
    }
    {
        let p = at(&app, "c", 0.5);
        drag(&mut app, p);
    }
    assert!(highlighted(&app, "c").is_some());
    {
        let p = at(&app, "a", 0.5);
        drag(&mut app, p);
    }
    assert_eq!(highlighted(&app, "b"), None);
    assert_eq!(highlighted(&app, "c"), None);
    assert!(highlighted(&app, "a").is_some());
}

#[test]
fn a_drag_upwards_selects_the_same_way() {
    let mut app = mount(
        THREE,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    let c = node(&app, "c");
    let c_len = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .get(c)
            .unwrap()
            .text_layout
            .as_ref()
            .unwrap()
            .text_content
            .len()
    };
    let (x, y, w, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, c)
    };
    // From past the end of #c, up to the start of #a.
    press(&mut app, (x + w - 1.0, y + 10.0));
    {
        let p = at(&app, "a", 0.0);
        drag(&mut app, p);
    }
    assert_eq!(highlighted(&app, "a"), Some((0, "first paragraph".len())));
    assert_eq!(highlighted(&app, "b"), Some((0, "second paragraph".len())));
    assert_eq!(highlighted(&app, "c"), Some((0, c_len)));
}

#[test]
fn an_unselectable_paragraph_inside_is_passed_over() {
    let mut app = mount(
        r#"<p id="a">first</p><p id="b" style="user-select: none">skipped</p><p id="c">third</p>"#,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    {
        let p = at(&app, "a", 0.0);
        press(&mut app, p);
    }
    {
        let p = at(&app, "c", 0.99);
        drag(&mut app, p);
    }
    assert_eq!(highlighted(&app, "b"), None);
    assert_eq!(app.text_selection_text().unwrap(), "first\nthird");
}

#[test]
fn a_new_press_clears_the_whole_selection() {
    let mut app = mount(
        THREE,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    {
        let p = at(&app, "a", 0.0);
        press(&mut app, p);
    }
    {
        let p = at(&app, "c", 0.5);
        drag(&mut app, p);
    }
    {
        let p = at(&app, "c", 0.5);
        release(&mut app, p);
    }
    {
        let p = at(&app, "b", 0.2);
        press(&mut app, p);
    }
    for id in ["a", "b", "c"] {
        assert_eq!(highlighted(&app, id), None, "#{id}");
    }
}

#[test]
fn loose_text_between_paragraphs_is_selected_in_document_order() {
    // Text beside blocks lays out in anonymous boxes, which stand in the
    // order of their text.
    let mut app = mount(
        r#"<div id="box">before <b>bold</b><p id="b">middle</p>after</div>"#,
        "user-select: text; font: 16px/20px sans-serif; width: 600px",
    );
    let (x, y, _, h) = {
        let n = node(&app, "box");
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, n)
    };
    press(&mut app, (x + 1.0, y + 5.0));
    drag(&mut app, (x + 300.0, y + h - 5.0));
    assert_eq!(
        app.text_selection_text().unwrap(),
        "before bold\nmiddle\nafter"
    );
}

#[test]
fn a_document_in_an_html_view_can_be_selected_and_copied() {
    // Mail and other documents select as in a browser, in an app whose own
    // text does not.
    use rinch_core::Component;
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font: 16px/20px sans-serif; width: 600px");
        let label = scope.create_element("div");
        label.set_attribute("id", "chrome");
        label.append_child(&scope.create_text("App label"));
        root.append_child(&label);
        let view = rinch_html::HtmlView {
            html: "<p>Dear Ada,</p><table><tr><td>Your order shipped.</td></tr></table>".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        view.set_attribute("id", "view");
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    {
        let p = at(&app, "chrome", 0.0);
        press(&mut app, p);
    }
    assert!(
        app.text_selection.is_none(),
        "the app's own text is not selectable"
    );
    let (x, y, w, h) = {
        let n = node(&app, "view");
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, n)
    };
    // The paragraph's first line, below its 16px top margin.
    press(&mut app, (x + 1.0, y + 26.0));
    drag(&mut app, (x + w - 1.0, y + h - 5.0));
    assert_eq!(
        app.text_selection_text().unwrap(),
        "Dear Ada,\nYour order shipped."
    );
}

#[test]
fn selectable_false_makes_an_html_views_document_unselectable() {
    use rinch_core::Component;
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "font: 16px/20px sans-serif; width: 600px");
        let view = rinch_html::HtmlView {
            html: r#"<div id="mail">Not for copying.</div>"#.into(),
            selectable: false,
            ..Default::default()
        }
        .render(scope, &[]);
        root.append_child(&view);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    {
        let p = at(&app, "mail", 0.1);
        press(&mut app, p);
    }
    assert!(app.text_selection.is_none());
}
