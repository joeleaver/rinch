//! Enter at the end of a paragraph in a scrolled editor keeps the scroller
//! where the caret is.
//!
//! Seen in Pimble: a 9,500-word note in a pane (an `overflow-y: auto` flex
//! column around the editor), scrolled ~2,400px, a click at the end of a
//! paragraph, End, Enter — the paragraph was split and the pane jumped to
//! scrollTop 0. Enter at a paragraph's end makes a fresh, EMPTY paragraph. The
//! input path's caret pass runs before the next layout, and the view placed an
//! empty block's caret at the block's box origin — which, for an element never
//! laid out, is its parent's origin: the top of the editor. The caret scroll
//! (`ScrollGate`, armed by the edit) was fulfilled on that caret, so the pane
//! scrolled to the editor's top. A textblock with text answered "no geometry
//! yet" there and the scroll waited for the layout; an empty one now does too.
//!
//! The text is the bundled Inter at a declared 16px / 24px.
#![cfg(feature = "desktop")]
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::{Cell, RefCell};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const PANE_H: f64 = 500.0;
const WORDS: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod \
    tempor incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud \
    exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat duis aute irure dolor \
    in reprehenderit in voluptate velit esse cillum";

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    pane: u64,
}

/// `paragraphs` paragraphs of ~48 words in a 500x500 pane. `flex_pane`: the
/// pane is Pimble's `.pimble-editor__content-wrap` (a scrolling flex column,
/// its own stacking context, the editor a `flex: 1 1 auto` item); otherwise a
/// plain block that scrolls.
fn rig(paragraphs: usize, flex_pane: bool) -> Rig {
    let pane_id = Rc::new(Cell::new(0));
    let pane_in = pane_id.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..paragraphs)
        .map(|i| format!("<p>{i} {WORDS}</p>"))
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px; display: flex");
        let pane = scope.create_element("div");
        pane.set_attribute(
            "style",
            if flex_pane {
                "width: 500px; height: 500px; display: flex; flex-direction: column; \
                 overflow-y: auto; position: relative; z-index: 0"
            } else {
                "width: 500px; height: 500px; overflow-y: auto"
            },
        );
        let editor = handle_in.mount(scope);
        editor.set_styles(&[
            ("font-family", "sans-serif"),
            ("font-size", "16px"),
            ("line-height", "24px"),
        ]);
        if flex_pane {
            editor.set_styles(&[("flex", "1 1 auto"), ("border", "none")]);
        }
        pane.append_child(&editor);
        root.append_child(&pane);
        pane_in.set(pane.node_id().0);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut r = Rig {
        app,
        handle,
        pane: pane_id.get(),
    };
    idle(&mut r.app);
    r
}

/// The desktop loop between two events: `AboutToWait`, then a frame.
fn idle(app: &mut RinchApp) {
    for _ in 0..4 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if app.has_pending_layout() || actions.contains(&AppAction::RequestRedraw) {
            app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        }
        let _ = app.build_pixels(1.0, VP, false);
    }
}

fn scroll_of(app: &RinchApp, id: u64) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn ev(app: &mut RinchApp, e: PlatformEvent) {
    app.handle_event(e, VP, 1.0);
}

fn press(app: &mut RinchApp, key: KeyCode) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
    );
    idle(app);
}

fn click(app: &mut RinchApp, x: f32, y: f32) {
    for e in [
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Left,
        },
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Left,
        },
    ] {
        ev(app, e);
    }
    idle(app);
}

/// Wheel the pane down (100px notches) until it is at `target` or further.
fn wheel_to(app: &mut RinchApp, pane: u64, target: f64) {
    for _ in 0..400 {
        if scroll_of(app, pane) >= target {
            return;
        }
        let before = scroll_of(app, pane);
        ev(
            app,
            PlatformEvent::MouseWheel {
                x: 250.0,
                y: 250.0,
                delta_x: 0.0,
                delta_y: -100.0,
            },
        );
        idle(app);
        if scroll_of(app, pane) == before {
            return; // at the end of the range
        }
    }
}

/// The caret to the end of the paragraph it is in (End reaches the end of the
/// visual line; Joe's click was past a paragraph's last line).
fn caret_to_paragraph_end(r: &mut Rig) {
    let head = r.handle.selection().head();
    let rp = r.handle.doc().resolve(head).unwrap();
    let end = rp.end(rp.depth());
    r.handle.set_selection(Selection::cursor(Pos(end)));
    idle(&mut r.app);
}

/// Enter, with the positive control that it made a new, empty paragraph.
fn enter(r: &mut Rig) {
    let blocks = r.handle.doc().child_count();
    press(&mut r.app, KeyCode::Enter);
    assert_eq!(
        r.handle.doc().child_count(),
        blocks + 1,
        "positive control: Enter split the paragraph"
    );
    let head = r.handle.selection().head();
    let rp = r.handle.doc().resolve(head).unwrap();
    assert_eq!(
        rp.parent().content_size(),
        0,
        "positive control: the caret is in a new, empty paragraph"
    );
}

/// The caret's window-space box (the caret overlay, as `caret_rect` reads it).
fn caret_y(r: &Rig) -> f32 {
    let head = r.handle.selection().head();
    r.handle.caret_rect(head).expect("caret geometry").y
}

/// Pimble's shape: click a line in the middle of the scrolled pane, End, the
/// caret at the paragraph's end, Enter. The pane must stay where it is, and the
/// caret be drawn on the new line, inside the pane.
fn enter_mid_pane_keeps_the_scroll(flex_pane: bool) {
    let mut r = rig(200, flex_pane);
    wheel_to(&mut r.app, r.pane, 2400.0);
    let before = scroll_of(&r.app, r.pane);
    assert!(
        before >= 2400.0,
        "positive control: the wheel scrolled ({before})"
    );
    click(&mut r.app, 300.0, 200.0);
    assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
    press(&mut r.app, KeyCode::End);
    caret_to_paragraph_end(&mut r);
    let caret_before = caret_y(&r);
    assert_eq!(scroll_of(&r.app, r.pane), before, "control: no scroll yet");
    enter(&mut r);
    let after = scroll_of(&r.app, r.pane);
    assert_eq!(
        after, before,
        "Enter moved the pane from {before} to {after} (flex_pane: {flex_pane})"
    );
    let y = caret_y(&r);
    assert!(
        y > caret_before && y < caret_before + 60.0,
        "the caret is on the new line just below ({caret_before} -> {y})"
    );
}

#[test]
fn enter_at_a_paragraph_end_in_a_scrolled_flex_pane_keeps_the_scroll() {
    enter_mid_pane_keeps_the_scroll(true);
}

#[test]
fn enter_at_a_paragraph_end_in_a_scrolled_block_pane_keeps_the_scroll() {
    enter_mid_pane_keeps_the_scroll(false);
}

/// The scroll is not dropped, only deferred to the layout: Enter at the end of
/// the document, with the pane at the bottom, scrolls DOWN to the new line
/// (it is below the pane's bottom edge until then).
#[test]
fn enter_at_the_bottom_scrolls_down_to_the_new_line() {
    let mut r = rig(60, false);
    wheel_to(&mut r.app, r.pane, f64::MAX);
    let bottom = scroll_of(&r.app, r.pane);
    assert!(
        bottom > 1000.0,
        "positive control: the wheel scrolled ({bottom})"
    );
    let end = r.handle.doc().content_size() - 1;
    r.handle.set_selection(Selection::cursor(Pos(end)));
    r.handle.focus();
    idle(&mut r.app);
    assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
    let at_end = scroll_of(&r.app, r.pane);
    enter(&mut r);
    enter(&mut r);
    let after = scroll_of(&r.app, r.pane);
    assert!(
        after > at_end,
        "the pane followed the caret down ({at_end} -> {after})"
    );
    let y = caret_y(&r) as f64;
    assert!(
        (0.0..PANE_H).contains(&y),
        "the caret ({y}) is inside the pane [0, {PANE_H})"
    );
}

/// `caret_rect` for the new empty paragraph, asked before the next layout (an
/// `on_selection_change` callback): "no geometry yet", as for a textblock
/// edited since the last layout — not the editor's top.
#[test]
fn caret_rect_of_a_new_empty_paragraph_waits_for_its_layout() {
    let mut r = rig(200, true);
    wheel_to(&mut r.app, r.pane, 2400.0);
    click(&mut r.app, 300.0, 200.0);
    caret_to_paragraph_end(&mut r);
    let seen: Rc<RefCell<Vec<Option<f32>>>> = Rc::new(RefCell::new(Vec::new()));
    let (seen_in, h) = (seen.clone(), r.handle.clone());
    r.handle.on_selection_change(move |sel| {
        seen_in
            .borrow_mut()
            .push(h.caret_rect(sel.head()).map(|b| b.y));
    });
    enter(&mut r);
    let seen = seen.borrow();
    assert!(!seen.is_empty(), "positive control: the callback ran");
    for y in seen.iter().flatten() {
        assert!(
            *y > 0.0,
            "caret_rect answered the editor's top ({y}) for an unlaid paragraph: {seen:?}"
        );
    }
    assert!(
        caret_y(&r) > 0.0,
        "after the layout it answers the new line"
    );
}
