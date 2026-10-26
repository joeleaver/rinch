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
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 30px 0 0 40px");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(GRID);
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
