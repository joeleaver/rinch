//! Reviewer fixtures for PR #1670 (#1667): padded textblocks off the
//! fixed point — RTL + centred text, a scaled editor, an empty code block
//! in a scaled editor, and the public `NodeHandle`/`DomDocument` text
//! queries on a padded non-editor element.

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
    scale: f32,
}

fn page(html: &str, wrapper_style: &'static str, css: &'static str, scale: f32) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let wrap = scope.create_element("div");
        wrap.set_attribute("style", wrapper_style);
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 300px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        wrap.append_child(&editor);
        root.append_child(&wrap);
        let style = scope.create_element("style");
        let css = scope.create_text(css);
        style.append_child(&css);
        root.append_child(&style);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.register_app_font(AppFont::monospace(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle, scale };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    fn caret_at(&mut self, pos: usize) {
        self.handle.set_selection(Selection::cursor(Pos(pos)));
        idle(&mut self.app);
    }
    fn block(&self, pos: usize) -> u64 {
        self.handle.caret_address(Pos(pos)).unwrap().0
    }
    fn content_origin(&self, block: u64) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, by, _, _) = painted_element_box(&doc.tree, block);
        let cs = &doc.tree.nodes[block].computed_style;
        (
            bx + self.scale * (cs.padding_left.to_px() + cs.border_left_width.to_px()),
            by + self.scale * (cs.padding_top.to_px() + cs.border_top_width.to_px()),
        )
    }
    fn expected_caret(&self, block: u64, byte: usize) -> (f32, f32) {
        let (cx, cy) = self.content_origin(block);
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let layout = &doc.tree.nodes[block].text_layout.as_ref().unwrap().layout;
        let (lx, ly) = rinch_dom::text_query::caret_position_for_offset_layout_with_affinity(
            layout,
            byte,
            CaretAffinity::Downstream,
        );
        (cx + self.scale * lx, cy + self.scale * ly)
    }
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
    #[track_caller]
    fn assert_caret(&self, what: &str, pos: usize, want: (f32, f32)) {
        let caret = self
            .caret()
            .unwrap_or_else(|| panic!("{what}: a caret is drawn"));
        let tol = 1.0 * self.scale;
        assert!(
            (caret.0 - want.0).abs() <= tol && (caret.1 - want.1).abs() <= tol,
            "{what}: overlay at ({}, {}), text caret at {want:?}",
            caret.0,
            caret.1
        );
        let rect = self.handle.caret_rect(Pos(pos)).expect("a caret rect");
        assert!(
            (rect.x - want.0).abs() <= 0.05 && (rect.y - want.1).abs() <= 0.05,
            "{what}: caret_rect ({}, {}), text caret at {want:?}",
            rect.x,
            rect.y
        );
    }
}

const PADDED_P: &str = "[data-pm-editor] p { margin: 0; padding: 11px 6px 4px 17px; \
   border-left: 4px solid red; border-top: 2px solid red; }";

#[test]
fn rtl_centred_padded_paragraph() {
    let mut p = page(
        "<p>abc def ghi</p>",
        "",
        "[data-pm-editor] p { margin: 0; padding: 11px 23px 4px 17px; \
           border-left: 4px solid red; border-right: 9px solid red; border-top: 2px solid red; \
           direction: rtl; text-align: center; }",
        1.0,
    );
    let block = p.block(1);
    for off in [0usize, 3, 7, 11] {
        p.caret_at(1 + off);
        let want = p.expected_caret(block, off);
        p.assert_caret(&format!("rtl centred, offset {off}"), 1 + off, want);
    }
}

#[test]
fn scaled_editor_padded_paragraph() {
    let mut p = page(
        "<p>ab<br>cd</p>",
        "transform: scale(1.5); transform-origin: 0 0; margin-left: 13px; margin-top: 7px",
        PADDED_P,
        1.5,
    );
    let block = p.block(1);
    for (pos, off) in [(2usize, 1usize), (4, 3)] {
        p.caret_at(pos);
        let want = p.expected_caret(block, off);
        p.assert_caret(&format!("scaled, pos {pos}"), pos, want);
    }
}

#[test]
fn scaled_editor_empty_code_block() {
    let mut p = page(
        "<p>x</p><pre></pre>",
        "transform: scale(1.5); transform-origin: 0 0; margin-left: 13px; margin-top: 7px",
        "[data-pm-editor] pre { font-size: 16px; line-height: 24px; }",
        1.5,
    );
    let pos = 4;
    let block = p.block(pos);
    p.caret_at(pos);
    let want = p.content_origin(block);
    let caret = p.caret().expect("a caret is drawn");
    assert!(
        (caret.0 - want.0).abs() <= 1.5 && (caret.1 - want.1).abs() <= 1.5,
        "scaled empty code block overlay at ({}, {}), content origin {want:?}",
        caret.0,
        caret.1
    );
    let rect = p.handle.caret_rect(Pos(pos)).expect("a caret rect");
    assert!(
        (rect.x - want.0).abs() <= 0.05 && (rect.y - want.1).abs() <= 0.05,
        "scaled empty code block caret_rect {rect:?}, content origin {want:?}"
    );
    assert!(
        (rect.height - 36.0).abs() < 0.5,
        "one 24px line * 1.5: {rect:?}"
    );
}

/// The vertical-motion anchor (`editor_caret_point`) and the IME candidate
/// box agree with the painted caret in a padded, bordered paragraph.
#[test]
fn ime_area_in_padded_paragraph() {
    let mut p = page("<p>abcd</p>", "", PADDED_P, 1.0);
    let block = p.block(1);
    p.caret_at(3);
    let want = p.expected_caret(block, 2);
    let ime = p.app.ime_state().cursor_tuple().expect("an IME area");
    assert!(
        (ime.0 - want.0).abs() <= 0.05 && (ime.1 - want.1).abs() <= 0.05,
        "IME area {ime:?}, caret {want:?}"
    );
}

/// An empty paragraph the app padded asymmetrically: the caret goes to the
/// content origin (left inset 21, not the right inset 10), and the overlay
/// and `caret_rect` agree on its height.
#[test]
fn empty_padded_paragraph() {
    let mut p = page("<p>x</p><p></p>", "", PADDED_P, 1.0);
    let pos = 4;
    let block = p.block(pos);
    p.caret_at(pos);
    let want = p.content_origin(block);
    let caret = p.caret().expect("a caret is drawn");
    let rect = p.handle.caret_rect(Pos(pos)).expect("a caret rect");
    assert!(
        (caret.0 - want.0).abs() <= 1.0 && (caret.1 - want.1).abs() <= 1.0,
        "overlay ({}, {}) vs content origin {want:?}",
        caret.0,
        caret.1
    );
    assert!(
        (rect.x - want.0).abs() <= 0.05 && (rect.y - want.1).abs() <= 0.05,
        "caret_rect {rect:?} vs {want:?}"
    );
    assert!(
        (caret.3 - rect.height).abs() < 1.0,
        "overlay and caret_rect agree on height"
    );
}

/// The same empty padded paragraph's caret is one line (24px) tall, as
/// Chrome draws it. On desktop `min-height: 1lh` sizes the border box
/// (#1278), so its content box is 7px: the caret is the line height, not the
/// content height (review of #1670, F1).
#[test]
fn empty_padded_paragraph_caret_is_one_line_tall() {
    let mut p = page("<p>x</p><p></p>", "", PADDED_P, 1.0);
    p.caret_at(4);
    let caret = p.caret().expect("a caret is drawn");
    assert!(
        (caret.3 - 24.0).abs() <= 1.0,
        "a caret one line (24px) tall, got {}",
        caret.3
    );
    let rect = p.handle.caret_rect(Pos(4)).expect("a caret rect");
    assert!(
        (rect.height - 24.0).abs() < 0.01,
        "caret_rect one line tall, got {rect:?}"
    );
}
