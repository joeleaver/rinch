//! The editor and the single-line fields agree on whether the host is macOS
//! (#3062). The fields read the app's `text_keymap`; the editor's
//! not-on-macOS bindings (Mod-y's redo, where Cmd+Y is Chrome's History)
//! read `rinch_editor_core::keymap::host_is_macos`, which the shell now sets
//! from that same `text_keymap` on every key. Before, the editor answered by
//! the target OS alone, so an app on the macOS keymap redid with Cmd+Y in the
//! editor and not in its fields.
//!
//! Driven through the real `PlatformEvent` path, `text_keymap` set the way
//! `input_commit_tests::undo_redo::cmd_y_is_not_redo_on_macos` sets it, so
//! both halves run on every host.

use super::*;
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// An editor over `<p>abc</p>` with the keyboard, given by a real press, and
/// "abc" selected.
fn focused_editor(keymap: TextKeymap) -> (RinchApp, crate::editor::EditorHandle) {
    type Ids = (u64, crate::editor::EditorHandle);
    let slot: Rc<RefCell<Option<Ids>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>abc</p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        container
    });
    app.text_keymap = keymap;
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, Pos(2))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    handle.set_selection(Selection::text(Pos(1), Pos(4)));
    (app, handle)
}

/// The build's primary chord (Cmd on a macOS build, Ctrl elsewhere) with the
/// letter `c`, as a real key press delivers it.
fn chord(app: &mut RinchApp, key: KeyCode, c: &str, shift: bool) {
    let modifiers = Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        shift,
        ..Default::default()
    };
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: Some(c.to_string()),
            text: Some(c.to_string()),
            modifiers,
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    app.handle_event(
        PlatformEvent::KeyUp {
            key,
            logical_key: Some(c.to_string()),
            modifiers,
        },
        VP,
        1.0,
    );
}

fn html(handle: &crate::editor::EditorHandle) -> String {
    node_to_html(&handle.doc())
}

/// Bold, then undo: the state a redo chord is tried in.
fn bold_then_undo(app: &mut RinchApp, handle: &crate::editor::EditorHandle) {
    chord(app, KeyCode::KeyB, "b", false);
    assert_eq!(
        html(handle),
        "<p><strong>abc</strong></p>",
        "positive control"
    );
    chord(app, KeyCode::KeyZ, "z", false);
    assert_eq!(html(handle), "<p>abc</p>", "undone");
}

/// On the macOS keymap the editor, like the fields, takes Cmd+Y for
/// Chrome's History: it redoes nothing, and Cmd+Shift+Z redoes.
#[test]
fn on_the_macos_keymap_the_editor_does_not_redo_with_y() {
    let (mut app, handle) = focused_editor(TextKeymap::MacOs);
    bold_then_undo(&mut app, &handle);
    chord(&mut app, KeyCode::KeyY, "y", false);
    assert_eq!(html(&handle), "<p>abc</p>", "Cmd+Y redoes nothing");
    chord(&mut app, KeyCode::KeyZ, "Z", true);
    assert_eq!(
        html(&handle),
        "<p><strong>abc</strong></p>",
        "Cmd+Shift+Z redoes"
    );
}

/// On the standard keymap the same chord redoes, in the editor as in the
/// fields.
#[test]
fn on_the_standard_keymap_the_editor_redoes_with_y() {
    let (mut app, handle) = focused_editor(TextKeymap::Standard);
    bold_then_undo(&mut app, &handle);
    chord(&mut app, KeyCode::KeyY, "y", false);
    assert_eq!(
        html(&handle),
        "<p><strong>abc</strong></p>",
        "Ctrl+Y redoes"
    );
}
