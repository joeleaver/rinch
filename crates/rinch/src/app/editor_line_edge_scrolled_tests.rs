//! Home / End (#1107) and ArrowUp / ArrowDown (#1121) on a caret line that
//! is scrolled out of view.
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
    page_styled(first, "24px", "")
}

/// [`page`] with another editor `line-height` and an app stylesheet `css`.
fn page_styled(first: &str, line_height: &str, css: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p>{first}</p>{FILLER}")));
    let handle_in = handle.clone();
    let style = format!(
        "width: 180px; height: 150px; overflow: auto; font-size: 16px; \
         line-height: {line_height}; font-family: sans-serif"
    );
    let css = css.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let editor = handle_in.mount(scope);
        editor.set_attribute("style", &style);
        root.append_child(&editor);
        if !css.is_empty() {
            let sheet = scope.create_element("style");
            sheet.append_child(&scope.create_text(&css));
            root.append_child(&sheet);
        }
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
    /// first paragraph (text only: no leaves before `i`), through the view's
    /// char → byte map.
    fn local(&self, i: u32) -> (f32, f32) {
        let (block, byte) = self.handle.caret_address(Pos(i as usize + 1)).unwrap();
        let doc = self.app.doc.as_ref().unwrap().borrow();
        doc.query_caret_position(block as u64, byte)
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

/// Affinity chooses a line only at a SOFT wrap: an upstream caret just after
/// a hard break is on the break's far line, as Parley draws it.
#[test]
fn upstream_after_a_hard_break_stays_on_the_far_line() {
    let p = page("alpha bravo<br>charlie");
    let doc = p.app.doc.as_ref().unwrap().borrow();
    let line =
        |b, a| rinch_dom::text_query::visual_line_range_for_node(&doc, p.block() as u64, b, a);
    // "alpha bravo" is bytes 0..11, the break byte 11, "charlie" 12..19.
    assert_eq!(line(12, CaretAffinity::Downstream), Some(12..19), "control");
    assert_eq!(line(12, CaretAffinity::Upstream), Some(12..19));
    assert_eq!(line(3, CaretAffinity::Upstream), Some(0..11));
}

const IMG: &str = "<img src=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==\" style=\"width: 40px; height: 16px\">";

/// An image that ends a wrapped line has no bytes, so the byte of a caret
/// right before it is where the NEXT line's bytes start — while Parley draws
/// that caret on the image's own line. Home goes to the start of the line the
/// caret is drawn on (review of #1118: picking the line by byte range made
/// Home a no-op here and sent End to the next line's end), visible or
/// scrolled away.
#[test]
fn home_before_an_image_that_ends_a_line_goes_to_that_lines_start() {
    for scrolled in [false, true] {
        let mut p = page(&format!(
            "alpha bravo{IMG}charlie delta echo foxtrot golf hotel"
        ));
        p.caret_at(11); // model pos 12: right before the image
        let y = p.app.editor_caret_point(&p.handle, Pos(12)).unwrap().1;
        let y0 = p.app.editor_caret_point(&p.handle, Pos(1)).unwrap().1;
        let y2 = p.app.editor_caret_point(&p.handle, Pos(14)).unwrap().1;
        assert_eq!(y, y0, "control: the caret before the image is on line 1");
        assert!(y2 > y0 + 10.0, "control: the text after the image wraps");
        if scrolled {
            p.scroll_first_paragraph_away();
        }
        key(&mut p.app, KeyCode::Home, false);
        assert_eq!(
            p.head(),
            0,
            "Home reaches line 1's start (scrolled: {scrolled})"
        );
    }
}

const HEBREW: &str = "שלום עולם זה טקסט ארוך בעברית שנשבר לכמה שורות בעורך הזה";

/// Home / End on a right-to-left line are its LOGICAL edges — Home the
/// line's first character (drawn at its right edge), End its wrap point —
/// as in Chrome 153 (Home 0, End at the wrap from offset 3 on the same text).
/// The hit-test code this replaced answered the visual edges: Home at the
/// line's logical end, End at 0.
#[test]
fn home_and_end_on_a_right_to_left_line_are_its_logical_edges() {
    let mut p = page(HEBREW);
    let starts = p.line_starts(HEBREW.chars().count() as u32);
    assert!(starts.len() >= 3, "positive control: 3+ lines, {starts:?}");
    p.caret_at(3);
    key(&mut p.app, KeyCode::Home, false);
    assert_eq!(p.head(), 0, "Home: the line's first character");
    p.caret_at(3);
    key(&mut p.app, KeyCode::End, false);
    assert_eq!(p.head(), starts[1], "End: the line's wrap point");
}

// ---------------------------------------------------------------------------
// ArrowUp / ArrowDown on a scrolled-away line (#1121).
//
// Up and Down found the target line by hit-testing a point above or below the
// caret, which a clipping scroller hides, so a caret in a paragraph scrolled
// out of view jumped to the next or previous paragraph. Every fixture below
// presses the same keys twice, once with the paragraph in view (where the hit
// test finds the line) and once scrolled away, and asks for the same answer;
// Chrome 155 moves within the paragraph either way and scrolls the caret back
// into view.
// ---------------------------------------------------------------------------

/// The caret `(head, affinity)` after placing it with `place` in a page whose
/// first paragraph is `first`, optionally scrolling that paragraph away, and
/// pressing `keys` (true = ArrowDown).
fn after_vertical_keys(
    first: &str,
    place: impl Fn(&mut Page),
    scrolled: bool,
    keys: &[bool],
) -> (u32, CaretAffinity) {
    let mut p = page(first);
    place(&mut p);
    if scrolled {
        p.scroll_first_paragraph_away();
    }
    for &down in keys {
        let k = if down {
            KeyCode::ArrowDown
        } else {
            KeyCode::ArrowUp
        };
        key(&mut p.app, k, false);
    }
    (p.head(), p.handle.caret_affinity())
}

/// Scrolled away and in view give one answer. The callers pin that answer
/// too: every literal below is what the hit test answered in view before
/// #1121 (measured at 1e7afee5), so the layout read is held to the old
/// in-view behaviour as well as to its own scrolled-away twin.
fn same_in_view_and_scrolled(
    first: &str,
    place: impl Fn(&mut Page) + Copy,
    keys: &[bool],
) -> (u32, CaretAffinity) {
    let seen = after_vertical_keys(first, place, false, keys);
    let away = after_vertical_keys(first, place, true, keys);
    assert_eq!(away, seen, "scrolled away vs in view, keys {keys:?}");
    seen
}

/// The paragraph's line starts and the caret three letters into line 2.
fn words_line2() -> (Vec<u32>, u32) {
    let p = page(WORDS);
    let starts = p.line_starts(WORDS.chars().count() as u32);
    assert!(starts.len() >= 4, "positive control: 4+ lines, {starts:?}");
    (starts.clone(), starts[1] + 3)
}

/// Down from line 2 lands on line 3 at the caret's column, not in the next
/// paragraph (it went to char 79, the paragraph after, before #1121).
#[test]
fn down_on_a_scrolled_away_line_stays_in_the_paragraph() {
    let (starts, caret) = words_line2();
    let (head, _) = same_in_view_and_scrolled(WORDS, move |p| p.caret_at(caret), &[true]);
    assert_eq!(head, 35, "the in-view answer before #1121");
    assert!(
        (starts[2]..starts[3]).contains(&head),
        "Down lands on line 3: {head}, starts {starts:?}"
    );
    assert_ne!(head, starts[2], "off the line's start: the column is kept");
}

/// Up from line 2 lands on line 1 at the caret's column (it went to the
/// start of the document before #1121).
#[test]
fn up_on_a_scrolled_away_line_stays_in_the_paragraph() {
    let (starts, caret) = words_line2();
    let (head, _) = same_in_view_and_scrolled(WORDS, move |p| p.caret_at(caret), &[false]);
    assert_eq!(head, 4, "the in-view answer before #1121");
    assert!(
        (1..starts[1]).contains(&head),
        "Up lands on line 1, off its start: {head}, starts {starts:?}"
    );
}

/// The goal column is kept across consecutive steps, and Down then Up comes
/// back to where it started.
#[test]
fn consecutive_steps_on_scrolled_away_lines_keep_the_column() {
    let (starts, caret) = words_line2();
    let (head, _) = same_in_view_and_scrolled(WORDS, move |p| p.caret_at(caret), &[true, true]);
    assert_eq!(head, 54, "the in-view answer before #1121");
    assert!(
        starts.len() < 5 || (starts[3]..starts[4]).contains(&head),
        "two Downs land on line 4: {head}, starts {starts:?}"
    );
    let (back, _) = same_in_view_and_scrolled(WORDS, move |p| p.caret_at(caret), &[true, false]);
    assert_eq!(back, 15, "the in-view answer before #1121");
    assert_eq!(back, caret, "Down then Up returns to the caret");
}

/// A goal column past the target line's end lands on its wrap point, drawn
/// at the end of that line: upstream (#301), scrolled away or not.
#[test]
fn a_goal_past_a_scrolled_away_lines_end_lands_upstream_at_its_wrap() {
    // "alpha bravo " (line 1) is narrower than "charlie echo " (line 2).
    let text = "alpha bravo charlie echo golfgolfgolfgolfgolfgolf hotel";
    let p0 = page(text);
    let starts = p0.line_starts(text.chars().count() as u32);
    drop(p0);
    assert_eq!(&starts[..3], &[0, 12, 25], "positive control: the lines");
    // End of line 2 (upstream at its wrap), then Up.
    let place = |p: &mut Page| {
        p.caret_at(14);
        key(&mut p.app, KeyCode::End, false);
        assert_eq!(
            (p.head(), p.handle.caret_affinity()),
            (25, CaretAffinity::Upstream),
            "control: End at line 2's wrap"
        );
    };
    let (head, affinity) = same_in_view_and_scrolled(text, place, &[false]);
    assert_eq!(
        (head, affinity),
        (12, CaretAffinity::Upstream),
        "line 1's wrap point, drawn at its end: the in-view answer before #1121"
    );
}

/// Down and Up across a line that an image ends (the image has no bytes, so
/// its line is the image's box's, #1116) give the in-view answer.
#[test]
fn steps_beside_an_image_on_scrolled_away_lines() {
    let first = format!("alpha bravo{IMG}charlie delta echo foxtrot golf hotel india");
    for (caret, keys, want) in [
        (3u32, &[true][..], 14),
        (11, &[true], 24), // right before the image
        (12, &[false], 0), // right after it
        (16, &[false], 4),
        (16, &[false, true], 16),
    ] {
        let got = same_in_view_and_scrolled(&first, move |p| p.caret_at(caret), keys);
        assert_eq!(
            got,
            (want, CaretAffinity::Downstream),
            "caret {caret}, keys {keys:?}: the in-view answer before #1121"
        );
    }
}

/// Down from the paragraph's last line still leaves it for the next one.
#[test]
fn down_from_a_scrolled_away_last_line_leaves_the_paragraph() {
    let mut p = page(WORDS);
    let starts = p.line_starts(WORDS.chars().count() as u32);
    let first = p.block();
    p.caret_at(*starts.last().unwrap() + 2);
    p.scroll_first_paragraph_away();
    key(&mut p.app, KeyCode::ArrowDown, false);
    let head = p.handle.selection().head();
    assert_ne!(
        p.handle.caret_address(head).map(|(t, _)| t),
        Some(first),
        "Down left the paragraph"
    );
}

/// The moved caret is scrolled back into view, as Chrome 155 does.
#[test]
fn a_step_on_a_scrolled_away_line_scrolls_the_caret_into_view() {
    let (mut p, starts, _) = scrolled_words();
    key(&mut p.app, KeyCode::ArrowDown, false);
    assert!(
        (starts[2]..starts[3]).contains(&p.head()),
        "control: line 3"
    );
    let head = p.handle.selection().head();
    let (_, y, h) = p.app.editor_caret_point(&p.handle, head).unwrap();
    let doc = p.app.doc.as_ref().unwrap().borrow();
    let (_, cy, _, ch) = painted_element_box(&doc.tree, p.container());
    assert!(
        y >= cy && y + h <= cy + ch,
        "caret {y}+{h} inside the editor {cy}+{ch}"
    );
}

/// A 40px x 16px image (the editor keeps an image's `width` / `height`
/// attributes, not its inline style).
const WIDE_IMG: &str = "<img src=\"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==\" width=\"40\" height=\"16\">";

/// Between two images a soft wrap separates, a downstream caret is drawn
/// before the second, on the lower line, while its byte is the upper line's
/// (#1116): Up and Down step from the line it is drawn on, one line. (In
/// view, Down went to line 4 before #1121: the probe was 1.5 caret heights
/// below a caret as tall as the image.)
#[test]
fn steps_from_a_caret_drawn_on_an_images_line() {
    let first =
        format!("alpha bravo{WIDE_IMG}{WIDE_IMG}{WIDE_IMG}charlie delta echo foxtrot golf hotel");
    let p = page(&first);
    // Chars: "alpha bravo" 0..11, images 11, 12, 13, "charlie" from 14.
    // Images 2 and 3 wrap: a downstream caret before image 2 (char 12) is
    // drawn on line 2, its byte (11) on line 1.
    let y = |i: usize| p.app.editor_caret_point(&p.handle, Pos(i + 1)).unwrap().1;
    assert!(
        y(12) > y(0) + 10.0,
        "positive control: before image 2 is on line 2"
    );
    assert!(
        y(11) < y(12),
        "positive control: before image 1 is on line 1"
    );
    assert!(y(30) > y(12) + 10.0, "positive control: there is a line 3");
    // Line 2 is image 2, image 3, "charlie " (chars 12..22); line 3 starts
    // at "delta" (22), line 4 at 41. The caret is at line 2's start.
    assert!(
        y(21) < y(22) && y(22) < y(41),
        "positive control: the lines"
    );
    drop(p);
    for (keys, want) in [(&[true][..], 22u32), (&[false], 0)] {
        let got = same_in_view_and_scrolled(&first, |p| p.caret_at(12), keys);
        assert_eq!(got, (want, CaretAffinity::Downstream), "keys {keys:?}");
    }
}

/// A goal column over an image lands beside it, on the side of the half it
/// is over (#1104), not on the byte after it.
#[test]
fn up_onto_an_images_right_half_lands_after_it() {
    let first = format!("alpha bravo{WIDE_IMG}charlie delta echo foxtrot golf hotel india");
    let p = page(&first);
    // The image is char 11; its right half, in window x.
    let (x0, _, _) = p.app.editor_caret_point(&p.handle, Pos(12)).unwrap();
    let (x1, y1, _) = p.app.editor_caret_point(&p.handle, Pos(13)).unwrap();
    assert!(
        x1 > x0 + 30.0,
        "positive control: the image is drawn, {x0}..{x1}"
    );
    // A caret on line 2 under the image's right half.
    let caret = (14..40u32)
        .find(|&i| {
            let (x, y, _) = p
                .app
                .editor_caret_point(&p.handle, Pos(i as usize + 1))
                .unwrap();
            y > y1 + 10.0 && x > (x0 + x1) / 2.0 + 2.0 && x < x1 - 2.0
        })
        .expect("positive control: a caret under the image's right half");
    drop(p);
    let got = same_in_view_and_scrolled(&first, move |p| p.caret_at(caret), &[false]);
    assert_eq!(
        got,
        (12, CaretAffinity::Downstream),
        "after the image: the in-view answer before #1121"
    );
}

/// The line under one that holds an image starts above that line's box's
/// bottom (the image on its baseline reaches down past it), and a caret on
/// it is on it: Home goes to its start, not to the paragraph's (#1121; the
/// first line box that contained the caret's top used to be taken).
#[test]
fn home_under_a_line_holding_an_image_stays_on_its_line() {
    let first = format!("alpha bravo{WIDE_IMG}charlie delta echo foxtrot golf hotel india");
    for scrolled in [false, true] {
        let mut p = page(&first);
        let y = |p: &Page, i: usize| p.app.editor_caret_point(&p.handle, Pos(i + 1)).unwrap().1;
        assert!(
            y(&p, 16) > y(&p, 12) + 10.0,
            "positive control: 16 is on line 2"
        );
        assert!(
            y(&p, 11) < y(&p, 16),
            "positive control: the image is on line 1"
        );
        p.caret_at(16);
        if scrolled {
            p.scroll_first_paragraph_away();
        }
        key(&mut p.app, KeyCode::Home, false);
        assert_eq!(p.head(), 12, "line 2's start (scrolled: {scrolled})");
    }
}

/// The Parley line the caret is drawn on, read independently of the code
/// under test: the line whose top is exactly the caret's top. rinch draws a
/// caret from its line's top: the line box's (`block_min_coord`), or its
/// runs' glyph tops, rounded, where they reach higher — under a line height
/// smaller than the text (`rinch_dom::text_query`; parley 0.11.1 folded that
/// into `block_min_coord`). `None` outside the first paragraph.
fn drawn_line(p: &Page) -> Option<usize> {
    let head = p.handle.selection().head();
    let (tb, byte) = p.handle.caret_address(head)?;
    if tb != p.block() {
        return None;
    }
    let doc = p.app.doc.as_ref().unwrap().borrow();
    let layout = &doc.tree.nodes.get(tb)?.text_layout.as_ref()?.layout;
    let (_, cy) = rinch_dom::text_query::caret_position_for_offset_layout_with_affinity(
        layout,
        byte,
        p.handle.caret_affinity(),
    );
    let lines: Vec<f32> = layout
        .lines()
        .map(|l| {
            let m = l.metrics();
            l.runs()
                .map(|r| m.baseline - r.font_metrics().ascent.round())
                .fold(m.block_min_coord, f32::min)
        })
        .collect();
    assert_eq!(
        lines.iter().filter(|&&m| m == cy).count(),
        1,
        "control: one line box starts at the caret's top, {lines:?}"
    );
    lines.iter().position(|&m| m == cy)
}

/// Down from line 0 until the caret leaves the first paragraph, then Up
/// back into it: one line per press each way (Chromium 155 moves one line
/// per press in both shapes below), in view and scrolled away.
fn walks_one_line_per_press(first: &str, line_height: &str, css: &str) {
    for scrolled in [false, true] {
        let mut p = page_styled(first, line_height, css);
        let lines = {
            let doc = p.app.doc.as_ref().unwrap().borrow();
            let layout = &doc
                .tree
                .nodes
                .get(p.block())
                .unwrap()
                .text_layout
                .as_ref()
                .unwrap()
                .layout;
            layout.len()
        };
        assert!(lines >= 4, "positive control: 4+ lines");
        p.caret_at(2);
        if scrolled {
            p.scroll_first_paragraph_away();
        }
        let mut seen = vec![drawn_line(&p)];
        for _ in 0..lines {
            key(&mut p.app, KeyCode::ArrowDown, false);
            seen.push(drawn_line(&p));
        }
        let want: Vec<Option<usize>> = (0..lines).map(Some).chain([None]).collect();
        assert_eq!(seen, want, "Down, scrolled: {scrolled}");
        if scrolled {
            p.scroll_first_paragraph_away();
        }
        let mut up = Vec::new();
        for _ in 0..lines {
            key(&mut p.app, KeyCode::ArrowUp, false);
            up.push(drawn_line(&p));
        }
        let want: Vec<Option<usize>> = (0..lines).rev().map(Some).collect();
        assert_eq!(up, want, "Up, scrolled: {scrolled}");
    }
}

/// A 72px span in 24px lines: its line's box starts ABOVE the line before
/// it (`[24,48]` then `[16,103]`), so neither a y probe nor the first or
/// last box around the caret's top names the line. Down used to stay on
/// the span's line for good (review of #1732).
#[test]
fn a_tall_span_in_short_lines_is_one_line_per_press() {
    walks_one_line_per_press(
        "alpha bravo charlie <strong>XY</strong> delta echo foxtrot golf hotel india juliet",
        "24px",
        "[data-pm-editor] strong { font-size: 72px; }",
    );
}

/// 16px text in 10px lines: every line box overlaps the next by half, and
/// a y in the target line's middle named the line after it, so Down skipped
/// every other line (review of #1732).
#[test]
fn overlapping_tight_lines_are_one_line_per_press() {
    walks_one_line_per_press(WORDS, "10px", "");
}

/// The goal column survives a short line and comes back: `x` is alone on
/// its line between three `alpha bravo` lines. Positions are model
/// positions minus one (chars, a break counting one).
#[test]
fn the_goal_column_survives_a_short_line() {
    let first = "alpha bravo<br>x<br>alpha bravo<br>alpha bravo";
    let place = |p: &mut Page| p.caret_at(9); // column 9 of line 1
    let downs = [true, true, true, false, false, false];
    let mut got = Vec::new();
    for n in 1..=downs.len() {
        got.push(same_in_view_and_scrolled(first, place, &downs[..n]).0);
    }
    // 13 = after "x" (the short line's end), 23 and 35 = column 9 of lines
    // 3 and 4, then back up the same way.
    assert_eq!(got, vec![13, 23, 35, 23, 13, 9]);
}
