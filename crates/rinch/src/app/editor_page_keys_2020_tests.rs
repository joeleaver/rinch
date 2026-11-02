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
    scroller: u64,
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
    let ids: Rc<Cell<u64>> = Rc::new(Cell::new(0));
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

fn scroll_of(app: &RinchApp, id: u64) -> f64 {
    app.doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .get(id)
        .expect("a live node")
        .scroll_offset
        .1
}

fn max_scroll(app: &RinchApp, id: u64) -> f64 {
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
        if bt <= y
            && bands
                .get(i + 1)
                .copied()
                .flatten()
                .is_some_and(|(nt, _)| y < nt)
        {
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
    assert_eq!(
        r.handle.selection(),
        before,
        "plain Page Down: the caret stays"
    );
    let scrolled = scroll_of(&r.app, r.scroller);
    assert!((scrolled - STEP).abs() <= 1.0, "...and a page scrolls");
    press(&mut r.app, KeyCode::PageUp, NONE);
    assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
    let option = Modifiers { alt: true, ..NONE };
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
    let (x0, mut y0, _) = caret(&r);
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
        let (x, y, h) = caret(&r);
        // Resolved against the block's own text, so laid out by then.
        assert!(
            (x - x0).abs() < 6.0,
            "press {press_no}: the caret kept its column (x {x0} -> {x})"
        );
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
    assert!(
        last > 100,
        "control: the pages went well past the first 40 ({last})"
    );
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
            assert!(
                x < x0 - 20.0,
                "control: a short line ends left of the column"
            );
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

// ───────────────────────── review-2028 probes ─────────────────────────
mod review_2028 {
    use super::*;
    use rinch_components::Modal;
    use rinch_core::{Component, Signal};

    fn body_scroll(app: &RinchApp) -> f64 {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.nodes[d.tree.body_id].scroll_offset.1
    }

    /// Where Page Down (Up) lands with the caret at `(block, 6)` in view —
    /// the scroller revealing it — in a fresh 40-block rig: the baseline the
    /// off-view presses must match.
    fn in_view_landing(block: usize, key: KeyCode) -> Pos {
        let mut r = rig(40, false);
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, block, 6)));
        r.handle.focus();
        idle(&mut r.app);
        press(&mut r.app, key, NONE);
        r.handle.selection().head()
    }

    /// The caret's band is inside the scroller's padding box, on screen.
    fn assert_caret_in_view(r: &Rig, what: &str) {
        let (_, y, h) = caret(r);
        let d = r.app.doc.as_ref().unwrap().borrow();
        let b = rinch_dom::paint::painted_border_box(&d.tree, r.scroller, 1.0);
        assert!(
            f64::from(y) >= b.y0 - 0.5 && f64::from(y + h) <= b.y1 + 0.5,
            "{what}: the caret ({y}..{}) is in the scroller's view ({}..{})",
            y + h,
            b.y0,
            b.y1
        );
    }

    /// R1 (F1): the caret scrolled out of view above (the wheel moved the
    /// scroller away from it), then Page Down. Chrome 155 and Firefox 155
    /// move the caret a page from where it is, wherever the view is (caret
    /// in block 2, scroller at 0, 400 or 800: Chrome lands in block 15 and
    /// Firefox in 11 each time), and the view follows the caret. The page
    /// used to resolve on screen, miss, and jump to the document's end.
    #[test]
    fn r1_page_down_with_the_caret_scrolled_above_the_view() {
        let want = in_view_landing(2, KeyCode::PageDown);
        let mut r = rig(40, false);
        start(&mut r);
        {
            let mut d = r.app.doc.as_ref().unwrap().borrow_mut();
            let _ = RinchApp::scroll_vertically_by(&mut d, r.scroller, 400.0);
        }
        r.app.resolve_and_repaint(800.0, 600.0);
        idle(&mut r.app);
        let s0 = scroll_of(&r.app, r.scroller);
        let (_, cy, _) = caret(&r);
        let d_top = {
            let d = r.app.doc.as_ref().unwrap().borrow();
            rinch_dom::paint::painted_border_box(&d.tree, r.scroller, 1.0).y0
        };
        assert!(
            s0 > 300.0 && f64::from(cy) < d_top,
            "control: the caret ({cy}) is above the view (scroll {s0})"
        );
        assert!(
            (3..30).contains(&block_of(&r.handle, want)),
            "control: an in-view page lands mid-document ({want:?})"
        );
        press(&mut r.app, KeyCode::PageDown, NONE);
        let got = r.handle.selection().head();
        assert_eq!(
            got, want,
            "Page Down from an off-view caret lands a page below the caret, as in view"
        );
        assert_caret_in_view(&r, "R1");
        assert!(
            scroll_of(&r.app, r.scroller) < max_scroll(&r.app, r.scroller) - 50.0,
            "the view did not jump to the bottom"
        );
    }

    /// R1b (F1): the caret scrolled below the view, then Page Up.
    #[test]
    fn r1b_page_up_with_the_caret_scrolled_below_the_view() {
        let want = in_view_landing(30, KeyCode::PageUp);
        let mut r = rig(40, false);
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, 30, 6)));
        r.handle.focus();
        idle(&mut r.app);
        assert!(
            scroll_of(&r.app, r.scroller) > 300.0,
            "control: the reveal scrolled to the caret"
        );
        {
            let mut d = r.app.doc.as_ref().unwrap().borrow_mut();
            let _ = RinchApp::scroll_vertically_by(&mut d, r.scroller, -10000.0);
        }
        r.app.resolve_and_repaint(800.0, 600.0);
        idle(&mut r.app);
        assert_eq!(
            scroll_of(&r.app, r.scroller),
            0.0,
            "control: wheeled to the top"
        );
        assert!(
            (3..30).contains(&block_of(&r.handle, want)),
            "control: an in-view page lands mid-document ({want:?})"
        );
        press(&mut r.app, KeyCode::PageUp, NONE);
        let got = r.handle.selection().head();
        assert_eq!(
            got, want,
            "Page Up from a caret below the view lands a page above the caret, as in view"
        );
        assert_caret_in_view(&r, "R1b");
    }

    /// R1c (F1): the caret above the view, then Page Up: a page above the
    /// caret, not the document's start (the caret is in block 20).
    #[test]
    fn r1c_page_up_with_the_caret_scrolled_above_the_view() {
        let want = in_view_landing(20, KeyCode::PageUp);
        let mut r = rig(40, false);
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, 20, 6)));
        r.handle.focus();
        idle(&mut r.app);
        {
            let mut d = r.app.doc.as_ref().unwrap().borrow_mut();
            let _ = RinchApp::scroll_vertically_by(&mut d, r.scroller, 10000.0);
        }
        r.app.resolve_and_repaint(800.0, 600.0);
        idle(&mut r.app);
        assert!(
            (3..20).contains(&block_of(&r.handle, want)),
            "control: {want:?}"
        );
        press(&mut r.app, KeyCode::PageUp, NONE);
        let got = r.handle.selection().head();
        assert_eq!(got, want, "a page above the caret, as in view");
        assert_caret_in_view(&r, "R1c");
    }

    /// Where Page Down from `(block, 6)` lands in view in the virtualized
    /// rig (300 blocks, the editor the scroller).
    fn virtualized_landing(block: usize) -> Pos {
        let mut r = rig_with(&blocks_html(300), true, 526, 9);
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, block, 6)));
        r.handle.focus();
        idle(&mut r.app);
        press(&mut r.app, KeyCode::PageDown, NONE);
        r.handle.selection().head()
    }

    /// R1d (F1, virtualized): the caret far below the view in an editor that
    /// lays out only the blocks near the view. The point a page below the
    /// caret is in blocks that are not laid out until the scroller is brought
    /// to them; the page lands exactly where it does with the caret in view.
    #[test]
    fn r1d_virtualized_editor_with_the_caret_far_from_the_view() {
        let want = virtualized_landing(150);
        let mut r = rig_with(&blocks_html(300), true, 526, 9);
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, 150, 6)));
        r.handle.focus();
        idle(&mut r.app);
        let (x0, _, _) = caret(&r);
        {
            let mut d = r.app.doc.as_ref().unwrap().borrow_mut();
            let _ = RinchApp::scroll_vertically_by(&mut d, r.scroller, -100_000.0);
        }
        r.app.resolve_and_repaint(800.0, 600.0);
        idle(&mut r.app);
        assert_eq!(
            scroll_of(&r.app, r.scroller),
            0.0,
            "control: wheeled to the top"
        );
        let landing = block_of(&r.handle, want);
        assert!(
            block_bands(&r)[landing].is_none(),
            "control: block {landing}, where the page lands, is not laid out"
        );
        press(&mut r.app, KeyCode::PageDown, NONE);
        assert_eq!(
            r.handle.selection().head(),
            want,
            "the page from the off-view caret lands where it does in view"
        );
        let (x, _, _) = caret(&r);
        assert!((x - x0).abs() < 6.0, "the column held: x {x0} -> {x}");
    }

    /// R2: a short document (no scroll possible): Page Down to the end, Page Up
    /// to the start.
    #[test]
    fn r2_short_document() {
        let mut r = rig(3, false);
        start(&mut r);
        press(&mut r.app, KeyCode::PageDown, NONE);
        assert_eq!(
            r.handle.selection().head(),
            Pos(r.handle.doc().content_size() - 1)
        );
        press(&mut r.app, KeyCode::PageUp, NONE);
        assert_eq!(r.handle.selection().head(), Pos(1));
    }

    /// R3: the editor is the page (no inner scroller; the body scrolls).
    #[test]
    fn r3_editor_in_a_scrolling_body() {
        let handle = crate::editor::create_editor();
        assert!(handle.load_html(&blocks_html(80)));
        let h = handle.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px");
            let editor = h.mount(scope);
            editor.set_styles(&[
                ("font-family", "sans-serif"),
                ("font-size", "16px"),
                ("line-height", "1.25"),
            ]);
            root.append_child(&editor);
            root
        });
        app.register_app_font(AppFont::sans_serif(INTER));
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let mut r = Rig {
            app,
            handle,
            scroller: 0,
        };
        idle(&mut r.app);
        r.scroller = r.app.doc.as_ref().unwrap().borrow().tree.body_id;
        start(&mut r);
        let (cx, cy, _) = caret(&r);
        press(&mut r.app, KeyCode::PageDown, NONE);
        let (nx, ny, _) = caret(&r);
        let s = body_scroll(&r.app);
        eprintln!(
            "R3: body scroll {s}, block {}, caret ({cx},{cy}) -> ({nx},{ny})",
            block_of(&r.handle, r.handle.selection().head())
        );
        assert!(
            (s - 525.0).abs() < 2.0,
            "a page of the viewport (0.875×600) scrolled: {s}"
        );
        assert!((ny - cy).abs() < 30.0);
    }

    /// R4 (F2): an editor inside a `lock_scroll` Modal over a page that
    /// scrolls. The page behind must not move, and the caret moves: the short
    /// editor has nothing a page below its caret, so it goes to the end. It
    /// used to page `<body>` (0 → 525) and leave the caret where it was.
    #[test]
    fn r4_editor_in_a_lock_scroll_modal_does_not_scroll_the_page() {
        let r = r4_with(KeyCode::PageDown, NONE, true);
        let end = Pos(r.handle.doc().content_size() - 1);
        assert_eq!(
            r.handle.selection().head(),
            end,
            "the caret went to the end"
        );
    }
    /// R4d (F2): an unlocked modal stops the walk too, at its `position:
    /// fixed` root: nothing outside it moves it.
    #[test]
    fn r4d_editor_in_an_unlocked_modal_does_not_scroll_the_page() {
        let r = r4_with(KeyCode::PageDown, NONE, false);
        let end = Pos(r.handle.doc().content_size() - 1);
        assert_eq!(
            r.handle.selection().head(),
            end,
            "the caret went to the end"
        );
        // And Page Up back to the start, the page still at 0.
        let mut r = r;
        press(&mut r.app, KeyCode::PageUp, NONE);
        assert_eq!(r.handle.selection().head(), Pos(1));
        assert_eq!(body_scroll(&r.app), 0.0);
    }
    #[test]
    fn r4b_ctrl_end_control() {
        r4_with(KeyCode::End, Modifiers { ctrl: true, ..NONE }, true);
    }
    #[test]
    fn r4c_arrow_down_control() {
        r4_with(KeyCode::ArrowDown, NONE, true);
    }
    /// The page behind a modal holding a 10-block editor, the caret in block
    /// 2: `key` must leave the page at 0.
    fn r4_with(key: KeyCode, m: Modifiers, lock_scroll: bool) -> Rig {
        let handle = crate::editor::create_editor();
        assert!(handle.load_html(&blocks_html(10)));
        let h = handle.clone();
        let open = Signal::new(true);
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px");
            let tall = scope.create_element("div");
            tall.set_attribute("style", "width: 800px; height: 3000px");
            root.append_child(&tall);
            let editor = h.mount(scope);
            editor.set_styles(&[
                ("font-family", "sans-serif"),
                ("font-size", "16px"),
                ("line-height", "1.25"),
            ]);
            let modal = Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                lock_scroll,
                ..Default::default()
            }
            .render(scope, &[editor]);
            root.append_child(&modal);
            root
        });
        app.register_app_font(AppFont::sans_serif(INTER));
        app.mount_component(800.0, 600.0);
        {
            let doc = app.doc.as_ref().unwrap();
            let mut d = doc.borrow_mut();
            d.load_css(&rinch_theme::generate_theme_css(
                &rinch_theme::Theme::default(),
            ));
            d.load_css(&rinch_components::generate_component_css());
            d.recompute_all_styles_full();
        }
        app.resolve_and_repaint(800.0, 600.0);
        let mut r = Rig {
            app,
            handle,
            scroller: 0,
        };
        idle(&mut r.app);
        {
            let d = r.app.doc.as_ref().unwrap().borrow();
            assert!(
                rinch_dom::paint::scrollbar::overflows(
                    &d.tree,
                    d.tree.body_id,
                    rinch_dom::paint::scrollbar::ScrollbarAxis::Vertical
                ),
                "control: the page overflows"
            );
            assert_eq!(
                !d.tree.scroll_lock_roots.is_empty(),
                lock_scroll,
                "control: the lock is as asked"
            );
        }
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, 2, 6)));
        r.handle.focus();
        idle(&mut r.app);
        assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
        assert_eq!(body_scroll(&r.app), 0.0, "control: the page starts at 0");
        let before = r.handle.selection().head();
        press(&mut r.app, key, m);
        assert_eq!(
            body_scroll(&r.app),
            0.0,
            "{key:?} in a modal scrolled the page behind it"
        );
        assert_ne!(
            r.handle.selection().head(),
            before,
            "control: the caret moved"
        );
        r
    }

    /// R4e (F2): the scroll lock on its own, with no fixed box: an editor in
    /// an ordinary `div` on a page that scrolls, while a lock is held on
    /// another box. The page is locked out, as for the wheel, so Page Down
    /// leaves it, and the caret moves a page (the caret is near the top, so
    /// the page lands on screen).
    #[test]
    fn r4e_the_scroll_lock_alone_keeps_the_page() {
        let want = in_page_landing(0, false);
        let mut r = rig_in_page(true);
        start_in_page(&mut r, 0);
        press(&mut r.app, KeyCode::PageDown, NONE);
        assert_eq!(body_scroll(&r.app), 0.0, "the locked page did not move");
        let got = block_of(&r.handle, r.handle.selection().head());
        assert_eq!(got, want, "the caret moved a page");
    }

    /// R4e's control: with no lock the same page pages.
    #[test]
    fn r4e_control_without_the_lock_the_page_pages() {
        let mut r = rig_in_page(false);
        start_in_page(&mut r, 0);
        press(&mut r.app, KeyCode::PageDown, NONE);
        assert!(
            (body_scroll(&r.app) - 525.0).abs() < 2.0,
            "a page of the viewport: {}",
            body_scroll(&r.app)
        );
    }

    /// R4f (F1 + F2): on the locked page a page from a caret lower down lands
    /// past the window's bottom, where nothing can scroll it in and the hit
    /// test finds nothing. It is resolved from the layout: the same block the
    /// unlocked page lands in, not the document's end.
    #[test]
    fn r4f_a_page_past_the_window_on_a_locked_page_lands_by_layout() {
        let want = in_page_landing(4, false);
        let mut r = rig_in_page(true);
        start_in_page(&mut r, 4);
        let (_, cy, ch) = caret(&r);
        assert!(
            f64::from(cy + ch * 0.5) + 525.0 > 600.0,
            "control: the target ({}) is past the window",
            f64::from(cy + ch * 0.5) + 525.0
        );
        press(&mut r.app, KeyCode::PageDown, NONE);
        let got = block_of(&r.handle, r.handle.selection().head());
        assert!((5..59).contains(&want), "control: {want}");
        assert_eq!(got, want, "the caret moved a page, not to the end");
    }

    /// Where Page Down from `(block, 6)` lands on the in-page rig.
    fn in_page_landing(block: usize, lock: bool) -> usize {
        let mut r = rig_in_page(lock);
        start_in_page(&mut r, block);
        press(&mut r.app, KeyCode::PageDown, NONE);
        block_of(&r.handle, r.handle.selection().head())
    }

    /// A 60-block editor in a `div` on a page that scrolls (the body is the
    /// scroller), and an unrelated empty `div` that — `lock` — holds a scroll
    /// lock.
    fn rig_in_page(lock: bool) -> Rig {
        let handle = crate::editor::create_editor();
        assert!(handle.load_html(&blocks_html(60)));
        let h = handle.clone();
        let other: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let other_in = other.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px");
            let editor = h.mount(scope);
            editor.set_styles(&[
                ("font-family", "sans-serif"),
                ("font-size", "16px"),
                ("line-height", "1.25"),
            ]);
            let wrap = scope.create_element("div");
            wrap.append_child(&editor);
            root.append_child(&wrap);
            let lock_root = scope.create_element("div");
            root.append_child(&lock_root);
            other_in.set(lock_root.node_id().0);
            root
        });
        app.register_app_font(AppFont::sans_serif(INTER));
        app.mount_component(800.0, 600.0);
        if lock {
            app.doc
                .as_ref()
                .unwrap()
                .borrow_mut()
                .tree
                .scroll_lock_roots
                .push(other.get());
        }
        app.resolve_and_repaint(800.0, 600.0);
        let mut r = Rig {
            app,
            handle,
            scroller: 0,
        };
        idle(&mut r.app);
        r.scroller = r.app.doc.as_ref().unwrap().borrow().tree.body_id;
        r
    }

    fn start_in_page(r: &mut Rig, block: usize) {
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, block, 6)));
        r.handle.focus();
        idle(&mut r.app);
        assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
        assert_eq!(body_scroll(&r.app), 0.0, "control: the page starts at 0");
    }

    /// F3: two editors stacked in one scroller, the first short. A page from
    /// the first lands in the second's text: the caret goes to the first
    /// editor's end and never takes a position the hit test read from the
    /// other editor.
    #[test]
    fn f3_a_page_into_a_neighbouring_editor_goes_to_this_ones_end() {
        let first = crate::editor::create_editor();
        assert!(first.load_html(&blocks_html(4)));
        let second = crate::editor::create_editor();
        assert!(second.load_html(&blocks_html(40)));
        let (a, b) = (first.clone(), second.clone());
        let ids: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let ids_in = ids.clone();
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px; height: 600px");
            let scroller = scope.create_element("div");
            scroller.set_attribute(
                "style",
                "width: 500px; height: 226px; padding: 13px; overflow-y: auto",
            );
            for h in [&a, &b] {
                let editor = h.mount(scope);
                editor.set_styles(&[
                    ("font-family", "sans-serif"),
                    ("font-size", "16px"),
                    ("line-height", "1.25"),
                ]);
                scroller.append_child(&editor);
            }
            root.append_child(&scroller);
            ids_in.set(scroller.node_id().0);
            root
        });
        app.register_app_font(AppFont::sans_serif(INTER));
        app.mount_component(800.0, 600.0);
        app.resolve_and_repaint(800.0, 600.0);
        let mut r = Rig {
            app,
            handle: first,
            scroller: ids.get(),
        };
        idle(&mut r.app);
        // The caret in the first editor's first block: a page (~198px) below
        // is well past its four blocks, in the second editor's text.
        r.handle
            .set_selection(Selection::cursor(pos_in_block(&r.handle, 0, 6)));
        r.handle.focus();
        idle(&mut r.app);
        assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
        let FocusTarget::Editor(container) = r.app.focus_target else {
            unreachable!()
        };
        let (gx, cy, ch) = caret(&r);
        // Positive control: what is a page below the caret is the second
        // editor's text — scroll a page by hand, hit-test the caret's place
        // on screen, and scroll back.
        let wheel = |r: &mut Rig, dy: f64| {
            {
                let mut d = r.app.doc.as_ref().unwrap().borrow_mut();
                let _ = RinchApp::scroll_vertically_by(&mut d, r.scroller, dy);
            }
            r.app.resolve_and_repaint(800.0, 600.0);
        };
        wheel(&mut r, STEP);
        let hit = r
            .app
            .editor_point(gx, cy + ch * 0.5)
            .expect("control: a page below the caret is on an editor");
        assert_ne!(
            hit.container, container,
            "control: a page below the caret is the second editor"
        );
        wheel(&mut r, -STEP);
        idle(&mut r.app);
        assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
        press(&mut r.app, KeyCode::PageDown, NONE);
        let end = Pos(r.handle.doc().content_size() - 1);
        assert_eq!(
            r.handle.selection().head(),
            end,
            "the caret went to its own editor's end"
        );
        assert!(
            second.selection().head() < Pos(3),
            "the second editor untouched"
        );
    }

    /// F3b: a second editor in a `position: fixed` box covers the strip of
    /// the window where a page from the first editor's caret lands. The hit
    /// test names the covering editor; the page is resolved in the first
    /// editor's own layout and lands where it does with nothing over it.
    #[test]
    fn f3b_a_page_under_another_editor_lands_in_this_one() {
        let landing = |cover: bool| {
            let mut r = rig_covered(cover);
            r.handle
                .set_selection(Selection::cursor(pos_in_block(&r.handle, 2, 6)));
            r.handle.focus();
            idle(&mut r.app);
            assert!(matches!(r.app.focus_target, FocusTarget::Editor(_)));
            let FocusTarget::Editor(container) = r.app.focus_target else {
                unreachable!()
            };
            let (gx, cy, ch) = caret(&r);
            press(&mut r.app, KeyCode::PageDown, NONE);
            assert!((scroll_of(&r.app, r.scroller) - STEP).abs() <= 1.0);
            // Where the page's target is on screen now: the caret's old place.
            let hit = r.app.editor_point(gx, cy + ch * 0.5).map(|h| h.container);
            (r.handle.selection().head(), hit == Some(container))
        };
        let (bare, bare_hit) = landing(false);
        let (covered, covered_hit) = landing(true);
        assert!(bare_hit, "control: uncovered, the hit is in this editor");
        assert!(
            !covered_hit,
            "control: covered, the hit is the other editor"
        );
        let end = Pos(rig(40, false).handle.doc().content_size() - 1);
        assert_ne!(bare, end, "control: the page lands mid-document");
        assert_eq!(covered, bare, "the covered page lands as the bare one");
    }

    /// `rig(40, false)` with — `cover` — a second editor in a fixed box over
    /// the window's strip 90..160.
    fn rig_covered(cover: bool) -> Rig {
        let ids: Rc<Cell<u64>> = Rc::new(Cell::new(0));
        let ids_in = ids.clone();
        let handle = crate::editor::create_editor();
        assert!(handle.load_html(&blocks_html(40)));
        let other = crate::editor::create_editor();
        assert!(other.load_html(&blocks_html(4)));
        let (h, o) = (handle.clone(), other.clone());
        let mut app = RinchApp::new(move |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_attribute("style", "width: 800px; height: 600px");
            let scroller = scope.create_element("div");
            scroller.set_attribute(
                "style",
                "width: 500px; height: 226px; padding: 13px; overflow-y: auto",
            );
            root.append_child(&scroller);
            for e in [&h, &o] {
                if !cover && std::ptr::eq(e, &o) {
                    continue;
                }
                let editor = e.mount(scope);
                editor.set_styles(&[
                    ("font-family", "sans-serif"),
                    ("font-size", "16px"),
                    ("line-height", "1.25"),
                ]);
                if std::ptr::eq(e, &h) {
                    scroller.append_child(&editor);
                } else {
                    let fixed = scope.create_element("div");
                    fixed.set_attribute(
                        "style",
                        "position: fixed; top: 90px; left: 0; width: 800px; height: 70px; \
                         overflow: hidden; background: white",
                    );
                    fixed.append_child(&editor);
                    root.append_child(&fixed);
                }
            }
            ids_in.set(scroller.node_id().0);
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

    /// R5: key auto-repeat delivered back to back (no frame between): each
    /// repeat is one page.
    #[test]
    fn r5_auto_repeat_back_to_back() {
        let mut r = rig(40, false);
        start(&mut r);
        for i in 0..3 {
            r.app.handle_event(
                PlatformEvent::KeyDown {
                    key: KeyCode::PageDown,
                    logical_key: None,
                    text: None,
                    modifiers: NONE,
                    repeat: if i == 0 {
                        KeyRepeat::Fresh
                    } else {
                        KeyRepeat::Repeat
                    },
                },
                VP,
                1.0,
            );
        }
        idle(&mut r.app);
        let s = scroll_of(&r.app, r.scroller);
        let b = block_of(&r.handle, r.handle.selection().head());
        eprintln!("R5: scroll {s} (3 pages = {}), block {b}", 3.0 * STEP);
        assert!((s - 3.0 * STEP).abs() <= 2.0);
        let (_, y, _) = caret(&r);
        eprintln!("R5 caret y {y}");
    }

    /// R6: the app's on_key sees PageDown first and can take it.
    #[test]
    fn r6_on_key_overrides() {
        let mut r = rig(40, false);
        start(&mut r);
        let seen = Rc::new(Cell::new(0));
        let s2 = seen.clone();
        r.handle.on_key(move |k| {
            if k.key == "PageDown" {
                s2.set(s2.get() + 1);
                true
            } else {
                false
            }
        });
        let before = r.handle.selection();
        press(&mut r.app, KeyCode::PageDown, NONE);
        assert_eq!(seen.get(), 1, "on_key saw PageDown");
        assert_eq!(r.handle.selection(), before);
        assert_eq!(scroll_of(&r.app, r.scroller), 0.0);
    }

    /// R7: a table in the middle: Page Down lands inside a cell.
    #[test]
    fn r7_table() {
        let mut html = blocks_html(6);
        html.push_str("<table><tbody>");
        for i in 0..20 {
            html.push_str(&format!(
                "<tr><td><p>cell a{i:02}</p></td><td><p>cell b{i:02}</p></td></tr>"
            ));
        }
        html.push_str("</tbody></table>");
        html.push_str(&blocks_html(10));
        let mut r = rig_with(&html, false, 226, 16);
        start(&mut r);
        let (cx, cy, _) = caret(&r);
        let mut heads = vec![];
        for _ in 0..4 {
            press(&mut r.app, KeyCode::PageDown, NONE);
            let (x, y, _) = caret(&r);
            heads.push((r.handle.selection().head(), x, y));
        }
        eprintln!(
            "R7 from ({cx},{cy}): {heads:?} end {}",
            r.handle.doc().content_size()
        );
        // Each page moves on and none jumps to the end; the first pages land
        // in the table's left column, at the caret's column.
        let end = Pos(r.handle.doc().content_size() - 1);
        assert!(heads.windows(2).all(|w| w[1].0 > w[0].0), "{heads:?}");
        assert!(heads.iter().take(2).all(|h| h.0 != end), "{heads:?}");
        assert!(
            heads.iter().take(2).all(|h| (h.1 - cx).abs() < 10.0),
            "{heads:?}"
        );
    }

    /// R8: RTL text keeps its column.
    #[test]
    fn r8_rtl() {
        let html: String = (0..40)
            .map(|i| format!("<p dir=\"rtl\">שלום עולם טוב {i:02}</p>"))
            .collect();
        let mut r = rig_with(&html, false, 226, 16);
        start(&mut r);
        let (cx, cy, _) = caret(&r);
        press(&mut r.app, KeyCode::PageDown, NONE);
        let (x, y, _) = caret(&r);
        eprintln!(
            "R8: ({cx},{cy}) -> ({x},{y}) block {}",
            block_of(&r.handle, r.handle.selection().head())
        );
        assert!((x - cx).abs() < 10.0);
    }
}
