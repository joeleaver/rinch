//! The element `onkeydown` / `onkeyup` handlers: ordering against the field,
//! the interceptor and the dismiss stack, and focus moves made by a handler.

use super::*;

/// Enter sends, Shift+Enter is a newline: the element's `onkeydown` takes
/// Enter before the field inserts a break or runs `onsubmit`, and an
/// unconsumed Shift+Enter goes on to both the ancestors and the field.
#[test]
fn onkeydown_consumes_enter_before_the_field_and_onsubmit() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("hi");
    f.take_log();

    f.press(KeyCode::Enter, None, false);
    assert_eq!(
        f.value(&f.textarea),
        "hi",
        "a consumed Enter inserts nothing"
    );
    let log = f.take_log();
    assert_eq!(log[0], "down:Enter", "{log:?}");
    assert!(
        !log.iter()
            .any(|e| e == "submit" || e.starts_with("outer:") || e.starts_with("input:")),
        "consumed: no submit, no ancestor, no edit: {log:?}"
    );

    f.press(KeyCode::Enter, None, true);
    let log = f.take_log();
    assert_eq!(
        log[..2],
        ["down:S-Enter", "outer:Enter"],
        "nearest first: {log:?}"
    );
    assert_eq!(
        f.value(&f.textarea),
        "hi\n",
        "Shift+Enter is the field's newline"
    );
}

/// The element sees a key after the interceptor (#3062: the app's capture
/// hook hears every key first) and before the dismiss stack — so a picker in
/// a field inside a `Modal` takes Escape and the modal stays — and an
/// unconsumed key still reaches the field.
#[test]
fn onkeydown_runs_after_the_interceptor_and_before_the_dismiss_stack() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let s1 = seen.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        if k.is_down() {
            s1.borrow_mut().push(format!("interceptor:{}", k.key));
        }
        false
    });
    let dismissed = Rc::new(Cell::new(0));
    let d1 = dismissed.clone();
    let _entry = rinch_core::push_dismiss_handler(f.app.doc_key(), move || {
        d1.set(d1.get() + 1);
        true
    });

    f.press(KeyCode::Escape, None, false);
    assert_eq!(dismissed.get(), 0, "the field's handler took Escape");
    assert_eq!(
        *seen.borrow(),
        ["interceptor:Escape"],
        "the interceptor first"
    );

    f.press(KeyCode::KeyX, Some("x"), false);
    assert_eq!(
        *seen.borrow(),
        ["interceptor:Escape", "interceptor:x"],
        "an unconsumed key goes on"
    );
    assert_eq!(f.value(&f.textarea), "x");
    rinch_core::clear_keyboard_interceptor();
}

/// `onkeyup` hears the release; a consuming one stops its ancestors, and the
/// interceptor hears the release whatever the element said.
#[test]
fn onkeyup_hears_the_release_and_the_interceptor_still_does() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    let ups = Rc::new(Cell::new(0));
    let u1 = ups.clone();
    rinch_core::set_keyboard_interceptor(move |k| {
        if k.is_up() {
            u1.set(u1.get() + 1);
        }
        false
    });
    f.take_log();
    f.press(KeyCode::KeyQ, Some("q"), false);
    assert!(f.take_log().contains(&"up:q".to_string()));
    assert_eq!(ups.get(), 1);
    rinch_core::clear_keyboard_interceptor();
}

/// A key the input method is composing with is the input method's: with a
/// preedit shown, `onkeydown` is not offered (a browser marks the event
/// `isComposing`).
#[test]
fn a_key_during_a_composition_is_not_offered() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.app.handle_event(
        PlatformEvent::Ime(rinch_platform::ImeEvent::Preedit {
            text: "ni".into(),
            cursor: None,
        }),
        VP,
        1.0,
    );
    f.take_log();
    f.press(KeyCode::Enter, None, false);
    assert!(
        !f.take_log().iter().any(|e| e.starts_with("down:")),
        "not offered during a composition"
    );
}

/// A focusable node's `onkeydown` is offered too, and a consumed Enter does
/// not activate it.
#[test]
fn a_focusable_nodes_onkeydown_runs_and_a_consumed_enter_does_not_activate() {
    let mut f = mount();
    let tile = f.tile.clone();
    f.click(&tile);
    f.take_log();
    f.press(KeyCode::Enter, None, false);
    assert_eq!(f.take_log(), ["tile:Enter"], "consumed: no click");
    f.press(KeyCode::KeyB, Some("b"), false);
    assert_eq!(f.take_log(), ["tile:b", "outer:b"]);
}

/// #1678 review F2: a raw `<textarea>` with only an `onkeydown` is a text
/// field on desktop as it is in a browser — it focuses, takes text, and its
/// handler runs.
#[test]
fn a_raw_control_with_only_a_key_handler_takes_text() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (l, o) = (log.clone(), out.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let l = l.clone();
        let root = rsx! {
            div {
                textarea {
                    style: "width: 200px; height: 60px",
                    onkeydown: move |k| {
                        l.borrow_mut().push(k.key.clone());
                        false
                    },
                }
            }
        };
        *o.borrow_mut() = Some(root.children()[0].clone());
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let ta = out.borrow().clone().unwrap();
    let mut f = Fixture {
        app,
        outer: ta.clone(),
        textarea: ta.clone(),
        input: ta.clone(),
        tile: ta.clone(),
        log,
    };
    f.click(&ta);
    f.type_str("a");
    assert_eq!(f.value(&ta), "a");
    assert_eq!(f.take_log(), ["a"]);
}

/// #1678 review F4: a handler that moves the focus without consuming sends
/// the key on to the newly focused field, as Chrome does.
#[test]
fn a_key_follows_a_focus_move_made_by_its_handler() {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let other: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let o2 = other.clone();
        let root = rsx! {
            div {
                input {
                    style: "width: 200px; height: 30px",
                    oninput: move |_v: String| {},
                    onkeydown: move |k| {
                        if k.key == "x"
                            && let Some(b) = o2.borrow().as_ref()
                        {
                            b.focus();
                        }
                        false
                    },
                }
                input { style: "width: 200px; height: 30px", oninput: move |_v: String| {} }
            }
        };
        let k = root.children();
        *other.borrow_mut() = Some(k[1].clone());
        *o.borrow_mut() = vec![k[0].clone(), k[1].clone()];
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let mut f = Fixture {
        app,
        outer: h[0].clone(),
        textarea: h[0].clone(),
        input: h[1].clone(),
        tile: h[1].clone(),
        log: Rc::new(RefCell::new(Vec::new())),
    };
    f.click(&h[0]);
    f.type_str("x");
    assert_eq!(
        (f.value(&h[0]), f.value(&h[1])),
        (String::new(), "x".into())
    );
}

/// While the built-in text context menu is open it owns the keyboard: the
/// field's `onkeydown` is not offered (as the editor's `on_key` is not).
#[test]
fn the_text_context_menu_keeps_its_keys_from_onkeydown() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("hi");
    let (x, y) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        let (x, y, _, h) = painted_element_box(&d.tree, ta.node_id().0);
        (x + 3.0, y + h / 2.0)
    };
    f.app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: MouseButton::Right,
        },
        VP,
        1.0,
    );
    f.app.handle_event(
        PlatformEvent::MouseUp {
            x,
            y,
            button: MouseButton::Right,
        },
        VP,
        1.0,
    );
    assert!(
        f.app.open_text_menu.is_some(),
        "positive control: the menu opened"
    );
    f.take_log();
    f.app.handle_event(
        PlatformEvent::KeyDown {
            key: KeyCode::ArrowDown,
            logical_key: None,
            text: None,
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
    assert!(
        !f.take_log().iter().any(|e| e.starts_with("down:")),
        "the menu's key"
    );
}

/// Review 2 of #1686, G5: a button's `onkeydown` that moves the focus to
/// another button and does not consume the key: Space clicks nothing (it
/// activates on its release, on the element that got the press), Enter
/// clicks the new focus — Chrome's answers, measured with trusted keys.
#[test]
fn space_whose_handler_moved_focus_clicks_nothing() {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let (l, o) = (log.clone(), out.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let other: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let o2 = other.clone();
        let (la, lb) = (l.clone(), l.clone());
        let root = rsx! {
            div {
                button {
                    style: "width: 60px; height: 20px",
                    onclick: move || la.borrow_mut().push("A".into()),
                    onkeydown: move |_k| {
                        if let Some(b) = o2.borrow().as_ref() {
                            b.focus();
                        }
                        false
                    },
                }
                button {
                    style: "width: 60px; height: 20px",
                    onclick: move || lb.borrow_mut().push("B".into()),
                }
            }
        };
        let k = root.children();
        *other.borrow_mut() = Some(k[1].clone());
        *o.borrow_mut() = vec![k[0].clone(), k[1].clone()];
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let mut f = Fixture {
        app,
        outer: h[0].clone(),
        textarea: h[0].clone(),
        input: h[0].clone(),
        tile: h[1].clone(),
        log: log.clone(),
    };
    for (key, text, want) in [
        (KeyCode::Space, Some(" "), Vec::<String>::new()),
        (KeyCode::Enter, None, vec!["B".to_string()]),
    ] {
        h[0].focus();
        f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        assert_eq!(f.app.focus_target, FocusTarget::Node(h[0].node_id().0));
        f.take_log();
        f.press(key, text, false);
        assert_eq!(f.take_log(), want, "{key:?}");
        assert_eq!(f.app.focus_target, FocusTarget::Node(h[1].node_id().0));
    }
}
