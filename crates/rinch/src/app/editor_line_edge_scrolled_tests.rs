//! Home / End on a caret line that is scrolled out of view (#1107).
//!
//! Desktop found the edge of the caret's visual line by hit-testing a point
//! on it. Hit testing is clip-aware, so once that line was scrolled out of an
//! `overflow: auto` editor the probe found nothing and Home / End fell back to
//! the whole textblock's edge. The line is now read from the Parley layout
//! the caret is in, which knows nothing of scrolling or clipping.
//!
//! Every fixture scrolls the editor so the caret's paragraph is entirely above
//! its box (a positive control asserts it), then presses a key. The text is
//! the bundled Inter at a declared 16px / 24px; line starts are measured from
//! the layout, never hard-coded.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_core::dom::{CaretAffinity, NodeId};
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

const WORDS: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike";
/// Enough paragraphs after the first for the editor to scroll it away.
const FILLER: &str = "<p>one</p><p>two</p><p>three</p><p>four</p><p>five</p>\
                      <p>six</p><p>seven</p><p>eight</p><p>nine</p><p>ten</p>";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

/// `first` (a paragraph's inner html) and the filler in a 180px x 150px
/// `overflow: auto` editor, focused and settled.
fn page(first: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p>{first}</p>{FILLER}")));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute(
            "style",
            "width: 180px; height: 150px; overflow: auto; font-size: 16px; \
             line-height: 24px; font-family: sans-serif",
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

impl Page {
    /// The first paragraph's host id.
    fn block(&self) -> usize {
        self.handle.caret_address(Pos(1)).unwrap().0
    }

    /// The editor container's host id.
    fn container(&self) -> usize {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-editor]")[0].0
    }

    /// The layout-local downstream caret `(x, y)` at char offset `i` of the
    /// first paragraph (text only: no leaves before `i`).
    fn local(&self, i: u32) -> (f32, f32) {
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(self.block() as u64, i as usize)
            .expect("laid out")
    }

    /// The char offsets each visual line of a text-only first paragraph of
    /// `n` chars starts at.
    fn line_starts(&self, n: u32) -> Vec<u32> {
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

    fn caret_at(&mut self, i: u32) {
        self.handle
            .set_selection(Selection::cursor(Pos(i as usize + 1)));
        idle(&mut self.app);
    }

    fn head(&self) -> u32 {
        (self.handle.selection().head().0 - 1) as u32
    }

    /// Scroll the editor until its first paragraph is wholly above its box,
    /// and assert that it is.
    fn scroll_first_paragraph_away(&mut self) {
        let c = self.container();
        {
            let doc = self.app.doc.as_ref().unwrap().clone();
            doc.borrow_mut().set_scroll_top(NodeId(c), 400.0);
        }
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        let _ = self.app.build_pixels(1.0, VP, false);
        let doc = self.app.doc.as_ref().unwrap().borrow();
        let (_, cy, _, _) = painted_element_box(&doc.tree, c);
        let (_, py, _, ph) = painted_element_box(&doc.tree, self.block());
        assert!(
            py + ph < cy,
            "positive control: the paragraph ({py}+{ph}) is above the editor's top ({cy})"
        );
    }
}

/// A multi-line paragraph, the caret three letters into its second line,
/// scrolled away.
fn scrolled_words() -> (Page, Vec<u32>, u32) {
    let mut p = page(WORDS);
    let starts = p.line_starts(WORDS.chars().count() as u32);
    assert!(starts.len() >= 4, "positive control: 4+ lines, {starts:?}");
    let caret = starts[1] + 3;
    p.caret_at(caret);
    p.scroll_first_paragraph_away();
    (p, starts, caret)
}

/// End lands on the line's end (its wrap point), not the paragraph's.
#[test]
fn end_on_a_scrolled_away_line_lands_at_the_lines_end() {
    let (mut p, starts, _) = scrolled_words();
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2], "End reaches line 2's end");
    assert_eq!(
        p.handle.caret_affinity(),
        CaretAffinity::Upstream,
        "drawn at the end of line 2, not the start of line 3 (#301)"
    );
}

/// Home lands on the line's start, not the paragraph's.
#[test]
fn home_on_a_scrolled_away_line_lands_at_the_lines_start() {
    let (mut p, starts, _) = scrolled_words();
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), starts[1]);
}

/// An upstream caret at a wrap point is on the upper line: End, scroll away,
/// Home comes back to the start of the line End was pressed on — not the
/// start of the next line, which is the same model position as the caret.
#[test]
fn home_after_end_uses_the_upstream_line() {
    let mut p = page(WORDS);
    let starts = p.line_starts(WORDS.chars().count() as u32);
    assert!(starts.len() >= 4, "{starts:?}");
    p.caret_at(starts[1] + 3);
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2], "control: End at the wrap point");
    assert_eq!(p.handle.caret_affinity(), CaretAffinity::Upstream);
    p.scroll_first_paragraph_away();
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), starts[1], "Home from the upstream caret");
    // And a second End from there stays on line 2.
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[2]);
}

/// Shift+End from a scrolled-away caret selects to the line's end.
#[test]
fn shift_end_on_a_scrolled_away_line_selects_to_the_lines_end() {
    let (mut p, starts, caret) = scrolled_words();
    key(&mut p.app, KeyCode::End, true);
    assert_eq!(
        p.handle.selection(),
        Selection::text(Pos(caret as usize + 1), Pos(starts[2] as usize + 1))
    );
}

/// On the last line End reaches the paragraph's end (the line's end is the
/// text's end there).
#[test]
fn end_on_the_last_scrolled_away_line_reaches_the_paragraphs_end() {
    let (mut p, starts, _) = scrolled_words();
    let n = WORDS.chars().count() as u32;
    p.caret_at(*starts.last().unwrap() + 1);
    p.scroll_first_paragraph_away();
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), n);
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), *starts.last().unwrap());
}

/// A hard break ends a line: End before it stops at the break, Home after it
/// stops just past it — the break is one model position, 11 = "alpha bravo".
#[test]
fn a_hard_break_bounds_a_scrolled_away_line() {
    let mut p = page("alpha bravo<br>charlie delta echo foxtrot golf hotel india");
    p.caret_at(3);
    p.scroll_first_paragraph_away();
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), 11, "End stops before the break");
    p.caret_at(15);
    p.scroll_first_paragraph_away();
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), 12, "Home stops after the break");
}
