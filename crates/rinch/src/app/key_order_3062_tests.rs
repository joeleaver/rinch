//! The key handler order (issue #3062) on desktop: the keyboard interceptor
//! runs before every handler but rinch's three keyboard owners, the app-wide
//! listeners (`on_app_key`) are offered the rich-text editor's keys before its
//! keymap, and an open text context menu or `<select>` list takes its keys
//! before the interceptor. Driven through the real `rsx!` output and the real
//! key path (`handle_event`), the way the desktop loop drives them.
//!
//! The order itself is `docs/internals/focus-and-keyboard.md` "The key handler
//! order"; `rinch-web/tests/key_order_3062.rs` pins the same steps in a
//! browser.

use super::*;
use rinch_core::events::{
    KeyEventData, clear_keyboard_interceptor, on_app_key, push_dismiss_handler, push_key_handler,
    set_keyboard_interceptor,
};
use rinch_editor_core::{Pos, Selection};
use rinch_macros::rsx;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

fn key_event(app: &mut RinchApp, key: KeyCode, text: Option<&str>, modifiers: Modifiers) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers,
            repeat: KeyRepeat::Unknown,
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

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    key_event(app, key, text, Modifiers::default());
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

/// An interceptor that logs every press and release, and takes the keys in
/// `takes`.
fn intercept(log: &Log, takes: &'static [&'static str]) {
    let l = log.clone();
    set_keyboard_interceptor(move |k: &KeyEventData| {
        let phase = if k.is_up() { "up" } else { "down" };
        l.borrow_mut()
            .push(format!("interceptor:{phase}:{}", k.key));
        k.is_down() && takes.contains(&k.key.as_str())
    });
}

struct Fields {
    app: RinchApp,
    root: NodeHandle,
    input: u64,
    select: u64,
}

/// An `<input>` whose `onkeydown` / `onkeyup` log every key and take F2, and
/// a `<select>` with three options.
fn fields(log: &Log) -> Fields {
    let ids: Rc<RefCell<Option<(NodeHandle, u64, u64)>>> = Rc::new(RefCell::new(None));
    let (l, ids_in) = (log.clone(), ids.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let (down, up) = (l.clone(), l.clone());
        let root = rsx! {
            div {
                input {
                    style: "width: 200px; height: 30px; font-size: 16px; padding: 0; border: 0",
                    oninput: move |_v: String| {},
                    onkeydown: move |k| {
                        down.borrow_mut().push(format!("element:down:{}", k.key));
                        k.key == "F2"
                    },
                    onkeyup: move |k| {
                        up.borrow_mut().push(format!("element:up:{}", k.key));
                        false
                    },
                }
                select {
                    style: "width: 120px; height: 24px",
                    option { value: "a", "Alpha" }
                    option { value: "b", "Beta" }
                    option { value: "c", "Gamma" }
                }
            }
        };
        let kids = root.children();
        *ids_in.borrow_mut() = Some((root.clone(), kids[0].node_id().0, kids[1].node_id().0));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (root, input, select) = ids.borrow_mut().take().expect("ids");
    Fields {
        app,
        root,
        input,
        select,
    }
}

impl Fields {
    /// Focus the input the way a user does, with a press on it: the
    /// document's active element moves too, which a key entry's owner check
    /// and the text context menu both read.
    fn focus_input(&mut self) {
        let (x, y) = (10.0, 15.0);
        let button = MouseButton::Left;
        self.app
            .handle_event(PlatformEvent::MouseMove { x, y }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseDown { x, y, button }, VP, 1.0);
        self.app
            .handle_event(PlatformEvent::MouseUp { x, y, button }, VP, 1.0);
        assert_eq!(self.app.focus_target, FocusTarget::Input(self.input));
    }
}

/// The interceptor is the app's capture-phase hook: it hears a press before
/// the focused element's `onkeydown`, and a press it takes never reaches the
/// element. Before #3062 the element was asked first.
#[test]
fn the_interceptor_runs_before_the_focused_elements_onkeydown() {
    clear_keyboard_interceptor();
    let log: Log = Default::default();
    let mut f = fields(&log);
    f.focus_input();
    intercept(&log, &["F4"]);

    press(&mut f.app, KeyCode::F3, None);
    assert_eq!(
        take(&log),
        [
            "interceptor:down:F3",
            "element:down:F3",
            "interceptor:up:F3",
            "element:up:F3"
        ],
        "the interceptor first, on the press and on the release"
    );

    press(&mut f.app, KeyCode::F4, None);
    assert_eq!(
        take(&log),
        ["interceptor:down:F4", "interceptor:up:F4", "element:up:F4"],
        "a press the interceptor took reaches no element handler; its release \
         still does, as every release does"
    );

    // Control: the element still takes what the interceptor leaves, ahead of
    // the popups' key entries.
    let entry_log: Log = Default::default();
    let el = entry_log.clone();
    let _entry = push_key_handler(
        &f.root,
        move |k: &KeyEventData| {
            el.borrow_mut().push(format!("entry:{}", k.key));
            true
        },
        || {},
    );
    press(&mut f.app, KeyCode::F2, None);
    assert_eq!(
        take(&log),
        [
            "interceptor:down:F2",
            "element:down:F2",
            "interceptor:up:F2",
            "element:up:F2"
        ]
    );
    assert!(
        take(&entry_log).is_empty(),
        "the element's handler took F2 before any key entry"
    );
    press(&mut f.app, KeyCode::F3, None);
    assert_eq!(take(&entry_log), ["entry:F3"], "a key the element leaves");
    clear_keyboard_interceptor();
}

/// The interceptor takes Escape ahead of the dismiss stack, and a field's
/// `onkeydown` still takes it ahead of the stack too (#1678's order kept).
#[test]
fn the_interceptor_then_the_element_then_the_dismiss_stack_hear_escape() {
    clear_keyboard_interceptor();
    let log: Log = Default::default();
    let mut f = fields(&log);
    f.focus_input();
    intercept(&log, &[]);
    let dl = log.clone();
    let _dismiss = push_dismiss_handler(f.app.doc_key(), move || {
        dl.borrow_mut().push("dismiss".into());
        true
    });
    f.app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::Escape,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    assert_eq!(
        take(&log),
        ["interceptor:down:Escape", "element:down:Escape", "dismiss"]
    );
    clear_keyboard_interceptor();
}

/// An open `<select>` list takes its keys before the interceptor, as a
/// browser's native popup takes the keyboard before the page sees it. Before
/// #3062 an interceptor taking every key left the list unable to close.
#[test]
fn an_open_select_list_takes_its_keys_before_the_interceptor() {
    clear_keyboard_interceptor();
    let log: Log = Default::default();
    let mut f = fields(&log);
    f.app.open_select_popup(f.select, VP.0 as f32, VP.1 as f32);
    assert!(f.app.is_select_open(), "precondition: the list is open");
    intercept(&log, &["Escape", "ArrowDown"]);

    press(&mut f.app, KeyCode::ArrowDown, None);
    assert!(f.app.is_select_open(), "ArrowDown moves inside the list");
    press(&mut f.app, KeyCode::Escape, None);
    assert!(!f.app.is_select_open(), "Escape closed the list");
    assert_eq!(
        take(&log),
        ["interceptor:up:ArrowDown", "interceptor:up:Escape"],
        "the presses were the list's; only their releases reached the interceptor"
    );

    // Control: with the list closed the interceptor takes the same press.
    press(&mut f.app, KeyCode::Escape, None);
    assert_eq!(
        take(&log),
        ["interceptor:down:Escape", "interceptor:up:Escape"]
    );
    clear_keyboard_interceptor();
}

/// The built-in text context menu takes its keys before the interceptor too.
#[test]
fn an_open_text_context_menu_takes_its_keys_before_the_interceptor() {
    clear_keyboard_interceptor();
    let log: Log = Default::default();
    let mut f = fields(&log);
    f.focus_input();
    assert!(
        f.app
            .open_text_context_menu(40.0, 15.0, VP.0 as f32, VP.1 as f32)
    );
    assert!(
        f.app.open_text_menu.is_some(),
        "precondition: the menu is open"
    );
    intercept(&log, &["Escape"]);
    press(&mut f.app, KeyCode::Escape, None);
    assert!(f.app.open_text_menu.is_none(), "Escape closed the menu");
    assert_eq!(
        take(&log),
        ["interceptor:up:Escape", "element:up:Escape"],
        "the press was the menu's"
    );
    clear_keyboard_interceptor();
}

struct EditorPage {
    app: RinchApp,
    root: NodeHandle,
    handle: crate::editor::EditorHandle,
}

/// One editor over `<p>hello</p>`, holding the keyboard, the caret after
/// "hello" (position 6).
fn editor_page() -> EditorPage {
    type Slot = Option<(NodeHandle, u64, crate::editor::EditorHandle)>;
    let slot: Rc<RefCell<Slot>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        let (container, handle) = crate::editor::mount_editor(scope);
        handle.load_html("<p>hello</p>");
        container.set_attribute(
            "style",
            "width: 400px; height: 200px; font-size: 16px; line-height: 24px",
        );
        root.append_child(&container);
        *slot_in.borrow_mut() = Some((root.clone(), container.node_id().0, handle));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (root, container, handle) = slot.borrow_mut().take().expect("captured at mount");
    app.set_focus_target(FocusTarget::Editor(container));
    assert_eq!(app.focus_target, FocusTarget::Editor(container));
    handle.set_selection(Selection::cursor(Pos(6)));
    EditorPage { app, root, handle }
}

impl EditorPage {
    /// The document's blocks' text, joined by `|`.
    fn text(&self) -> String {
        fn collect(node: &rinch_editor_core::Node, out: &mut String) {
            if let Some(t) = node.text() {
                out.push_str(t);
            }
            for i in 0..node.child_count() {
                collect(node.child(i), out);
            }
        }
        let doc = self.handle.doc();
        (0..doc.child_count())
            .map(|i| {
                let mut s = String::new();
                collect(doc.child(i), &mut s);
                s
            })
            .collect::<Vec<_>>()
            .join("|")
    }
}

/// `on_app_key` is offered the editor's keys before its keymap and text
/// insertion: a listener that takes a key takes it from the editor. Before
/// #3062 the listeners never saw a key the editor held.
#[test]
fn on_app_key_is_offered_the_editors_keys_before_the_keymap() {
    clear_keyboard_interceptor();
    let mut page = editor_page();
    let log: Log = Default::default();
    let l = log.clone();
    let listener = on_app_key(move |k| {
        if k.is_down() {
            l.borrow_mut().push(format!("app:{}", k.key));
        }
        k.is_down() && (k.key == "x" || k.key == "Enter")
    });

    press(&mut page.app, KeyCode::KeyX, Some("x"));
    press(&mut page.app, KeyCode::Enter, Some("\r"));
    assert_eq!(take(&log), ["app:x", "app:Enter"]);
    assert_eq!(
        page.text(),
        "hello",
        "the listener took the letter and Enter: no insert, no split"
    );

    // Control: a key the listener leaves goes on to the editor.
    press(&mut page.app, KeyCode::KeyY, Some("y"));
    assert_eq!(take(&log), ["app:y"]);
    assert_eq!(page.text(), "helloy");
    listener.remove();
}

/// The editor's keys meet the interceptor first, then the editor's own
/// `on_key`, then the popups' key entries, then `on_app_key`, then the keymap.
#[test]
fn an_editor_key_meets_the_interceptor_on_key_entries_and_listeners_in_order() {
    clear_keyboard_interceptor();
    let mut page = editor_page();
    let log: Log = Default::default();
    intercept(&log, &[]);
    let ol = log.clone();
    page.handle.on_key(move |k| {
        ol.borrow_mut().push(format!("on_key:{}", k.key));
        false
    });
    let el = log.clone();
    let _entry = push_key_handler(
        &page.root,
        move |k: &KeyEventData| {
            el.borrow_mut().push(format!("entry:{}", k.key));
            k.key == "q"
        },
        || {},
    );
    let al = log.clone();
    let listener = on_app_key(move |k| {
        if k.is_down() {
            al.borrow_mut().push(format!("app:{}", k.key));
        }
        false
    });
    page.app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::KeyZ,
            logical_key: None,
            text: Some("z".into()),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    assert_eq!(
        take(&log),
        ["interceptor:down:z", "on_key:z", "entry:z", "app:z"]
    );
    assert_eq!(
        page.text(),
        "helloz",
        "the keymap's text insertion ran last"
    );

    // A key entry that takes the key takes it from the listeners and the keymap.
    press(&mut page.app, KeyCode::KeyQ, Some("q"));
    assert_eq!(
        take(&log),
        [
            "interceptor:down:q",
            "on_key:q",
            "entry:q",
            "interceptor:up:q"
        ]
    );
    assert_eq!(page.text(), "helloz");
    listener.remove();
    clear_keyboard_interceptor();
}

/// A listener that logs every press and release it is offered, by phase, and
/// consumes none.
fn log_app_keys(log: &Log) -> rinch_core::AppKeyListener {
    let l = log.clone();
    on_app_key(move |k| {
        let phase = if k.is_up() { "up" } else { "down" };
        l.borrow_mut().push(format!("app:{phase}:{}", k.key));
        false
    })
}

/// While the rich-text editor shows an IME composition its keys are the input
/// method's: `on_app_key` is not offered them, press or release, as it is not
/// offered a composing field's and the editor's `on_key` is not. Before the
/// fix the desktop asked only a focused `<input>` whether it was composing.
#[test]
fn on_app_key_is_not_offered_an_editor_key_during_composition() {
    clear_keyboard_interceptor();
    let mut page = editor_page();
    let log: Log = Default::default();
    let listener = log_app_keys(&log);

    page.handle.ime_set_preedit("ka", None);
    assert!(page.handle.is_composing());
    press(&mut page.app, KeyCode::KeyX, Some("x"));
    assert_eq!(take(&log), Vec::<String>::new(), "the input method's key");

    // Control: once the composition ends, the listener hears the editor's keys.
    page.handle.ime_clear_preedit();
    press(&mut page.app, KeyCode::KeyX, Some("x"));
    assert_eq!(take(&log), ["app:down:x", "app:up:x"]);
    listener.remove();
}

/// `on_app_key` hears a release only when it heard the press: a key the
/// element's `onkeydown` or the editor's `on_key` consumed is not the
/// listeners' on the way up either. The element's own `onkeyup` still hears
/// every release, as a browser's `keyup` does.
#[test]
fn on_app_key_hears_a_release_only_after_its_press() {
    clear_keyboard_interceptor();
    let log: Log = Default::default();
    let mut f = fields(&log);
    f.focus_input();
    take(&log);
    let listener = log_app_keys(&log);

    // F2 is the input's `onkeydown`'s.
    press(&mut f.app, KeyCode::F2, None);
    assert_eq!(take(&log), ["element:down:F2", "element:up:F2"]);

    // Control: a key the element leaves reaches the listener both ways.
    press(&mut f.app, KeyCode::F3, None);
    assert_eq!(
        take(&log),
        [
            "element:down:F3",
            "app:down:F3",
            "element:up:F3",
            "app:up:F3"
        ]
    );
    listener.remove();

    // The editor's `on_key` consuming a press keeps its release too.
    let mut page = editor_page();
    let ol = log.clone();
    page.handle.on_key(move |k| {
        ol.borrow_mut().push(format!("on_key:{}", k.key));
        k.key == "q"
    });
    let listener = log_app_keys(&log);
    press(&mut page.app, KeyCode::KeyQ, Some("q"));
    assert_eq!(take(&log), ["on_key:q"]);
    press(&mut page.app, KeyCode::KeyW, Some("w"));
    assert_eq!(take(&log), ["on_key:w", "app:down:w", "app:up:w"]);
    listener.remove();
}
