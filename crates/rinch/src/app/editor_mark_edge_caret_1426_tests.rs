//! The editor's caret beside a padded mark wrapper (#1426, part 2).
//!
//! A mark wrapper's horizontal padding (`code`, `mark`, a padded link) takes
//! room on its line but holds no flat bytes, so at a run boundary one byte
//! names both sides of it, and the byte's caret is drawn before the padding.
//! The view draws the caret where the next typed character lands
//! (`RinchDomEditorView::mark_edges_beside`): it joins the run before or
//! after whose marks it takes, or makes its own between them.
//!
//! The oracle is the review of #1832's: the caret is drawn where the next
//! typed character lands. Each fixture types a `Z` at the caret and reads
//! where its glyph is drawn from the paragraph's Parley layout.

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
}

impl Page {
    /// The painted caret's `(x, y)`, then a `Z` typed there: where its glyph
    /// is drawn, `(x, top of its line)`.
    fn caret_and_typed(&mut self) -> ((f32, f32), (f32, f32)) {
        let caret = {
            let doc = self.app.doc.as_ref().unwrap().borrow();
            let id = doc
                .query_selector_all("[data-pm-caret]")
                .into_iter()
                .next()
                .expect("a caret");
            let b = painted_element_box(&doc.tree, id.0);
            (b.0, b.1)
        };
        let head = self.handle.selection().head();
        assert!(self.handle.insert_text("Z"));
        idle(&mut self.app);
        let block = self.handle.caret_address(head).unwrap().0;
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (bx, by, ..) = painted_element_box(&doc.tree, block);
        let il = doc.tree.nodes[block].text_layout.as_ref().unwrap();
        let byte = il.text_content.find('Z').expect("the typed Z");
        let cluster = parley::layout::Cluster::from_byte_index(&il.layout, byte).unwrap();
        let x = bx + cluster.visual_offset().unwrap();
        let y = by + cluster.line().metrics().block_min_coord;
        (caret, (x, y))
    }

    /// The caret is drawn where the next typed character lands (#1426,
    /// review of #1832): on its line, at its glyph's left edge.
    #[track_caller]
    fn assert_typed_lands_at_the_caret(&mut self, what: &str) {
        let (caret, typed) = self.caret_and_typed();
        assert!(
            (caret.0 - typed.0).abs() <= 1.0 && (caret.1 - typed.1).abs() <= 3.0,
            "{what}: the caret is drawn at {caret:?}, a typed Z lands at {typed:?}"
        );
    }
}

/// The caret at a paragraph's start, before a code run: it carries `code`,
/// so it is drawn inside the pill, where `Z` joins the run.
#[test]
fn at_a_paragraph_start_before_a_code_run() {
    let mut p = page("<p><code>bc</code>d</p>");
    p.caret_at(1);
    p.assert_typed_lands_at_the_caret("paragraph start, code run");
}

/// At a paragraph's start before a link (not inclusive, dropped there): `Z`
/// lands outside the link, before its padding.
#[test]
fn at_a_paragraph_start_before_a_link() {
    let mut p = page(r#"<p><a href="x">ef</a>g</p>"#);
    p.caret_at(1);
    p.assert_typed_lands_at_the_caret("paragraph start, link");
}

/// After a hard break, before a code run: the caret carries the break's
/// marks (none), so `Z` lands before the code's padding at the line start.
#[test]
fn after_a_hard_break_before_a_code_run() {
    let mut p = page("<p>a<br><code>b</code></p>");
    // `a` 1..2, the break 2..3, `b` 3..4.
    p.caret_at(3);
    p.assert_typed_lands_at_the_caret("after a hard break");
}

/// At a soft-wrapped line's start before a code run, reached with Home: `Z`
/// joins the plain run before it, at the line start, before the padding.
#[test]
fn at_a_soft_wrapped_line_start_before_a_code_run() {
    let mut p = page("<p>aaaa aaaa aaaa aaaa aaaa aaaa aaaa aaaa <code>bbbbbbbb</code> c</p>");
    // Put the caret inside the code run, then Home to its line's start.
    let start = 1 + "aaaa ".len() * 8;
    p.caret_at(start + 3);
    {
        let block = p.handle.caret_address(Pos(start)).unwrap().0;
        let doc = p.app.doc.as_ref().unwrap().borrow();
        let il = doc.tree.nodes[block].text_layout.as_ref().unwrap();
        let line = parley::layout::Cluster::from_byte_index(&il.layout, start - 1)
            .unwrap()
            .line();
        assert_eq!(
            line.text_range().start,
            start - 1,
            "positive control: the code run starts a soft-wrapped line"
        );
    }
    p.app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Home,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    idle(&mut p.app);
    assert_eq!(p.handle.selection().head(), Pos(start), "Home");
    p.assert_typed_lands_at_the_caret("soft-wrapped line start, after Home");
}

/// After a code run (inclusive): `Z` joins it, inside the pill.
#[test]
fn after_a_code_run() {
    let mut p = page("<p>a<code>bc</code>d</p>");
    p.caret_at(4);
    p.assert_typed_lands_at_the_caret("after a code run");
}

/// Before a code run after plain text: `Z` joins the plain run, before the
/// padding.
#[test]
fn before_a_code_run_after_text() {
    let mut p = page("<p>a<code>bc</code>d</p>");
    p.caret_at(2);
    p.assert_typed_lands_at_the_caret("before a code run");
}

/// After a link (not inclusive): `Z` lands past the link's padding.
#[test]
fn after_a_link() {
    let mut p = page(r#"<p>d<a href="x">ef</a>g</p>"#);
    p.caret_at(4);
    p.assert_typed_lands_at_the_caret("after a link");
}

/// One link split into two runs (the second also bold): the caret at the
/// seam carries the link and the first run's marks, so `Z` joins the first
/// run, inside it, before its end padding (review of #1832: drawn past the
/// second run's start padding).
#[test]
fn at_the_seam_of_a_link_split_into_two_runs() {
    let mut p = page(r#"<p>d<a href="x">e</a><a href="x"><strong>f</strong></a>g</p>"#);
    // `d` 1..2, `e` 2..3, `f` 3..4.
    p.caret_at(3);
    p.assert_typed_lands_at_the_caret("at the seam");
}

/// Typing `` `bc` ``: the input rule makes `bc` code and drops the stored
/// mark, so the caret after the pill does not carry `code`, and `Z` lands
/// past its padding (review of #1832: drawn inside the pill).
#[test]
fn after_a_code_input_rule() {
    let mut p = page("<p>a </p>");
    p.caret_at(3);
    for ch in ["`", "b", "c", "`"] {
        assert!(p.handle.insert_text(ch));
        idle(&mut p.app);
    }
    assert!(
        format!("{:?}", p.handle.doc()).contains("code"),
        "positive control: the rule made code: {:?}",
        p.handle.doc()
    );
    p.assert_typed_lands_at_the_caret("after the input rule");
}

/// Toggling `code` at a code run's start (after plain text) stores the mark,
/// so `Z` joins the code run, inside its padding.
#[test]
fn toggling_code_at_a_code_runs_start() {
    let mut p = page("<p>a<code>bc</code>d</p>");
    p.caret_at(2);
    assert!(p.handle.command("toggleCode"));
    idle(&mut p.app);
    p.assert_typed_lands_at_the_caret("toggled code at the run's start");
}
