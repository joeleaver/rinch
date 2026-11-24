//! Tab in an editable editor (#2210): a Tab the editor has no use for moves
//! the focus on, and `TabBehavior::MoveFocus` makes every Tab do so.
//!
//! Before, an editable editor offered Tab to its keymap (`Tab` →
//! `sinkListItem`, `Shift-Tab` → `liftListItem`) and consumed the matched
//! binding whether or not the command applied, so in a plain paragraph Tab
//! did nothing and the keyboard could not leave: a trap (WCAG 2.1.2). Chrome's
//! `contenteditable` never keeps Tab; rinch keeps it where it does something
//! (a list item, a table cell), the default `TabBehavior`, and lets it go
//! everywhere else.
//!
//! Driven through the real `PlatformEvent` path with a Tab stop on each side
//! of the editor (the #833 file's set-up), so "focus left" is also "in the
//! right direction".

use super::*;
use crate::editor::TabBehavior;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// `<p>plain</p>` (text 1..6), then a two-item list: "one" at 10..13, "two"
/// at 17..20.
const CONTENT: &str = "<p>plain</p><ul><li><p>one</p></li><li><p>two</p></li></ul>";
/// In the plain paragraph.
const IN_PLAIN: Pos = Pos(3);
/// In the first list item, which has no previous sibling to sink into.
const IN_FIRST_ITEM: Pos = Pos(11);
/// In the second list item, which Tab nests into the first.
const IN_SECOND_ITEM: Pos = Pos(18);

struct Page {
    app: RinchApp,
    container: u64,
    before: u64,
    after: u64,
    handle: crate::editor::EditorHandle,
}

/// `<button>` · editor over [`CONTENT`] · `<button>`, the editor given the
/// keyboard by a real press in the plain paragraph, then the caret put at
/// `caret`.
fn page_with(content: &str, caret: Pos) -> Page {
    type Ids = (u64, u64, u64, crate::editor::EditorHandle);
    let slot: Rc<RefCell<Option<Ids>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let content = content.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let before = scope.create_element("button");
        before.set_attribute("style", "width: 80px; height: 24px");
        root.append_child(&before);
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html(&content);
        container.set_attribute(
            "style",
            "width: 400px; height: 300px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&container);
        let after = scope.create_element("button");
        after.set_attribute("style", "width: 80px; height: 24px");
        root.append_child(&after);
        *slot_in.borrow_mut() = Some((
            container.node_id().0,
            before.node_id().0,
            after.node_id().0,
            handle,
        ));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, before, after, handle) = slot.borrow_mut().take().expect("captured at mount");
    let mut page = Page {
        app,
        container,
        before,
        after,
        handle,
    };
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(2))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    ev(&mut page.app, PlatformEvent::MouseDown { x, y, button });
    ev(&mut page.app, PlatformEvent::MouseUp { x, y, button });
    assert_eq!(page.app.focus_target, FocusTarget::Editor(page.container));
    page.handle.set_selection(Selection::cursor(caret));
    page.app.resolve_and_repaint(800.0, 600.0);
    page
}

fn page(caret: Pos) -> Page {
    page_with(CONTENT, caret)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn tab(app: &mut RinchApp, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Default::default()
    };
    ev(
        app,
        PlatformEvent::KeyDown {
            key: KeyCode::Tab,
            logical_key: None,
            text: Some("\t".into()),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
    );
    ev(
        app,
        PlatformEvent::KeyUp {
            key: KeyCode::Tab,
            logical_key: None,
            modifiers,
        },
    );
}

/// The list's top-level item count: 2 as loaded, 1 once the second item is
/// nested into the first.
fn top_level_items(handle: &crate::editor::EditorHandle) -> usize {
    handle.doc().child(1).child_count()
}

/// The trap itself: Tab and Shift+Tab in a paragraph outside any list leave
/// the editor, in their own direction, and edit nothing.
#[test]
fn tab_in_a_plain_paragraph_moves_the_focus_on() {
    for shift in [false, true] {
        let mut p = page(IN_PLAIN);
        let doc = p.handle.doc();
        tab(&mut p.app, shift);
        let want = if shift { p.before } else { p.after };
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Node(want),
            "shift={shift}: the focus goes to the Tab stop on that side"
        );
        assert!(p.handle.doc().same_ref(&doc), "shift={shift}: no edit");
    }
}

/// A list item that cannot sink (it has no previous sibling) is the same
/// trap in a list: the binding runs, does nothing, and the key moves on.
#[test]
fn tab_in_a_first_list_item_that_cannot_sink_moves_the_focus_on() {
    let mut p = page(IN_FIRST_ITEM);
    let doc = p.handle.doc();
    tab(&mut p.app, false);
    assert_eq!(p.app.focus_target, FocusTarget::Node(p.after));
    assert!(p.handle.doc().same_ref(&doc), "no edit");
}

/// The default keeps the list behaviour: Tab nests the second item and
/// Shift+Tab lifts it back, the keyboard staying in the editor.
#[test]
fn the_default_still_indents_a_list_item_that_can_sink() {
    let mut p = page(IN_SECOND_ITEM);
    assert_eq!(p.handle.tab_behavior(), TabBehavior::IndentInListsAndTables);
    assert_eq!(top_level_items(&p.handle), 2, "precondition");
    tab(&mut p.app, false);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
    assert_eq!(top_level_items(&p.handle), 1, "Tab nested the item");
    tab(&mut p.app, true);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
    assert_eq!(top_level_items(&p.handle), 2, "Shift+Tab lifted it back");
}

/// `MoveFocus` is Chrome's `contenteditable`: even where Tab could indent,
/// it moves the focus and edits nothing.
#[test]
fn move_focus_lets_every_tab_go_even_in_a_list() {
    for shift in [false, true] {
        let mut p = page(IN_SECOND_ITEM);
        p.handle.set_tab_behavior(TabBehavior::MoveFocus);
        let doc = p.handle.doc();
        tab(&mut p.app, shift);
        let want = if shift { p.before } else { p.after };
        assert_eq!(p.app.focus_target, FocusTarget::Node(want), "shift={shift}");
        assert!(p.handle.doc().same_ref(&doc), "shift={shift}: no edit");
    }
}

/// Table cells keep Tab under the default: it goes to the next cell, not out.
#[test]
fn the_default_still_moves_between_table_cells() {
    let mut p = page_with(
        "<p>plain</p><table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>",
        IN_PLAIN,
    );
    // Into the first cell's text: table 7, row 8, cell 9, p 10, "a" at 11.
    p.handle.set_selection(Selection::cursor(Pos(11)));
    tab(&mut p.app, false);
    assert_eq!(p.app.focus_target, FocusTarget::Editor(p.container));
    assert!(
        p.handle.selection().head().0 > 12,
        "the caret went to the second cell, at {:?}",
        p.handle.selection()
    );
}
