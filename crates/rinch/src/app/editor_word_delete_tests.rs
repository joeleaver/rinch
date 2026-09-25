//! Word delete in the rich-text editor (issue #303), driven through the real
//! `PlatformEvent` path: Ctrl+Backspace / Ctrl+Delete (Alt on macOS) reach the
//! core's `deleteWordBackward` / `deleteWordForward` through the default
//! keymap. The commands' own rules — block edges, selections, marks, inline
//! atoms — are pinned in `rinch-editor-core`; what is pinned here is that a
//! window's key press arrives at them at all. Desktop had no word delete
//! before; the chord fell through the keymap and did nothing.
//!
//! Every chord has a plain-key control beside it through the same events, so a
//! fixture cannot pass by a plain Backspace running instead.

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

/// One editor over `<p>hello world</p><p>second</p>`, focused by a real press.
/// "hello world" is 1..12 ("world" 7..12), "second" 14..20.
fn page() -> (RinchApp, crate::editor::EditorHandle) {
    let slot: Rc<RefCell<Option<crate::editor::EditorHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>hello world</p><p>second</p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px; \
             font-family: sans-serif",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some(handle);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let handle = slot.borrow_mut().take().expect("captured at mount");
    let (x, y, h) = app
        .editor_caret_point(&handle, Pos(3))
        .expect("the position has a caret");
    let (x, y) = (x + 1.0, y + h / 2.0);
    let button = MouseButton::Left;
    app.handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
    app.handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
    assert!(matches!(app.focus_target, FocusTarget::Editor(_)));
    (app, handle)
}

fn key_with(app: &mut RinchApp, key: KeyCode, modifiers: Modifiers) {
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
}

/// The platform's word-delete chord modifier: Ctrl on Windows / Linux, Alt
/// (Option) on macOS.
fn word_chord() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        alt: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn text(handle: &crate::editor::EditorHandle) -> Vec<String> {
    let doc = handle.doc();
    (0..doc.child_count())
        .map(|i| {
            let block = doc.child(i);
            (0..block.child_count())
                .filter_map(|j| block.child(j).text().map(str::to_string))
                .collect()
        })
        .collect()
}

/// The caret in "wor|ld" (10): the chord takes "wor", plain Backspace one char.
#[test]
fn the_word_chord_with_backspace_deletes_the_word_before_the_caret() {
    let (mut control, handle) = page();
    handle.set_selection(Selection::cursor(Pos(10)));
    key_with(&mut control, KeyCode::Backspace, Modifiers::default());
    assert_eq!(text(&handle), ["hello wold", "second"], "control");

    let (mut app, handle) = page();
    handle.set_selection(Selection::cursor(Pos(10)));
    key_with(&mut app, KeyCode::Backspace, word_chord());
    assert_eq!(text(&handle), ["hello ld", "second"]);
    assert_eq!(handle.selection(), Selection::cursor(Pos(7)));
}

/// The caret in "wor|ld" (10): the chord takes "ld", plain Delete one char.
#[test]
fn the_word_chord_with_delete_deletes_the_word_after_the_caret() {
    let (mut control, handle) = page();
    handle.set_selection(Selection::cursor(Pos(10)));
    key_with(&mut control, KeyCode::Delete, Modifiers::default());
    assert_eq!(text(&handle), ["hello word", "second"], "control");

    let (mut app, handle) = page();
    handle.set_selection(Selection::cursor(Pos(10)));
    key_with(&mut app, KeyCode::Delete, word_chord());
    assert_eq!(text(&handle), ["hello wor", "second"]);
}

/// At a block's start the chord joins with the block before, as Backspace does.
#[test]
fn the_word_chord_at_a_block_start_joins_the_blocks() {
    let (mut app, handle) = page();
    handle.set_selection(Selection::cursor(Pos(14)));
    key_with(&mut app, KeyCode::Backspace, word_chord());
    assert_eq!(text(&handle), ["hello worldsecond"]);
}
