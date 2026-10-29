//! The editor's caret and selection overlays in a textblock with its own
//! padding and border, on desktop (#1667).
//!
//! Every code block has padding (`padding: 0.85em 1em` in the editor's
//! stylesheet). The view places the caret at the block's border-box origin
//! (summed from `query_node_layout`) plus the block-local caret
//! `DomDocument::query_caret_position_with_affinity` answers. rinch-web
//! answers that relative to the block's `getBoundingClientRect` — its border
//! box — and rinch-dom answered it relative to the block's **content** box,
//! where Parley lays the text out, so on desktop the caret in a code block
//! was drawn one padding (and border) left of and above the text: 14px and
//! 12px at the stock code block's 16px font.
//!
//! The oracle here is independent of the view and of the trait: the block's
//! painted border box, plus its computed padding and border, plus Parley's
//! layout-local caret (`text_query::*_layout*`, which the fix does not
//! touch). The cases sit off the fixed point: asymmetric paddings and
//! borders, so a dropped border or a swapped axis shows.
//!
//! - the stock code block (`<pre>`), the user-visible case;
//! - a paragraph given padding and a border by the app's CSS, with a hard
//!   break (the caret after a `<br>` is asked of the inline box, not by
//!   byte);
//! - a paragraph in a padded blockquote (the container's padding is an
//!   ancestor's, already in the summed layout offsets: a control);
//! - an empty code block (no text layout: the view places the caret from the
//!   block's own box).
//!
//! The face is the bundled Inter for prose and code at a declared
//! 16px / 24px.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::dom::CaretAffinity;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

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

/// `html` in a 400px editor, focused and settled. Code blocks keep the
/// stylesheet's padding; paragraphs get an asymmetric padding and border
/// (not inside a blockquote, whose own padding is asymmetric too).
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
            "width: 400px; height: 500px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        let style = scope.create_element("style");
        let css = scope.create_text(
            "[data-pm-editor] pre { font-size: 16px; line-height: 24px; } \
             [data-pm-editor] p { margin: 0; padding: 11px 6px 4px 17px; \
               border-left: 4px solid red; border-top: 2px solid red; } \
             [data-pm-editor] blockquote { margin: 0; padding: 9px 0 3px 23px; } \
             [data-pm-editor] blockquote p { padding: 0; border: none; }",
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
    assert!(matches!(page.app.focus_target, FocusTarget::Editor(_)));
    page
}

impl Page {
    fn caret_at(&mut self, pos: usize) {
        self.handle.set_selection(Selection::cursor(Pos(pos)));
        idle(&mut self.app);
    }

    fn select(&mut self, from: usize, to: usize) {
        self.handle
            .set_selection(Selection::text(Pos(from), Pos(to)));
        idle(&mut self.app);
    }

    /// The host element of the textblock holding `pos`.
    fn block(&self, pos: usize) -> u64 {
        self.handle.caret_address(Pos(pos)).unwrap().0
    }

    /// Where the block's text starts on screen: its painted border box moved
    /// in by its padding and border.
    fn content_origin(&self, block: u64) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, by, _, _) = painted_element_box(&doc.tree, block);
        let cs = &doc.tree.nodes[block].computed_style;
        (
            bx + cs.padding_left.to_px() + cs.border_left_width.to_px(),
            by + cs.padding_top.to_px() + cs.border_top_width.to_px(),
        )
    }

    /// Where the caret at flat byte `byte` of `block` is drawn, by Parley's
    /// layout-local answer from the content origin.
    fn expected_caret(&self, block: u64, byte: usize) -> (f32, f32) {
        let (cx, cy) = self.content_origin(block);
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let layout = &doc.tree.nodes[block]
            .text_layout
            .as_ref()
            .expect("an IFC root")
            .layout;
        let (lx, ly) = rinch_dom::text_query::caret_position_for_offset_layout_with_affinity(
            layout,
            byte,
            CaretAffinity::Downstream,
        );
        (cx + lx, cy + ly)
    }

    /// The painted caret overlay, `None` when hidden.
    fn caret(&self) -> Option<(f32, f32, f32, f32)> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .next()?;
        let cs = &doc.tree.nodes[id.0].computed_style;
        if cs.display == rinch_dom::computed_style::DisplayValue::None
            || cs.visibility != rinch_dom::computed_style::VisibilityValue::Visible
        {
            return None;
        }
        Some(painted_element_box(&doc.tree, id.0))
    }

    /// The painted selection highlights, top to bottom.
    fn highlights(&self) -> Vec<(f32, f32, f32, f32)> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let mut v: Vec<_> = doc
            .query_selector_all("[data-pm-selection]")
            .into_iter()
            .filter(|id| {
                doc.tree.nodes[id.0].computed_style.display
                    != rinch_dom::computed_style::DisplayValue::None
            })
            .map(|id| painted_element_box(&doc.tree, id.0))
            .filter(|b| b.2 > 0.0 && b.3 > 0.0)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    /// The caret overlay and `caret_rect` both sit at `want`.
    #[track_caller]
    fn assert_caret(&self, what: &str, pos: usize, want: (f32, f32)) {
        let caret = self
            .caret()
            .unwrap_or_else(|| panic!("{what}: a caret is drawn"));
        assert!(
            (caret.0 - want.0).abs() <= 1.0 && (caret.1 - want.1).abs() <= 1.0,
            "{what}: the caret overlay is drawn at ({}, {}), the text's caret is at {want:?}",
            caret.0,
            caret.1
        );
        let rect = self.handle.caret_rect(Pos(pos)).expect("a caret rect");
        assert!(
            (rect.x - want.0).abs() <= 0.01 && (rect.y - want.1).abs() <= 0.01,
            "{what}: caret_rect answers ({}, {}), the text's caret is at {want:?}",
            rect.x,
            rect.y
        );
    }
}

/// The stock code block: the caret at the text's start, after a few
/// characters and on the second line sits on the glyphs, and so does the
/// highlight of the first characters.
#[test]
fn the_caret_in_a_code_block_sits_on_its_text() {
    let mut p = page("<pre>bullet\nsecond line</pre>");
    let block = p.block(1);
    // Positive control: the stock code block is padded (0.85em 1em).
    {
        let doc = p.app.doc.as_ref().unwrap().borrow();
        let cs = &doc.tree.nodes[block].computed_style;
        assert!(
            cs.padding_left.to_px() > 10.0 && cs.padding_top.to_px() > 10.0,
            "the code block has its stylesheet padding"
        );
    }
    for (what, char_offset) in [("start", 0usize), ("after `bul`", 3), ("line 2", 9)] {
        p.caret_at(1 + char_offset);
        let want = p.expected_caret(block, char_offset);
        p.assert_caret(&format!("code block, {what}"), 1 + char_offset, want);
    }
    p.select(1, 4);
    let hl = p.highlights();
    let (x0, y0) = p.expected_caret(block, 0);
    let (x1, _) = p.expected_caret(block, 3);
    assert_eq!(hl.len(), 1, "one highlight: {hl:?}");
    assert!(
        (hl[0].0 - x0).abs() <= 1.0 && (hl[0].0 + hl[0].2 - x1).abs() <= 1.0,
        "the highlight spans {}..{}, the selected text {x0}..{x1}",
        hl[0].0,
        hl[0].0 + hl[0].2
    );
    assert!(
        hl[0].1 <= y0 + 1.0 && hl[0].1 + hl[0].3 >= y0 + 16.0,
        "the highlight {hl:?} covers the first line, which starts at {y0}"
    );
}

/// A paragraph the app gave padding and a border (asymmetric, so a dropped
/// border or a swapped axis shows), including the caret on the line after a
/// hard break and the caret after an image, which is asked of the image's
/// inline box (`query_inline_box_caret`, #1104) rather than by byte.
#[test]
fn the_caret_in_a_padded_bordered_paragraph_sits_on_its_text() {
    let mut p = page("<p>ab<br>cd<img src=\"missing-1667.png\" width=\"20\" height=\"16\">ef</p>");
    let block = p.block(1);
    p.caret_at(2);
    let want = p.expected_caret(block, 1);
    p.assert_caret("padded paragraph, after `a`", 2, want);
    // After the break: `ab` is bytes 0..2, the `<br>` byte 2, `c` byte 3.
    p.caret_at(4);
    let want = p.expected_caret(block, 3);
    assert!(
        want.1 > p.content_origin(block).1 + 20.0,
        "positive control: `c` is on the second line"
    );
    p.assert_caret("padded paragraph, after the hard break", 4, want);
    // After the image: content offsets a0 b1 br2 c3 d4 img5 e6, so Pos 7.
    p.caret_at(7);
    let want = {
        let (cx, cy) = p.content_origin(block);
        let doc = p.app.doc.as_ref().unwrap().borrow();
        let img = doc.query_selector_all("img")[0].0;
        let layout = &doc.tree.nodes[block].text_layout.as_ref().unwrap().layout;
        let (x, y, _) = rinch_dom::text_query::inline_box_caret(layout, img, true)
            .expect("the image is an inline box of the paragraph");
        (cx + x, cy + y)
    };
    assert!(
        want.0 > p.expected_caret(block, 4).0 + 15.0,
        "positive control: the caret after the image is past it"
    );
    p.assert_caret("padded paragraph, after the image", 7, want);
    p.select(1, 3);
    let hl = p.highlights();
    let (x0, _) = p.expected_caret(block, 0);
    let (x1, _) = p.expected_caret(block, 2);
    assert!(
        !hl.is_empty() && (hl[0].0 - x0).abs() <= 1.0 && (hl[0].0 + hl[0].2 - x1).abs() <= 1.0,
        "the highlight {hl:?} spans the selected text {x0}..{x1}"
    );
}

/// A paragraph inside a padded blockquote: the padding is the container's,
/// which the summed layout offsets already carry. A control: right before
/// the fix as after it.
#[test]
fn the_caret_in_a_padded_blockquote_sits_on_its_text() {
    let mut p = page("<blockquote><p>quoted text</p></blockquote>");
    let pos = 2; // the blockquote opens at 0, the paragraph at 1
    let block = p.block(pos);
    p.caret_at(pos + 2);
    let want = p.expected_caret(block, 2);
    p.assert_caret("blockquote paragraph", pos + 2, want);
}

/// An empty code block has no text layout: the caret is drawn at its content
/// origin, one line tall, not at its border-box corner as tall as its padded
/// box.
#[test]
fn the_caret_in_an_empty_code_block_sits_at_its_content_origin() {
    let mut p = page("<p>x</p><pre></pre>");
    let pos = 4; // `<p>x</p>` is 0..3, the code block's inside starts at 4
    let block = p.block(pos);
    p.caret_at(pos);
    let want = p.content_origin(block);
    let caret = p.caret().expect("a caret is drawn");
    assert!(
        (caret.0 - want.0).abs() <= 1.0 && (caret.1 - want.1).abs() <= 1.0,
        "the caret in an empty code block is drawn at ({}, {}), its content origin is {want:?}",
        caret.0,
        caret.1
    );
    assert!(
        (caret.3 - 24.0).abs() <= 1.0,
        "the caret is one line (24px) tall, not the padded box's height: {}",
        caret.3
    );
    let rect = p.handle.caret_rect(Pos(pos)).expect("a caret rect");
    assert!(
        (rect.x - want.0).abs() <= 0.01
            && (rect.y - want.1).abs() <= 0.01
            && (rect.height - 24.0).abs() < 0.01,
        "caret_rect in an empty code block answers {rect:?}, its content origin is {want:?}"
    );
}
