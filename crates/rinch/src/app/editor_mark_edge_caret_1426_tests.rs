//! The editor's caret beside a padded mark wrapper (#1426, part 2).
//!
//! A mark wrapper's horizontal padding (`code`, `mark`, a padded link) takes
//! room on its line but holds no flat bytes, so at a run boundary one byte
//! names both sides of it, and the byte's caret is drawn before the padding.
//! The view draws the caret on the side the caret's marks say
//! (`RinchDomEditorView::mark_edges_beside`): inside a mark the caret
//! carries, outside one it does not — `code` is inclusive (a caret after a
//! code run carries it, so it is drawn inside the pill), a link is not (a
//! caret after a link is drawn past its padding, where typed text goes).
//!
//! The oracle is the paragraph's Parley layout read directly: where the
//! glyph of the character after the caret starts. That is past every edge
//! box at its byte — the caret's place when it is drawn past them.

use super::hit_testing::painted_element_box;
use super::*;
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
            "[data-pm-editor] p { margin: 0; padding: 0; } \
             [data-pm-editor] code { font: inherit; padding: 0 7px; } \
             [data-pm-editor] a { padding: 0 5px; }",
        );
        style.append_child(&css);
        root.append_child(&style);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    page
}

impl Page {
    fn caret_at(&mut self, pos: usize) {
        self.handle.set_selection(Selection::cursor(Pos(pos)));
        idle(&mut self.app);
    }

    /// The painted caret overlay's left edge.
    fn caret_x(&self) -> f32 {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .next()
            .expect("a caret");
        painted_element_box(&doc.tree, id.0).0
    }

    /// Where, on screen, the glyph at flat byte `byte` of the paragraph
    /// holding `pos` starts, and where the byte's own caret is drawn.
    fn glyph_and_byte_caret_x(&self, pos: usize, byte: usize) -> (f32, f32) {
        let block = self.handle.caret_address(Pos(pos)).unwrap().0;
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, ..) = painted_element_box(&doc.tree, block);
        let layout = &doc.tree.nodes[block].text_layout.as_ref().unwrap().layout;
        let glyph = parley::layout::Cluster::from_byte_index(layout, byte)
            .and_then(|c| c.visual_offset())
            .expect("a glyph at the byte");
        let (caret, _) = rinch_dom::text_query::caret_position_for_offset_layout_with_affinity(
            layout,
            byte,
            rinch_core::dom::CaretAffinity::Downstream,
        );
        (bx + glyph, bx + caret)
    }

    #[track_caller]
    fn assert_caret(&self, what: &str, pos: usize, want: f32) {
        let x = self.caret_x();
        assert!(
            (x - want).abs() <= 1.0,
            "{what}: the caret overlay is at {x}, want {want}"
        );
        let rect = self.handle.caret_rect(Pos(pos)).expect("a caret rect");
        assert!(
            (rect.x - want).abs() <= 0.01,
            "{what}: caret_rect answers {}, want {want}",
            rect.x
        );
    }
}

/// A paragraph that starts with a code run: the caret at its start carries
/// `code`, so it is drawn inside the pill, past its 7px padding.
#[test]
fn a_caret_at_the_start_of_a_code_run_is_inside_its_padding() {
    let mut p = page("<p><code>bc</code>d</p>");
    p.caret_at(1);
    let (glyph, byte_caret) = p.glyph_and_byte_caret_x(1, 0);
    // Positive control: the padding is there, and the byte's caret is
    // before it.
    assert!(
        (glyph - byte_caret - 7.0).abs() < 0.01,
        "{glyph} {byte_caret}"
    );
    p.assert_caret("start of a code run", 1, glyph);
}

/// After a link (not inclusive): the caret does not carry it, so it is
/// drawn past the link's 5px end padding, where `g` starts.
#[test]
fn a_caret_after_a_link_is_past_its_padding() {
    let mut p = page(r#"<p>d<a href="x">ef</a>g</p>"#);
    // `d` 1..2, `ef` 2..4, `g` 4..5: the caret after `ef` is Pos(4), byte 3.
    p.caret_at(4);
    let (glyph, byte_caret) = p.glyph_and_byte_caret_x(4, 3);
    assert!(
        (glyph - byte_caret - 5.0).abs() < 0.01,
        "{glyph} {byte_caret}"
    );
    p.assert_caret("after a link", 4, glyph);
}

/// After a code run (inclusive): the caret carries `code`, so it stays inside
/// the pill, before its end padding — the byte's own caret. And before a
/// code run that follows plain text, the caret carries no code and stays
/// outside, before the start padding — the byte's caret again.
#[test]
fn a_caret_after_a_code_run_stays_inside_and_before_one_outside() {
    let mut p = page("<p>a<code>bc</code>d</p>");
    // `a` 1..2, `bc` 2..4, `d` 4..5.
    p.caret_at(4);
    let (glyph, byte_caret) = p.glyph_and_byte_caret_x(4, 3);
    assert!((glyph - byte_caret - 7.0).abs() < 0.01);
    p.assert_caret("after a code run", 4, byte_caret);
    p.caret_at(2);
    let (glyph, byte_caret) = p.glyph_and_byte_caret_x(2, 1);
    assert!((glyph - byte_caret - 7.0).abs() < 0.01);
    p.assert_caret("before a code run", 2, byte_caret);
}
