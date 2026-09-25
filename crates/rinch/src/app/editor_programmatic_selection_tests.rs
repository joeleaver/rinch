//! A selection change made by **app code** — `EditorHandle::set_selection` or
//! `command("selectAll")` from a timer, an effect, a menu callback — renders the
//! caret and the selection highlight on desktop, with no input event (#1001).
//!
//! The overlay pass (`update_all_carets`) runs after layout, inside
//! `resolve_and_repaint`, which short-circuits when nothing is dirty; a
//! selection-only transaction changes no DOM. The input paths call
//! `refresh_editor_overlays` themselves; app code has nobody to, so the handle
//! owes the pass (`registry::overlay_pass_owed`) and the runtime folds that into
//! its "is there anything to do?" predicates, as it does an owed reveal (#922).
//!
//! Everything is driven through `handle_event` with the frame clock the shell
//! runs (`AboutToWait`) and a redraw when one is asked for — never by calling
//! `resolve_and_repaint` or `refresh_editor_overlays` directly, because the
//! point is that the runtime wakes for the change by itself.
//!
//! The text is set in the bundled Inter (`sans-serif`, claimed by an app font)
//! with a declared `font-size` and `line-height`, and every position is checked
//! against the paragraph's own box: where inside a line box Parley puts a
//! selection rect depends on the face's metrics, and CI's DejaVu put the same
//! highlight a pixel lower than this host's default sans-serif did.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// The bundled face the text is set in — see the module doc.
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// Paragraph `i` is `line NNN`: 8 characters, spanning `1 + 10i` to `9 + 10i`.
fn start_of(i: usize) -> Pos {
    Pos(1 + 10 * i)
}
fn end_of(i: usize) -> Pos {
    Pos(9 + 10 * i)
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// Six paragraphs in an editor, focused (through `EditorHandle::focus`, so no
/// press placed anything) and settled.
fn page() -> Page {
    let handle = crate::editor::create_editor();
    let html: String = (0..6).map(|i| format!("<p>line {i:03}</p>")).collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 400px; height: 300px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle };
    page.handle.focus();
    idle(&mut page.app);
    assert!(
        matches!(page.app.focus_target, FocusTarget::Editor(_)),
        "precondition: the editor holds the keyboard"
    );
    page
}

/// What the desktop loop does between inputs: turn the frame clock, and
/// redraw (resolve what is pending, paint) whenever a turn asks for it.
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

/// The visible selection-highlight rects in the document.
fn highlights(app: &RinchApp) -> Vec<(f32, f32, f32, f32)> {
    let doc = app.doc.as_ref().unwrap().borrow();
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

/// Paragraph `i`'s painted box: one 24px line, so its line box.
fn para_box(app: &RinchApp, i: usize) -> (f32, f32, f32, f32) {
    let doc = app.doc.as_ref().unwrap().borrow();
    let ps = doc.query_selector_all("p");
    assert_eq!(ps.len(), 6, "the six paragraphs");
    painted_element_box(&doc.tree, ps[i].0)
}

/// The editor container's left and top border widths. The overlays are
/// anchored at its padding box, where the text's own geometry is measured from
/// its border box, so they sit this far right of and below the text.
fn editor_border(app: &RinchApp) -> (f32, f32) {
    let doc = app.doc.as_ref().unwrap().borrow();
    let ed = doc.query_selector_all("[data-pm-editor]");
    let cs = &doc.tree.nodes[ed[0].0].computed_style;
    (cs.border_left_width.to_px(), cs.border_top_width.to_px())
}

/// The caret overlay's painted box, if it is shown.
fn caret_box(app: &RinchApp) -> Option<(f32, f32, f32, f32)> {
    let doc = app.doc.as_ref().unwrap().borrow();
    let id = doc
        .query_selector_all("[data-pm-caret]")
        .into_iter()
        .next()?;
    if doc.tree.nodes[id.0].computed_style.display == rinch_dom::computed_style::DisplayValue::None
    {
        return None;
    }
    Some(painted_element_box(&doc.tree, id.0))
}

/// `set_selection` of a range from app code draws its highlight after one idle
/// turn of the loop, over the paragraph it names.
#[test]
fn a_range_set_from_app_code_draws_its_highlight() {
    let mut page = page();
    assert!(highlights(&page.app).is_empty(), "no range yet");

    // Off the fixed point: the third paragraph, not the first.
    page.handle
        .set_selection(Selection::text(start_of(2), end_of(2)));
    idle(&mut page.app);

    let rects = highlights(&page.app);
    assert_eq!(rects.len(), 1, "one line selected: {rects:?}");
    let (x, y, w, h) = rects[0];
    let (px, py, _, ph) = para_box(&page.app, 2);
    let (bl, bt) = editor_border(&page.app);
    assert_eq!(x, px + bl, "the highlight starts at the line's start");
    // `selection_rects_for_layout` puts the rect's top at `baseline - ascent`
    // and gives it the line's height, so it starts one half-leading below the
    // line box — `(24 - (ascent + descent)) / 2`, a font metric (3px in the
    // bundled Inter; host faces gave 1px, 2px on CI's DejaVu, 4px on another) — and overhangs the
    // line's bottom by as much. What is font-independent: the full line height,
    // and a top in the upper half of the third line's box (a stale highlight,
    // or one on another line, is 36px away).
    assert_eq!(h, ph, "the highlight is one line tall");
    assert!(
        y >= py + bt && y < py + bt + ph / 2.0,
        "the highlight sits on the third line: {:?} vs line box y {py} h {ph}",
        rects[0]
    );
    assert!(w > 20.0, "and spans its text: {w}");
}

/// A caret moved from app code is drawn where it now is, after one idle turn.
#[test]
fn a_caret_set_from_app_code_is_drawn_where_it_now_is() {
    let mut page = page();
    let target = Pos(start_of(4).0 + 3);
    page.handle.set_selection(Selection::cursor(target));
    idle(&mut page.app);

    let (cx, _, _) = page
        .app
        .editor_caret_point(&page.handle, target)
        .expect("the fifth paragraph has geometry");
    let (_, py, _, ph) = para_box(&page.app, 4);
    let (bl, bt) = editor_border(&page.app);
    let (x, y, _, h) = caret_box(&page.app).expect("the caret is shown");
    assert_eq!(
        (y, h),
        (py + bt, ph),
        "the caret spans the fifth line's box (a stale caret sits on the first)"
    );
    // The overlay's x is the text caret's, snapped to a whole pixel.
    assert!(
        (x - (cx + bl)).abs() <= 1.0,
        "the caret is drawn at x {x}, the selection puts it at {}",
        cx + bl
    );
}

/// `command("selectAll")` from app code highlights every line.
#[test]
fn select_all_from_app_code_highlights_every_line() {
    let mut page = page();
    assert!(page.handle.command("selectAll"), "positive control");
    idle(&mut page.app);
    assert_eq!(
        highlights(&page.app).len(),
        6,
        "one rect per paragraph: {:?}",
        highlights(&page.app)
    );
}

/// Owing the pass costs nothing once it has run: the loop goes idle again
/// rather than resolving every turn (the flag is cleared by the pass).
#[test]
fn the_owed_pass_runs_once_and_the_loop_goes_idle() {
    let mut page = page();
    page.handle
        .set_selection(Selection::text(start_of(1), end_of(1)));
    idle(&mut page.app);
    assert_eq!(highlights(&page.app).len(), 1, "positive control");
    assert!(!page.app.has_pending_layout(), "nothing is owed any more");
    assert!(!page.app.has_dirty_nodes(), "nothing is owed any more");
}
