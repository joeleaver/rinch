//! CSS generated content (`::before` / `::after`) inside an editor textblock
//! does not move the caret, the click mapping or the selection (#891).
//!
//! rinch-dom lays a pseudo-element's text out in its textblock's inline
//! formatting context, so it holds bytes of the flat offsets every text query
//! takes; the model does not hold it. The view used to map a model position
//! to a flat byte counting only the document's text, so every caret after an
//! `a::after { content: "\2197" }` was drawn three bytes (the arrow's UTF-8)
//! short, and a press was mapped three characters too far.
//!
//! Every oracle here is read off the laid-out text itself — Parley's caret at
//! the flat byte where a substring of the document text really is — so none
//! sits on the fixed point the old mapping shares (offsets before any
//! generated content). Text in the bundled Inter (`perf_expect::new_app`),
//! sizes declared.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (1000, 600);

/// The issue's rule: a small arrow after every web link.
const ARROW_AFTER: &str =
    "[data-pm-editor] a[href^=\"http\"]::after { content: \"\\2197\"; font-size: 0.7em; }";

/// The issue's paragraph, the URL linked.
const ISSUE_P: &str = "<p>hello world see <a href=\"https://example.com/docs\">https://example.com/docs</a>. and more text after it</p>";

/// Its text, as the model holds it (position `1 + i` is before char `i`).
const ISSUE_TEXT: &str = "hello world see https://example.com/docs. and more text after it";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    /// The (first) textblock.
    block: u64,
    /// The model text of that block.
    text: &'static str,
}

/// One 900px editor holding `html` (one `<p>`, model text `text`), with the
/// author `css`; focused by a real press.
fn page(html: &'static str, text: &'static str, css: &'static str) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(html);
        container.set_attribute(
            "style",
            "width: 900px; height: 300px; font-size: 16px; line-height: 20px; \
             font-family: sans-serif",
        );
        root.append_child(&container);
        let style = scope.create_element("style");
        let text = scope.create_text(css);
        style.append_child(&text);
        root.append_child(&style);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let block = {
        let doc = app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-editor] p")[0].0
    };
    let mut p = Page {
        app,
        handle,
        block,
        text,
    };
    // Focus with a press on the block's first glyph.
    let (x, y, _, h) = p.glyph(0);
    p.press(x + 1.0, y + h / 2.0);
    assert!(matches!(p.app.focus_target, FocusTarget::Editor(_)));
    p
}

impl Page {
    fn repaint(&mut self) {
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }

    fn press(&mut self, x: f32, y: f32) {
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        self.repaint();
    }

    /// The textblock's laid-out flat text, generated content included.
    fn laid_out(&self) -> String {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.tree.nodes[self.block]
            .text_layout
            .as_ref()
            .expect("the block has an inline layout")
            .text_content
            .clone()
    }

    /// The flat byte where the `n`th occurrence (from 0) of `needle` starts
    /// in the laid-out text.
    fn flat_nth(&self, needle: &str, n: usize) -> usize {
        let laid = self.laid_out();
        laid.match_indices(needle)
            .nth(n)
            .unwrap_or_else(|| panic!("{needle:?} #{n} is laid out in {laid:?}"))
            .0
    }

    fn flat_of(&self, needle: &str) -> usize {
        self.flat_nth(needle, 0)
    }

    /// The model position before the `n`th occurrence of `needle` in the
    /// block's model text (chars, plus the block's opening position).
    fn pos_nth(&self, needle: &str, n: usize) -> Pos {
        let byte = self
            .text
            .match_indices(needle)
            .nth(n)
            .unwrap_or_else(|| panic!("{needle:?} #{n} in the model text"))
            .0;
        Pos(1 + self.text[..byte].chars().count())
    }

    fn pos_of(&self, needle: &str) -> Pos {
        self.pos_nth(needle, 0)
    }

    fn end(&self) -> Pos {
        Pos(1 + self.text.chars().count())
    }

    /// Parley's caret at `flat` in the block, in window pixels: `(x, y)`.
    fn parley_caret(&self, flat: usize) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (x, y, _) = doc
            .text_caret_window_rect(self.block, flat)
            .expect("a laid-out caret");
        (x, y)
    }

    /// The glyph at `flat`, in window pixels `(x, y, w, h)`.
    fn glyph(&self, flat: usize) -> (f32, f32, f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.text_glyph_window_rect(self.block, flat)
            .expect("a laid-out glyph")
    }

    /// `EditorHandle::caret_rect` at `pos`: `(x, y)`.
    fn caret_rect(&self, pos: Pos) -> (f32, f32) {
        let r = self.handle.caret_rect(pos).expect("caret rect");
        (r.x, r.y)
    }

    /// Put the cursor at `pos` and answer the painted caret overlay's x.
    fn painted_caret_at(&mut self, pos: Pos) -> f32 {
        self.handle.set_selection(Selection::cursor(pos));
        self.repaint();
        self.painted_caret()
    }

    fn painted_caret(&self) -> f32 {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc.query_selector_all("[data-pm-caret]")[0];
        painted_element_box(&doc.tree, id.0).0
    }

    /// The painted selection highlight boxes `(x, w)`, left to right.
    fn highlight(&mut self, from: Pos, to: Pos) -> Vec<(f32, f32)> {
        self.handle.set_selection(Selection::text(from, to));
        self.repaint();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let mut boxes: Vec<(f32, f32)> = doc
            .query_selector_all("[data-pm-selection]")
            .into_iter()
            .map(|id| {
                let (x, _, w, _) = painted_element_box(&doc.tree, id.0);
                (x, w)
            })
            .filter(|&(_, w)| w > 0.0)
            .collect();
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
        boxes
    }

    /// Assert the caret at `pos` — `EditorHandle::caret_rect` and the painted
    /// overlay both — is Parley's caret at `flat`.
    fn assert_caret(&mut self, pos: Pos, flat: usize, what: &str) {
        let (want_x, want_y) = self.parley_caret(flat);
        let (x, y) = self.caret_rect(pos);
        assert!(
            (x - want_x).abs() < 0.5 && (y - want_y).abs() < 0.5,
            "{what}: caret_rect({pos:?}) = ({x}, {y}), Parley's caret at flat {flat} is \
             ({want_x}, {want_y}); laid out {:?}",
            self.laid_out()
        );
        let painted = self.painted_caret_at(pos);
        assert!(
            (painted - want_x).abs() <= 1.0,
            "{what}: the caret is painted at {painted}, Parley's caret at flat {flat} is {want_x}"
        );
    }
}

/// The positive control every fixture leans on: the rule's arrow really is
/// in the block's inline layout, three bytes the model does not hold.
#[test]
fn the_arrow_is_laid_out_in_the_textblock() {
    let p = page(ISSUE_P, ISSUE_TEXT, ARROW_AFTER);
    let laid = p.laid_out();
    assert_eq!(
        laid,
        "hello world see https://example.com/docs\u{2197}. and more text after it"
    );
    assert_eq!(laid.len(), ISSUE_TEXT.len() + 3);
}

/// The issue's case: the caret at the end of the text is drawn after `it`,
/// not between `after` and ` it`.
#[test]
fn the_caret_at_the_end_is_after_the_last_character() {
    let mut p = page(ISSUE_P, ISSUE_TEXT, ARROW_AFTER);
    let end = p.end();
    let flat = p.laid_out().len();
    p.assert_caret(end, flat, "end of the paragraph");
}

#[test]
fn a_caret_after_the_link_is_beside_its_own_character() {
    let mut p = page(ISSUE_P, ISSUE_TEXT, ARROW_AFTER);
    for needle in ["more", "it", " and"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
    // Before the link and inside it nothing was ever shifted (the fixed
    // point the old mapping shared): still right.
    for needle in ["see", "example"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
}

/// A press just right of the arrow, on the left half of the `.`, puts the
/// cursor right after the link — not three characters on — and the caret is
/// drawn there before the arrow, as Chrome draws the caret at the end of a
/// link with an `::after` (#2035's review, F2: Chrome 156 draws `<a>link</a>`'s
/// end at 71.4 with its arrow at 71.4–84.8, and a press anywhere on the
/// arrow gives that position). A typed character still lands after the
/// (non-inclusive) link, past the arrow.
#[test]
fn a_press_right_of_the_arrow_puts_the_caret_after_the_link() {
    let mut p = page(ISSUE_P, ISSUE_TEXT, ARROW_AFTER);
    let arrow = p.glyph(p.flat_of("\u{2197}"));
    let dot = p.glyph(p.flat_of(". and"));
    assert!(dot.2 > 2.0, "the dot has a left half to press: {dot:?}");
    p.press(dot.0 + 1.0, dot.1 + dot.3 / 2.0);
    let after_link = p.pos_of(". and");
    assert_eq!(p.handle.selection(), Selection::cursor(after_link));
    let painted = p.painted_caret();
    assert!(
        (painted - arrow.0).abs() <= 1.0,
        "the caret is drawn before the arrow, at {}: {painted}",
        arrow.0
    );
    p.assert_caret(after_link, p.flat_of("\u{2197}"), "the link's end");
    // A press further on lands on its own character.
    let m = p.glyph(p.flat_of("more"));
    p.press(m.0 + m.2 * 0.75, m.1 + m.3 / 2.0);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(p.pos_of("more").0 + 1))
    );
    // A press on the arrow's right half is the position after the link.
    // (Not right after the press on the dot: two presses that close are a
    // double click.)
    p.press(arrow.0 + arrow.2 * 0.75, arrow.1 + arrow.3 / 2.0);
    assert_eq!(p.handle.selection(), Selection::cursor(after_link));
}

/// A selection over the link runs from the first selected character to the
/// last one, and leaves the arrow unpainted, as a browser leaves generated
/// content out of a selection.
#[test]
fn a_selection_over_the_link_highlights_its_own_characters() {
    let mut p = page(ISSUE_P, ISSUE_TEXT, ARROW_AFTER);
    let (from, to) = (p.pos_of("see"), p.pos_of(" and"));
    let left = p.parley_caret(p.flat_of("see")).0;
    let right = p.parley_caret(p.flat_of(" and")).0;
    let arrow = p.glyph(p.flat_of("\u{2197}"));
    let boxes = p.highlight(from, to);
    assert!(!boxes.is_empty());
    let (l, r) = (
        boxes[0].0,
        boxes.iter().map(|b| b.0 + b.1).fold(f32::MIN, f32::max),
    );
    assert!(
        (l - left).abs() <= 1.0 && (r - right).abs() <= 1.0,
        "the highlight {boxes:?} spans {left}..{right}"
    );
    let arrow_mid = arrow.0 + arrow.2 / 2.0;
    assert!(
        !boxes
            .iter()
            .any(|b| b.0 < arrow_mid && arrow_mid < b.0 + b.1),
        "the arrow at {arrow_mid} is not highlighted: {boxes:?}"
    );
    // A selection that ends after the link ends after its own last
    // character too, past the arrow's place.
    let (from, to) = (p.pos_of("more"), p.end());
    let boxes = p.highlight(from, to);
    let r = boxes.iter().map(|b| b.0 + b.1).fold(f32::MIN, f32::max);
    let want = p.parley_caret(p.laid_out().len()).0;
    assert!((r - want).abs() <= 1.0, "{boxes:?} ends at {want}");
    // And one that starts after the link starts at its own first character
    // (#2035's review, F3: its left edge was never checked).
    let want = p.parley_caret(p.flat_of("more")).0;
    assert!(
        (boxes[0].0 - want).abs() <= 1.0,
        "{boxes:?} starts at {want}"
    );
}

/// Two decorated links: the shifts add up.
#[test]
fn two_decorated_links_shift_the_text_after_both() {
    let html = "<p>see <a href=\"https://a.example\">alpha</a> then <a href=\"https://b.example\">beta</a> and the end</p>";
    let text = "see alpha then beta and the end";
    let mut p = page(html, text, ARROW_AFTER);
    assert_eq!(p.laid_out().len(), text.len() + 6);
    for needle in ["then", "beta", "and", "end"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
    // Each link's end is drawn before its own arrow.
    for (needle, n) in [(" then", 0), (" and", 1)] {
        let (pos, flat) = (p.pos_of(needle), p.flat_nth("\u{2197}", n));
        p.assert_caret(pos, flat, needle);
    }
    let (end, flat) = (p.end(), p.laid_out().len());
    p.assert_caret(end, flat, "end");
    // A press on `end`'s right half.
    let e = p.glyph(p.flat_of("end"));
    p.press(e.0 + e.2 * 0.75, e.1 + e.3 / 2.0);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(p.pos_of("end").0 + 1))
    );
}

/// `::before`, several characters: the text inside the link and after it.
#[test]
fn a_before_on_a_mark_shifts_nothing_either() {
    // `\2192 ` takes the one space after it as the escape's end.
    let css = "[data-pm-editor] a[href^=\"http\"]::before { content: \"\\2192  \"; }";
    let mut p = page(ISSUE_P, ISSUE_TEXT, css);
    assert_eq!(
        p.laid_out(),
        "hello world see \u{2192} https://example.com/docs. and more text after it"
    );
    for needle in ["example", "more", "it"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
    let (end, flat) = (p.end(), p.laid_out().len());
    p.assert_caret(end, flat, "end");
    // Before the link's first character a typed character goes before the
    // (non-inclusive) link, so the caret is drawn before its `::before`
    // text, where that character lands; one character on it is inside.
    let (pos, flat) = (p.pos_of("https"), p.flat_of("\u{2192}"));
    p.assert_caret(pos, flat, "the link's start");
    let (pos, flat) = (Pos(pos.0 + 1), p.flat_of("ttps"));
    p.assert_caret(pos, flat, "after the link's first character");
    // A press on the left half of `h` is the link's start too.
    let h = p.glyph(p.flat_of("https"));
    p.press(h.0 + 1.0, h.1 + h.3 / 2.0);
    assert_eq!(p.handle.selection(), Selection::cursor(p.pos_of("https")));
}

/// Generated content on the textblock itself: a `::before` at its start, an
/// `::after` at its end.
#[test]
fn generated_content_on_the_textblock_itself() {
    let css = "[data-pm-editor] p::before { content: \"\\00a7  \"; } \
               [data-pm-editor] p::after { content: \" \\00b6\"; }";
    let mut p = page(ISSUE_P, ISSUE_TEXT, css);
    let laid = p.laid_out();
    assert!(
        laid.starts_with("\u{a7} ") && laid.ends_with(" \u{b6}"),
        "{laid:?}"
    );
    // The start is after the `§ `, before `hello`.
    p.assert_caret(Pos(1), p.flat_of("hello"), "start");
    for needle in ["world", "more"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
    // The end is after `it`, before the ` ¶`.
    let (end, flat) = (p.end(), p.flat_of(" \u{b6}"));
    p.assert_caret(end, flat, "end");
}

/// An inclusive mark (bold) with generated content after it: a caret at the
/// end of the bold run types into it, so it is drawn inside the run, before
/// the generated text; one character later it is past it.
#[test]
fn the_end_of_an_inclusive_run_is_drawn_before_its_after_content() {
    let css = "[data-pm-editor] strong::after { content: \"\\00bb\\00bb\"; }";
    let html = "<p>one <strong>two</strong> three</p>";
    let text = "one two three";
    let mut p = page(html, text, css);
    assert_eq!(p.laid_out(), "one two\u{bb}\u{bb} three");
    let end_of_bold = p.pos_of(" three");
    let flat = p.flat_of("\u{bb}");
    p.assert_caret(end_of_bold, flat, "end of the bold run");
    let (pos, flat) = (Pos(end_of_bold.0 + 1), p.flat_of("three"));
    p.assert_caret(pos, flat, "after the space");
}

/// Generated content on a mark nested in another: bold inside the link.
#[test]
fn generated_content_on_a_nested_mark() {
    let css = "[data-pm-editor] a strong::after { content: \"\\00bb\\00bb\"; }";
    let html = "<p>one <a href=\"https://x.example\"><strong>two</strong> four</a> three</p>";
    let text = "one two four three";
    let mut p = page(html, text, css);
    assert_eq!(p.laid_out(), "one two\u{bb}\u{bb} four three");
    for needle in [" four", "four", "three"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        if needle == " four" {
            // The bold run's end: inside it, before the generated text.
            p.assert_caret(pos, p.flat_of("\u{bb}"), needle);
        } else {
            p.assert_caret(pos, flat, needle);
        }
    }
    let (end, flat) = (p.end(), p.laid_out().len());
    p.assert_caret(end, flat, "end");
}

/// Right-to-left text after a decorated link (Parley's bidi; rinch-dom reads
/// no CSS `direction`): a caret inside the Arabic is at its own character.
#[test]
fn right_to_left_text_after_a_decorated_link() {
    let html = "<p>see <a href=\"https://a.example\">alpha</a> \u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}</p>";
    let text = "see alpha \u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}";
    let mut p = page(html, text, ARROW_AFTER);
    for needle in ["\u{631}", "\u{628}\u{627}\u{644}", "\u{639}"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
        p.assert_caret(pos, flat, needle);
    }
}

// ---- #2035's review: F1 (wrapped generated text), F3 (the missing pins) ----

impl Page {
    fn key_mod(&mut self, key: KeyCode, modifiers: Modifiers) {
        self.app.handle_event(
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
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
            VP,
            1.0,
        );
        self.repaint();
    }

    fn key(&mut self, key: KeyCode) {
        self.key_mod(key, Modifiers::default());
    }

    /// The painted caret overlay's `(x, y)`.
    fn caret_xy(&self) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc.query_selector_all("[data-pm-caret]")[0];
        let b = painted_element_box(&doc.tree, id.0);
        (b.0, b.1)
    }

    /// The painted selection highlight boxes `(x, y, w)`, top to bottom,
    /// left to right.
    fn highlight_xyw(&mut self, from: Pos, to: Pos) -> Vec<(f32, f32, f32)> {
        self.handle.set_selection(Selection::text(from, to));
        self.repaint();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let mut boxes: Vec<(f32, f32, f32)> = doc
            .query_selector_all("[data-pm-selection]")
            .into_iter()
            .map(|id| {
                let (x, y, w, _) = painted_element_box(&doc.tree, id.0);
                (x, y, w)
            })
            .filter(|&(_, _, w)| w > 0.0)
            .collect();
        boxes.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.total_cmp(&b.0)));
        boxes
    }
}

/// A press inside multi-character generated text — the right half of the `→`
/// of a link's `::before` `"→ "` — is the position that text stands at, the
/// link's start (Chrome 156 gives the link's start for a press there too),
/// not a position before it.
#[test]
fn a_press_inside_multi_character_before_text_is_the_link_start() {
    let css = "[data-pm-editor] a[href^=\"http\"]::before { content: \"\\2192  \"; }";
    let mut p = page(ISSUE_P, ISSUE_TEXT, css);
    let arrow = p.glyph(p.flat_of("\u{2192}"));
    assert!(arrow.2 > 4.0, "the arrow has a right half: {arrow:?}");
    p.press(arrow.0 + arrow.2 * 0.75, arrow.1 + arrow.3 / 2.0);
    assert_eq!(p.handle.selection(), Selection::cursor(p.pos_of("https")));
}

/// The inner side of a start edge: a caret at the start of a paragraph that
/// opens with a bold run carries bold (ProseMirror's marks at a block's
/// start are the run's after it), so typed text goes into the run, and the
/// caret is drawn inside it, after its `::before` text. Chrome 156 draws
/// `<p><b>bold</b> rest</p>`'s `b@0` after `b::before`'s `§ ` (23.6, the
/// `§ ` 10–23.6), and typing there goes into the `<b>`.
#[test]
fn a_caret_inside_a_start_edge_is_drawn_after_its_before_text() {
    let css = "[data-pm-editor] strong::before { content: \"\\00a7  \"; }";
    let html = "<p><strong>bold</strong> rest</p>";
    let text = "bold rest";
    let mut p = page(html, text, css);
    assert_eq!(p.laid_out(), "\u{a7} bold rest");
    p.assert_caret(Pos(1), p.flat_of("bold"), "the start, inside the bold run");
    let (pos, flat) = (Pos(2), p.flat_of("old"));
    p.assert_caret(pos, flat, "one character in");
}

/// A selection over two lines whose second line runs under the arrow: only
/// the arrow's own line has the hole (Chrome leaves generated content out of
/// the highlight; the cut is per line).
#[test]
fn a_two_line_selection_cuts_the_arrow_on_its_own_line_only() {
    let css = "[data-pm-editor] a[href^=\"http\"]::after { content: \"\\2197\"; font-size: 0.7em; } \
               [data-pm-editor] p { width: 380px; }";
    let html = "<p>hello world see <a href=\"https://example.com/docs\">https://example.com/docs</a>. and more text after it, enough to run the second line well past the arrow</p>";
    let text = "hello world see https://example.com/docs. and more text after it, enough to run the second line well past the arrow";
    let mut p = page(html, text, css);
    let arrow = p.glyph(p.flat_of("\u{2197}"));
    let boxes = p.highlight_xyw(p.pos_of("see"), p.end());
    let arrow_mid = arrow.0 + arrow.2 / 2.0;
    // The line under the arrow's line runs past the arrow's x.
    let line2 = boxes
        .iter()
        .map(|b| b.1)
        .find(|&y| y > arrow.1 + 10.0)
        .unwrap_or_else(|| panic!("the selection wraps: {boxes:?}"));
    let right2 = boxes
        .iter()
        .filter(|b| (b.1 - line2).abs() < 1.0)
        .map(|b| b.0 + b.2)
        .fold(f32::MIN, f32::max);
    assert!(
        right2 > arrow.0 + arrow.2 + 2.0,
        "line 2 runs past the arrow ({arrow:?}): {boxes:?}"
    );
    assert!(
        !boxes
            .iter()
            .any(|b| b.1 < line2 - 1.0 && b.0 < arrow_mid && arrow_mid < b.0 + b.2),
        "the arrow at {arrow_mid} is not highlighted on its line: {boxes:?}"
    );
    assert!(
        boxes
            .iter()
            .any(|b| (b.1 - line2).abs() < 1.0 && b.0 < arrow_mid && arrow_mid < b.0 + b.2),
        "line 2 is highlighted under the arrow's x {arrow_mid}: {boxes:?}"
    );
}

/// A page whose link ends line 1 and whose `::after` (` ↗↗↗`) wraps: the
/// space stays on line 1, the arrows go to line 2. `pad` is extra CSS for
/// the link.
fn wrapped_arrows(pad: &str) -> Page {
    let html = "<p>aaaa bbbb <a href=\"https://z.example\">link</a> tail words more words</p>";
    let text = "aaaa bbbb link tail words more words";
    for w in (60..400).step_by(2) {
        let css: &'static str = Box::leak(
            format!(
                "[data-pm-editor] a[href^=\"http\"]::after {{ content: \" \\2197\\2197\\2197\"; }} \
                 [data-pm-editor] a[href^=\"http\"] {{ {pad} }} \
                 [data-pm-editor] p {{ width: {w}px; }}"
            )
            .into_boxed_str(),
        );
        let p = page(html, text, css);
        let first = p.glyph(0);
        let link_end = p.glyph(p.flat_of("link") + 3);
        let arrow = p.glyph(p.flat_of("\u{2197}"));
        if (first.1 - link_end.1).abs() < 0.5 && arrow.1 > link_end.1 + 10.0 {
            return p;
        }
    }
    panic!("no width wraps the arrows alone");
}

/// #2035's review, F1: a link that ends line 1, its `::after` arrows wrapped
/// to line 2. Chrome 156 (`p { width: 120px }`, the same shape): End on line 1
/// is the link's end, drawn on line 1 at the link's right edge (128.3, 70);
/// ArrowDown goes to line 2 (" tail"@5, 80.3, 90); ArrowUp comes back to the
/// link's end on line 1; a press right of the link on line 1 is the link's
/// end on line 1; text typed there goes after `</a>`.
#[test]
fn a_link_end_whose_after_text_wraps_stays_on_its_line() {
    for pad in ["", "padding-right: 6px;"] {
        let mut p = wrapped_arrows(pad);
        let line1 = p.glyph(0).1;
        let line2 = p.glyph(p.flat_of("\u{2197}")).1;
        let link_end = p.pos_of(" tail");
        // Parley's caret right after `link`, on its line.
        let want_x = {
            let l = p.glyph(p.flat_of("link") + 3);
            l.0 + l.2
        };
        let on = |xy: (f32, f32), y: f32| (xy.1 - y).abs() < 2.0;

        p.handle.set_selection(Selection::cursor(Pos(1)));
        p.repaint();
        p.key(KeyCode::End);
        let end = (p.handle.selection().head(), p.caret_xy());
        assert_eq!(end.0, link_end, "{pad}: End on line 1 is the link's end");
        assert!(
            on(end.1, line1) && (end.1.0 - want_x).abs() <= 1.0,
            "{pad}: End draws the caret on line 1 at {want_x}, {line1}: {end:?}"
        );

        p.key(KeyCode::ArrowDown);
        let down = (p.handle.selection().head(), p.caret_xy());
        assert!(
            down.0 > link_end && on(down.1, line2),
            "{pad}: ArrowDown goes to line 2 ({line2}): {down:?}"
        );
        p.key(KeyCode::ArrowUp);
        let up = (p.handle.selection().head(), p.caret_xy());
        assert!(
            up.0 <= link_end && on(up.1, line1),
            "{pad}: ArrowUp comes back to line 1 ({line1}): {up:?}"
        );

        // A press right of the link, at the end of line 1.
        let l = p.glyph(p.flat_of("link") + 3);
        p.press(l.0 + l.2 + 3.0, l.1 + l.3 / 2.0);
        assert_eq!(p.handle.selection(), Selection::cursor(link_end), "{pad}");
        let xy = p.caret_xy();
        assert!(
            on(xy, line1) && (xy.0 - want_x).abs() <= 1.0,
            "{pad}: the pressed caret is on line 1 at {want_x}: {xy:?}"
        );
    }
}
