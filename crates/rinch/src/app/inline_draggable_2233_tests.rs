//! Issue #2233 — a press on an inline `draggable="true"` element inside a
//! paragraph starts its drag, as Chrome 156 starts a drag of an inline
//! `<span draggable>` or `<a draggable>` from a press on its text.
//!
//! An IFC's text is hit as its root (its text nodes stretch over the line box
//! and are skipped), so the draggable search used to walk up from the
//! paragraph and never passed the inline element. It now starts where the
//! click walk starts, at the text node under the glyphs
//! (`hit_testing::event_target_at`).

use super::hit_testing::painted_element_box;
use super::text_selection_across_tests::{drag, node, press, release};
use super::*;
use rinch_core::Component;

const VP: (u32, u32) = (800, 600);

/// `html` under a root in a 16px/20px font; `data-ondragstart` on `#d` pushes
/// `"d"` to the log.
fn mount_with(html: &'static str, selectable: bool) -> (RinchApp, Rc<RefCell<Vec<&'static str>>>) {
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "font: 16px/20px sans-serif; width: 600px; margin: 0",
        );
        if selectable {
            let view = rinch_html::HtmlView {
                html: html.into(),
                ..Default::default()
            }
            .render(scope, &[]);
            root.append_child(&view);
        } else {
            root.set_inner_html(html);
        }
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let sink = log.clone();
    let start = events::register_handler(Rc::new(move || sink.borrow_mut().push("d")));
    let d = node(&app, "d");
    app.doc.as_ref().unwrap().borrow_mut().set_attribute(
        rinch_core::dom::NodeId(d),
        "data-ondragstart",
        &start.0.to_string(),
    );
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, log)
}

/// The centre of the third glyph of inline `#id`'s text, in window
/// coordinates — on its glyphs, off its first edge. The inline has no box of
/// its own; its paragraph (its parent, the IFC root) lays its text out.
fn inside(app: &RinchApp, id: &str) -> (f32, f32) {
    let n = node(app, id);
    let d = app.doc.as_ref().unwrap().borrow();
    let el = d.tree.get(n).unwrap();
    let text = el.children[0];
    let root_id = el.parent.unwrap();
    let root = d.tree.get(root_id).unwrap();
    let inline = root
        .text_layout
        .as_ref()
        .expect("the parent is the IFC root");
    let range = inline
        .text_ranges
        .iter()
        .find(|r| r.node_id == text)
        .expect("the inline's text is laid out in its parent");
    let g =
        rinch_dom::text_query::glyph_bounds_for_offset_layout(&inline.layout, range.flat_start + 2)
            .unwrap();
    let (x, y, _, _) = painted_element_box(&d.tree, root_id);
    let (ox, oy) = rinch_dom::paint::ifc_root_content_origin(root);
    (x + ox + g.x + g.width / 2.0, y + oy + g.y + g.height / 2.0)
}

fn press_and_move(app: &mut RinchApp, p: (f32, f32)) {
    press(app, p);
    drag(app, (p.0 + 20.0, p.1 + 3.0));
}

#[test]
fn a_press_on_an_inline_draggable_span_starts_its_drag() {
    let (mut app, log) = mount_with(
        r#"<p>drag <span id="d" draggable="true">this word</span> please</p>"#,
        false,
    );
    let p = inside(&app, "d");
    press_and_move(&mut app, p);
    assert_eq!(*log.borrow(), vec!["d"], "the span's drag started");
    assert!(app.active_dnd.is_some());
    release(&mut app, (p.0 + 20.0, p.1 + 3.0));
}

/// The control: a press on the paragraph's own text, beside the span, starts
/// nothing.
#[test]
fn a_press_beside_the_inline_draggable_starts_nothing() {
    let (mut app, log) = mount_with(
        r#"<p id="p">drag <span id="d" draggable="true">this word</span> please and more text</p>"#,
        false,
    );
    let (x, y, w, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "p"))
    };
    let _ = w;
    // Well past "drag this word please": in "more text".
    let p = (x + 230.0, y + 10.0);
    press_and_move(&mut app, p);
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
    assert!(app.active_dnd.is_none());
}

/// A `pointer-events: none` inline is not where a press lands (Chrome
/// delivers it to the paragraph), so its `draggable` starts nothing.
#[test]
fn a_pointer_events_none_inline_draggable_starts_nothing() {
    let (mut app, log) = mount_with(
        r#"<p>drag <span id="d" draggable="true" style="pointer-events: none">this word</span> please</p>"#,
        false,
    );
    let p = inside(&app, "d");
    press_and_move(&mut app, p);
    assert!(log.borrow().is_empty(), "{:?}", log.borrow());
}

/// In selectable text a `draggable="true"` link drags, as in Chrome, rather
/// than selecting (a link without the attribute still selects on a drag:
/// `text_selection_gestures_tests::a_drag_across_a_link_selects_it_and_follows_nothing`).
#[test]
fn a_draggable_link_in_selectable_text_drags_and_does_not_select() {
    let (mut app, log) = mount_with(
        r#"<div>Read <a id="d" href="https://shop.example/" draggable="true">the whole story</a> online.</div>"#,
        true,
    );
    let p = inside(&app, "d");
    press_and_move(&mut app, p);
    assert_eq!(*log.borrow(), vec!["d"], "the link's drag started");
    let sel = app
        .text_selection
        .as_ref()
        .map(|s| (s.anchor_offset, s.focus_offset));
    assert!(
        sel.is_none_or(|(a, f)| a == f),
        "nothing is selected: {sel:?}"
    );
    release(&mut app, (p.0 + 20.0, p.1 + 3.0));
}
