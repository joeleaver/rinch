//! The editor caret's scroll-into-view, over a real editor in a real scroller:
//! what brings the caret into view, and — the half these mostly pin — what must
//! not pull a user who scrolled away back to it.
//!
//! #837 first gated the scroll on the caret overlay's *geometry* changing, and
//! its review measured four ways that pulls a user back to a caret that did not
//! move in the document (a virtualized editor re-measuring blocks, an edit above
//! the caret, a resize reflow, and a peer's edit on the web). The gate is now
//! the handle's `ScrollGate`, armed only by a local edit or selection move and by
//! focus; these are its end-to-end pins. The handle's own unit tests pin the
//! state machine.
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
    for _ in 0..4 {
        r.app.resolve_and_repaint(vw, 600.0);
    }
    assert!(
        scroll_of(&r.app, r.scroller) > 0.0,
        "positive control: the caret move scrolled"
    );
}

/// A window resize reflows the text above the caret. The caret's geometry key
/// changes, but the caret did not MOVE in the document; a browser does not
/// scroll to the caret on resize. A user who scrolled away should stay.
#[test]
fn a_resize_reflow_does_not_yank_the_scroller_back_to_the_caret() {
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
fn an_edit_above_the_caret_does_not_yank_the_scroller_back() {
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
fn scrolling_a_virtualized_editor_does_not_yank_back_to_the_caret() {
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

/// Nested scrollers: the editor's 120px scroller sits 2000px down the page
/// (the body is the outer scroller). A caret move scrolls the inner scroller;
/// does anything bring the scroller itself on screen, as `scrollIntoView` does
/// on the web (every scrollable ancestor)?
#[test]
#[ignore = "issue #842: desktop scrolls only the nearest scroll container"]
fn nested_scrollers_the_outer_one_follows_too() {
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
    assert!(inner > 0.0, "positive control: the inner scroller scrolled");
    assert!(
        outer > 0.0,
        "the page did not follow (body scroll {outer}); the caret is still 1400px below the window"
    );
}

/// Typing that wraps the caret onto a new line below the fold is scrolled to
/// where the caret really lands — one input event and one frame per word, the
/// real desktop path (the input handler's caret pass, then the frame's).
#[test]
fn typing_that_wraps_below_the_fold_follows_the_caret_onto_the_new_line() {
    let mut r = rig(
        4,
        "width: 800px; height: 600px; font-size: 16px; line-height: 20px",
        "width: 300px; height: 100px; overflow-y: auto",
        false,
        800.0,
    );
    let doc = r.handle.doc();
    r.handle
        .set_selection(Selection::cursor(Pos(doc.content_size() - 1)));
    r.app.refresh_editor_overlays();
    r.app.resolve_and_repaint(800.0, 600.0);
    r.app.resolve_and_repaint(800.0, 600.0);
    let start = scroll_of(&r.app, r.scroller);
    // Type word by word, one input event and one frame each, as a user would.
    for _ in 0..30 {
        assert!(r.handle.insert_text(" wrap"));
        r.app.refresh_editor_overlays();
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let scroll = scroll_of(&r.app, r.scroller);
    assert!(
        scroll > start,
        "positive control: typing scrolled ({start} -> {scroll})"
    );
    let style = caret_top(&r.app).expect("a caret");
    let num = |k: &str| {
        style
            .split(';')
            .find_map(|d| {
                let (n, v) = d.split_once(':')?;
                (n.trim() == k)
                    .then(|| v.trim().trim_end_matches("px").parse::<f64>().ok())
                    .flatten()
            })
            .expect(k)
    };
    let (top, height) = (num("top"), num("height"));
    assert!(
        top >= scroll - 0.5 && top + height <= scroll + 100.5,
        "the caret [{top}, {}] is inside the scrolled band [{scroll}, {}]",
        top + height,
        scroll + 100.0
    );
}

// ── review-846 fixtures ──────────────────────────────────────────────────────

fn r846_ev(app: &mut RinchApp, e: PlatformEvent) {
    app.handle_event(e, (800, 600), 1.0);
}

fn r846_click(app: &mut RinchApp, x: f32, y: f32) {
    r846_ev(
        app,
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
    );
    r846_ev(
        app,
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    );
}

/// Caret at the end (scrolled), the user scrolls back to the top, 3 frames: 0.
fn r846_scrolled_away(r: &mut Rig) {
    caret_to_end(r, 800.0);
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(
        scroll_of(&r.app, r.scroller),
        0.0,
        "control: user stayed at 0"
    );
}

/// A click in the checkbox gutter of a task item at the TOP of an unfocused
/// editor toggles the box and does not move the caret. It also focuses the
/// editor, and the focus arm then scrolls to the old caret at the bottom —
/// away from the checkbox the user just clicked.
#[test]
fn r846_a_focusing_checkbox_click_does_not_yank_to_the_old_caret() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    // Make the first paragraph a task item via the markdown input rule.
    r.handle.set_selection(Selection::cursor(Pos(1)));
    for c in ["[", " ", "]", " "] {
        assert!(r.handle.insert_text(c));
    }
    assert_eq!(
        r.handle.doc().child(0).type_name(),
        "task_list",
        "positive control: task list"
    );
    r.app.resolve_and_repaint(800.0, 600.0);
    r846_scrolled_away(&mut r);
    // Blur the editor (focus elsewhere), and let the frame hide its overlays.
    r.app.set_focus_target(FocusTarget::None);
    r.app.refresh_editor_overlays();
    r.app.resolve_and_repaint(800.0, 600.0);
    let (bx, by) = {
        let doc = r.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let item = (0..d.tree.nodes.len())
            .find(|&id| {
                d.tree.get(id).is_some_and(|n| {
                    n.attributes.get("data-pm-type").map(String::as_str) == Some("task_item")
                })
            })
            .expect("a task item");
        let (x, y, _, h) = painted_element_box(&d.tree, item);
        (x + 4.0, y + h / 2.0)
    };
    let checked = |r: &Rig| {
        r.handle
            .doc()
            .child(0)
            .child(0)
            .attrs()
            .get_bool("checked")
            .unwrap_or(false)
    };
    assert!(!checked(&r));
    r846_click(&mut r.app, bx, by);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert!(checked(&r), "positive control: the click toggled the box");
    assert!(
        r.app.focused_editor_id().is_some(),
        "positive control: focused"
    );
    let after = scroll_of(&r.app, r.scroller);
    assert_eq!(
        after, 0.0,
        "the checkbox click yanked the scroller to the old caret ({after})"
    );
}

/// Alt-tab away and back: `WindowFocus(false)` then `(true)`. A browser does
/// not scroll a contenteditable to its caret on window refocus.
#[test]
fn r846_window_refocus_does_not_scroll_to_the_caret() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    r846_scrolled_away(&mut r);
    r846_ev(&mut r.app, PlatformEvent::WindowFocus(false));
    r.app.resolve_and_repaint(800.0, 600.0);
    r846_ev(&mut r.app, PlatformEvent::WindowFocus(true));
    for _ in 0..3 {
        r.app.refresh_editor_overlays();
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
}

/// An IME's surrounding-text delete at an off-screen caret is the user's own
/// input at the caret (the IME recomposing what they typed), so it reveals the
/// caret like Backspace does — even though the transaction only maps the
/// selection, which through the public `update(..)` would not scroll.
#[test]
fn an_ime_surrounding_delete_at_an_offscreen_caret_reveals_it() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    r846_scrolled_away(&mut r);
    let size = r.handle.doc().content_size();
    r.handle.ime_delete_surrounding(2, 0);
    assert!(
        r.handle.doc().content_size() < size,
        "positive control: the IME delete landed"
    );
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert!(
        scroll_of(&r.app, r.scroller) > 0.0,
        "the IME delete at the caret did not reveal it"
    );
}

/// Toolbar Bold with a collapsed caret changes stored marks only: no scroll.
/// Undo that restores text at the (off-screen) caret: scrolls.
#[test]
fn r846_toggle_bold_does_not_scroll_undo_does() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
    assert!(r.handle.insert_text("X"));
    r.app.refresh_editor_overlays();
    r.app.resolve_and_repaint(800.0, 600.0);
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    r.app.resolve_and_repaint(800.0, 600.0);
    assert!(r.handle.command("toggleBold"));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "bold must not scroll");
    // ...nor leave a request queued for the user's next wheel to drain.
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    r.app.resolve_and_repaint(800.0, 600.0);
    assert_eq!(
        scroll_of(&r.app, r.scroller),
        0.0,
        "bold queued a stale scroll"
    );
    assert!(r.handle.command("undo"));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert!(
        scroll_of(&r.app, r.scroller) > 0.0,
        "undo reveals the change"
    );
}

/// An app inserting text AT the caret through the public `update` (tr.insert_text
/// sets the selection) scrolls; one that inserts at the caret position with a
/// raw step (no set_selection) does not.
#[test]
fn r846_public_update_scrolls_only_with_set_selection() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    r846_scrolled_away(&mut r);
    assert!(r.handle.update(|s| {
        let mut tr = s.tr();
        let at = s.selection.head().0;
        let t = s.schema().text("raw").unwrap();
        tr.replace_with(at, at, rinch_editor_core::Fragment::from_node(t))
            .unwrap();
        Some(tr)
    }));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let raw = scroll_of(&r.app, r.scroller);
    assert!(r.handle.update(|s| {
        let mut tr = s.tr();
        tr.insert_text("typed").unwrap();
        Some(tr)
    }));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let typed = scroll_of(&r.app, r.scroller);
    eprintln!("R846 raw-step scroll={raw} insert_text scroll={typed}");
    assert_eq!(raw, 0.0);
    assert!(typed > 0.0);
}

/// Delete (forward) at an off-screen caret: the caret does not move, the doc
/// changes. The PR says it "still reveals" the caret. Measure when.
#[test]
fn r846_forward_delete_at_an_offscreen_caret_reveals_it_at_once_not_on_the_next_wheel() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
    let end = r.handle.doc().content_size() - 1;
    r.handle.set_selection(Selection::cursor(Pos(end - 4)));
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "control");
    let before = r.handle.selection();
    assert!(r.handle.command("deleteCharForward"));
    assert_eq!(r.handle.selection(), before, "control: caret did not move");
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let s1 = scroll_of(&r.app, r.scroller);
    // Idle frames with nothing dirty.
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let s1b = scroll_of(&r.app, r.scroller);
    // The user scrolls away again: nothing stale may pull them back.
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let s2 = scroll_of(&r.app, r.scroller);
    eprintln!("R846 delete: after delete {s1}, idle {s1b}, after next wheel {s2}");
    assert!(s1 > 0.0, "the delete did not reveal the caret in its frame");
    assert_eq!(s2, 0.0, "a stale scroll pulled the user back");
}

/// A command that changes the document at an off-screen caret without moving
/// the caret overlay (align-left on an already left-aligned paragraph). The
/// gate fires on a caret pass whose overlay did not move, so desktop's in-frame
/// apply (which runs only when an overlay moved) is skipped and the request
/// sits in the queue. Whatever it does, it must not fire on the user's NEXT
/// wheel.
#[test]
fn r846_a_doc_change_that_does_not_move_the_caret_leaves_no_stale_scroll() {
    let mut r = rig(
        40,
        "width: 800px; height: 600px",
        "width: 400px; height: 160px; overflow-y: auto",
        false,
        800.0,
    );
    caret_to_end(&mut r, 800.0);
    assert!(r.handle.command("insertTable"), "control: table inserted");
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "control: user at 0");
    let before_doc = r.handle.doc();
    let caret0 = caret_top(&r.app);
    let sel0 = r.handle.selection();
    assert!(
        r.handle.command("addRowAfter"),
        "control: the command applied"
    );
    assert_eq!(r.handle.selection(), sel0, "control: selection unchanged");
    assert!(
        !r.handle.doc().same_ref(&before_doc),
        "control: the doc changed"
    );
    r.app.refresh_editor_overlays();
    for _ in 0..3 {
        r.app.resolve_and_repaint(800.0, 600.0);
    }
    let s1 = scroll_of(&r.app, r.scroller);
    assert_eq!(
        caret_top(&r.app),
        caret0,
        "control: the caret overlay did not move"
    );
    user_scrolls_to(&mut r.app, r.scroller, 0.0);
    r.app.resolve_and_repaint(800.0, 600.0);
    let s2 = scroll_of(&r.app, r.scroller);
    eprintln!("R846 align: after command {s1}, after next wheel {s2}");
    assert_eq!(
        s2, 0.0,
        "a stale scroll fired on the user's next wheel ({s1} -> {s2})"
    );
}
