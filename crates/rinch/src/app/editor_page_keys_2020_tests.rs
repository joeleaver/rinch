//! Page Up / Page Down in the rich-text editor (#2020), through the real
//! `PlatformEvent::KeyDown` path.
//!
//! What Chrome 155 does in a `contenteditable` (measured headless, the probe
//! is in the PR): Page Down moves the caret down by **0.875 of the scrollport's
//! height** (`ScrollableArea::MinFractionToStepWhenPaging`), keeping its x;
//! Page Up the reverse; Shift extends; a page that reaches the last (first)
//! line puts the caret at the document's end (start). A read-only field
//! scrolls a page and leaves the caret where it is. Here the scroller moves by
//! the same amount as the caret, so the caret keeps its place on screen.
//!
//! Off the fixed points: the scroller has 13px of padding (its padding box,
//! 226px, is the page), it is much shorter than the document, the blocks mix
//! 20px paragraphs with 30px headings, and the caret starts mid-line in the
//! third block with the scroller at 0. Text is the bundled Inter at declared
//! sizes, so x geometry does not move with the host's fonts.
#![cfg(feature = "desktop")]
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
/// The scroller's padding box: 200px of content height plus 2 × 13px padding.
const PAGE_BOX: f64 = 226.0;
/// Chrome's page step: 0.875 of the scrollport.
const STEP: f64 = PAGE_BOX * 0.875;

struct Rig {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    scroller: usize,
}

fn blocks_html(n: usize) -> String {
    (0..n)
        .map(|i| {
            if i % 3 == 1 {
                format!("<h2>abcdefghij {i:02}</h2>")
            } else {
                format!("<p>abcdefghij {i:02}</p>")
            }
        })
        .collect()
}

/// `blocks` blocks in a 500 × 200 scroller with 13px padding (the editor
/// inside it), or — `editor_is_scroller` — the editor itself the scroller,
/// which is what turns on block virtualization at >= 60 blocks.
fn rig(blocks: usize, editor_is_scroller: bool) -> Rig {
    let ids: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&blocks_html(blocks)));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_styles(&[
            ("font-family", "sans-serif"),
            ("font-size", "16px"),
            // A number: a paragraph's line is 20px, a heading's 30px.
            ("line-height", "1.25"),
        ]);
        let scroller_style = "width: 500px; height: 200px; padding: 13px; overflow-y: auto";
        if editor_is_scroller {
            editor.set_attribute("style", scroller_style);
            editor.set_styles(&[
                ("font-family", "sans-serif"),
                ("font-size", "16px"),
                ("line-height", "1.25"),
            ]);
            root.append_child(&editor);
            ids_in.set(editor.node_id().0);
        } else {
            let scroller = scope.create_element("div");
            scroller.set_attribute("style", scroller_style);
            scroller.append_child(&editor);
            root.append_child(&scroller);
            ids_in.set(scroller.node_id().0);
        }
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut r = Rig {
        app,
        handle,
        scroller: ids.get(),
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

fn scroll_of(app: &RinchApp, id: usize) -> f64 {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id]
        .scroll_offset
        .1
}

fn max_scroll(app: &RinchApp, id: usize) -> f64 {
    let d = app.doc.as_ref().unwrap().borrow();
    let nid = rinch_core::dom::NodeId(id);
    (d.scroll_height(nid) - d.client_height(nid)).max(0.0)
}

fn press(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
    app.handle_event(
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
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    idle(app);
}

const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    meta: false,
};
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ctrl: false,
    alt: false,
    meta: false,
};

/// The model position `off` characters into top-level block `i`.
fn pos_in_block(handle: &crate::editor::EditorHandle, i: usize, off: usize) -> Pos {
    let doc = handle.doc();
    let before: usize = (0..i).map(|j| doc.child(j).node_size()).sum();
    Pos(before + 1 + off)
}

/// The top-level block a position is in.
fn block_of(handle: &crate::editor::EditorHandle, pos: Pos) -> usize {
    let doc = handle.doc();
    let mut start = 0;
    for j in 0..doc.child_count() {
        let size = doc.child(j).node_size();
        if pos.0 < start + size {
            return j;
        }
        start += size;
    }
    doc.child_count()
}

/// Each block's painted box `(top, bottom)` in window space, for the blocks
/// that are laid out.
fn block_bands(r: &Rig) -> Vec<Option<(f64, f64)>> {
    let n = r.handle.doc().child_count();
    let doc = r.app.doc.as_ref().unwrap().borrow();
    (0..n)
        .map(|i| {
            let (tb, _) = r.handle.caret_address(pos_in_block(&r.handle, i, 0))?;
            doc.tree.get(tb)?.text_layout.as_ref()?;
            let b = rinch_dom::paint::painted_border_box(&doc.tree, tb, 1.0);
            Some((b.y0, b.y1))
        })
        .collect()
}

/// Where Chrome's page step puts the caret: the block whose band holds the
/// point one step below (above) the caret's middle, measured BEFORE the key —
/// or, in the gap between two blocks, either of them.
fn expected_blocks(bands: &[Option<(f64, f64)>], y: f64) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, b) in bands.iter().enumerate() {
        let Some((t, bt)) = *b else { continue };
        if (t..bt).contains(&y) {
            return vec![i];
        }
        if bt <= y && bands.get(i + 1).copied().flatten().is_some_and(|(nt, _)| y < nt) {
            out = vec![i, i + 1];
        }
    }
    out
}

fn caret(r: &Rig) -> (f32, f32, f32) {
    let head = r.handle.selection().head();
    r.app
        .editor_caret_point(&r.handle, head)
        .expect("the caret has geometry")
}

/// The caret in block 2, six characters in, the editor focused, the
/// scroller at 0.
fn start(r: &mut Rig) {
    r.handle
        .set_selection(Selection::cursor(pos_in_block(&r.handle, 2, 6)));
    r.handle.focus();
    idle(&mut r.app);
    assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "control: not scrolled");
}

#[test]
fn page_down_moves_the_caret_a_page_and_the_scroller_with_it() {
    let mut r = rig(40, false);
    start(&mut r);
    let (cx, cy, ch) = caret(&r);
    let bands = block_bands(&r);
    let want = expected_blocks(&bands, f64::from(cy + ch * 0.5) + STEP);
    assert!(!want.is_empty(), "control: the target is laid out");

    press(&mut r.app, KeyCode::PageDown, NONE);

    let head = r.handle.selection().head();
    let got = block_of(&r.handle, head);
    assert!(
        want.contains(&got),
        "Page Down put the caret in block {got}; one page ({STEP}px) below is {want:?}"
    );
    assert!(
        r.handle.selection().is_empty(),
        "a plain Page Down collapses"
    );
    let scrolled = scroll_of(&r.app, r.scroller);
    assert!(
        (scrolled - STEP).abs() <= 1.0,
        "the scroller moved {scrolled}px, one page is {STEP}px"
    );
    let (nx, ny, _) = caret(&r);
    assert!(
        (nx - cx).abs() < 8.0,
        "the caret kept its column: x {cx} -> {nx}"
    );
    assert!(
        (ny - cy).abs() < 30.0,
        "the caret kept its place on screen: y {cy} -> {ny}"
    );

    // And back: Page Up undoes it.
    press(&mut r.app, KeyCode::PageUp, NONE);
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0, "Page Up scrolled back");
    assert_eq!(
        block_of(&r.handle, r.handle.selection().head()),
        2,
        "Page Up went back to the starting block"
    );
    let (bx, _, _) = caret(&r);
    assert!((bx - cx).abs() < 8.0, "the column held: x {cx} -> {bx}");
}

#[test]
fn page_down_at_the_end_goes_to_the_document_end_and_page_up_to_its_start() {
    let mut r = rig(40, false);
    start(&mut r);
    let end = Pos(r.handle.doc().content_size() - 1);
    let mut last = r.handle.selection().head();
    for i in 0..12 {
        press(&mut r.app, KeyCode::PageDown, NONE);
        let head = r.handle.selection().head();
        assert!(head >= last, "press {i}: Page Down moved the caret back");
        last = head;
    }
    assert_eq!(last, end, "Page Down ends at the document's end");
    let bottom = max_scroll(&r.app, r.scroller);
    assert!(bottom > 400.0, "control: the document overflows ({bottom})");
    assert!(
        (scroll_of(&r.app, r.scroller) - bottom).abs() <= 1.0,
        "the scroller is at its end"
    );
    for _ in 0..12 {
        press(&mut r.app, KeyCode::PageUp, NONE);
    }
    assert_eq!(
        r.handle.selection().head(),
        Pos(1),
        "Page Up ends at the start"
    );
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
}

#[test]
fn shift_page_down_extends_the_selection() {
    let mut r = rig(40, false);
    start(&mut r);
    let anchor = r.handle.selection().head();
    let (_, cy, ch) = caret(&r);
    let want = expected_blocks(&block_bands(&r), f64::from(cy + ch * 0.5) + STEP);
    press(&mut r.app, KeyCode::PageDown, SHIFT);
    let sel = r.handle.selection();
    assert_eq!(sel.anchor(), anchor, "Shift keeps the anchor");
    assert!(
        want.contains(&block_of(&r.handle, sel.head())),
        "the head went a page down ({:?}, want {want:?})",
        block_of(&r.handle, sel.head())
    );
    press(&mut r.app, KeyCode::PageUp, SHIFT);
    let sel = r.handle.selection();
    assert_eq!(sel.anchor(), anchor, "Shift keeps the anchor");
    assert_eq!(block_of(&r.handle, sel.head()), 2);
}

/// Chrome's read-only `<textarea>` (and a non-editable scroller): a page of
/// scroll, the selection untouched.
#[test]
fn a_read_only_editor_scrolls_a_page_and_keeps_the_caret() {
    let mut r = rig(40, false);
    start(&mut r);
    r.handle.set_read_only(true);
    idle(&mut r.app);
    let before = r.handle.selection();
    press(&mut r.app, KeyCode::PageDown, NONE);
    assert_eq!(r.handle.selection(), before, "the caret stays");
    let scrolled = scroll_of(&r.app, r.scroller);
    assert!(
        (scrolled - STEP).abs() <= 1.0,
        "a page of scroll ({scrolled}, want {STEP})"
    );
    press(&mut r.app, KeyCode::PageDown, SHIFT);
    assert_eq!(r.handle.selection(), before, "Shift does not select either");
    press(&mut r.app, KeyCode::PageUp, NONE);
    press(&mut r.app, KeyCode::PageUp, NONE);
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
}

/// macOS (Chrome's Mac key bindings): Page Down scrolls and leaves the caret,
/// Option+Page Down moves it, Shift+Page Down extends.
#[test]
fn on_macos_page_down_scrolls_and_option_page_down_moves() {
    let mut r = rig(40, false);
    r.app.text_keymap = TextKeymap::MacOs;
    start(&mut r);
    let before = r.handle.selection();
    press(&mut r.app, KeyCode::PageDown, NONE);
    assert_eq!(r.handle.selection(), before, "plain Page Down: the caret stays");
    let scrolled = scroll_of(&r.app, r.scroller);
    assert!((scrolled - STEP).abs() <= 1.0, "...and a page scrolls");
    press(&mut r.app, KeyCode::PageUp, NONE);
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
    let option = Modifiers {
        alt: true,
        ..NONE
    };
    press(&mut r.app, KeyCode::PageDown, option);
    assert!(r.handle.selection().is_empty());
    assert!(
        block_of(&r.handle, r.handle.selection().head()) > 2,
        "Option+Page Down moves the caret"
    );
    press(&mut r.app, KeyCode::PageUp, SHIFT);
    assert!(!r.handle.selection().is_empty(), "Shift+Page Up extends");
}

/// A virtualized editor (its own scroller, 300 blocks): the blocks a page
/// lands in were never laid out until the page scrolled to them. Every press
/// moves the caret on, into a block with geometry, inside the scroller.
#[test]
fn page_down_in_a_virtualized_editor_lands_in_blocks_never_laid_out() {
    let mut r = rig(300, true);
    start(&mut r);
    let far = 150;
    assert!(
        block_bands(&r)[far].is_none(),
        "positive control: block {far} is not laid out (virtualized)"
    );
    let scroller_box = {
        let d = r.app.doc.as_ref().unwrap().borrow();
        let b = rinch_dom::paint::painted_border_box(&d.tree, r.scroller, 1.0);
        (b.y0, b.y1)
    };
    let mut last = 2;
    let mut presses = 0;
    while last < far {
        press(&mut r.app, KeyCode::PageDown, NONE);
        presses += 1;
        assert!(presses < 60, "Page Down stalled at block {last}");
        let got = block_of(&r.handle, r.handle.selection().head());
        assert!(got > last, "press {presses}: block {last} -> {got}");
        // About a page: a 226px page holds 8 to 12 of these blocks.
        assert!(
            (5..=16).contains(&(got - last)),
            "press {presses}: moved {} blocks ({last} -> {got})",
            got - last
        );
        let (_, y, h) = caret(&r);
        assert!(
            f64::from(y) >= scroller_box.0 && f64::from(y + h) <= scroller_box.1,
            "press {presses}: the caret ({y}) is inside the scroller {scroller_box:?}"
        );
        last = got;
    }
}
