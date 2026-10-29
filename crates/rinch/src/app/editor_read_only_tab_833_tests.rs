//! A read-only editor lets Tab and Shift+Tab move the focus on (#833).
//!
//! The editor's keymap binds `Tab`→`sinkListItem` and `Shift-Tab`→
//! `liftListItem`, and a matched binding is consumed whether or not its
//! command applied, so an editable editor never lets Tab out. A read-only
//! editor refuses both commands, and consuming the key left a keyboard user
//! who tabbed or clicked into a read-only document with no way out. It now
//! behaves like a `readonly` field: Tab and Shift+Tab go to the next and the
//! previous Tab stop.
//!
//! Driven through the real `PlatformEvent` path, with a Tab stop on each side
//! of the editor, so "focus left" is also "focus left *in the right
//! direction*".

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    container: usize,
    before: usize,
    after: usize,
    handle: crate::editor::EditorHandle,
}

/// `<button>` · editor over a two-item bullet list · `<button>`, the editor
/// focused by a real press in the second item ("two", 10..13) and switched to
/// `read_only`. The caret in the second item is where an editable editor's Tab
/// nests the item, so the control has an edit to make.
fn page(read_only: bool) -> Page {
    page_focused(read_only, true)
}

/// [`page`], the editor given the keyboard by a press or, with `by_press`
/// false, by `EditorHandle::focus` — which leaves the DOM's focused node
/// unset, so Tab can start only from the arbiter's claim.
fn page_focused(read_only: bool, by_press: bool) -> Page {
    type Ids = (usize, usize, usize, crate::editor::EditorHandle);
    let slot: Rc<RefCell<Option<Ids>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let before = scope.create_element("button");
        before.set_attribute("style", "width: 80px; height: 24px");
        root.append_child(&before);
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<ul><li><p>one</p></li><li><p>two</p></li></ul>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
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
    if by_press {
        focus_by_press(&mut page);
    } else {
        page.handle.focus();
        ev(&mut page.app, PlatformEvent::AboutToWait);
        assert_eq!(page.app.focus_target, FocusTarget::Editor(page.container));
    }
    page.handle.set_read_only(read_only);
    page.app.resolve_and_repaint(800.0, 600.0);
    page
}

/// A real press in "two", which gives the editor the keyboard.
fn focus_by_press(page: &mut Page) {
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(11))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    ev(&mut page.app, PlatformEvent::MouseDown { x, y, button });
    ev(&mut page.app, PlatformEvent::MouseUp { x, y, button });
    assert_eq!(page.app.focus_target, FocusTarget::Editor(page.container));
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn tab(app: &mut RinchApp, shift: bool) {
    tab_with(
        app,
        Modifiers {
            shift,
            ..Default::default()
        },
    );
}

fn tab_with(app: &mut RinchApp, modifiers: Modifiers) {
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

/// The list's item count at the top level: 2 as loaded, 1 once Tab nested the
/// second item into the first.
fn top_level_items(handle: &crate::editor::EditorHandle) -> usize {
    handle.doc().child(0).child_count()
}

#[test]
fn tab_leaves_a_read_only_editor_for_the_next_tab_stop() {
    let mut p = page(true);
    let doc = p.handle.doc();
    tab(&mut p.app, false);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Node(p.after),
        "Tab goes on to the button after the editor"
    );
    assert!(p.handle.doc().same_ref(&doc), "and edits nothing");
}

#[test]
fn shift_tab_leaves_a_read_only_editor_for_the_previous_tab_stop() {
    let mut p = page(true);
    let doc = p.handle.doc();
    tab(&mut p.app, true);
    assert_eq!(
        p.app.focus_target,
        FocusTarget::Node(p.before),
        "Shift+Tab goes back to the button before the editor"
    );
    assert!(p.handle.doc().same_ref(&doc), "and edits nothing");
}

/// Focused with no press, the editor's own place in the document is still
/// where Tab starts: not the first Tab stop of the page (the `before` button,
/// where a start-less Tab goes) but the one after the editor.
#[test]
fn tab_from_a_programmatically_focused_read_only_editor_starts_at_the_editor() {
    for shift in [false, true] {
        let mut p = page_focused(true, false);
        tab(&mut p.app, shift);
        let want = if shift { p.before } else { p.after };
        assert_eq!(p.app.focus_target, FocusTarget::Node(want), "shift={shift}");
    }
}

/// The control, and the switch going off again: an editable editor keeps Tab
/// and nests the item, as it always has.
#[test]
fn an_editable_editor_still_indents_on_tab_also_after_read_only_is_switched_off() {
    for was_read_only in [false, true] {
        let mut p = page(was_read_only);
        p.handle.set_read_only(false);
        assert_eq!(top_level_items(&p.handle), 2, "precondition");
        p.handle.set_selection(Selection::cursor(Pos(11)));
        tab(&mut p.app, false);
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "was_read_only={was_read_only}: the editor keeps the keyboard"
        );
        assert_eq!(
            top_level_items(&p.handle),
            1,
            "was_read_only={was_read_only}: Tab nested the second item"
        );
        tab(&mut p.app, true);
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "was_read_only={was_read_only}: Shift+Tab keeps the keyboard too"
        );
        assert_eq!(
            top_level_items(&p.handle),
            2,
            "was_read_only={was_read_only}: Shift+Tab lifted it back out"
        );
    }
}

/// Ctrl+Tab and Meta+Tab are not Tab: they are the platform's (tab switching)
/// or an app's, and a read-only editor does not move the focus on them.
#[test]
fn ctrl_or_meta_tab_does_not_move_the_focus_out_of_a_read_only_editor() {
    for (ctrl, meta) in [(true, false), (false, true)] {
        let mut p = page(true);
        tab_with(
            &mut p.app,
            Modifiers {
                ctrl,
                meta,
                ..Default::default()
            },
        );
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "ctrl={ctrl} meta={meta}: the focus stays in the editor"
        );
    }
}
