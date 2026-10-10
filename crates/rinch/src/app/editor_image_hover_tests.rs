//! Floating controls over a picture in the desktop editor: which source an
//! image's `<img>` is loaded from (`rinch::editor::set_image_source`), that
//! each such source loads and reloads on its own, and the image the pointer is
//! over (`EditorHandle::on_image_hover`), driven through the real
//! `PlatformEvent` path.
//!
//! The change-only hover bookkeeping is pinned in `rinch-editor-view`
//! (`images.rs`); what is pinned here is the desktop half: the hit, the
//! image's position and attrs, and its painted box.

use super::*;
use rinch_core::image::ImageLoadResult;
use rinch_editor_core::{AttrValue, Pos, SetNodeAttrStep};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const VP: (u32, u32) = (800, 600);

const RED_3X2_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x9d, 0x74, 0x66,
    0x1a, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x86, 0x19, 0x90, 0x39, 0x00, 0x9b, 0x7e, 0x0b, 0xf5, 0x0f, 0x5f, 0x26, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// An editor holding `html`, under `css`, mounted in an app.
fn page(html: &'static str, css: &'static str) -> (RinchApp, crate::editor::EditorHandle) {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let h = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(css));
        root.append_child(&style);
        root.set_attribute(
            "style",
            "padding: 20px; font-size: 16px; line-height: 20px; font-family: sans-serif",
        );
        root.append_child(&h.mount(scope));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    (app, handle)
}

fn frame(app: &mut RinchApp) {
    app.refresh_editor_overlays();
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
}

/// Every `<img>` in the document, in tree order: `(node id, its src)`.
fn images(app: &RinchApp) -> Vec<(usize, String)> {
    let d = app.doc.as_ref().expect("mounted").borrow();
    let mut ids: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| n.tag() == Some("img") && n.parent.is_some())
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let src = d.tree.nodes[id].attributes.get("src").cloned();
            (id, src.unwrap_or_default())
        })
        .collect()
}

fn painted_box(app: &RinchApp, id: usize) -> (f32, f32, f32, f32) {
    let d = app.doc.as_ref().unwrap().borrow();
    crate::app::hit_testing::painted_element_box(&d.tree, id)
}

fn pointer_move(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
}

/// Every source the loader is asked for, in order, and frames until it has
/// been asked for `n` in all.
fn wait_for_asks(app: &mut RinchApp, asked: &Mutex<Vec<String>>, n: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while asked.lock().unwrap().len() < n && Instant::now() < deadline {
        frame(app);
        std::thread::sleep(Duration::from_millis(1));
    }
    // And a little longer, to see that no more come.
    for _ in 0..20 {
        frame(app);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut got = asked.lock().unwrap().clone();
    got.sort();
    got
}

/// Two pictures with one `src` and different attrs are two sources: each is
/// loaded once, `reload_image` of one asks for that one alone, and an edit of
/// an image's attrs asks for its new source.
#[test]
fn each_source_an_app_chooses_loads_and_reloads_on_its_own() {
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let asked_in = asked.clone();
    crate::image::register_image_scheme("hover-test-blob", move |src: &str| {
        asked_in.lock().unwrap().push(src.to_string());
        ImageLoadResult::Loaded(RED_3X2_PNG.to_vec())
    });
    crate::editor::set_image_source(|attrs| {
        let title = attrs.get_str("title").filter(|t| !t.is_empty())?;
        Some(format!("{}#overlay={title}", attrs.get_str("src")?))
    });
    const S: &str = "hover-test-blob:store/blob";
    let (mut app, handle) = page(
        r#"<p><img src="hover-test-blob:store/blob" title="b1"> <img src="hover-test-blob:store/blob"> <img src="hover-test-blob:store/blob" title="b2"></p>"#,
        "",
    );
    let srcs: Vec<String> = images(&app).into_iter().map(|(_, s)| s).collect();
    assert_eq!(
        srcs,
        [
            format!("{S}#overlay=b1"),
            S.into(),
            format!("{S}#overlay=b2")
        ]
    );
    assert_eq!(
        wait_for_asks(&mut app, &asked, 3),
        [
            S.to_string(),
            format!("{S}#overlay=b1"),
            format!("{S}#overlay=b2")
        ],
        "three sources, each loaded once"
    );

    asked.lock().unwrap().clear();
    std::thread::spawn(|| crate::image::reload_image("hover-test-blob:store/blob#overlay=b2"))
        .join()
        .unwrap();
    assert_eq!(
        wait_for_asks(&mut app, &asked, 1),
        [format!("{S}#overlay=b2")],
        "a reload asks for its own source only"
    );

    // Marking the middle picture up: its `<img>` asks for its new source.
    asked.lock().unwrap().clear();
    assert!(handle.update(|state| {
        let mut tr = state.tr();
        // `doc`, `paragraph`, image, space: the middle image starts at 3.
        tr.step(Box::new(SetNodeAttrStep::new(
            3,
            "title",
            AttrValue::from("b3"),
        )))
        .ok()?;
        Some(tr)
    }));
    frame(&mut app);
    assert_eq!(images(&app)[1].1, format!("{S}#overlay=b3"));
    assert_eq!(
        wait_for_asks(&mut app, &asked, 1),
        [format!("{S}#overlay=b3")]
    );
    // The document still names the picture's own `src`.
    let html = rinch_editor_core::serialize::node_to_html(&handle.doc());
    assert!(!html.contains("#overlay"), "{html}");
    crate::editor::clear_image_source();
}

type Seen = Rc<RefCell<Vec<Option<crate::editor::ImageHover>>>>;

fn record(handle: &crate::editor::EditorHandle) -> Seen {
    let seen: Seen = Rc::default();
    let seen_in = seen.clone();
    handle.on_image_hover(move |h| seen_in.borrow_mut().push(h.cloned()));
    seen
}

const SIZED: &str = "[data-pm-editor] img { width: 40px; height: 30px; }";

#[test]
fn hovering_an_image_reports_its_position_attrs_and_painted_box() {
    let (mut app, handle) = page(
        r#"<p>ab<img src="x.png" alt="first">cd</p><p>ef<img src="y.png" alt="second"></p>"#,
        SIZED,
    );
    assert!(!crate::editor::image_hover_wanted());
    let seen = record(&handle);
    assert!(crate::editor::image_hover_wanted());
    let imgs = images(&app);
    assert_eq!(imgs.len(), 2);
    let (x, y, w, h) = painted_box(&app, imgs[0].0);
    assert_eq!((w, h), (40.0, 30.0), "control: the stylesheet sized it");

    // Text first: no image.
    let (tx, ty, th) = app.editor_caret_point(&handle, Pos(1)).expect("caret");
    pointer_move(&mut app, (tx + 2.0, ty + th / 2.0));
    assert!(seen.borrow().is_empty());

    // Onto the first picture, then around inside it: one report.
    pointer_move(&mut app, (x + 5.0, y + 5.0));
    pointer_move(&mut app, (x + 30.0, y + 20.0));
    assert_eq!(seen.borrow().len(), 1);
    let first = seen.borrow()[0].clone().expect("entered");
    assert_eq!(first.pos, Pos(3), "`doc`, `paragraph`, \"ab\"");
    assert_eq!(first.attrs.get_str("alt"), Some("first"));
    assert_eq!(first.attrs.get_str("src"), Some("x.png"));
    assert_eq!(
        (
            first.rect.x,
            first.rect.y,
            first.rect.width,
            first.rect.height
        ),
        (x, y, w, h)
    );

    // An edit of the picture's attrs while the pointer rests on it is
    // reported on the next move.
    assert!(handle.update(|state| {
        let mut tr = state.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            3,
            "alt",
            AttrValue::from("edited"),
        )))
        .ok()?;
        Some(tr)
    }));
    frame(&mut app);
    pointer_move(&mut app, (x + 6.0, y + 6.0));
    assert_eq!(seen.borrow().len(), 2);
    assert_eq!(
        seen.borrow()[1].as_ref().unwrap().attrs.get_str("alt"),
        Some("edited")
    );

    // Straight to the other picture: one report, for it.
    let (x2, y2, _, _) = painted_box(&app, images(&app)[1].0);
    pointer_move(&mut app, (x2 + 5.0, y2 + 5.0));
    assert_eq!(seen.borrow().len(), 3);
    assert_eq!(
        seen.borrow()[2].as_ref().unwrap().attrs.get_str("alt"),
        Some("second")
    );
    // Away from every picture: one `None`.
    pointer_move(&mut app, (700.0, 500.0));
    pointer_move(&mut app, (710.0, 500.0));
    assert_eq!(seen.borrow().len(), 4);
    assert!(seen.borrow()[3].is_none());
}

/// The image under the pointer is read from the move's hit **before** any
/// `on_link_hover` callback runs: one that replaces the document as the
/// pointer leaves a link for a picture used to leave the image hover walking a
/// detached `<img>`, which reported nothing.
#[test]
fn the_image_hover_is_read_before_a_link_hover_callback_changes_the_document() {
    let (mut app, handle) = page(
        r#"<p><a href="https://example.com/l">link text</a> <img src="x.png" alt="first"></p>"#,
        SIZED,
    );
    let seen = record(&handle);
    let h2 = handle.clone();
    let link_calls = Rc::new(RefCell::new(Vec::new()));
    let link_calls_in = link_calls.clone();
    handle.on_link_hover(move |link| {
        link_calls_in.borrow_mut().push(link.is_some());
        if link.is_none() {
            assert!(h2.load_html(r#"<p>replaced <img src="z.png" alt="second"></p>"#));
        }
    });
    let img = images(&app)[0].0;
    let (x, y, _, _) = painted_box(&app, img);
    // Onto the link's text first, then straight onto the picture.
    let (tx, ty, th) = app.editor_caret_point(&handle, Pos(2)).expect("caret");
    pointer_move(&mut app, (tx + 1.0, ty + th / 2.0));
    assert_eq!(*link_calls.borrow(), [true], "control: on the link");
    pointer_move(&mut app, (x + 5.0, y + 5.0));
    assert_eq!(
        *link_calls.borrow(),
        [true, false],
        "control: left the link"
    );
    let seen = seen.borrow();
    assert_eq!(seen.len(), 1, "the picture under the pointer is reported");
    assert_eq!(
        seen[0].as_ref().unwrap().attrs.get_str("alt"),
        Some("first"),
        "as the hit found it"
    );
}
