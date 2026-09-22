//! Review fixtures for PR #837 (caret scroll-into-view). Scratch — not part of the PR.
#![cfg(feature = "desktop")]
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const WORDS: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor incididunt ut labore et dolore magna aliqua";

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    scroller: usize,
}

/// `editor_is_scroller`: the editor container itself is the `overflow-y: auto`
/// box (which is what turns on block virtualization at >= 60 blocks).
fn rig(
    blocks: usize,
    root_style: &'static str,
    scroller_style: &'static str,
    editor_is_scroller: bool,
    vw: f32,
) -> Rig {
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..blocks)
        .map(|i| format!("<p>{i} {}</p>", &WORDS[..(i * 37) % WORDS.len()]))
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", root_style);
        let editor = handle_in.mount(scope);
        if editor_is_scroller {
            editor.set_attribute("style", scroller_style);
            root.append_child(&editor);
            ids_in.set(Some((editor.node_id().0, editor.node_id().0)));
        } else {
            let scroller = scope.create_element("div");
            scroller.set_attribute("style", scroller_style);
            scroller.append_child(&editor);
            root.append_child(&scroller);
            ids_in.set(Some((scroller.node_id().0, editor.node_id().0)));
        }
        root
    });
    app.mount_component(vw, 600.0);
    app.resolve_and_repaint(vw, 600.0);
    let (scroller, editor) = ids.get().unwrap();
    app.focus_target = FocusTarget::Editor(editor);
    Rig {
        app,
        handle,
        scroller,
    }
}

fn scroll_of(app: &RinchApp, id: usize) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn caret_top(app: &RinchApp) -> Option<String> {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    (0..d.tree.nodes.len())
        .find(|&id| {
            d.tree
                .nodes
                .get(id)
                .is_some_and(|n| n.attributes.contains_key("data-pm-caret"))
        })
        .and_then(|id| d.tree.nodes[id].attributes.get("style").cloned())
}

fn user_scrolls_to(app: &mut RinchApp, id: usize, y: f64) {
    let doc = app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    d.tree.nodes[id].scroll_offset.1 = y;
    d.tree.dirty_nodes.insert(id);
}

fn caret_to_end(r: &mut Rig, vw: f32) {
    let doc = r.handle.doc();
    r.handle
        .set_selection(Selection::cursor(Pos(doc.content_size() - 1)));
    r.app.refresh_editor_overlays();
    for i in 0..4 {
        let ran = r.app.resolve_and_repaint(vw, 600.0);
        eprintln!(
            "r837 frame {i}: ran={ran} caret={:?} scroll={}",
            caret_top(&r.app).map(|s| s.split("top:").nth(1).unwrap_or("").to_string()),
            scroll_of(&r.app, r.scroller)
        );
    }
    eprintln!(
        "r837 caret after move: {:?} scroll={}",
        caret_top(&r.app),
        scroll_of(&r.app, r.scroller)
    );
    assert!(
        scroll_of(&r.app, r.scroller) > 0.0,
        "positive control: the caret move scrolled"
    );
}

/// A window resize reflows the text above the caret. The caret's geometry key
/// changes, but the caret did not MOVE in the document; a browser does not
/// scroll to the caret on resize. A user who scrolled away should stay.
#[test]
fn r837_a_resize_reflow_does_not_yank_the_scroller_back_to_the_caret() {
    let mut r = rig(
        40,
        "width: 100%; height: 600px",
        "width: 50%; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(
        scroll_of(&r.app, r.scroller),
        0.0,
        "control: no yank at a stable size"
    );
    let before = caret_top(&r.app);
    // Resize: 800 -> 560 wide. The scroller narrows, text rewraps.
    r.app.resize_layout(560, 600);
    // The next dirtying event after the resize (here: the user's own wheel,
    // still at the top) runs the caret pass, which sees the reflowed caret.
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    r.app.resolve_and_repaint(560.0, 600.0);
    r.app.resolve_and_repaint(560.0, 600.0);
    let after_caret = caret_top(&r.app);
    eprintln!("r837 resize caret before={before:?} after={after_caret:?}");
    assert_ne!(
        before, after_caret,
        "positive control: the reflow moved the caret"
    );
    let after = scroll_of(&r.app, r.scroller);
    assert_eq!(
        after, 0.0,
        "a resize must not scroll the user back to the caret (got {after})"
    );
}

/// A programmatic edit ABOVE the caret (the shape a collaborator's insertion
/// takes: the local caret is mapped, not moved by the user) shifts the caret's
/// geometry. Browsers do not scroll a contenteditable to the caret on a DOM
/// mutation elsewhere.
#[test]
fn r837_an_edit_above_the_caret_does_not_yank_the_scroller_back() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "control");
    // Insert text into the FIRST paragraph without moving the selection
    // (the selection is mapped through the step, as a remote change's is).
    let sel_before = r.handle.selection();
    let ok = r.handle.update(|state| {
        let mut tr = state.tr();
        let text = state
            .schema()
            .text(&" extra words that make the first paragraph wrap onto more lines".repeat(4))
            .unwrap();
        tr.replace_with(1, 1, rinch_editor_core::Fragment::from_node(text))
            .unwrap();
        Some(tr)
    });
    assert!(ok);
    assert_ne!(
        r.handle.selection(),
        sel_before,
        "positive control: the caret was mapped"
    );
    r.app.resolve_and_repaint(800.0, 600.0);
    r.app.resolve_and_repaint(800.0, 600.0);
    let after = scroll_of(&r.app, r.scroller);
    assert_eq!(
        after, 0.0,
        "an edit above the caret must not scroll the user back (got {after})"
    );
}

/// Block virtualization: the editor is its own scroller with >= 60 blocks. The
/// user wheel-scrolls into the middle; blocks there materialize and replace the
/// 24px estimate with their real height, so every block below — the caret's
/// included — moves. That move must not yank the user back to the caret.
#[test]
fn r837_scrolling_a_virtualized_editor_does_not_yank_back_to_the_caret() {
    let mut r = rig(
        300,
        "width: 800px; height: 600px",
        "width: 400px; height: 300px; overflow-y: auto",
        true,
        800.0,
    );
    // The user wheel-scrolls straight to the bottom (a scrollbar drag: one
    // jump, so the blocks in between are never materialized and keep the 24px
    // estimate), then clicks into the last paragraph.
    for _ in 0..6 {
        user_scrolls_to(&mut r.app, r.scroller, 1.0e9);
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let doc = r.handle.doc();
    r.handle
        .set_selection(Selection::cursor(Pos(doc.content_size() - 1)));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        let cur = scroll_of(&r.app, r.scroller);
        user_scrolls_to(&mut r.app, r.scroller, cur);
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let bottom = scroll_of(&r.app, r.scroller);
    let caret0 = caret_top(&r.app);
    eprintln!("r837 virt: bottom={bottom} caret={caret0:?}");
    assert!(
        caret0.is_some() && bottom > 0.0,
        "positive control: caret placed at the bottom"
    );
    // Now the user drags the scrollbar to a third of the way down.
    let mid = (bottom / 3.0).round();
    user_scrolls_to(&mut r.app, r.scroller, mid);
    for _ in 0..4 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let after = scroll_of(&r.app, r.scroller);
    eprintln!(
        "r837 virt: mid={mid} after={after} caret={:?}",
        caret_top(&r.app)
    );
    assert_ne!(
        caret_top(&r.app),
        caret0,
        "positive control: materializing moved the caret's geometry"
    );
    assert!(
        (after - mid).abs() < 1.0,
        "wheel scroll to {mid} was undone to {after} (caret scroll was {bottom})"
    );
}

#[test]
fn r837_probe_editor_as_scroller_small() {
    let mut r = rig(
        50,
        "width: 800px; height: 600px",
        "width: 400px; height: 300px; overflow-y: auto",
        true,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
}

/// Nested scrollers: the editor's 120px scroller sits 2000px down the page
/// (the body is the outer scroller). A caret move scrolls the inner scroller;
/// does anything bring the scroller itself on screen, as `scrollIntoView` does
/// on the web (every scrollable ancestor)?
#[test]
#[ignore = "issue #842: desktop scrolls only the nearest scroll container"]
fn r837_nested_scrollers_the_outer_one_follows_too() {
    let ids: Rc<Cell<Option<(usize, usize)>>> = Rc::new(Cell::new(None));
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..40).map(|i| format!("<p>line {i}</p>")).collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px");
        let spacer = scope.create_element("div");
        spacer.set_attribute("style", "height: 2000px");
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", "width: 400px; height: 120px; overflow-y: auto");
        let editor = handle_in.mount(scope);
        scroller.append_child(&editor);
        root.append_child(&spacer);
        root.append_child(&scroller);
        ids_in.set(Some((scroller.node_id().0, editor.node_id().0)));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (scroller, editor) = ids.get().unwrap();
    app.focus_target = FocusTarget::Editor(editor);
    let body = app.doc.as_ref().unwrap().borrow().tree.body_id;
    let doc = handle.doc();
    handle.set_selection(Selection::cursor(Pos(doc.content_size() - 1)));
    app.refresh_editor_overlays();
    app.resolve_and_repaint(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (inner, outer) = (scroll_of(&app, scroller), scroll_of(&app, body));
    eprintln!("r837 nested: inner={inner} body={outer}");
    assert!(inner > 0.0, "positive control: the inner scroller scrolled");
    assert!(
        outer > 0.0,
        "the page did not follow (body scroll {outer}); the caret is still 1400px below the window"
    );
}
