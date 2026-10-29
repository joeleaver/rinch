//! Issue #845: moving the caret into a block of a virtualized editor that was
//! never laid out must bring the caret into view.
//!
//! A desktop editor that is its own `overflow-y: auto` scroller with >= 60
//! blocks is block-virtualized: blocks outside a window around the scroll
//! position are collapsed to an estimated height and have no laid-out text. The
//! caret scroll (`ScrollGate`, armed by the caret move) is fulfilled on the
//! positioned caret overlay, which needs the caret's block laid out; the window
//! follows the scroll offset, which is what the caret scroll is waiting to
//! change. So nothing ever scrolled, and no caret was drawn.
//!
//! The document is built so the estimates are wrong: the first blocks (which
//! the window measures first, and whose average becomes every unmeasured
//! block's estimate) are one line, the rest several lines. A scroll to a
//! block's *estimated* position therefore lands far from where it is once
//! measured; the caret has to be in view after the blocks around it are.
//!
//! The text is the bundled Inter at a declared 16px / 20px.
#![cfg(feature = "desktop")]
use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const BLOCKS: usize = 300;
/// The first `SHORT` blocks are one line; the rest wrap onto several.
const SHORT: usize = 60;
const LONG: &str = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod \
    tempor incididunt ut labore et dolore magna aliqua ut enim ad minim veniam quis nostrud \
    exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat";
/// Frames a caret move may take to settle (the scroll, then the window moving
/// to it and measuring the blocks there).
const SETTLE_FRAMES: usize = 6;

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    editor: usize,
}

/// `BLOCKS` paragraphs; the editor is the 400x300 scroller, focused, caret at
/// the start, scrolled to the top.
fn rig() -> Rig {
    let id = Rc::new(Cell::new(0));
    let id_in = id.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..BLOCKS)
        .map(|i| {
            if i < SHORT {
                format!("<p>{i} short</p>")
            } else {
                format!("<p>{i} {}</p>", &LONG[..LONG.len() - (i * 7) % 60])
            }
        })
        .collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 300px; overflow-y: auto; font-family: sans-serif; \
             font-size: 16px; line-height: 20px",
        );
        root.append_child(&editor);
        id_in.set(editor.node_id().0);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.text_keymap = TextKeymap::Standard;
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let editor = id.get();
    app.focus_target = FocusTarget::Editor(editor);
    let mut r = Rig {
        app,
        handle,
        editor,
    };
    frames(&mut r, 4);
    assert_eq!(scroll_of(&r), 0.0, "control: at the top");
    r
}

/// One turn of the shell's frame clock.
fn frame(r: &mut Rig) {
    let actions = r.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    if r.app.has_pending_layout() || actions.contains(&AppAction::RequestRedraw) {
        r.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }
}

fn frames(r: &mut Rig, n: usize) {
    for _ in 0..n {
        frame(r);
    }
}

fn scroll_of(r: &Rig) -> f64 {
    r.app.doc.as_ref().unwrap().borrow().tree.nodes[r.editor]
        .scroll_offset
        .1
}

/// Run frames until the scroll offset has not moved for three in a row;
/// answer how many frames that took. Fails past `SETTLE_FRAMES + 3`.
fn settle(r: &mut Rig) -> usize {
    let mut last = scroll_of(r);
    let mut still = 0;
    for n in 1..=SETTLE_FRAMES + 3 {
        frame(r);
        let now = scroll_of(r);
        if now == last {
            still += 1;
            if still == 3 {
                return n - 3;
            }
        } else {
            still = 0;
            last = now;
        }
    }
    panic!("the scroll offset did not settle within {SETTLE_FRAMES} frames");
}

fn press(r: &mut Rig, key: KeyCode, modifiers: Modifiers) {
    r.app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    r.app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::default()
    }
}

/// The editor's direct child holding `pos`.
fn block_of(r: &Rig, pos: Pos) -> usize {
    let (mut id, _) = r.handle.caret_address(pos).expect("a caret address");
    let doc = r.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    while let Some(parent) = d.tree.nodes[id].parent {
        if parent == r.editor {
            return id;
        }
        id = parent;
    }
    panic!("{pos:?} is not in the editor");
}

fn is_laid_out(r: &Rig, block: usize) -> bool {
    r.app.doc.as_ref().unwrap().borrow().tree.nodes[block]
        .estimated_height
        .is_none()
}

/// Whether a caret overlay is drawn.
fn caret_drawn(r: &Rig) -> bool {
    let doc = r.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes.iter().any(|(_, n)| {
        n.attributes.contains_key("data-pm-caret")
            && n.attributes
                .get("style")
                .is_some_and(|s| s.contains("visibility: visible"))
    })
}

/// The caret line at the selection head sits inside the scroller's painted box.
fn assert_caret_in_view(r: &Rig, what: &str) {
    let head = r.handle.selection().head();
    let block = block_of(r, head);
    assert!(
        is_laid_out(r, block),
        "{what}: the caret's block is laid out"
    );
    assert!(caret_drawn(r), "{what}: a caret is drawn");
    let (top, bottom) = {
        let doc = r.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        let (_, y, _, h) = painted_element_box(&d.tree, r.editor);
        (y, y + h)
    };
    let line = r.handle.caret_rect(head).expect("the caret is laid out");
    assert!(
        line.y >= top - 0.5 && line.y + line.height <= bottom + 0.5,
        "{what}: the caret line [{}, {}] is inside the scroller [{top}, {bottom}] \
         (scroll {})",
        line.y,
        line.y + line.height,
        scroll_of(r)
    );
}

/// The caret's position is the same model position as before, and once settled
/// the scroller stays put: more frames move nothing.
fn assert_stays(r: &mut Rig, what: &str) {
    let at = scroll_of(r);
    frames(r, 4);
    assert_eq!(scroll_of(r), at, "{what}: nothing moves once settled");
}

/// The issue's own fixture: Ctrl+End from the top.
#[test]
fn ctrl_end_from_the_top_scrolls_to_the_last_block() {
    let mut r = rig();
    let end = Pos(r.handle.doc().content_size() - 1);
    let last = block_of(&r, end);
    assert!(
        !is_laid_out(&r, last),
        "control: the last block was never laid out"
    );
    press(&mut r, KeyCode::End, ctrl());
    assert_eq!(r.handle.selection().head(), end, "Ctrl+End moved the caret");
    let n = settle(&mut r);
    assert!(n <= SETTLE_FRAMES, "settled in {n} frames");
    assert!(scroll_of(&r) > 0.0, "it scrolled");
    assert_caret_in_view(&r, "Ctrl+End");
    assert_stays(&mut r, "Ctrl+End");
}

/// A programmatic caret far below, in the middle of the document: the scroll
/// lands at a block whose neighbours above are measured only after it.
#[test]
fn a_programmatic_caret_far_below_is_scrolled_to() {
    let mut r = rig();
    let doc = r.handle.doc();
    let mut pos = 0;
    for i in 0..200 {
        pos += doc.child(i).node_size();
    }
    let target = Pos(pos + 3);
    assert!(
        !is_laid_out(&r, block_of(&r, target)),
        "control: block 200 was never laid out"
    );
    // From app code (a timer, an effect, a deep link): the selection change
    // owes an overlay pass, which is what wakes the frame.
    r.handle.set_selection(Selection::cursor(target));
    let n = settle(&mut r);
    assert!(n <= SETTLE_FRAMES, "settled in {n} frames");
    assert_caret_in_view(&r, "set_selection");
    assert_stays(&mut r, "set_selection");
}

/// Then back: Ctrl+Home from the bottom, through the measured tail into the
/// start (laid out once, collapsed again since).
#[test]
fn ctrl_home_after_ctrl_end_comes_back_to_the_top() {
    let mut r = rig();
    press(&mut r, KeyCode::End, ctrl());
    settle(&mut r);
    assert_caret_in_view(&r, "Ctrl+End");
    press(&mut r, KeyCode::Home, ctrl());
    let n = settle(&mut r);
    assert!(n <= SETTLE_FRAMES, "settled in {n} frames");
    // `nearest`: the caret line at the scroller's top edge, which is the
    // editor's padding and the first paragraph's margin below 0.
    assert!(scroll_of(&r) < 40.0, "back at the top ({})", scroll_of(&r));
    assert_caret_in_view(&r, "Ctrl+Home");
}

/// A user who scrolls away after the caret move settled is left there: the
/// fix materializes and scrolls for an owed caret scroll only.
#[test]
fn scrolling_away_after_ctrl_end_stays_away() {
    let mut r = rig();
    press(&mut r, KeyCode::End, ctrl());
    settle(&mut r);
    assert_caret_in_view(&r, "Ctrl+End");
    let bottom = scroll_of(&r);
    let mid = (bottom / 2.0).round();
    {
        let doc = r.app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.tree.nodes[r.editor].scroll_offset.1 = mid;
        d.tree.dirty_nodes.insert(r.editor);
    }
    frames(&mut r, 6);
    assert!(
        (scroll_of(&r) - mid).abs() < 1.0,
        "a user scroll to {mid} was undone to {}",
        scroll_of(&r)
    );
}
