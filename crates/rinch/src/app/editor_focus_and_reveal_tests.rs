//! `EditorHandle::focus` and `EditorHandle::scroll_into_view` on desktop, with
//! no click: what an app does when it opens a note at a deep link — select the
//! quoted words, bring them on screen, give the editor the keyboard.
//!
//! Everything is driven through `handle_event` with the frame clock the shell
//! runs (`AboutToWait`), never by calling `resolve_and_repaint` directly: the
//! point of both calls is that they work from outside any input event, so the
//! runtime must wake for them by itself. The handle's own rules (which boxes,
//! in which order, what carries or drops a pending reveal) are pinned in
//! `rinch-editor-view`'s `handle::tests::reveal`.

use super::hit_testing::painted_element_box;
use super::*;
use rinch_editor_core::{Pos, Selection};
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

/// Paragraph `i` is `line NNN`: 8 characters, so it spans `1 + 10i` to
/// `9 + 10i`.
fn start_of(i: usize) -> Pos {
    Pos(1 + 10 * i)
}
fn end_of(i: usize) -> Pos {
    Pos(9 + 10 * i)
}

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
    scroller: usize,
    editor: usize,
    /// A `tabindex` box outside the editor that can hold the keyboard.
    other: usize,
}

/// `blocks` paragraphs in a 160px scroller. `editor_is_scroller` makes the
/// editor container the scroller itself (which virtualizes at 60 blocks).
/// Nothing is focused.
fn page_with(blocks: usize, editor_is_scroller: bool) -> Page {
    let ids: Rc<Cell<(usize, usize, usize)>> = Rc::default();
    let ids_in = ids.clone();
    let handle = crate::editor::create_editor();
    let html: String = (0..blocks).map(|i| format!("<p>line {i:03}</p>")).collect();
    assert!(handle.load_html(&html));
    let handle_in = handle.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px");
        let other = scope.create_element("div");
        other.set_attribute("tabindex", "0");
        other.set_attribute("style", "height: 20px");
        root.append_child(&other);
        let editor = handle_in.mount(scope);
        let style = "width: 400px; height: 160px; overflow-y: auto; font-size: 16px; \
                     line-height: 24px; font-family: sans-serif";
        let scroller = if editor_is_scroller {
            editor.set_attribute("style", style);
            root.append_child(&editor);
            editor.node_id().0
        } else {
            let scroller = scope.create_element("div");
            scroller.set_attribute("style", style);
            scroller.append_child(&editor);
            root.append_child(&scroller);
            scroller.node_id().0
        };
        ids_in.set((scroller, editor.node_id().0, other.node_id().0));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (scroller, editor, other) = ids.get();
    assert_eq!(app.focus_target, FocusTarget::None, "nothing is focused");
    Page {
        app,
        handle,
        scroller,
        editor,
        other,
    }
}

fn page() -> Page {
    page_with(40, false)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

/// What the shell does between inputs: run the frame clock a few times.
fn idle(app: &mut RinchApp) {
    for _ in 0..4 {
        ev(app, PlatformEvent::AboutToWait);
    }
}

fn type_char(app: &mut RinchApp, key: KeyCode, text: &str) {
    let modifiers = Modifiers::default();
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: Some(text.to_string()),
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
}

fn scroll_of(page: &Page) -> f64 {
    page.app.doc.as_ref().unwrap().borrow().tree.nodes[page.scroller]
        .scroll_offset
        .1
}

fn set_scroll(page: &mut Page, y: f64) {
    let doc = page.app.doc.as_ref().unwrap();
    let mut d = doc.borrow_mut();
    d.tree.nodes[page.scroller].scroll_offset.1 = y;
    d.tree.dirty_nodes.insert(page.scroller);
}

/// The scroller's painted `(top, bottom)` in window coordinates.
fn scroller_span(page: &Page) -> (f32, f32) {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let (_, y, _, h) = painted_element_box(&d.tree, page.scroller);
    (y, y + h)
}

/// The caret line at `pos`, `(top, bottom)` in window coordinates.
fn line_at(page: &Page, pos: Pos) -> (f32, f32) {
    let r = page
        .handle
        .caret_rect(pos)
        .expect("the position is laid out");
    (r.y, r.y + r.height)
}

fn on_screen(page: &Page, pos: Pos) -> bool {
    let (top, bottom) = scroller_span(page);
    let (a, b) = line_at(page, pos);
    a >= top - 0.5 && b <= bottom + 0.5
}

fn para_text(page: &Page, i: usize) -> String {
    let doc = page.handle.doc();
    let block = doc.child(i);
    (0..block.child_count())
        .filter_map(|j| block.child(j).text().map(str::to_string))
        .collect()
}

fn caret_shown(app: &RinchApp) -> bool {
    let doc = app.doc.as_ref().unwrap();
    let d = doc.borrow();
    d.tree.nodes.iter().any(|(_, n)| {
        n.attributes.contains_key("data-pm-caret")
            && n.attributes
                .get("style")
                .is_some_and(|s| s.contains("visibility: visible"))
    })
}

// ── focus ────────────────────────────────────────────────────────────────

#[test]
fn focus_gives_the_editor_the_keyboard_without_a_click() {
    let mut p = page();
    p.handle.set_selection(Selection::cursor(end_of(0)));

    // Control: with nothing focused, the key reaches no editor.
    type_char(&mut p.app, KeyCode::KeyX, "X");
    idle(&mut p.app);
    assert_eq!(
        para_text(&p, 0),
        "line 000",
        "control: no editor had the keyboard"
    );

    p.handle.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.editor));
    assert!(
        caret_shown(&p.app),
        "the caret is drawn where the selection is"
    );
    type_char(&mut p.app, KeyCode::KeyZ, "Z");
    assert_eq!(para_text(&p, 0), "line 000Z", "the key went to the editor");
}

#[test]
fn focus_from_an_event_handler_lands_before_the_next_key() {
    // The request is drained after the handler that posted it, as
    // `NodeHandle::focus` is: here, the `ReRender` a signal change causes.
    let mut p = page();
    p.handle.set_selection(Selection::cursor(end_of(1)));
    p.handle.focus();
    ev(&mut p.app, PlatformEvent::UserEvent(UserEvent::ReRender));
    type_char(&mut p.app, KeyCode::KeyZ, "Z");
    assert_eq!(para_text(&p, 1), "line 001Z");
}

#[test]
fn focus_takes_the_keyboard_from_another_owner_and_moves_nothing() {
    let mut p = page();
    rinch_core::request_focus(p.app.doc_key(), p.other);
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Node(p.other), "control");

    let range = Selection::text(start_of(30), end_of(30));
    p.handle.set_selection(range.clone());
    idle(&mut p.app);
    p.handle.focus();
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.editor));
    assert_eq!(p.handle.selection(), range, "the selection is where it was");
    assert_eq!(scroll_of(&p), 0.0, "focus scrolls nothing");
}

#[test]
fn a_node_handle_focus_on_the_container_focuses_the_editor_too() {
    let mut p = page();
    rinch_core::request_focus(p.app.doc_key(), p.editor);
    idle(&mut p.app);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.editor));
}

// ── scroll_into_view ─────────────────────────────────────────────────────

#[test]
fn a_far_range_comes_on_screen_without_focus_or_a_click() {
    let mut p = page();
    let (from, to) = (start_of(30), end_of(31));
    p.handle.set_selection(Selection::text(from, to));
    idle(&mut p.app);
    assert_eq!(
        scroll_of(&p),
        0.0,
        "control: selecting a range scrolls nothing"
    );
    assert!(!on_screen(&p, from), "control: the range starts off screen");

    p.handle.scroll_into_view(from, to);
    assert!(
        p.app.has_pending_layout(),
        "the request wakes the runtime with nothing dirty"
    );
    idle(&mut p.app);
    assert!(on_screen(&p, from), "the start is on screen");
    assert!(on_screen(&p, to), "and the end, since both fit");
    let (_, bottom) = scroller_span(&p);
    let (_, end_bottom) = line_at(&p, to);
    assert!(
        bottom - end_bottom >= 15.0,
        "with the margin below it (got {})",
        bottom - end_bottom
    );
    assert_eq!(p.app.focus_target, FocusTarget::None, "focus untouched");
    assert!(!p.app.has_pending_layout(), "and the runtime goes idle");
}

#[test]
fn a_caret_position_comes_on_screen() {
    let mut p = page();
    p.handle.scroll_into_view(start_of(35), start_of(35));
    idle(&mut p.app);
    assert!(scroll_of(&p) > 0.0);
    assert!(on_screen(&p, start_of(35)));
}

#[test]
fn a_range_above_the_view_comes_in_at_the_top_with_the_margin() {
    let mut p = page();
    set_scroll(&mut p, 700.0);
    idle(&mut p.app);
    assert!(!on_screen(&p, start_of(3)), "control: scrolled past it");
    p.handle.scroll_into_view(start_of(3), end_of(3));
    idle(&mut p.app);
    let (top, _) = scroller_span(&p);
    let (line_top, _) = line_at(&p, start_of(3));
    assert!(on_screen(&p, start_of(3)));
    assert!(
        (line_top - top - 16.0).abs() <= 1.5,
        "the start sits 16px under the top edge (got {})",
        line_top - top
    );
}

#[test]
fn a_range_taller_than_the_view_shows_its_start_from_either_side() {
    // Ten lines of 24px in a 160px scroller: the start wins.
    let (from, to) = (start_of(20), end_of(29));
    let mut p = page();
    p.handle.scroll_into_view(from, to);
    idle(&mut p.app);
    assert!(on_screen(&p, from), "from above: the start is shown");
    assert!(!on_screen(&p, to), "control: the range does not fit");

    set_scroll(&mut p, 5000.0);
    idle(&mut p.app);
    assert!(!on_screen(&p, from), "control: scrolled past the range");
    p.handle.scroll_into_view(from, to);
    idle(&mut p.app);
    assert!(on_screen(&p, from), "from below: the start is shown");
}

#[test]
fn a_range_already_in_view_moves_nothing() {
    let mut p = page();
    p.handle.scroll_into_view(start_of(1), end_of(2));
    idle(&mut p.app);
    assert_eq!(scroll_of(&p), 0.0);
}

#[test]
fn a_virtualized_editor_lays_out_the_block_to_reveal() {
    let mut p = page_with(150, true);
    p.handle.scroll_into_view(start_of(120), end_of(120));
    idle(&mut p.app);
    assert!(scroll_of(&p) > 0.0, "it scrolled");
    assert!(on_screen(&p, start_of(120)), "to the block");
}

/// The whole deep-link sequence: select the words, reveal them, focus, type.
#[test]
fn select_reveal_focus_then_typing_replaces_the_words() {
    let mut p = page();
    let (from, to) = (Pos(start_of(33).0 + 5), end_of(33));
    p.handle.set_selection(Selection::text(from, to));
    p.handle.scroll_into_view(from, to);
    p.handle.focus();
    idle(&mut p.app);
    assert!(on_screen(&p, from));
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.editor));
    type_char(&mut p.app, KeyCode::KeyZ, "Z");
    assert_eq!(para_text(&p, 33), "line Z");
}

// ── scroll_into_view_aligned ─────────────────────────────────────────────

use crate::editor::ScrollAlign;

const THIRD: ScrollAlign = ScrollAlign::Fraction(1.0 / 3.0);

/// How far below the scroller's top edge the caret line at `pos` sits.
fn offset_in_view(page: &Page, pos: Pos) -> f32 {
    let (top, _) = scroller_span(page);
    let (line_top, _) = line_at(page, pos);
    line_top - top
}

fn max_scroll(page: &Page) -> f64 {
    let doc = page.app.doc.as_ref().unwrap();
    let d = doc.borrow();
    let id = rinch_core::dom::NodeId(page.scroller);
    d.scroll_height(id) - d.client_height(id)
}

#[test]
fn a_far_range_lands_a_third_of_the_way_down() {
    let mut p = page();
    let (from, to) = (start_of(30), end_of(31));
    p.handle.set_selection(Selection::text(from, to));
    idle(&mut p.app);
    assert!(!on_screen(&p, from), "control: the range starts off screen");

    p.handle.scroll_into_view_aligned(from, to, THIRD);
    assert!(
        p.app.has_pending_layout(),
        "the request wakes the runtime with nothing dirty"
    );
    idle(&mut p.app);
    let at = offset_in_view(&p, from);
    assert!(
        (at - 160.0 / 3.0).abs() <= 2.0,
        "the start sits a third of the way down the 160px view (got {at})"
    );
    assert!(on_screen(&p, to), "and the end, which fits below it");
    assert_eq!(p.app.focus_target, FocusTarget::None, "focus untouched");
    assert!(!p.app.has_pending_layout(), "and the runtime goes idle");
}

#[test]
fn a_placed_range_above_the_view_lands_at_the_same_place() {
    let mut p = page();
    set_scroll(&mut p, 900.0);
    idle(&mut p.app);
    assert!(!on_screen(&p, start_of(20)), "control: scrolled past it");
    p.handle
        .scroll_into_view_aligned(start_of(20), end_of(20), THIRD);
    idle(&mut p.app);
    let at = offset_in_view(&p, start_of(20));
    assert!((at - 160.0 / 3.0).abs() <= 2.0, "got {at}");
}

#[test]
fn a_range_near_the_top_of_the_document_stays_near_the_top() {
    let mut p = page();
    set_scroll(&mut p, 700.0);
    idle(&mut p.app);
    // Paragraph 0's line is 17px down the content: a third of the way down
    // the view would need a negative scroll.
    p.handle
        .scroll_into_view_aligned(start_of(0), end_of(0), THIRD);
    idle(&mut p.app);
    assert_eq!(scroll_of(&p), 0.0, "clamped to the top");
    assert!(on_screen(&p, start_of(0)));
}

#[test]
fn a_range_near_the_end_of_the_document_stops_at_the_bottom() {
    let mut p = page();
    p.handle
        .scroll_into_view_aligned(start_of(39), end_of(39), THIRD);
    idle(&mut p.app);
    assert!(
        (scroll_of(&p) - max_scroll(&p)).abs() <= 0.5,
        "clamped to the end: {} of {}",
        scroll_of(&p),
        max_scroll(&p)
    );
    assert!(on_screen(&p, start_of(39)));
    assert!(offset_in_view(&p, start_of(39)) > 160.0 / 3.0);
}

#[test]
fn a_placed_range_moves_even_when_it_is_in_view() {
    let mut p = page();
    // Paragraph 2's line is 94px down the content, on screen at scroll 0.
    assert!(on_screen(&p, start_of(2)), "control");
    p.handle
        .scroll_into_view_aligned(start_of(2), end_of(2), ScrollAlign::Fraction(0.0));
    idle(&mut p.app);
    let at = offset_in_view(&p, start_of(2));
    assert!(
        (at - 16.0).abs() <= 1.5,
        "Fraction(0.0) is the top edge plus the margin (got {at})"
    );
}

#[test]
fn a_range_taller_than_the_space_below_still_puts_its_start_there() {
    let mut p = page();
    let (from, to) = (start_of(20), end_of(29));
    p.handle.scroll_into_view_aligned(from, to, THIRD);
    idle(&mut p.app);
    let at = offset_in_view(&p, from);
    assert!((at - 160.0 / 3.0).abs() <= 2.0, "got {at}");
    assert!(!on_screen(&p, to), "control: the rest runs off the bottom");
}

#[test]
fn nearest_is_scroll_into_view() {
    let mut p = page();
    p.handle
        .scroll_into_view_aligned(start_of(1), end_of(2), ScrollAlign::Nearest);
    idle(&mut p.app);
    assert_eq!(scroll_of(&p), 0.0, "a range in view moves nothing");

    p.handle
        .scroll_into_view_aligned(start_of(30), end_of(31), ScrollAlign::Nearest);
    idle(&mut p.app);
    let (_, bottom) = scroller_span(&p);
    let (_, end_bottom) = line_at(&p, end_of(31));
    assert!(
        (bottom - end_bottom - 16.0).abs() <= 1.5,
        "the end comes in at the bottom with the margin (got {})",
        bottom - end_bottom
    );
}

#[test]
fn a_placed_reveal_waits_for_a_virtualized_block_to_be_laid_out() {
    let mut p = page_with(150, true);
    p.handle
        .scroll_into_view_aligned(start_of(120), end_of(120), THIRD);
    idle(&mut p.app);
    assert!(on_screen(&p, start_of(120)), "to the block");
    // Not exact: the blocks the scroll brings into the window were laid out
    // at their estimated heights when the scroll was computed, and settle to
    // their measured ones after it, moving the start by a fraction of a line.
    let at = offset_in_view(&p, start_of(120));
    assert!(
        (at - 160.0 / 3.0).abs() <= 24.0,
        "about a third down (got {at})"
    );
}
