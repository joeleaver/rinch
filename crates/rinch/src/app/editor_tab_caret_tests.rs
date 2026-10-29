//! A tab in an editor textblock on desktop (#1109, #1122).
//!
//! rinch-dom lays a preserved tab out as one space widened to its tab stop
//! (#1122), so a tab is one byte of the flat offsets its text queries take
//! and its pointer hit answers (`DomDocument::tab_flat_bytes`), as it is in
//! the model. (It was four spaces, four bytes, until then; #1109 taught the
//! caret map to count them.)
//!
//! **The oracle is Chrome 155** on the same text in a `pre-wrap` block of
//! the bundled Inter at `16px/24px` — the editor's prose and code block are
//! both `pre-wrap`, and both faces are Inter here. A space is 4.5 wide, so
//! the stops fall every 36px: `a\tb\tc` draws its carets at
//! 0, 8.98, 36, 45.80, 72, 81.14. Measured in Chrome 153 on
//! `<pre contenteditable>a\tb\tc</pre>` (#1109), a press on the tab's left
//! quarter lands before it (offset 1), on its right quarter after it (2),
//! and on `b`'s right quarter after `b` (3).
//!
//! Two tabs per line, so a caret map off by a byte per tab is off by two
//! after the second.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;

fn idle(app: &mut RinchApp) {
    for _ in 0..3 {
        let actions = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        if actions.contains(&AppAction::RequestRedraw) {
            if app.has_pending_layout() {
                app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            }
            let _ = app.build_pixels(1.0, VP, false);
        }
    }
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// One textblock in a 400px editor, Inter 16px / 24px for both the prose and
/// the code block's monospace, focused and settled.
fn page(html: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 400px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        let style = scope.create_element("style");
        let css = scope.create_text(
            "[data-pm-editor] pre, [data-pm-editor] code { font-size: 16px; \
             line-height: 24px; padding: 0; margin: 0; }",
        );
        style.append_child(&css);
        root.append_child(&style);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.register_app_font(AppFont::monospace(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    fn block(&self) -> u64 {
        self.handle.caret_address(Pos(1)).unwrap().0
    }
    fn block_box(&self) -> (f32, f32, f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, self.block())
    }
    fn click(&mut self, x: f32, y: f32) {
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        idle(&mut self.app);
    }
    /// The head as a char offset into the textblock.
    fn head(&self) -> usize {
        self.handle.selection().head().0 - 1
    }
    /// The layout-local caret for char offset `i`, through the model's caret
    /// address — what the caret overlay is placed from.
    fn local(&self, i: usize) -> (f32, f32) {
        let (block, byte) = self.handle.caret_address(Pos(i + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(block, byte).expect("laid out")
    }
    /// The painted boxes of the visible selection-highlight rects.
    fn highlights(&self) -> Vec<(f32, f32, f32, f32)> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter(|id| {
                doc.tree.nodes[id.0].computed_style.display
                    != rinch_dom::computed_style::DisplayValue::None
            })
            .map(|id| painted_element_box(&doc.tree, id.0))
            .filter(|b| b.2 > 0.0 && b.3 > 0.0)
            .collect()
    }
    fn text(&self) -> String {
        let doc = self.handle.doc();
        let block = doc.child(0);
        (0..block.child_count())
            .filter_map(|i| block.child(i).text().map(str::to_owned))
            .collect()
    }
}

/// Chrome 155's caret x for each char offset of `a\tb\tc`.
const CHROME: [f32; 6] = [0.0, 8.98, 36.0, 45.80, 72.0, 81.14];

/// Every caret on a line with two tabs is drawn where Chrome draws it: after
/// a tab is its stop, the next glyph's left edge. For both a code block and a
/// paragraph, since both are `pre-wrap` in the editor.
#[test]
fn the_caret_after_a_tab_is_drawn_at_its_stop() {
    for tabs in ["<pre><code>a\tb\tc</code></pre>", "<p>a\tb\tc</p>"] {
        let t = page(tabs);
        assert_eq!(
            t.text(),
            "a\tb\tc",
            "positive control: the tab survives the load"
        );
        for (i, want) in CHROME.into_iter().enumerate() {
            let (tx, ty) = t.local(i);
            assert_eq!(ty, t.local(0).1, "{tabs}: char {i} on one line");
            assert!(
                (tx - want).abs() < 0.05,
                "{tabs}: caret {i} at {tx}, Chrome {want}"
            );
        }
    }
}

/// A selection over `b\tc` (chars 2..5) highlights from `b`'s left edge to
/// the end of `c`: 36 to 81.14, the overlay placed at whole pixels.
#[test]
fn a_selection_across_a_tab_highlights_the_tab_and_what_follows() {
    let mut t = page("<pre><code>a\tb\tc</code></pre>");
    t.handle.set_selection(Selection::text(Pos(3), Pos(6)));
    idle(&mut t.app);
    let th = t.highlights();
    assert_eq!(th.len(), 1, "one line: {th:?}");
    let (bx, _, _, _) = t.block_box();
    let a = th[0];
    assert!(
        (a.0 - (bx + CHROME[2])).abs() <= 1.0 && (a.2 - (CHROME[5] - CHROME[2])).abs() <= 1.0,
        "tab highlight {a:?}, block at {bx}"
    );
}

/// A press on a tab's left fifth lands before it and on its right fifth after
/// it, and a press on the glyph after it lands on that glyph's nearer side —
/// Chrome 153's answers (1, 2, 3 for the first tab; 3, 4, 5 for the second).
/// The presses stay clear of the tab's middle. Where to press is read off Chrome's
/// carets, not off the caret map under test. Each press is on a fresh page,
/// so no two read as a double click.
#[test]
fn a_press_on_or_after_a_tab_lands_on_the_side_hit() {
    let press = |from: usize, frac: f32| {
        let (x0, x1) = (CHROME[from], CHROME[from + 1]);
        let mut p = page("<pre><code>a\tb\tc</code></pre>");
        let (bx, by, _, _) = p.block_box();
        p.click(bx + x0 + (x1 - x0) * frac, by + LINE / 2.0);
        p.head()
    };
    assert_eq!(press(1, 0.2), 1, "first tab, left: before it");
    assert_eq!(press(1, 0.8), 2, "first tab, right: after it");
    assert_eq!(press(2, 0.75), 3, "`b`, right half: after it");
    assert_eq!(press(3, 0.2), 3, "second tab, left: before it");
    assert_eq!(press(3, 0.8), 4, "second tab, right: after it");
    assert_eq!(press(4, 0.75), 5, "`c`, right half: the end");
}

/// A press right after the first tab, then typing, inserts there: `a\tXb\tc`.
/// The first tab still ends at 36 and the second, now after `Xb`, still at
/// 72 — Chrome 155: 0, 8.98, 36, 46.91, 56.70, 72, 81.14.
#[test]
fn typing_after_a_press_behind_a_tab_inserts_after_the_tab() {
    let mut p = page("<pre><code>a\tb\tc</code></pre>");
    let (bx, by, _, _) = p.block_box();
    let (x1, x2) = (CHROME[1], CHROME[2]);
    p.click(bx + x1 + (x2 - x1) * 0.8, by + LINE / 2.0);
    assert_eq!(p.head(), 2, "after the first tab");
    assert!(p.handle.insert_text("X"));
    idle(&mut p.app);
    assert_eq!(p.text(), "a\tXb\tc");
    assert_eq!(p.head(), 3);
    for (i, want) in [0.0, 8.98, 36.0, 46.91, 56.70, 72.0, 81.14]
        .into_iter()
        .enumerate()
    {
        let x = p.local(i).0;
        assert!((x - want).abs() < 0.05, "caret {i} at {x}, Chrome {want}");
    }
}

fn key(app: &mut RinchApp, key: KeyCode) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    idle(app);
}

/// End, Home, and ArrowDown then End on a wrapped line holding tabs land
/// where Chrome 155 breaks it (#1107's Home/End read the caret's line from its
/// layout, through the caret map). The code block is 358px wide (the
/// editor's 400 less its padding) and each `xxx\t` ends at a stop 36px on,
/// so the tenth tab ends at 360, past the edge: it hangs, and the first line
/// ends after it, char 40. The second line starts with `kkk` and ends with
/// the text, char 63. Chrome 155 at 358px draws char 20 at 180 and char 24
/// at 216 on the first line, char 52 at 144 on the second.
#[test]
fn home_and_end_on_a_wrapped_line_with_tabs_land_where_chrome_wraps() {
    let words = "aaa\tbbb\tccc\tddd\teee\tfff\tggg\thhh\tiii\tjjj\tkkk\tlll\tmmm\tnnn\tooo\tppp";
    let mut t = page(&format!("<pre><code>{words}</code></pre>"));
    // The caret inside "bbb" on the first visual line.
    t.handle.set_selection(Selection::cursor(Pos(1 + 5)));
    idle(&mut t.app);
    let line1 = t.local(5).1;
    assert_eq!(
        t.block_box().2,
        358.0,
        "positive control: the block's width"
    );
    for (i, x, line) in [(20, 180.0, 0.0), (24, 216.0, 0.0), (52, 144.0, LINE)] {
        let (cx, cy) = t.local(i);
        assert!(
            (cx - x).abs() < 0.05 && cy == line1 + line,
            "char {i} at ({cx}, {cy}), Chrome ({x}, {})",
            line1 + line
        );
    }
    key(&mut t.app, KeyCode::End);
    assert_eq!(t.head(), 40, "End");
    key(&mut t.app, KeyCode::Home);
    assert_eq!(t.head(), 0, "Home");
    key(&mut t.app, KeyCode::ArrowDown);
    key(&mut t.app, KeyCode::End);
    assert_eq!(t.head(), 63, "End on line 2");
}
