//! The caret after preserved spaces that hang past a line's end is drawn at
//! the end of the textblock's box, not out past it (#1041).
//!
//! Since #1018 trailing `pre-wrap` spaces that overflow a line hang (CSS Text
//! 3 §4.1.3) instead of making a phantom line, so the caret after them was at
//! the spaces' true x, past the textblock: under the editor's `overflow: auto`
//! it was clipped away, and without it it was painted outside the editor.
//!
//! Chrome 155, measured with headless screenshots (caret-color red, 1px
//! caret): the caret is clamped to the **border box** of the block that holds
//! the line, less its own width — a 90px `pre-wrap` block's caret column is
//! at `left + 89` from the third hanging space on; with `padding: 0 10px;
//! border: 5px` it is at `border right - 1`, not at the content box; a `<p>`
//! with `padding-right: 30px; border-right: 5px` inside a wider editable
//! clamps at the `<p>`'s border edge; `text-align: center` clamps the same.
//! A block that clips its overflow (`hidden`, `auto`, `clip`) is clamped the
//! same: at `right - 1` (measured by the review of #1668 with focus
//! emulation; a first probe here read the caret as missing, because with a
//! right border the clamped caret sits in the border strip, which a clipping
//! block clips away — rinch's overlay is a child of the editor, not of the
//! block, and is drawn there). The Range rects keep the
//! unclamped x, and so do rinch's selection highlight and
//! `EditorHandle::caret_rect` (which the vertical motions read): only the
//! drawn caret is kept in.
//!
//! Driven through real key presses; text in the bundled Inter
//! (`perf_expect::new_app`), sizes declared.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::dom::CARET_WIDTH;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    /// The textblock the caret is in.
    block: u64,
}

/// One editor, 130px wide (the editor stylesheet's 20px side padding inside
/// it), holding `html`, with the extra author `css`; focused by a real press
/// and the caret put at `end`.
fn page(html: &'static str, css: &'static str, end: Pos) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(html);
        container.set_attribute(
            "style",
            "width: 130px; height: 200px; font-size: 16px; line-height: 20px; \
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
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, Pos(2))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    handle.set_selection(Selection::cursor(end));
    app.resolve_and_repaint(800.0, 600.0);
    let block = {
        let doc = app.doc.as_ref().unwrap().borrow();
        let sel = if html.starts_with("<pre") { "pre" } else { "p" };
        doc.query_selector_all(sel)[0].0
    };
    Page { app, handle, block }
}

fn space(app: &mut RinchApp) {
    let modifiers = Modifiers::default();
    app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Space,
            logical_key: None,
            text: Some(" ".to_string()),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key: KeyCode::Space,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
}

impl Page {
    /// The block's painted border box's right edge, and its content box's.
    fn block_right(&self) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let b = painted_element_box(&doc.tree, self.block);
        let cs = &doc.tree.nodes[self.block].computed_style;
        let r = cs.padding_right.to_px() + cs.border_right_width.to_px();
        (b.0 + b.2, b.0 + b.2 - r)
    }

    /// The painted caret overlay `(x, y, w, h)`.
    fn caret(&self) -> (f32, f32, f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc.query_selector_all("[data-pm-caret]")[0];
        let cs = &doc.tree.nodes[id.0].computed_style;
        assert_eq!(
            cs.visibility,
            rinch_dom::computed_style::VisibilityValue::Visible,
            "the caret is shown"
        );
        painted_element_box(&doc.tree, id.0)
    }

    /// `EditorHandle::caret_rect` at the head: `(x, y)`.
    fn caret_rect(&self) -> (f32, f32) {
        let r = self
            .handle
            .caret_rect(self.handle.selection().head())
            .expect("caret rect");
        (r.x, r.y)
    }

    /// The selection highlight's right edge over `from..head`, which is not
    /// clamped: where the hanging spaces really end.
    fn highlight_right(&mut self, from: Pos) -> f32 {
        let head = self.handle.selection().head();
        self.handle.set_selection(Selection::text(from, head));
        self.app.resolve_and_repaint(800.0, 600.0);
        let right = {
            let doc = self.app.doc.as_ref().unwrap().borrow();
            doc.query_selector_all("[data-pm-selection]")
                .into_iter()
                .map(|id| {
                    let (x, _, w, _) = painted_element_box(&doc.tree, id.0);
                    x + w
                })
                .fold(f32::MIN, f32::max)
        };
        self.handle.set_selection(Selection::cursor(head));
        self.app.resolve_and_repaint(800.0, 600.0);
        right
    }
}

/// Type twelve spaces and answer the caret overlay's painted box before and
/// after each; asserts each caret stays on the start's line.
fn type_spaces(p: &mut Page) -> Vec<(f32, f32, f32, f32)> {
    let start = p.caret();
    let mut boxes = vec![start];
    for n in 1..=12 {
        space(&mut p.app);
        let c = p.caret();
        assert_eq!(
            c.1, start.1,
            "space {n}: the caret stays on the line ({c:?})"
        );
        boxes.push(c);
    }
    boxes
}

#[test]
fn the_caret_after_spaces_hanging_past_a_paragraphs_end_stays_at_its_end() {
    // "bullet text" is 1..12.
    let end = Pos(12);
    let mut p = page("<p>bullet text</p>", "", end);
    let start = p.caret();
    let mut overlay = vec![start];
    let mut rects = vec![p.caret_rect().0];
    for n in 1..=12 {
        space(&mut p.app);
        let c = p.caret();
        assert_eq!(
            c.1, start.1,
            "space {n}: the caret stays on the line ({c:?})"
        );
        overlay.push(c);
        rects.push(p.caret_rect().0);
    }
    let (right, _) = p.block_right();
    let spaces_end = p.highlight_right(end);
    assert!(
        spaces_end > right + 20.0,
        "the twelve spaces hang well past the block ({spaces_end} vs {right})"
    );
    // Off the fixed point: the first spaces fit and move the caret right,
    // drawn where `caret_rect` says.
    assert!(
        overlay[1].0 > overlay[0].0 && overlay[2].0 > overlay[1].0,
        "the first spaces move it: {overlay:?}"
    );
    for n in 0..3 {
        assert!(
            (overlay[n].0 - rects[n]).abs() <= 1.0,
            "space {n}: inside the block the caret is drawn at caret_rect {rects:?}, {overlay:?}"
        );
    }
    for (n, c) in overlay.iter().enumerate() {
        assert!(
            c.0 + c.2 <= right + 0.5,
            "space {n}: the caret {c:?} is painted inside the block (right edge {right})"
        );
    }
    let last = overlay.last().unwrap();
    assert!(
        (last.0 + last.2 - right).abs() <= 0.5 && (last.2 - CARET_WIDTH).abs() < 0.01,
        "the last caret ends at the block's right edge {right}: {last:?}"
    );
    let at_edge = overlay
        .iter()
        .filter(|c| (c.0 + c.2 - right).abs() <= 0.5)
        .count();
    assert!(
        at_edge >= 4,
        "it reached the edge and stayed there: {overlay:?}"
    );
    // `caret_rect` keeps the text's own x, as a collapsed Range does.
    let last_rect = *rects.last().unwrap();
    assert!(
        (last_rect - spaces_end).abs() <= 1.0,
        "caret_rect {last_rect} is where the spaces end, {spaces_end}"
    );
}

#[test]
fn a_padded_code_block_keeps_the_caret_in_its_border_box_not_its_content_box() {
    // "bullet" is 1..7; the code block has a 1em side padding.
    let end = Pos(7);
    let mut p = page("<pre><code>bullet</code></pre>", "", end);
    let boxes = type_spaces(&mut p);
    let (right, content_right) = p.block_right();
    let spaces_end = p.highlight_right(end);
    assert!(content_right < right - 10.0, "{content_right} vs {right}");
    assert!(spaces_end > right + 20.0, "{spaces_end} vs {right}");
    assert!(
        boxes[1].0 > boxes[0].0 && boxes[2].0 > boxes[1].0,
        "the first spaces move it: {boxes:?}"
    );
    for (n, c) in boxes.iter().enumerate() {
        assert!(c.0 + c.2 <= right + 0.5, "space {n}: {c:?} past {right}");
    }
    let last = boxes.last().unwrap();
    assert!(
        (last.0 + last.2 - right).abs() <= 0.5,
        "the caret stops at the border box's edge {right}, past the content box \
         {content_right}: {boxes:?}"
    );
}

#[test]
fn a_textblock_that_clips_its_overflow_keeps_its_caret_in_too() {
    // Chrome 155: `overflow: hidden` on the block changes nothing; the caret
    // stops at its right edge. The overlay is not inside the `<p>`, so the
    // `<p>`'s clip would not hide a caret drawn past it.
    let end = Pos(12);
    let mut p = page(
        "<p>bullet text</p>",
        "[data-pm-editor] p { overflow: hidden; }",
        end,
    );
    let boxes = type_spaces(&mut p);
    let (right, _) = p.block_right();
    let spaces_end = p.highlight_right(end);
    assert!(spaces_end > right + 20.0, "{spaces_end} vs {right}");
    let last = boxes.last().unwrap();
    assert!(
        (last.0 + last.2 - right).abs() <= 0.5,
        "the caret stops at the clipping block's edge {right}: {boxes:?}"
    );
}

#[test]
fn a_narrow_clipping_block_in_a_wide_editor_keeps_its_caret_in() {
    // A 60px `<p>` in the 130px editor: a caret past it would be drawn in the
    // editor's empty space beside the paragraph.
    let end = Pos(12);
    let mut p = page(
        "<p>bullet text</p>",
        "[data-pm-editor] p { overflow: hidden; width: 60px; }",
        end,
    );
    let boxes = type_spaces(&mut p);
    let (right, _) = p.block_right();
    for (n, c) in boxes.iter().enumerate() {
        assert!(
            c.0 + c.2 <= right + 0.5,
            "space {n}: the caret {c:?} is painted inside the 60px block (right {right})"
        );
    }
    let last = boxes.last().unwrap();
    assert!(
        (last.0 + last.2 - right).abs() <= 0.5,
        "{boxes:?} vs {right}"
    );
}
