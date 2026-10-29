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

/// `blocks` blocks in a 500 × 226 scroller with 13px padding (the editor
/// inside it), or — `editor_is_scroller` — the editor itself the scroller,
/// which is what turns on block virtualization at >= 60 blocks.
fn rig(blocks: usize, editor_is_scroller: bool) -> Rig {
    rig_with(&blocks_html(blocks), editor_is_scroller, 226, 16)
}

/// `height`: the scroller's border box, which is its padding box here.
/// `font_px`: the editor's font size (its lines are 1.25 of it).
fn rig_with(html: &str, editor_is_scroller: bool, height: u32, font_px: u32) -> Rig {
    let ids: Rc<Cell<usize>> = Rc::new(Cell::new(0));
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_styles(&[
            ("font-family", "sans-serif"),
            ("font-size", &format!("{font_px}px")),
            // A number: a paragraph's line is 20px, a heading's 30px.
            ("line-height", "1.25"),
        ]);
        // Rinch sizes every box as `border-box` (#1278): 226px is the
        // padding box Chrome's `height: 200px; padding: 13px` has.
        let scroller_style =
            format!("width: 500px; height: {height}px; padding: 13px; overflow-y: auto");
        if editor_is_scroller {
            editor.set_attribute("style", &scroller_style);
            editor.set_styles(&[
                ("font-family", "sans-serif"),
                ("font-size", &format!("{font_px}px")),
                ("line-height", "1.25"),
            ]);
            root.append_child(&editor);
            ids_in.set(editor.node_id().0);
        } else {
            let scroller = scope.create_element("div");
            scroller.set_attribute("style", &scroller_style);
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

/// A virtualized editor (its own scroller, 300 blocks) keeps 10 blocks laid
/// out past the bottom of its view (`virtual_window::OVERSCAN`). With the
/// caret on the view's last line and 9px text, a page (0.875 × 526px, some
/// 20 blocks) lands beyond them: in a block that was never laid out until the page
/// scrolled to it. Every press moves the caret on a page, into a block with
/// geometry, at its place on screen.
#[test]
fn page_down_in_a_virtualized_editor_lands_in_blocks_never_laid_out() {
    let mut r = rig_with(&blocks_html(300), true, 526, 9);
    start(&mut r);
    let view = {
        let d = r.app.doc.as_ref().unwrap().borrow();
        let b = rinch_dom::paint::painted_border_box(&d.tree, r.scroller, 1.0);
        (b.y0 + 13.0, b.y1 - 13.0)
    };
    // The caret to the last block wholly in view.
    let bands = block_bands(&r);
    let last_in_view = (0..bands.len())
        .rev()
        .find(|&i| bands[i].is_some_and(|(_, b)| b <= view.1))
        .expect("a block in view");
    r.handle
        .set_selection(Selection::cursor(pos_in_block(&r.handle, last_in_view, 6)));
    idle(&mut r.app);
    let mut last = last_in_view;
    let (_, mut y0, _) = caret(&r);
    for press_no in 1..=8 {
        let before = block_bands(&r);
        press(&mut r.app, KeyCode::PageDown, NONE);
        let got = block_of(&r.handle, r.handle.selection().head());
        // The first 40 blocks start laid out (`INITIAL_MATERIALIZED`).
        assert!(
            got < 40 || before[got].is_none(),
            "press {press_no}: positive control: block {got} was not laid out before the key"
        );
        assert!(
            (10..=25).contains(&(got - last)),
            "press {press_no}: moved {} blocks ({last} -> {got})",
            got - last
        );
        let (_, y, h) = caret(&r);
        assert!(
            f64::from(y) >= view.0 && f64::from(y + h) <= view.1,
            "press {press_no}: the caret ({y}) is inside the view {view:?}"
        );
        assert!(
            (y - y0).abs() < 30.0,
            "press {press_no}: the caret kept its place on screen ({y0} -> {y})"
        );
        (last, y0) = (got, y);
    }
    assert!(last > 100, "control: the pages went well past the first 40 ({last})");
}

/// A page that lands on the document's last line goes to the document's end
/// (Chrome), not to the goal column on that line. The scroller's height is
/// chosen so that a page from a block's caret lands mid-way down the last
/// line.
#[test]
fn a_page_that_lands_on_the_last_line_goes_to_the_end() {
    let probe = rig(40, false);
    let bands = block_bands(&probe);
    let n = bands.len();
    let (lt, lb) = bands[n - 1].unwrap();
    let k = n - 7;
    let (kt, kb) = bands[k].unwrap();
    // 0.875 × height = the distance between the two lines' middles.
    let height = (((lt + lb) - (kt + kb)) / 2.0 / 0.875).round();
    drop(probe);
    let mut r = rig_with(&blocks_html(40), false, height as u32, 16);
    r.handle
        .set_selection(Selection::cursor(pos_in_block(&r.handle, k, 3)));
    r.handle.focus();
    idle(&mut r.app);
    let start_scroll = scroll_of(&r.app, r.scroller);
    let bands = block_bands(&r);
    let (lt, lb) = bands[n - 1].unwrap();
    let (_, cy, ch) = caret(&r);
    let target = f64::from(cy + ch * 0.5) + height * 0.875;
    assert!(
        target > lt + 2.0 && target < lb - 2.0,
        "control: a page from block {k} lands on the last line ({target} in {lt}..{lb}, \
         scrolled {start_scroll})"
    );
    press(&mut r.app, KeyCode::PageDown, NONE);
    assert_eq!(
        r.handle.selection().head(),
        Pos(r.handle.doc().content_size() - 1),
        "the document's end, not the column on its last line"
    );
}

/// The goal column survives a page that lands on a short line, as it does a
/// run of Up / Down: the next page goes back to it.
#[test]
fn the_goal_column_survives_a_short_line() {
    // Every fourth block is two characters long.
    let html: String = (0..60)
        .map(|i| match i % 4 {
            3 => format!("<p>{i:02}</p>"),
            1 => format!("<h2>abcdefghij {i:02}</h2>"),
            _ => format!("<p>abcdefghij {i:02}</p>"),
        })
        .collect();
    let mut r = rig_with(&html, false, 226, 16);
    r.handle
        .set_selection(Selection::cursor(pos_in_block(&r.handle, 2, 12)));
    r.handle.focus();
    idle(&mut r.app);
    let (x0, _, _) = caret(&r);
    let (mut short, mut long_after_short) = (false, false);
    for _ in 0..5 {
        press(&mut r.app, KeyCode::PageDown, NONE);
        let block = block_of(&r.handle, r.handle.selection().head());
        let (x, _, _) = caret(&r);
        if block % 4 == 3 {
            short = true;
            assert!(x < x0 - 20.0, "control: a short line ends left of the column");
        } else {
            long_after_short |= short;
            assert!(
                (x - x0).abs() < 8.0,
                "block {block}: the caret is back at the goal column (x {x0} -> {x})"
            );
        }
    }
    assert!(
        short && long_after_short,
        "positive control: a page landed on a short line and a later one on a long line"
    );
}
