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
    block: usize,
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
/// drawn there: past the arrow, where the next typed character (outside the
/// non-inclusive link) lands.
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
    let arrow_right = arrow.0 + arrow.2;
    assert!(
        (painted - arrow_right).abs() <= 1.0,
        "the caret is drawn past the arrow, at {arrow_right}: {painted}"
    );
    // A press on the arrow's right half is the same position.
    p.press(arrow.0 + arrow.2 * 0.75, arrow.1 + arrow.3 / 2.0);
    assert_eq!(p.handle.selection(), Selection::cursor(after_link));
    // And a press further on lands on its own character.
    let m = p.glyph(p.flat_of("more"));
    p.press(m.0 + m.2 * 0.75, m.1 + m.3 / 2.0);
    assert_eq!(
        p.handle.selection(),
        Selection::cursor(Pos(p.pos_of("more").0 + 1))
    );
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
}

/// Two decorated links: the shifts add up.
#[test]
fn two_decorated_links_shift_the_text_after_both() {
    let html = "<p>see <a href=\"https://a.example\">alpha</a> then <a href=\"https://b.example\">beta</a> and the end</p>";
    let text = "see alpha then beta and the end";
    let mut p = page(html, text, ARROW_AFTER);
    assert_eq!(p.laid_out().len(), text.len() + 6);
    for needle in ["then", "beta", " and", "end"] {
        let (pos, flat) = (p.pos_of(needle), p.flat_of(needle));
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
    // The first character of the link: inside it the caret is past the
    // generated text, before `h`.
    let (pos, flat) = (p.pos_of("https"), p.flat_of("https"));
    p.assert_caret(pos, flat, "the link's first character");
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
