//! #1181: U+2028, U+2029, U+0085 and U+000C in an editor textblock. rinch-dom
//! lays them out as substitutes of another length (NBSP + ZWSP, ZWSP, and
//! nothing), so the editor view's caret map must count the host's bytes for
//! each (`DomDocument::substituted_char_flat_bytes`) or every caret, highlight
//! and press after one is off (found by #1269's review). Each is checked
//! against an oracle page holding the same glyphs: `a<NBSP>bc` for a
//! separator, `abc` for the zero-width two. Bundled Inter, declared 24px lines.
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

/// The editor's page with `sep` typed between `a` and `bc`, settled at a new
/// viewport so the edit is laid out; and the oracle page with the same glyphs.
fn pages(sep: char) -> (Page, Page) {
    let mut t = page("<p>abc</p>");
    t.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(t.handle.insert_text(&sep.to_string()));
    idle(&mut t.app);
    t.app.resolve_and_repaint(801.0, 600.0);
    idle(&mut t.app);
    assert_eq!(t.text(), format!("a{sep}bc"), "positive control");
    let oracle = match sep {
        '\u{2028}' | '\u{2029}' => "<p>a\u{a0}bc</p>",
        _ => "<p>abc</p>",
    };
    (t, page(oracle))
}

/// The oracle char for editor char `i`: a zero-width char shares its
/// position with the one before it.
fn oracle_char(sep: char, i: usize) -> usize {
    match sep {
        '\u{2028}' | '\u{2029}' => i,
        _ => i.saturating_sub(usize::from(i >= 2)),
    }
}

const SEPS: [char; 4] = ['\u{2028}', '\u{2029}', '\u{85}', '\u{c}'];

/// Every caret of `a<sep>bc` is drawn where the oracle's is.
#[test]
fn carets_after_a_substituted_char_are_where_its_glyphs_put_them() {
    let mut bad = vec![];
    for sep in SEPS {
        let (t, s) = pages(sep);
        for i in 0..=4 {
            let (tx, _) = t.local(i);
            let (sx, _) = s.local(oracle_char(sep, i));
            if (tx - sx).abs() > 0.01 {
                bad.push(format!(
                    "U+{:04X} caret {i} at {tx}, oracle at {sx}",
                    sep as u32
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A selection of everything highlights exactly the oracle's text, and one
/// of `c` alone starts where the oracle's `c` does.
#[test]
fn a_selection_across_a_substituted_char_highlights_its_glyphs() {
    let mut bad = vec![];
    for sep in SEPS {
        let (mut t, mut s) = pages(sep);
        for (from, to) in [(0, 4), (3, 4)] {
            t.handle
                .set_selection(Selection::text(Pos(from + 1), Pos(to + 1)));
            idle(&mut t.app);
            let (sf, st) = (oracle_char(sep, from), oracle_char(sep, to));
            s.handle
                .set_selection(Selection::text(Pos(sf + 1), Pos(st + 1)));
            idle(&mut s.app);
            let (th, sh) = (t.highlights(), s.highlights());
            let same = th.len() == sh.len()
                && th
                    .iter()
                    .zip(&sh)
                    .all(|(a, b)| (a.0 - b.0).abs() < 0.5 && (a.2 - b.2).abs() < 0.5);
            if !same || th.is_empty() {
                bad.push(format!(
                    "U+{:04X} {from}..{to}: {th:?} vs oracle {sh:?}",
                    sep as u32
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A press right of `c` lands at the end, one inside `b` before or after it
/// by its half, and for a separator a press on each half of its space lands
/// on that side of it.
#[test]
fn a_press_after_a_substituted_char_lands_on_the_char_it_hit() {
    let mut bad = vec![];
    for sep in SEPS {
        let (t0, s) = pages(sep);
        let (x, y, _, h) = t0.block_box();
        let mut t = t0;
        let y = y + h / 2.0;
        let ox = |i: usize| s.local(oracle_char(sep, i)).0;
        let mut presses = vec![
            (ox(4) + 2.0, 4),
            // A preserved U+000C occupies no flat bytes, so the positions
            // either side of it share one and a press reads as the side
            // before it, as beside an image (#1104). Same place on screen.
            (
                ox(2) + (ox(3) - ox(2)) * 0.25,
                if sep == '\u{c}' { 1 } else { 2 },
            ),
            (ox(2) + (ox(3) - ox(2)) * 0.75, 3),
        ];
        if matches!(sep, '\u{2028}' | '\u{2029}') {
            presses.push((ox(1) + (ox(2) - ox(1)) * 0.2, 1));
            presses.push((ox(1) + (ox(2) - ox(1)) * 0.8, 2));
        }
        for (px, want) in presses {
            // A fresh page per press: a second press nearby is a double click.
            t = pages(sep).0;
            t.click(x + px, y);
            if t.head() != want {
                bad.push(format!(
                    "U+{:04X} press at {px}: head {}, want {want}",
                    sep as u32,
                    t.head()
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
