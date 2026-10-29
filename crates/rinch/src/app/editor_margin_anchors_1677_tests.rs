//! Margin anchors and the `Editor` overlay slot on the desktop (#1677), driven
//! the way the shell drives a frame: an `AboutToWait` turn, then a redraw when
//! one is asked for.
//!
//! The oracle for "where is the line" is the painted box of a one-line
//! paragraph (a declared `line-height: 24px` in the bundled Inter, so the line
//! box is a declaration and not the host's fonts), and for "in which frame" it
//! is a child of the overlay slot placed at the reported `top`: it must paint
//! level with the line. The editor keeps the default stylesheet's 1px border
//! and 16px padding, so a frame that forgot either is off.

use super::hit_testing::painted_element_box;
use super::*;
use crate::editor::{MarginAnchor, MarginLine};
use rinch_core::Component;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Pos, Selection, Slice};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// Wraps to three lines in the 300px editor's 260px column.
const LONG: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    /// Every report the callback heard.
    reports: Rc<RefCell<Vec<Vec<MarginLine>>>>,
    /// Presses the slot's button heard.
    clicks: Rc<Cell<usize>>,
}

/// `<p>top</p><p>LONG</p><p>end</p>` in an `Editor` whose overlay slot holds a
/// marker (placed by the callback at the line of the anchor keyed `mark`) and
/// a button. `editor_style` is added to the editor's own style.
fn page(editor_style: &str) -> Page {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html(&format!("<p>top</p><p>{LONG}</p><p>end</p>")));
    let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
    let clicks: Rc<Cell<usize>> = Rc::default();
    let (handle_in, reports_in, clicks_in) = (handle.clone(), reports.clone(), clicks.clone());
    let editor_style = editor_style.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        // A picture is as tall as this says, loaded or not.
        let sheet = scope.create_element("style");
        sheet.append_child(
            &scope.create_text("[data-pm-editor] img { width: 12px; height: 60px; }"),
        );
        root.append_child(&sheet);
        let marker = scope.create_element("div");
        marker.set_attribute("data-marker", "");
        marker.set_attribute(
            "style",
            "position: absolute; left: 0; top: 0; width: 6px; height: 6px",
        );
        let button = scope.create_element("button");
        button.set_attribute(
            "style",
            "position: absolute; left: 260px; top: 40px; width: 30px; height: 20px",
        );
        let c = clicks_in.clone();
        let rid = scope.register_handler(move || c.set(c.get() + 1));
        button.set_attribute("data-rid", &rid.0.to_string());
        let (m, r) = (marker.clone(), reports_in.clone());
        handle_in.on_margin_layout(move |lines: &[MarginLine]| {
            if let Some(line) = lines.iter().find(|l| l.key == "mark") {
                m.set_style("top", &format!("{}px", line.top));
            }
            r.borrow_mut().push(lines.to_vec());
        });
        let editor = crate::editor::Editor {
            editor: Some(handle_in.clone()),
            ..Default::default()
        }
        .render(scope, &[marker, button]);
        editor.set_attribute(
            "style",
            &format!(
                "width: 300px; font-size: 16px; line-height: 24px; font-family: sans-serif; \
                 {editor_style}"
            ),
        );
        root.append_child(&editor);
        // Something else on the page, laid out with it.
        let sibling = scope.create_element("div");
        sibling.set_attribute("data-sibling", "");
        sibling.set_attribute("style", "width: 10px; height: 10px");
        root.append_child(&sibling);
        root
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let mut page = Page {
        app,
        handle,
        reports,
        clicks,
    };
    idle(&mut page.app);
    page
}

/// The desktop loop between inputs.
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

fn boxes(app: &RinchApp, selector: &str) -> Vec<(f32, f32, f32, f32)> {
    let doc = app.doc.as_ref().unwrap().borrow();
    doc.query_selector_all(selector)
        .into_iter()
        .map(|id| painted_element_box(&doc.tree, id.0))
        .collect()
}

/// The frame the lines are reported in: the overlay slot's origin.
fn frame(app: &RinchApp) -> (f32, f32) {
    let b = boxes(app, "[data-pm-overlay]")[0];
    (b.0, b.1)
}

/// Paragraph `i`'s painted box.
fn para(app: &RinchApp, i: usize) -> (f32, f32, f32, f32) {
    boxes(app, "p")[i]
}

/// The end of `LONG`'s paragraph (positions: `top` 1..4, `LONG` from 6).
fn long_end() -> Pos {
    Pos(6 + LONG.chars().count())
}

fn last(page: &Page) -> Vec<MarginLine> {
    page.reports.borrow().last().cloned().expect("a report")
}

fn line(lines: &[MarginLine], key: &str) -> (f32, f32) {
    let l = lines
        .iter()
        .find(|l| l.key == key)
        .unwrap_or_else(|| panic!("{key} reported: {lines:?}"));
    (l.top, l.height)
}

fn anchors() -> Vec<MarginAnchor> {
    vec![
        MarginAnchor::at("a", Pos(2)),
        MarginAnchor::at("b", Pos(6)),
        MarginAnchor::at("mark", long_end()),
        MarginAnchor::at("d", Pos(long_end().0 - 1)),
        // Between `top` and `LONG`: the nearest text after it.
        MarginAnchor::at("e", Pos(5)),
    ]
}

/// Each anchor reports the line box it sits in, in the slot's frame, and a
/// child of the slot placed at that `top` paints level with the line. Two
/// anchors on one line report one box.
#[test]
fn each_anchor_reports_its_line_and_a_slot_child_there_paints_level_with_it() {
    let mut page = page("");
    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    let lines = last(&page);
    assert_eq!(
        lines.iter().map(|l| l.key.as_str()).collect::<Vec<_>>(),
        ["a", "b", "mark", "d", "e"],
        "every anchor, in the order given"
    );
    let (_, fy) = frame(&page.app);
    let (p0, p1) = (para(&page.app, 0), para(&page.app, 1));
    assert!(p1.3 >= 72.0, "precondition: LONG wraps, {p1:?}");
    assert_eq!(line(&lines, "a"), (p0.1 - fy, 24.0));
    assert_eq!(line(&lines, "b"), (p1.1 - fy, 24.0));
    assert_eq!(line(&lines, "mark"), (p1.1 + p1.3 - 24.0 - fy, 24.0));
    assert_eq!(line(&lines, "d"), line(&lines, "mark"), "one line, one box");
    assert_eq!(line(&lines, "e"), line(&lines, "b"));
    // The frame end to end: the marker the callback placed is on the line.
    let marker = boxes(&page.app, "[data-marker]")[0];
    assert_eq!(
        marker.1,
        p1.1 + p1.3 - 24.0,
        "the marker is level with its line"
    );
    // The column's right edge: the blocks' border box, 260px wide.
    assert_eq!(
        page.handle.margin_text_right(),
        Some(p1.0 + p1.2 - frame(&page.app).0)
    );
}

/// An edit above the anchors moves their lines and reports it; the anchors
/// keep their text. A selection change moves no line and reports nothing.
#[test]
fn an_edit_above_reports_the_moved_lines_and_a_selection_change_reports_nothing() {
    let mut page = page("");
    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    let before = page.reports.borrow().len();
    let (_, b_top) = (0, line(&last(&page), "b").0);

    page.handle.set_selection(Selection::cursor(Pos(3)));
    idle(&mut page.app);
    assert_eq!(page.reports.borrow().len(), before, "no line moved");

    // A new first paragraph.
    assert!(page.handle.update(|state| {
        let mut tr = state.tr();
        let s = state.schema();
        let p = s
            .branch("paragraph", Fragment::from_node(s.text("new").ok()?))
            .ok()?;
        tr.replace(0, 0, Slice::from_fragment(Fragment::from_node(p)))
            .ok()?;
        Some(tr)
    }));
    idle(&mut page.app);
    assert_eq!(page.reports.borrow().len(), before + 1, "one report");
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let p2 = para(&page.app, 2);
    assert_eq!(line(&lines, "b").0, p2.1 - fy, "b still starts LONG");
    assert!(line(&lines, "b").0 > b_top);
    let marker = boxes(&page.app, "[data-marker]")[0];
    assert_eq!(marker.1, p2.1 + p2.3 - 24.0, "the marker followed");
}

/// The lines are in the content's frame: scrolling the editor (here the
/// scroller itself) reports nothing, and the slot child scrolls with its line.
#[test]
fn scrolling_reports_nothing_and_the_slot_scrolls_with_the_text() {
    let mut page = page("height: 90px; overflow: auto");
    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    let before = page.reports.borrow().len();
    let container = {
        let doc = page.app.doc.as_ref().unwrap().borrow();
        doc.query_selector_all("[data-pm-editor]")[0]
    };
    let line_y = |app: &RinchApp| {
        let p1 = para(app, 1);
        p1.1 + p1.3 - 24.0
    };
    let was = line_y(&page.app);
    {
        let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
        doc.set_scroll_top(container, 30.0);
    }
    page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    idle(&mut page.app);
    assert_eq!(line_y(&page.app), was - 30.0, "precondition: it scrolled");
    assert_eq!(
        page.reports.borrow().len(),
        before,
        "a scroll moves no line"
    );
    let marker = boxes(&page.app, "[data-marker]")[0];
    assert_eq!(
        marker.1,
        line_y(&page.app),
        "the marker scrolled with the text"
    );
}

/// No anchors, no measuring — a callback alone costs nothing — and `N`
/// anchors are one pass of `N` measures per layout.
#[test]
fn no_anchors_measure_nothing_and_n_anchors_measure_once_per_layout() {
    let mut page = page("");
    let (p0, m0) = crate::editor::__margin_counters();
    for i in 0..3 {
        page.handle.set_selection(Selection::cursor(Pos(2)));
        assert!(page.handle.insert_text(&i.to_string()));
        idle(&mut page.app);
    }
    assert_eq!(
        crate::editor::__margin_counters(),
        (p0, m0),
        "nothing measured"
    );

    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    let (p1, m1) = crate::editor::__margin_counters();
    assert_eq!((p1 - p0, m1 - m0), (1, 5), "one pass for the new set");
    assert!(page.handle.insert_text("x"));
    idle(&mut page.app);
    let (p2, m2) = crate::editor::__margin_counters();
    assert_eq!((p2 - p1, m2 - m1), (1, 5), "one pass for the edit's layout");

    // Cleared: one empty report, then nothing again.
    page.handle.set_margin_anchors(Vec::new());
    idle(&mut page.app);
    assert_eq!(last(&page), Vec::<MarginLine>::new());
    let (p3, m3) = crate::editor::__margin_counters();
    assert!(page.handle.insert_text("y"));
    idle(&mut page.app);
    assert_eq!(crate::editor::__margin_counters(), (p3, m3));
}

/// A press on a child of the overlay slot is the child's: its handler runs
/// and the editor's selection does not move.
#[test]
fn a_press_on_a_slot_child_is_the_childs() {
    let mut page = page("");
    page.handle.set_selection(Selection::cursor(Pos(2)));
    idle(&mut page.app);
    let b = boxes(&page.app, "button")[0];
    let (x, y) = (b.0 + b.2 / 2.0, b.1 + b.3 / 2.0);
    let button = MouseButton::Left;
    page.app
        .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    page.app
        .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    idle(&mut page.app);
    assert_eq!(page.clicks.get(), 1, "the button's handler ran");
    assert_eq!(page.handle.selection(), Selection::cursor(Pos(2)));
}

/// Anchors are view state: a read-only editor reports them, and setting them
/// changes neither the document nor its history.
#[test]
fn anchors_report_on_a_read_only_editor_and_change_no_document() {
    let mut page = page("");
    page.handle.set_read_only(true);
    let doc = page.handle.doc();
    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    assert_eq!(last(&page).len(), 5);
    assert!(page.handle.doc().same_ref(&doc), "the document is the same");
    page.handle.set_read_only(false);
    assert!(!page.handle.can_run("undo"), "nothing to undo");
}

/// An anchor in a table cell reports that cell's line.
#[test]
fn an_anchor_in_a_table_cell_reports_its_line() {
    let mut page = page("");
    assert!(
        page.handle
            .load_html("<p>top</p><table><tr><td><p>one</p></td><td><p>two</p></td></tr></table>")
    );
    idle(&mut page.app);
    // `top` 0..5; table 5, row 6, cell 7, its paragraph 8 (text 9..12);
    // the second cell 13, its paragraph 14 (text 15..).
    page.handle.set_margin_anchors(vec![
        MarginAnchor::at("one", Pos(10)),
        MarginAnchor::at("two", Pos(16)),
    ]);
    idle(&mut page.app);
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let ps = boxes(&page.app, "p");
    assert_eq!(ps.len(), 3, "top and the two cells' paragraphs");
    assert_eq!(line(&lines, "one"), (ps[1].1 - fy, 24.0));
    assert_eq!(line(&lines, "two"), (ps[2].1 - fy, 24.0));
}

/// Typing in an anchor's line that leaves the line where it is reports
/// nothing — in particular no report from the pass a key runs before its
/// layout, where the edited paragraph has no geometry yet and would be left
/// out, only to come back after the layout.
#[test]
fn typed_text_that_moves_no_line_reports_nothing() {
    let mut page = page("");
    page.handle.set_margin_anchors(anchors());
    page.handle.set_selection(Selection::cursor(Pos(4)));
    page.handle.focus();
    idle(&mut page.app);
    assert!(
        matches!(page.app.focus_target, FocusTarget::Editor(_)),
        "precondition: the editor holds the keyboard"
    );
    let before = page.reports.borrow().len();
    for _ in 0..3 {
        let modifiers = Modifiers::default();
        page.app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::KeyX,
                logical_key: None,
                text: Some("x".into()),
                modifiers,
                repeat: KeyRepeat::Fresh,
            },
            VP,
            1.0,
        );
        page.app.handle_event(
            PlatformEvent::KeyUp {
                key: KeyCode::KeyX,
                logical_key: None,
                modifiers,
            },
            VP,
            1.0,
        );
        idle(&mut page.app);
    }
    let doc = page.handle.doc();
    assert_eq!(doc.child(0).child(0).text(), Some("topxxx"), "typed");
    assert_eq!(page.reports.borrow().len(), before, "no line moved");
}

/// A line holding text in two sizes (a `code` run beside plain text) is one
/// line box, the line's own: both anchors on it report it.
#[test]
fn a_line_of_mixed_sizes_reports_its_line_box() {
    let mut page = page("");
    assert!(
        page.handle
            .load_html("<p>top</p><p>plain <code>code</code> more</p>")
    );
    idle(&mut page.app);
    // `plain ` 6..12, `code` 12..16.
    page.handle.set_margin_anchors(vec![
        MarginAnchor::at("plain", Pos(8)),
        MarginAnchor::at("code", Pos(14)),
    ]);
    idle(&mut page.app);
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let p1 = para(&page.app, 1);
    // One line, taller than the paragraph's 24px: since parley 0.12 the
    // `code` run's box (0.9em of the host's monospace, its own half-leading)
    // is on the line beside the text's, and they do not share a baseline.
    // Chrome 156, the same styles with the bundled Inter: 27 (rinch 24 until
    // the upgrade). The host's monospace is in it, so a range.
    assert!(
        (24.0..=28.0).contains(&p1.3),
        "precondition: one line, got {}",
        p1.3
    );
    assert_eq!(line(&lines, "plain"), (p1.1 - fy, p1.3));
    assert_eq!(line(&lines, "code"), (p1.1 - fy, p1.3));
}

/// An anchor right beside a picture taller than the text reports the line the
/// picture sits on — as tall as the caret drawn in its text — not the
/// picture's own box.
#[test]
fn an_anchor_beside_a_tall_picture_reports_its_line() {
    let mut page = page("");
    assert!(
        page.handle
            .load_html(r#"<p>top</p><p><img src="missing.png">xyz</p>"#)
    );
    idle(&mut page.app);
    // The picture is 6..7, `xyz` 7..10. Alone, so no other anchor's box on
    // the line is merged with it.
    page.handle
        .set_margin_anchors(vec![MarginAnchor::at("before", Pos(6))]);
    idle(&mut page.app);
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let img = boxes(&page.app, "img")[0];
    let caret = page.handle.caret_rect(Pos(8)).expect("the text's caret");
    assert!(
        img.3 >= 60.0 && caret.height > img.3,
        "precondition: the line is taller than the picture, {img:?} {caret:?}"
    );
    assert_eq!(
        line(&lines, "before"),
        (caret.y - fy, caret.height),
        "the line, not the picture"
    );
}

/// An anchor in a subscript reports the line it is on, where the caret in
/// the plain text beside it is drawn, not a box moved down with its glyphs.
#[test]
fn an_anchor_in_a_subscript_reports_its_line() {
    let mut page = page("");
    assert!(
        page.handle
            .load_html("<p>top</p><p>plain<sub>low</sub></p>")
    );
    idle(&mut page.app);
    // `plain` 6..11, `low` 11..14.
    page.handle
        .set_margin_anchors(vec![MarginAnchor::at("low", Pos(13))]);
    idle(&mut page.app);
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let plain = page.handle.caret_rect(Pos(8)).expect("the text's caret");
    let low = page
        .handle
        .caret_rect(Pos(13))
        .expect("the subscript's caret");
    assert!(
        low.y > plain.y,
        "precondition: the subscript's caret is lower"
    );
    assert_eq!(line(&lines, "low"), (plain.y - fy, plain.height));
}

/// Two anchors on adjacent lines of one paragraph report two boxes: the
/// one-line merge does not reach the next line (review of #1683, M3).
#[test]
fn adjacent_lines_in_one_paragraph_report_two_boxes() {
    let mut page = page("");
    let first = page.handle.caret_rect(Pos(7)).expect("a caret").y;
    let second = (8..long_end().0)
        .map(Pos)
        .find(|&q| page.handle.caret_rect(q).expect("a caret").y > first + 1.0)
        .expect("a second line");
    let second = Pos(second.0 + 1);
    page.handle.set_margin_anchors(vec![
        MarginAnchor::at("l1", Pos(7)),
        MarginAnchor::at("l2", second),
    ]);
    idle(&mut page.app);
    let lines = last(&page);
    let (_, fy) = frame(&page.app);
    let p1 = para(&page.app, 1);
    assert_eq!(line(&lines, "l1"), (p1.1 - fy, 24.0));
    assert_eq!(line(&lines, "l2"), (p1.1 + 24.0 - fy, 24.0));
}

/// An anchor in a padded textblock (a code block) reports the line its text
/// is on, below the block's padding and border (review of #1683, M6).
#[test]
fn an_anchor_in_a_padded_code_block_reports_its_line() {
    let mut page = page("");
    assert!(
        page.handle
            .load_html("<p>top</p><pre><code>code line</code></pre>")
    );
    idle(&mut page.app);
    page.handle
        .set_margin_anchors(vec![MarginAnchor::at("code", Pos(8))]);
    idle(&mut page.app);
    let (_, fy) = frame(&page.app);
    let caret = page.handle.caret_rect(Pos(8)).expect("a caret");
    let pre = boxes(&page.app, "pre")[0];
    assert!(
        caret.y > pre.1 + 4.0,
        "precondition: padded, {pre:?} {caret:?}"
    );
    let (top, height) = line(&last(&page), "code");
    assert!(
        (top - (caret.y - fy)).abs() < 0.01 && (height - caret.height).abs() < 0.01,
        "({top}, {height}) vs the caret {caret:?} in frame {fy}"
    );
}

/// A layout of the page that moves nothing in the editor measures nothing;
/// one that rewraps its text (the editor's own style) measures and reports.
#[test]
fn an_unrelated_layout_measures_nothing_and_a_rewrap_reports() {
    let mut page = page("");
    page.handle.set_margin_anchors(anchors());
    idle(&mut page.app);
    let (sibling, editor, long_p) = {
        let doc = page.app.doc.as_ref().unwrap().borrow();
        (
            doc.query_selector_all("[data-sibling]")[0],
            doc.query_selector_all("[data-pm-editor]")[0],
            doc.query_selector_all("p")[1],
        )
    };
    // `LONG` in a box of its own fixed height: its lines can move with no
    // block's box moving.
    {
        let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
        doc.set_style(long_p, "height", "200px");
    }
    page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    idle(&mut page.app);
    let counted = crate::editor::__margin_counters();
    let reports = page.reports.borrow().len();
    for w in 11..21 {
        let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
        doc.set_style(sibling, "width", &format!("{w}px"));
        drop(doc);
        page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        idle(&mut page.app);
    }
    assert_eq!(
        crate::editor::__margin_counters(),
        counted,
        "ten layouts of the page, no measure"
    );
    {
        let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
        doc.set_style(editor, "letter-spacing", "3px");
    }
    page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    idle(&mut page.app);
    assert_eq!(
        page.reports.borrow().len(),
        reports + 1,
        "the rewrap reports"
    );
    let (_, fy) = frame(&page.app);
    let caret = page.handle.caret_rect(long_end()).expect("a caret");
    assert_eq!(line(&last(&page), "mark"), (caret.y - fy, 24.0));
}

/// Anchors and a callback set before the editor mounts report once it does.
#[test]
fn anchors_set_before_the_mount_report_after_it() {
    let handle = crate::editor::create_editor();
    assert!(handle.load_html("<p>top</p><p>two</p>"));
    let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
    let r = reports.clone();
    handle.on_margin_layout(move |lines: &[MarginLine]| r.borrow_mut().push(lines.to_vec()));
    handle.set_margin_anchors(vec![MarginAnchor::at("a", Pos(7))]);
    let h = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        crate::editor::Editor {
            editor: Some(h.clone()),
            ..Default::default()
        }
        .render(scope, &[])
    });
    app.register_app_font(AppFont::sans_serif(INTER));
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    idle(&mut app);
    let reports = reports.borrow();
    let last = reports.last().expect("a report after the mount");
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].key, "a");
}

/// Review round 2 of #1683: the margin pass's gate.
mod review2_probes {
    use super::*;

    const TABLE: &str =
        "<table><tr><td><p>a1</p><p>a2</p><p>a3</p></td><td><p>bb</p></td></tr></table>";
    /// Inside `bb` (table 0, row 1, cell 2, p 3, `a1` 4..6, `a2` 8..10,
    /// `a3` 12..14, cell B opens at 16, its p at 17, `bb` 18..20).
    const IN_B: Pos = Pos(19);

    fn add_sheet(page: &mut Page, css: &str) {
        {
            let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
            let body = doc.query_selector_all("[data-sibling]")[0];
            let style = doc.create_element("style");
            let text = doc.create_text(css);
            doc.append_child(style, text);
            let parent = doc.parent_node(body).unwrap();
            doc.insert_before(parent, style, body);
        }
        page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        idle(&mut page.app);
    }

    fn caret_line(page: &Page, at: Pos) -> f32 {
        let (_, fy) = frame(&page.app);
        page.handle.caret_rect(at).expect("a caret").y - fy
    }

    /// A line in a table cell moves (a class on the editor gives the short
    /// cell some top padding; the row's height is the tall cell's) with no
    /// text rebuilt and no top-level block's box moving: the anchored block's
    /// own offset moved, and the report follows it (review 2 of #1683, G1).
    #[test]
    fn r2_a_cell_padding_change_moves_a_line_the_gate_does_not_see() {
        let mut page = page("");
        assert!(page.handle.load_html(TABLE));
        idle(&mut page.app);
        add_sheet(
            &mut page,
            "[data-pm-editor][data-x] td + td { padding-top: 30px; }",
        );
        page.handle
            .set_margin_anchors(vec![MarginAnchor::at("b", IN_B)]);
        idle(&mut page.app);
        let before = caret_line(&page, IN_B);
        assert!(
            (line(&last(&page), "b").0 - before).abs() < 0.01,
            "baseline: {:?} vs caret {before}",
            last(&page)
        );
        let table_before = boxes(&page.app, "table")[0];
        let (passes0, measures0) = crate::editor::__margin_counters();
        let n0 = page.reports.borrow().len();
        {
            let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
            let ed = doc.query_selector_all("[data-pm-editor]")[0];
            doc.set_attribute(ed, "data-x", "");
        }
        page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        idle(&mut page.app);
        let after = caret_line(&page, IN_B);
        let table_after = boxes(&page.app, "table")[0];
        let (passes1, measures1) = crate::editor::__margin_counters();
        eprintln!(
            "R2-A caret {before} -> {after}; table {table_before:?} -> {table_after:?}; \
             passes {passes0}->{passes1} measures {measures0}->{measures1}; reports {n0}->{}; last {:?}",
            page.reports.borrow().len(),
            last(&page)
        );
        assert!(after > before + 20.0, "precondition: the line moved");
        assert_eq!(table_before, table_after, "precondition: the table did not");
        let reported = line(&last(&page), "b").0;
        // Positive control: a forced measure finds the moved line.
        page.handle
            .set_margin_anchors(vec![MarginAnchor::at("b", IN_B)]);
        idle(&mut page.app);
        let forced = line(&last(&page), "b").0;
        eprintln!("R2-A reported {reported}, forced {forced}, caret {after}");
        assert!((forced - after).abs() < 0.01, "control: a measure finds it");
        assert!(
            (reported - after).abs() < 0.01,
            "STALE: reported {reported}, the line is at {after}"
        );
    }

    /// A text edit elsewhere on the page (a status label) rebuilds no anchored
    /// block's text and moves none: nothing is measured (review 2 of #1683, G2).
    #[test]
    fn r2_a_text_edit_outside_the_editor_measures() {
        let mut page = page("");
        page.handle.set_margin_anchors(anchors());
        idle(&mut page.app);
        let sibling = {
            let doc = page.app.doc.as_ref().unwrap().borrow();
            doc.query_selector_all("[data-sibling]")[0]
        };
        {
            let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
            doc.set_style(sibling, "width", "100px");
            doc.set_text_content(sibling, "words: 0");
        }
        page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
        idle(&mut page.app);
        let c0 = crate::editor::__margin_counters();
        for i in 1..11 {
            let mut doc = page.app.doc.as_ref().unwrap().borrow_mut();
            doc.set_text_content(sibling, &format!("words: {i}"));
            drop(doc);
            page.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
            idle(&mut page.app);
        }
        let c1 = crate::editor::__margin_counters();
        assert_eq!(c1, c0, "a label's text elsewhere measures no anchor");
    }
}
