//! #1109: a tab in an editor textblock on desktop.
//!
//! rinch-dom's inline formatting context lays a tab out as four spaces, so a
//! tab is four bytes of the flat offsets its text queries take and its pointer
//! hit answers (`DomDocument::tab_flat_bytes`). The editor view's caret map
//! counted it as the one byte it is in the model, so every caret, highlight
//! and press after a tab in the same textblock was three bytes off per tab.
//!
//! **The oracle is the same line with the tabs spelled as four spaces**, which
//! rinch-dom lays out identically: the caret after a tab is where the caret
//! after the fourth space is, i.e. at the left edge of the glyph after the tab.
//! That relation is Chrome's too — measured in Chrome 153 on
//! `<pre contenteditable>a\tb\tc</pre>`, the caret after the tab is at `b`'s
//! left edge, a press on the tab's left quarter lands before it (offset 1), on
//! its right quarter after it (2), and on `b`'s right quarter after `b` (3).
//! Chrome's tab is *wider* (it advances to the next `tab-size: 8` stop, where
//! rinch-dom draws four spaces), so no absolute x here is Chrome's; every
//! assertion is a relation both engines satisfy.
//!
//! Two tabs per line, so the second tab's error (six bytes) is not the first's
//! (three) — a map that corrected only one tab, or by a fixed amount, fails.

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
    fn block(&self) -> usize {
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
        doc.query_caret_position(block as u64, byte)
            .expect("laid out")
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

/// `a\tb\tc` against `a    b    c`: the char offsets of the tab line and the
/// ones of the spaces line that draw at the same place.
const TAB_TO_SPACES: [(usize, usize); 6] = [(0, 0), (1, 1), (2, 5), (3, 6), (4, 10), (5, 11)];

/// Every caret on a line with two tabs is drawn where the same caret on the
/// four-spaces line is: after a tab is the next glyph's left edge. For both a
/// code block and a paragraph, since both are `pre-wrap` in the editor.
#[test]
fn the_caret_after_a_tab_is_drawn_at_the_next_glyph() {
    for (tabs, spaces) in [
        (
            "<pre><code>a\tb\tc</code></pre>",
            "<pre><code>a    b    c</code></pre>",
        ),
        ("<p>a\tb\tc</p>", "<p>a    b    c</p>"),
    ] {
        let t = page(tabs);
        let s = page(spaces);
        assert_eq!(
            t.text(),
            "a\tb\tc",
            "positive control: the tab survives the load"
        );
        let (x5, _) = s.local(5);
        assert!(
            x5 > s.local(1).0 + 12.0,
            "positive control: four spaces are wide: {x5}"
        );
        for (ti, si) in TAB_TO_SPACES {
            let (tx, ty) = t.local(ti);
            let (sx, sy) = s.local(si);
            assert_eq!(ty, sy, "{tabs}: char {ti} on one line");
            assert!(
                (tx - sx).abs() < 0.01,
                "{tabs}: caret {ti} at {tx}, spaces caret {si} at {sx}"
            );
        }
    }
}

/// A selection over `b\tc` (chars 2..5) highlights from `b`'s left edge to
/// the end of `c`: the same rect as `b    c` on the spaces line.
#[test]
fn a_selection_across_a_tab_highlights_the_tab_and_what_follows() {
    let mut t = page("<pre><code>a\tb\tc</code></pre>");
    let mut s = page("<pre><code>a    b    c</code></pre>");
    t.handle.set_selection(Selection::text(Pos(3), Pos(6)));
    s.handle.set_selection(Selection::text(Pos(6), Pos(12)));
    idle(&mut t.app);
    idle(&mut s.app);
    let (th, sh) = (t.highlights(), s.highlights());
    assert_eq!(th.len(), 1, "one line: {th:?}");
    assert_eq!(sh.len(), 1, "positive control, one line: {sh:?}");
    let (a, b) = (th[0], sh[0]);
    assert!(
        (a.0 - b.0).abs() < 0.01 && (a.2 - b.2).abs() < 0.01,
        "tab highlight {a:?} vs spaces highlight {b:?}"
    );
}

/// A press on a tab lands on its nearer side, and a press on the glyph after
/// it lands on that glyph's nearer side — Chrome 153's answers (1, 2, 3 for
/// the first tab; 3, 4, 5 for the second). Where to press is read off the
/// four-spaces line, not off the caret map under test. Each press is on a
/// fresh page, so no two read as a double click.
#[test]
fn a_press_on_or_after_a_tab_lands_on_the_side_hit() {
    let spaces = page("<pre><code>a    b    c</code></pre>");
    let x = |i: usize| spaces.local(TAB_TO_SPACES[i].1).0;
    let press = |from: usize, frac: f32| {
        let (x0, x1) = (x(from), x(from + 1));
        assert!(
            x1 > x0 + 4.0,
            "positive control: {from}..{} has width",
            from + 1
        );
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

/// A press right after the first tab, then typing, inserts there: `a\tXb\tc`,
/// and the caret after `X` is drawn at `b`'s left edge in the new layout.
#[test]
fn typing_after_a_press_behind_a_tab_inserts_after_the_tab() {
    let s = page("<pre><code>a    Xb    c</code></pre>");
    let before = page("<pre><code>a    b    c</code></pre>");
    let mut p = page("<pre><code>a\tb\tc</code></pre>");
    let (bx, by, _, _) = p.block_box();
    let (x1, x2) = (before.local(1).0, before.local(5).0);
    p.click(bx + x1 + (x2 - x1) * 0.8, by + LINE / 2.0);
    assert_eq!(p.head(), 2, "after the first tab");
    assert!(p.handle.insert_text("X"));
    idle(&mut p.app);
    assert_eq!(p.text(), "a\tXb\tc");
    assert_eq!(p.head(), 3);
    assert!((p.local(3).0 - s.local(6).0).abs() < 0.01, "after `X`");
    assert!(
        (p.local(5).0 - s.local(11).0).abs() < 0.01,
        "after the second tab"
    );
}
