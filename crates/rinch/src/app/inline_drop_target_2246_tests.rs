//! Issue #2246 — a drop target on an inline element inside a paragraph is
//! found from a pointer over its text, for an element drag (`data-ondragenter`,
//! `data-ondragover`, `data-ondrop`) and for files dragged in from the OS
//! (`data-onfiledragenter`, `data-onfiledrop`).
//!
//! An IFC's text is hit as its root (its text nodes stretch over the line box
//! and are skipped), so these walks used to start at the paragraph and never
//! passed the inline element. Chrome 158 targets the inline element
//! (`document.elementFromPoint` over its text answers the `<span>`, and drag
//! events go to the element under the pointer), and passes a
//! `pointer-events: none` one's point to the paragraph. The walks now start
//! where the click walk starts (`RinchApp::shared_event_target`).

use super::hit_testing::painted_element_box;
use super::text_selection_across_tests::node;
use super::*;
use std::path::PathBuf;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

/// A block draggable `#src` above `#outer`, a block holding `html`. `#outer`
/// and `#t` carry `data-ondragenter`, `data-ondragover`, `data-ondrop`,
/// `data-onfiledragenter` and `data-onfiledrop`, each logging
/// `"<event>:<id>"`.
fn mount(html: &'static str) -> (RinchApp, Log) {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "font: 16px/20px sans-serif; width: 600px; margin: 0",
        );
        root.set_inner_html(&format!(
            r#"<div id="src" draggable="true" style="height: 40px">source</div><div id="outer">{html}</div>"#
        ));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let log: Log = Rc::default();
    for id in ["outer", "t"] {
        let n = rinch_core::dom::NodeId(node(&app, id));
        let doc = app.doc.clone().unwrap();
        let mut d = doc.borrow_mut();
        for (attr, what) in [
            ("data-ondragenter", "enter"),
            ("data-ondragover", "over"),
            ("data-ondrop", "drop"),
            ("data-onfiledragenter", "fileenter"),
        ] {
            let log = log.clone();
            let h = events::register_handler(Rc::new(move || {
                log.borrow_mut().push(format!("{what}:{id}"));
            }));
            d.set_attribute(n, attr, &h.0.to_string());
        }
        let log = log.clone();
        let h = events::register_file_drop_handler(events::FileDropCallback::new(
            move |_paths: Vec<PathBuf>| {
                log.borrow_mut().push(format!("filedrop:{id}"));
            },
        ));
        d.set_attribute(n, "data-onfiledrop", &h.0.to_string());
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, log)
}

/// The centre of the third glyph of inline `#id`'s text, in window
/// coordinates: on its glyphs, off its first edge.
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

fn send(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, VP, 1.0);
}

/// Drag `#src` to `to` and drop it there; the log of what the targets heard.
fn drag_src_to(app: &mut RinchApp, log: &Log, to: (f32, f32)) -> Vec<String> {
    let button = MouseButton::Left;
    send(
        app,
        PlatformEvent::MouseDown {
            x: 30.0,
            y: 20.0,
            button,
        },
    );
    send(app, PlatformEvent::MouseMove { x: 60.0, y: 30.0 });
    assert!(
        app.active_dnd.is_some(),
        "positive control: the drag went live"
    );
    log.borrow_mut().clear();
    send(app, PlatformEvent::MouseMove { x: to.0, y: to.1 });
    send(
        app,
        PlatformEvent::MouseUp {
            x: to.0,
            y: to.1,
            button,
        },
    );
    log.borrow().clone()
}

/// Drag a file in from the OS to `to` and drop it there.
fn drop_file_at(app: &mut RinchApp, log: &Log, to: (f32, f32)) -> Vec<String> {
    log.borrow_mut().clear();
    let position = (to.0 as f64, to.1 as f64);
    send(app, PlatformEvent::FileDragMoved { position });
    send(
        app,
        PlatformEvent::FileDropped {
            paths: vec![PathBuf::from("/tmp/a.txt")],
            position,
        },
    );
    log.borrow().clone()
}

const SPAN: &str = r#"<p id="p">drop <span id="t">on this word</span> here and more text</p>"#;

#[test]
fn an_element_dragged_over_an_inline_drop_target_drops_on_it() {
    let (mut app, log) = mount(SPAN);
    let p = inside(&app, "t");
    assert_eq!(
        drag_src_to(&mut app, &log, p),
        ["enter:t", "over:t", "drop:t"]
    );
}

#[test]
fn a_file_dragged_over_an_inline_drop_target_drops_on_it() {
    let (mut app, log) = mount(SPAN);
    let p = inside(&app, "t");
    assert_eq!(
        drop_file_at(&mut app, &log, p),
        ["fileenter:t", "filedrop:t"]
    );
}

/// The control: the paragraph's own text beside the span is the block's.
#[test]
fn the_paragraphs_own_text_drops_on_the_block_around_it() {
    let (mut app, log) = mount(SPAN);
    let (x, y, _, _) = {
        let d = app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, node(&app, "p"))
    };
    // Well past "drop on this word here": in "more text".
    let p = (x + 230.0, y + 10.0);
    assert_eq!(
        drag_src_to(&mut app, &log, p),
        ["enter:outer", "over:outer", "drop:outer"]
    );
    assert_eq!(
        drop_file_at(&mut app, &log, p),
        ["fileenter:outer", "filedrop:outer"]
    );
}

/// A `pointer-events: none` inline is not where the pointer is (Chrome's
/// `elementFromPoint` answers the paragraph), so the block takes the drop.
#[test]
fn a_pointer_events_none_inline_passes_the_drop_to_the_block() {
    let (mut app, log) =
        mount(r#"<p>drop <span id="t" style="pointer-events: none">on this word</span> here</p>"#);
    let p = inside(&app, "t");
    assert_eq!(
        drag_src_to(&mut app, &log, p),
        ["enter:outer", "over:outer", "drop:outer"]
    );
    assert_eq!(
        drop_file_at(&mut app, &log, p),
        ["fileenter:outer", "filedrop:outer"]
    );
}
