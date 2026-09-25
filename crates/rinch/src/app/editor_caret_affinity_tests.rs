//! Caret affinity at a soft wrap (#301, PR #1019), on desktop.
//!
//! The end of one visual line and the start of the next are one model
//! position. The editor keeps a view-level hint beside the selection
//! (`EditorHandle::caret_affinity`): End, a click past a wrapped line's end and
//! a vertical move onto a wrap point set **upstream** (the caret draws at the end
//! of the upper line); Home and every other selection write leave the default,
//! **downstream** (the start of the lower line). Before it, desktop's End
//! stepped back one position to stay on its line, which on a GLYPH wrap — a
//! word broken by `overflow-wrap`, no hanging space — stopped one character
//! short and typed before the line's last letter.
//!
//! Driven through `handle_event` (key presses, a press) and the frame clock,
//! with the caret overlay's painted box as the answer to "where is it drawn".
//! The text is the bundled Inter at a declared 16px / 24px; line starts are
//! measured from the layout (downstream caret positions, the default), never
//! hard-coded.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const LINE: f32 = 24.0;

const WORD: &str =
    "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghij";
const WORDS: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    text: &'static str,
}

/// One paragraph of `text` in a 180px editor, focused through
/// `EditorHandle::focus` and settled.
fn page(text: &'static str, p_style: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p style=\"{p_style}\">{text}</p>")));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 180px; height: 400px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&editor);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page { app, handle, text };
    page.handle.focus();
    idle(&mut page.app);
    assert!(matches!(page.app.focus_target, FocusTarget::Editor(_)));
    page
}

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

fn key(app: &mut RinchApp, key: KeyCode, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    app.handle_event(
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
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
        VP,
        1.0,
    );
    idle(app);
}

fn type_x(app: &mut RinchApp) {
    let key = KeyCode::KeyX;
    let c = "X";
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: Some(c.to_string()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers: Modifiers::default(),
        },
        VP,
        1.0,
    );
    idle(app);
}

impl Page {
    /// The paragraph's host id.
    fn block(&self) -> usize {
        self.handle.caret_address(Pos(1)).unwrap().0
    }

    /// The layout-local downstream caret `(x, y)` at char offset `i` — the
    /// default Parley answer, the measuring stick.
    fn local(&self, i: u32) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(self.block() as u64, i as usize)
            .expect("laid out")
    }

    /// The char offsets each visual line starts at.
    fn line_starts(&self) -> Vec<u32> {
        let n = self.text.chars().count() as u32;
        let mut starts = vec![0];
        let mut y = self.local(0).1;
        for i in 1..n {
            let yi = self.local(i).1;
            if yi > y + 1.0 {
                starts.push(i);
                y = yi;
            }
        }
        starts
    }

    /// The paragraph's painted top.
    fn para_top(&self) -> f32 {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, self.block()).1
    }

    /// Which visual line the caret overlay is drawn on.
    fn caret_line(&self) -> Option<usize> {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let id = doc
            .query_selector_all("[data-pm-caret]")
            .into_iter()
            .next()?;
        if doc.tree.nodes[id.0].computed_style.display
            == rinch_dom::computed_style::DisplayValue::None
        {
            return None;
        }
        let (_, y, _, h) = painted_element_box(&doc.tree, id.0);
        drop(doc);
        Some(((y + h / 2.0 - self.para_top()) / LINE).floor() as usize)
    }

    fn caret_at(&mut self, i: u32) {
        self.handle
            .set_selection(Selection::cursor(Pos(i as usize + 1)));
        idle(&mut self.app);
    }

    fn head(&self) -> u32 {
        (self.handle.selection().head().0 - 1) as u32
    }

    fn text_now(&self) -> String {
        let doc = self.handle.doc();
        let p = doc.child(0);
        (0..p.child_count())
            .filter_map(|j| p.child(j).text().map(str::to_string))
            .collect()
    }
}

fn slice(s: &str, from: u32, to: u32) -> String {
    s.chars()
        .skip(from as usize)
        .take((to - from) as usize)
        .collect()
}

/// A glyph-wrapped word, the caret three letters into line 2 (index 1).
fn glyph_wrapped() -> (Page, Vec<u32>, u32) {
    let mut p = page(WORD, "overflow-wrap: anywhere");
    let starts = p.line_starts();
    assert!(starts.len() >= 4, "positive control: 4+ lines, {starts:?}");
    let caret = starts[1] + 3;
    p.caret_at(caret);
    assert_eq!(
        p.caret_line(),
        Some(1),
        "positive control: the caret is on line 2"
    );
    (p, starts, caret)
}

/// End on a glyph wrap lands AT the wrap point and draws on the line it was
/// pressed on; a second End stays; Home comes back to that line's start.
#[test]
fn end_on_a_glyph_wrap_lands_at_the_wrap_upstream() {
    let (mut p, starts, _) = glyph_wrapped();
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(
        p.head(),
        starts[2],
        "End lands on the wrap point, not one short"
    );
    assert_eq!(p.caret_line(), Some(1), "and draws on its own line");
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2], "a second End stays");
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(
        p.head(),
        starts[1],
        "End, Home comes back to the line's start"
    );
    assert_eq!(p.caret_line(), Some(1));
}

/// Home, Home stays; Home, End reaches the same line's end; Shift+Home,
/// Shift+End selects from the caret to that end.
#[test]
fn home_on_a_glyph_wrap_stays_on_its_line() {
    let (mut p, starts, caret) = glyph_wrapped();
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), starts[1]);
    assert_eq!(
        p.caret_line(),
        Some(1),
        "Home draws on the line it was pressed on"
    );
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), starts[1], "a second Home stays");
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2], "Home, End reaches the line's end");
    p.caret_at(caret);
    key(&mut p.app, KeyCode::Home, true);
    key(&mut p.app, KeyCode::End, true);
    assert_eq!(
        p.handle.selection(),
        Selection::text(Pos(caret as usize + 1), Pos(starts[2] as usize + 1))
    );
}

/// Typing after End on a glyph wrap appends to the upper line.
#[test]
fn typing_after_end_on_a_glyph_wrap_appends_to_the_upper_line() {
    let (mut p, starts, _) = glyph_wrapped();
    key(&mut p.app, KeyCode::End, false);
    type_x(&mut p.app);
    let n = WORD.chars().count() as u32;
    assert_eq!(
        p.text_now(),
        format!(
            "{}X{}",
            slice(WORD, 0, starts[2]),
            slice(WORD, starts[2], n)
        )
    );
}

/// The hint belongs to the selection it came with: an app's `set_selection` of
/// the same caret clears it, and the caret draws downstream.
#[test]
fn an_app_set_selection_clears_the_hint() {
    let (mut p, starts, _) = glyph_wrapped();
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.caret_line(), Some(1), "control: End draws upstream");
    p.caret_at(starts[2]);
    assert_eq!(
        p.caret_line(),
        Some(2),
        "a plain caret at the wrap draws downstream"
    );
}

/// On a space wrap End lands on the wrap point too (after the hanging space),
/// where it used to step back before it.
#[test]
fn end_on_a_space_wrap_lands_after_the_hanging_space() {
    let mut p = page(WORDS, "");
    let starts = p.line_starts();
    assert!(starts.len() >= 4, "{starts:?}");
    p.caret_at(starts[1] + 3);
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2]);
    assert_eq!(p.caret_line(), Some(1));
}

/// A vertical move whose goal column lies past the target line's end lands on
/// that line's wrap point, drawn on the line it moved to.
#[test]
fn a_vertical_move_onto_a_wrap_point_draws_on_the_target_line() {
    let (mut p, starts, _) = glyph_wrapped();
    // End on line 2 puts the goal column at the right edge.
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2]);
    key(&mut p.app, KeyCode::ArrowDown, false);
    assert_eq!(p.head(), starts[3], "down onto line 3's wrap point");
    assert_eq!(p.caret_line(), Some(2), "drawn on line 3, not line 4");
    key(&mut p.app, KeyCode::ArrowUp, false);
    assert_eq!(p.head(), starts[2]);
    assert_eq!(p.caret_line(), Some(1), "back up, drawn on line 2");
}

/// A press past a wrapped line's end lands on its wrap point and draws on the
/// clicked line.
#[test]
fn a_click_past_a_wrapped_lines_end_draws_on_the_clicked_line() {
    let mut p = page(WORDS, "");
    let starts = p.line_starts();
    assert!(starts.len() >= 4, "{starts:?}");
    // A middle line (neither first nor last) with room past its end: its last
    // glyph ends at the local x of its hanging space.
    let (bx, by, bw, _) = {
        let doc = p.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&doc.tree, p.block())
    };
    let li = (1..starts.len() - 1)
        .find(|&i| bw - p.local(starts[i + 1] - 1).0 > 20.0)
        .expect("positive control: a ragged middle line");
    let (lx, ly) = p.local(starts[li + 1] - 1);
    let x = bx + (lx + bw) / 2.0;
    let y = by + ly + LINE / 2.0;
    let button = MouseButton::Left;
    p.app
        .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    p.app
        .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    idle(&mut p.app);
    assert_eq!(
        p.head(),
        starts[li + 1],
        "the press lands on the wrap point"
    );
    assert_eq!(p.caret_line(), Some(li), "and draws on the clicked line");
}
