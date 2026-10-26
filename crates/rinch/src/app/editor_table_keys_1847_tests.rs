//! #1847, review of #1931: Shift+arrow and Shift+click across table cells on
//! the desktop input path (key events through `move_editor`, which asks
//! `EditorHandle::shift_arrow_cells`; a press with Shift held).
//!
//! Positions: `p(x)` 0..3; the table's cells at 5 ("ab", text 7..9), 11
//! ("cd", 13..15), 19 ("ef", 21..23), 25 ("gh", 27..29); `p(zw)` 33..37.

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

const GRID: &str = "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
                    <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table><p>zw</p>";

struct Page {
    app: RinchApp,
    handle: crate::editor::EditorHandle,
}

fn page() -> Page {
    page_of(GRID)
}

fn page_of(html: &'static str) -> Page {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(html);
        container.set_attribute(
            "style",
            "width: 400px; font-size: 16px; line-height: 24px; font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    Page { app, handle }
}

fn ev(app: &mut RinchApp, event: PlatformEvent) {
    app.handle_event(event, VP, 1.0);
}

fn press(p: &mut Page, pos: usize) {
    let (x, y, h) = p
        .app
        .editor_caret_point(&p.handle, Pos(pos))
        .expect("a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    ev(&mut p.app, PlatformEvent::MouseDown { x, y, button });
    ev(&mut p.app, PlatformEvent::MouseUp { x, y, button });
    p.app.resolve_and_repaint(800.0, 600.0);
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Modifiers::default()
    }
}

fn shift_key(p: &mut Page, key: KeyCode) {
    let modifiers = shift();
    ev(
        &mut p.app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: None,
            modifiers,
            repeat: KeyRepeat::Fresh,
        },
    );
    ev(
        &mut p.app,
        PlatformEvent::KeyUp {
            key,
            logical_key: None,
            modifiers,
        },
    );
    p.app.resolve_and_repaint(800.0, 600.0);
}

#[test]
fn shift_arrows_select_cells_on_the_desktop() {
    let mut p = page();
    press(&mut p, 8);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(8)));
    shift_key(&mut p, KeyCode::ArrowRight);
    assert_eq!(p.handle.selection(), Selection::text(Pos(8), Pos(9)));
    shift_key(&mut p, KeyCode::ArrowRight);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(5), Pos(11)));
    shift_key(&mut p, KeyCode::ArrowDown);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(5), Pos(25)));
    shift_key(&mut p, KeyCode::ArrowLeft);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(5), Pos(19)));
    shift_key(&mut p, KeyCode::ArrowUp);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(5), Pos(5)));
}

#[test]
fn shift_down_from_a_cells_only_line_selects_the_cell_below() {
    let mut p = page();
    press(&mut p, 8);
    shift_key(&mut p, KeyCode::ArrowDown);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(5), Pos(19)));
    // From the bottom row, down leaves the table: the plain extension.
    let mut p = page();
    press(&mut p, 22);
    shift_key(&mut p, KeyCode::ArrowDown);
    assert!(
        matches!(p.handle.selection(), Selection::Text(t) if t.anchor == Pos(22) && t.head.0 > 33),
        "{:?}",
        p.handle.selection()
    );
}

#[test]
fn shift_click_from_a_cell_selection_keeps_its_anchor_cell() {
    let mut p = page();
    press(&mut p, 14);
    p.handle.set_selection(Selection::cell(Pos(11), Pos(11)));
    ev(&mut p.app, PlatformEvent::ModifiersChanged(shift()));
    press(&mut p, 8);
    ev(
        &mut p.app,
        PlatformEvent::ModifiersChanged(Modifiers::default()),
    );
    assert_eq!(p.handle.selection(), Selection::cell(Pos(11), Pos(5)));
}

// ── adopted from the second review of #1931 ─────────────────────────────────

fn at(doc: &rinch_editor_core::Node, text: &str) -> usize {
    let mut f = None;
    doc.nodes_between(0, doc.content().size(), &mut |n, pos, _| {
        if f.is_none() && n.is_text() && n.text().is_some_and(|t| t.contains(text)) {
            f = Some(pos + n.text().unwrap().find(text).unwrap());
        }
        f.is_none()
    });
    f.unwrap_or_else(|| panic!("no {text}"))
}

#[test]
fn shift_down_from_the_top_right_cell_selects_the_cell_below() {
    let mut p = page();
    press(&mut p, 14);
    shift_key(&mut p, KeyCode::ArrowDown);
    assert_eq!(p.handle.selection(), Selection::cell(Pos(11), Pos(25)));
}

/// The head cell for Shift+Down from a cell two columns wide is the one
/// under its left column: the plain step lands under the caret's x (the
/// right column) or, by document order, in "B", and neither normalizes to
/// this, whatever the fonts. Kills a desktop without its call after the
/// vertical step.
#[test]
fn shift_down_from_a_wide_cell_takes_the_left_column() {
    const WIDE: &str = "<p>x</p><table><tr><td colspan=\"2\"><p>AAAAAAAAAAAAAAAAAAAA</p></td>\
        <td><p>B</p></td></tr><tr><td><p>C</p></td><td><p>D</p></td><td><p>E</p></td></tr>\
        </table><p>zw</p>";
    let mut p = page_of(WIDE);
    let a = at(&p.handle.doc(), "AAAA");
    press(&mut p, a + 20);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(a + 20)));
    shift_key(&mut p, KeyCode::ArrowDown);
    let c = at(&p.handle.doc(), "C") - 2;
    assert_eq!(p.handle.selection(), Selection::cell(Pos(a - 2), Pos(c)));
}

/// Shift+Down out of the bottom row, then Shift+Up back: the step into the
/// table from below lands in the cell under the caret, so the range is back
/// inside the anchor's cell — text, not a cell selection of the last cell
/// (or, under a rowspan, five cells).
#[test]
fn shift_down_out_of_the_table_and_back_up_is_text() {
    let mut p = page();
    press(&mut p, 22);
    shift_key(&mut p, KeyCode::ArrowDown);
    shift_key(&mut p, KeyCode::ArrowUp);
    let sel = p.handle.selection();
    assert!(
        matches!(sel, Selection::Text(t) if t.anchor == Pos(22) && (21..=23).contains(&t.head.0)),
        "{sel:?}"
    );
    const MERGED: &str = "<p>x</p><table><tr><td colspan=\"2\"><p>A</p></td><td><p>B</p></td></tr>\
        <tr><td rowspan=\"2\"><p>C</p></td><td><p>D</p></td><td><p>E</p></td></tr>\
        <tr><td><p>F</p></td><td><p>G</p></td></tr></table><p>zw</p>";
    let mut p = page_of(MERGED);
    let f = at(&p.handle.doc(), "F");
    press(&mut p, f + 1);
    shift_key(&mut p, KeyCode::ArrowDown);
    shift_key(&mut p, KeyCode::ArrowUp);
    let sel = p.handle.selection();
    assert!(
        matches!(sel, Selection::Text(t) if t.anchor == Pos(f + 1) && (f..=f + 1).contains(&t.head.0)),
        "{sel:?}"
    );
}

#[test]
fn a_plain_up_into_a_table_from_below_lands_under_the_caret() {
    // "zw" sits under the left column; Up lands in "ef", not "gh".
    let mut p = page();
    let zw = at(&p.handle.doc(), "zw");
    press(&mut p, zw + 1);
    ev(
        &mut p.app,
        PlatformEvent::KeyDown {
            key: KeyCode::ArrowUp,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Fresh,
        },
    );
    let head = p.handle.selection().head().0;
    assert!((21..=23).contains(&head), "{:?}", p.handle.selection());
}

#[test]
fn a_nested_tables_paragraph_above_extends_as_text() {
    const NESTED: &str = "<table><tr><td><p>ab</p><table><tr><td><p>in1</p></td>\
        <td><p>in2</p></td></tr></table><p>after</p></td><td><p>cd</p></td></tr>\
        <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table><p>zw</p>";
    for key in [KeyCode::ArrowRight, KeyCode::ArrowDown] {
        let mut p = page_of(NESTED);
        let ab = at(&p.handle.doc(), "ab") + 2;
        press(&mut p, ab);
        shift_key(&mut p, key);
        let sel = p.handle.selection();
        assert!(
            matches!(sel, Selection::Text(t) if t.anchor == Pos(ab)),
            "{key:?}: {sel:?}"
        );
    }
}

#[test]
fn shift_arrows_at_the_documents_edges_do_nothing_on_the_desktop() {
    const ONE_ROW: &str = "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr></table>";
    let mut p = page_of(ONE_ROW);
    let ab = at(&p.handle.doc(), "ab");
    press(&mut p, ab);
    p.handle.set_selection(Selection::cursor(Pos(ab)));
    shift_key(&mut p, KeyCode::ArrowLeft);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(ab)));
    let cd = at(&p.handle.doc(), "cd") + 2;
    p.handle.set_selection(Selection::cursor(Pos(cd)));
    shift_key(&mut p, KeyCode::ArrowRight);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(cd)));
}
