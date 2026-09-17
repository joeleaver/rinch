//! A read-only rich-text editor (`EditorHandle::set_read_only`), driven through
//! the real `PlatformEvent` path.
//!
//! The guarantee itself is the handle's — one gate where every local change
//! lands — and is pinned in `rinch-editor-view`. What is pinned here is the
//! desktop half around it: the keys, IME events and clipboard chords a window
//! delivers all end at that gate, the ones a reader needs (caret, selection,
//! select-all, copy) still work, the OS input method stays off, and Cut does
//! not quietly turn into Copy.
//!
//! Every refusal has an editable **control** beside it, run through the same
//! events: a key that would have done nothing anyway proves nothing.

use super::*;
use rinch_editor_core::{Pos, Selection};

const VP: (u32, u32) = (800, 600);

struct Page {
    app: RinchApp,
    container: usize,
    handle: crate::editor::EditorHandle,
}

/// One editor over `<p>hello world</p><p>second</p>`, focused by a real press
/// in the first paragraph, then switched to `read_only`. "hello world" is
/// 1..12, "second" is 14..20.
fn page(read_only: bool) -> Page {
    let slot: Rc<RefCell<Option<(usize, crate::editor::EditorHandle)>>> =
        Rc::new(RefCell::new(None));
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
        *slot_in.borrow_mut() = Some((container.node_id().0, handle));
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let (container, handle) = slot.borrow_mut().take().expect("captured at mount");
    let mut page = Page {
        app,
        container,
        handle,
    };
    let (x, y) = point_at(&page, 3);
    press(&mut page.app, x, y);
    assert_eq!(page.app.focus_target, FocusTarget::Editor(page.container));
    page.handle.set_read_only(read_only);
    // The switch marks the container (`data-pm-readonly`), which dirties it like
    // any attribute write; a frame follows in a running app, and the fixtures
    // that measure caret geometry straight away need that layout too.
    page.app.resolve_and_repaint(800.0, 600.0);
    page
}

/// A window point just after the caret for `pos`.
fn point_at(page: &Page, pos: usize) -> (f32, f32) {
    let (x, y, h) = page
        .app
        .editor_caret_point(&page.handle, Pos(pos))
        .expect("the position has a caret");
    (x + 1.0, y + h / 2.0)
}

fn ev(app: &mut RinchApp, event: PlatformEvent) -> Vec<AppAction> {
    app.handle_event(event, VP, 1.0)
}

fn press(app: &mut RinchApp, x: f32, y: f32) {
    let button = MouseButton::Left;
    ev(app, PlatformEvent::MouseDown { x, y, button });
    ev(app, PlatformEvent::MouseUp { x, y, button });
}

fn key_with(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    ev(
        app,
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
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

fn primary() -> Modifiers {
    Modifiers {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

/// A key press by name: the selection it acts on, the key, the text it carries
/// and its modifiers.
type EditingKey = (
    &'static str,
    Selection,
    KeyCode,
    Option<&'static str>,
    Modifiers,
);

/// The editing keys a window delivers.
fn editing_keys() -> Vec<EditingKey> {
    let none = Modifiers::default;
    let caret = || Selection::cursor(Pos(6));
    let word = || Selection::text(Pos(7), Pos(12));
    vec![
        ("a letter", caret(), KeyCode::KeyX, Some("x"), none()),
        (
            "a letter over a selection",
            word(),
            KeyCode::KeyX,
            Some("x"),
            none(),
        ),
        ("Space", caret(), KeyCode::Space, Some(" "), none()),
        ("Backspace", caret(), KeyCode::Backspace, None, none()),
        ("Delete", caret(), KeyCode::Delete, None, none()),
        ("Enter", caret(), KeyCode::Enter, None, none()),
        ("Shift+Enter", caret(), KeyCode::Enter, None, shift()),
        (
            "Mod+B over a selection",
            word(),
            KeyCode::KeyB,
            None,
            primary(),
        ),
        (
            "Mod+I over a selection",
            word(),
            KeyCode::KeyI,
            None,
            primary(),
        ),
    ]
}

#[test]
fn editing_keys_change_nothing_in_a_read_only_editor() {
    for (what, selection, key, text, modifiers) in editing_keys() {
        let mut control = page(false);
        control.handle.set_selection(selection.clone());
        let before = control.handle.doc();
        key_with(&mut control.app, key, text, modifiers);
        assert!(
            !control.handle.doc().same_ref(&before),
            "control: {what} edits an editable editor"
        );

        let mut p = page(true);
        p.handle.set_selection(selection.clone());
        let before = p.handle.doc();
        key_with(&mut p.app, key, text, modifiers);
        assert!(
            p.handle.doc().same_ref(&before),
            "{what} must leave a read-only document alone"
        );
        assert_eq!(
            p.handle.selection(),
            selection,
            "{what}: nor move the selection"
        );
        assert_eq!(
            p.app.focus_target,
            FocusTarget::Editor(p.container),
            "{what}: the editor keeps the keyboard"
        );
    }
}

/// Undo is an edit. The control types and undoes; the read-only editor is
/// locked *after* typing, so there is history the chord would have replayed.
#[test]
fn undo_by_chord_is_refused_and_works_again_once_writable() {
    let mut p = page(false);
    p.handle.set_selection(Selection::cursor(Pos(12)));
    key_with(&mut p.app, KeyCode::Digit1, Some("!"), Modifiers::default());
    let typed = p.handle.doc();
    assert_eq!(typed.child(0).child(0).text(), Some("hello world!"));

    p.handle.set_read_only(true);
    key_with(&mut p.app, KeyCode::KeyZ, None, primary());
    assert!(p.handle.doc().same_ref(&typed), "Mod+Z did nothing");

    p.handle.set_read_only(false);
    key_with(&mut p.app, KeyCode::KeyZ, None, primary());
    assert_eq!(
        p.handle.doc().child(0).child(0).text(),
        Some("hello world"),
        "writable again: the same chord undoes"
    );
}

/// What a reader does with the keyboard and the pointer still works.
#[test]
fn the_caret_and_the_selection_still_follow_the_keyboard_and_the_pointer() {
    let mut p = page(true);
    let doc = p.handle.doc();

    // A press places the caret...
    let (x, y) = point_at(&p, 8);
    press(&mut p.app, x, y);
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(8)));
    // ...arrows move it, Shift extends, Home/End jump...
    key_with(&mut p.app, KeyCode::ArrowRight, None, Modifiers::default());
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(9)));
    key_with(&mut p.app, KeyCode::ArrowRight, None, shift());
    assert_eq!(p.handle.selection(), Selection::text(Pos(9), Pos(10)));
    key_with(&mut p.app, KeyCode::ArrowDown, None, Modifiers::default());
    assert!(
        p.handle.selection().head().0 >= 14,
        "Down reaches the second paragraph: {:?}",
        p.handle.selection()
    );
    key_with(&mut p.app, KeyCode::Home, None, Modifiers::default());
    assert_eq!(p.handle.selection(), Selection::cursor(Pos(14)));
    // ...a drag selects...
    let (ax, ay) = point_at(&p, 2);
    let (bx, by) = point_at(&p, 6);
    let button = MouseButton::Left;
    ev(
        &mut p.app,
        PlatformEvent::MouseDown {
            x: ax,
            y: ay,
            button,
        },
    );
    ev(&mut p.app, PlatformEvent::MouseMove { x: bx, y: by });
    ev(
        &mut p.app,
        PlatformEvent::MouseUp {
            x: bx,
            y: by,
            button,
        },
    );
    assert_eq!(p.handle.selection(), Selection::text(Pos(2), Pos(6)));
    assert_eq!(p.handle.selection_clipboard().unwrap().1, "ello");
    // ...and Mod+A takes the lot.
    key_with(&mut p.app, KeyCode::KeyA, None, primary());
    assert_eq!(
        p.handle.selection_clipboard().unwrap().1,
        "hello world\nsecond"
    );
    assert!(p.handle.doc().same_ref(&doc), "none of it was an edit");
}

/// The preedit overlay's inline style, or `None` if the view never built one.
fn preedit_style(app: &RinchApp) -> Option<String> {
    let doc = app.doc.as_ref()?.borrow();
    doc.tree
        .nodes
        .iter()
        .find(|(_, n)| n.attributes.contains_key("data-pm-preedit"))
        .map(|(_, n)| n.attributes.get("style").cloned().unwrap_or_default())
}

fn preedit_visible(app: &RinchApp) -> bool {
    preedit_style(app).is_some_and(|s| !s.replace(' ', "").contains("display:none"))
}

#[test]
fn a_read_only_editor_composes_nothing() {
    let preedit = || {
        PlatformEvent::Ime(ImeEvent::Preedit {
            text: "ne".into(),
            cursor: None,
        })
    };

    // Control: an editable editor turns the OS input method on, shows the
    // composition, and commits it.
    let mut control = page(false);
    assert!(control.app.ime_state().enabled);
    ev(&mut control.app, preedit());
    assert!(
        preedit_visible(&control.app),
        "control: the preedit is shown"
    );
    ev(
        &mut control.app,
        PlatformEvent::Ime(ImeEvent::Commit("ね".into())),
    );
    assert_eq!(
        control.handle.doc().child(0).child(0).text(),
        Some("heねllo world")
    );

    let mut p = page(true);
    let doc = p.handle.doc();
    assert!(
        !p.app.ime_state().enabled,
        "no input method over text that cannot take its result"
    );
    // A backend that composes anyway still gets nowhere.
    ev(&mut p.app, preedit());
    assert!(!preedit_visible(&p.app), "no composition is shown");
    ev(
        &mut p.app,
        PlatformEvent::Ime(ImeEvent::Commit("ね".into())),
    );
    ev(
        &mut p.app,
        PlatformEvent::Ime(ImeEvent::DeleteSurrounding {
            before: 2,
            after: 0,
        }),
    );
    assert!(p.handle.doc().same_ref(&doc));

    // The switch going on mid-composition takes the overlay down with it, and
    // going off again gives the input method back.
    let mut p = page(false);
    ev(&mut p.app, preedit());
    assert!(preedit_visible(&p.app));
    p.handle.set_read_only(true);
    p.app.refresh_editor_overlays();
    p.app.resolve_and_repaint(800.0, 600.0);
    assert!(
        !preedit_visible(&p.app),
        "locked mid-composition: overlay gone"
    );
    assert!(!p.app.ime_state().enabled);
    p.handle.set_read_only(false);
    assert!(p.app.ime_state().enabled);
}

/// The context menu and any shell toolbar read `text_edit_state`: a read-only
/// editor is a `readonly` field there — Copy and Select all, no Cut, no Paste.
#[test]
fn the_text_edit_state_offers_copy_and_select_all_only() {
    let clip = cfg!(feature = "clipboard");
    let flags = |s: TextEditState| (s.can_cut, s.can_copy, s.can_paste, s.can_select_all);

    let mut control = page(false);
    control
        .handle
        .set_selection(Selection::text(Pos(7), Pos(12)));
    assert_eq!(
        flags(control.app.text_edit_state().unwrap()),
        (clip, clip, clip, true)
    );

    let mut p = page(true);
    p.handle.set_selection(Selection::text(Pos(7), Pos(12)));
    assert_eq!(
        flags(p.app.text_edit_state().unwrap()),
        (false, clip, false, true)
    );
    // It follows the switch, read not cached.
    p.handle.set_read_only(false);
    assert_eq!(
        flags(p.app.text_edit_state().unwrap()),
        (clip, clip, clip, true)
    );
}

#[cfg(feature = "clipboard")]
mod clipboard {
    use super::*;

    /// The clipboard is one per process and the test binary is many threads.
    fn clipboard_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        rinch_clipboard::use_in_memory_clipboard();
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Cut in a read-only editor is not "copy": it would change the clipboard
    /// while appearing to do nothing. Copy is copy. By chord and by the menu's
    /// own entry point alike.
    #[test]
    fn cut_touches_neither_the_document_nor_the_clipboard_and_copy_still_copies() {
        let _lock = clipboard_lock();
        for via_action in [false, true] {
            let mut p = page(true);
            p.handle.set_selection(Selection::text(Pos(7), Pos(12)));
            let doc = p.handle.doc();
            rinch_clipboard::clear().unwrap();
            rinch_clipboard::copy_text("before").unwrap();

            if via_action {
                p.app.perform_text_edit(TextEditAction::Cut);
            } else {
                key_with(&mut p.app, KeyCode::KeyX, None, primary());
            }
            assert!(p.handle.doc().same_ref(&doc), "cut deleted nothing");
            assert_eq!(
                rinch_clipboard::paste_text().unwrap_or_default(),
                "before",
                "and copied nothing (via_action={via_action})"
            );

            if via_action {
                p.app.perform_text_edit(TextEditAction::Copy);
            } else {
                key_with(&mut p.app, KeyCode::KeyC, None, primary());
            }
            // The copy is queued on the clipboard worker; a blocking read
            // behind it in the queue sees its result.
            assert_eq!(rinch_clipboard::paste_text().unwrap_or_default(), "world");
        }
    }
}
