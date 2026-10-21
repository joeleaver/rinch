//! Raw text controls for a chat composer (issue #1678): an element's own
//! `onkeydown` / `onkeyup`, a selection read in UTF-16 units, and the rect of
//! the caret paint draws.
//!
//! Driven through the real `rsx!` output and the real key path
//! (`handle_event`), the way the desktop loop drives them.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use rinch_core::dom::SelectionDirection;
use rinch_core::events::KeyEventData;
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

type Log = Rc<RefCell<Vec<String>>>;

struct Fixture {
    app: RinchApp,
    outer: NodeHandle,
    textarea: NodeHandle,
    input: NodeHandle,
    tile: NodeHandle,
    log: Log,
}

/// A composer: a `<textarea>` whose `onkeydown` takes Enter (no Shift) as
/// "send" and Escape as "close the picker", inside a `div` with an
/// `onkeydown` of its own; an `<input>` with a placeholder and no key
/// handler; a `tabindex` tile with a click handler and an `onkeydown`.
fn mount() -> Fixture {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let handles: Rc<RefCell<Option<[NodeHandle; 4]>>> = Rc::new(RefCell::new(None));
    let (log_in, handles_in) = (log.clone(), handles.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let l = log_in.clone();
        let (l1, l2, l3, l4, l5, l6, l7) = (
            l.clone(),
            l.clone(),
            l.clone(),
            l.clone(),
            l.clone(),
            l.clone(),
            l.clone(),
        );
        let root = rsx! {
            div {
                style: "padding: 10px",
                onkeydown: move |k| {
                    l1.borrow_mut().push(format!("outer:{}", k.key));
                    k.key == "F2"
                },
                textarea {
                    style: "width: 300px; height: 80px; font-size: 16px; line-height: 20px; padding: 4px 6px; border: 0",
                    oninput: move |v: String| l2.borrow_mut().push(format!("input:{v}")),
                    onsubmit: move || l3.borrow_mut().push("submit".into()),
                    onkeydown: move |k: &KeyEventData| {
                        l4.borrow_mut().push(format!("down:{}{}", if k.shift { "S-" } else { "" }, k.key));
                        (k.key == "Enter" && !k.shift) || k.key == "Escape"
                    },
                    onkeyup: move |k| {
                        l5.borrow_mut().push(format!("up:{}", k.key));
                        true
                    },
                }
                input {
                    style: "width: 200px; height: 30px; font-size: 16px; padding: 0 8px; border: 0",
                    placeholder: "Say something",
                    oninput: move |_v: String| {},
                }
                div {
                    tabindex: "0",
                    style: "width: 50px; height: 20px",
                    onclick: move || l6.borrow_mut().push("click".into()),
                    onkeydown: move |k| {
                        l7.borrow_mut().push(format!("tile:{}", k.key));
                        k.key == "Enter"
                    },
                }
            }
        };
        let kids = root.children();
        *handles_in.borrow_mut() = Some([
            root.clone(),
            kids[0].clone(),
            kids[1].clone(),
            kids[2].clone(),
        ]);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let [outer, textarea, input, tile] = handles.borrow_mut().take().expect("handles");
    log.borrow_mut().clear();
    Fixture {
        app,
        outer,
        textarea,
        input,
        tile,
        log,
    }
}

impl Fixture {
    fn press(&mut self, key: KeyCode, text: Option<&str>, shift: bool) {
        let modifiers = Modifiers {
            shift,
            ..Modifiers::default()
        };
        self.app.handle_event(
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
        self.app.handle_event(
            PlatformEvent::KeyUp {
                key,
                logical_key: None,
                modifiers,
            },
            VP,
            1.0,
        );
    }

    fn type_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.press(KeyCode::KeyA, Some(&ch.to_string()), false);
        }
    }

    fn click(&mut self, node: &NodeHandle) {
        let (x, y) = {
            let d = self.app.doc.as_ref().unwrap().borrow();
            let (x, y, w, h) = painted_element_box(&d.tree, node.node_id().0);
            (x + w.min(6.0) / 2.0, y + h / 2.0)
        };
        for ev in [
            PlatformEvent::MouseDown {
                x,
                y,
                button: MouseButton::Left,
            },
            PlatformEvent::MouseUp {
                x,
                y,
                button: MouseButton::Left,
            },
        ] {
            self.app.handle_event(ev, VP, 1.0);
        }
        self.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    }

    fn value(&self, node: &NodeHandle) -> String {
        let d = self.app.doc.as_ref().unwrap().borrow();
        rinch_dom::form_control::control_value(&d.tree.nodes, node.node_id().0)
            .unwrap_or_default()
            .into_owned()
    }

    fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }
}

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

/// The element sees a key before the document-level interceptor and the
/// dismiss stack — so a picker in a field inside a `Modal` takes Escape and
/// the modal stays — and an unconsumed key still reaches both.
#[test]
fn onkeydown_runs_before_the_interceptor_and_the_dismiss_stack() {
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
    assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());

    f.press(KeyCode::KeyX, Some("x"), false);
    assert_eq!(
        *seen.borrow(),
        ["interceptor:x"],
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

/// The selection reads back in UTF-16 code units, with its direction —
/// before the control is focused, while it is, and after it lost the
/// keyboard.
#[test]
fn selection_range_reads_utf16_units_focused_unfocused_and_after_blur() {
    let mut f = mount();
    let ta = f.textarea.clone();
    // Never focused: a caret at the start of an empty textarea, as Chrome
    // answers (a value is covered by `an_unfocused_controls_selection_is_chromes`).
    assert_eq!(
        ta.selection_range(),
        Some((0, 0, SelectionDirection::Forward))
    );
    assert_eq!(f.tile.selection_range(), None, "not a text control");
    assert_eq!(f.outer.selection_range(), None);

    f.click(&ta);
    f.type_str("a😀bc");
    // "a😀bc": 😀 is 4 bytes and 2 UTF-16 units; the caret is after `c`.
    assert_eq!(
        ta.selection_range(),
        Some((5, 5, SelectionDirection::Forward))
    );
    // Shift+Left twice: anchor 5, head 3 — backward.
    for _ in 0..2 {
        f.app.handle_event(
            PlatformEvent::KeyDown {
                key: KeyCode::ArrowLeft,
                logical_key: None,
                text: None,
                modifiers: Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        );
    }
    assert_eq!(
        ta.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );

    // Blur: the selection stays with the control.
    let input = f.input.clone();
    f.click(&input);
    assert_eq!(
        ta.selection_range(),
        Some((3, 5, SelectionDirection::Backward))
    );

    // A range set on an unfocused control reads back at once, swapped and
    // clamped, and is what the next focus shows.
    ta.set_selection_range(9, 1, SelectionDirection::Forward);
    assert_eq!(
        ta.selection_range(),
        Some((1, 5, SelectionDirection::Forward))
    );
    // Drained (`AboutToWait` applies posted requests): now it is the stash
    // that answers, and the next focus that shows it.
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert!(
        rinch_core::peek_pending_text_selection_request(f.app.doc_key(), ta.node_id().0).is_none(),
        "the request was drained"
    );
    assert_eq!(
        ta.selection_range(),
        Some((1, 5, SelectionDirection::Forward))
    );

    // An unspecified direction reads back `Forward`, before focus as after,
    // as Chrome answers.
    ta.set_selection_range(2, 4, SelectionDirection::None);
    assert_eq!(
        ta.selection_range(),
        Some((2, 4, SelectionDirection::Forward))
    );
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    assert_eq!(
        ta.selection_range(),
        Some((2, 4, SelectionDirection::Forward))
    );
}

/// An unfocused control with a value reads what Chrome 155 reads: a caret
/// after the value once a `value` write set it (rinch-web writes through the
/// value setter, which moves it there), at the start of a parsed or default
/// value; and
/// `None` for the input types whose `selectionStart` is `null`.
#[test]
fn an_unfocused_controls_selection_is_chromes() {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        let mut add = |tag: &str, attrs: &[(&str, &str)]| {
            let el = __scope.create_element(tag);
            for (k, v) in attrs {
                el.set_attribute(k, v);
            }
            root.append_child(&el);
            o.borrow_mut().push(el);
        };
        add("input", &[("value", "abc")]);
        add("input", &[]);
        add("textarea", &[]);
        add("input", &[("type", "email"), ("value", "a@b")]);
        add("input", &[("type", "Number"), ("value", "1")]);
        add("input", &[("type", "date")]);
        let ta = out_text(__scope);
        root.append_child(&ta);
        o.borrow_mut().push(ta);
        o.borrow_mut().push(root.clone());
        root
    });
    fn out_text(scope: &mut RenderScope) -> NodeHandle {
        let ta = scope.create_element("textarea");
        ta.append_child(&scope.create_text("hello"));
        ta
    }
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let fwd = SelectionDirection::Forward;
    assert_eq!(h[0].selection_range(), Some((3, 3, fwd)), "input value=abc");
    assert_eq!(h[1].selection_range(), Some((0, 0, fwd)), "empty input");
    h[1].set_attribute("value", "hello");
    assert_eq!(
        h[1].selection_range(),
        Some((5, 5, fwd)),
        "after a value write"
    );
    // A range set and not yet applied: the write still wins (review 2, G1).
    h[1].set_selection_range(1, 2, fwd);
    h[1].set_attribute("value", "hello world");
    assert_eq!(
        h[1].selection_range(),
        Some((11, 11, fwd)),
        "a write moves it to the end"
    );
    h[2].set_attribute("value", "xy");
    assert_eq!(
        h[2].selection_range(),
        Some((2, 2, fwd)),
        "textarea value write"
    );
    for (i, what) in [(3, "email"), (4, "number"), (5, "date")] {
        assert_eq!(h[i].selection_range(), None, "{what}");
    }
    assert_eq!(
        h[6].selection_range(),
        Some((0, 0, fwd)),
        "textarea text child"
    );
    // Parsed markup sets its value with no setter: Chrome 155 reads the
    // untouched `<input>`'s caret at the start.
    h[7].set_inner_html("<input value=\"abcd\">");
    let parsed = h[7].children()[0].clone();
    assert_eq!(parsed.selection_range(), Some((0, 0, fwd)), "parsed input");
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

/// The caret rect at the head follows the caret's affinity: End on a soft-
/// wrapped first line puts the caret at that line's end (upstream), which
/// the downstream rect at the same offset draws at the next line's start.
#[test]
fn the_head_caret_rect_follows_its_affinity() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda");
    // To the first line, then its end.
    for key in [
        KeyCode::Home,
        KeyCode::ArrowUp,
        KeyCode::ArrowUp,
        KeyCode::End,
    ] {
        f.press(key, None, false);
    }
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (_, head, _) = ta.selection_range().unwrap();
    let up = ta.caret_rect().unwrap();
    let down = ta.caret_rect_at(head).unwrap();
    let c0 = ta.caret_rect_at(0).unwrap();
    assert!((up.y - c0.y).abs() < 0.5, "upstream: line 1 {up:?} {c0:?}");
    assert!(down.y > up.y + 10.0, "downstream: line 2 {down:?} {up:?}");
}

/// No box, no caret: a `display: none` field answers `None`.
#[test]
fn a_hidden_field_has_no_caret_rect() {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        let el = __scope.create_element("input");
        el.set_attribute("value", "abc");
        el.set_attribute("style", "display: none");
        root.append_child(&el);
        *o.borrow_mut() = Some(el);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let el = out.borrow().clone().unwrap();
    assert_eq!(el.caret_rect(), None);
    el.set_attribute("style", "display: block; width: 100px; height: 20px");
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert!(
        el.caret_rect().is_some(),
        "positive control: shown, it has one"
    );
}

/// The caret rect is where paint draws the caret — the same answer the
/// app's own painter-matched caret geometry gives — and moves with the
/// offset, the line and the placeholder.
#[test]
fn caret_rect_is_the_painted_caret() {
    let mut f = mount();
    let ta = f.textarea.clone();
    f.click(&ta);
    f.type_str("ab\ncd");
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);

    let id = ta.node_id().0;
    let painted = |f: &mut Fixture, byte: usize| {
        let app = &mut f.app;
        let d = app.doc.as_ref().unwrap().borrow();
        RinchApp::input_caret_rect_for_offset(
            &d.tree,
            &mut app.hit_test_font_cx,
            &mut app.hit_test_layout_cx,
            id,
            byte,
        )
        .expect("laid out")
    };

    let head = ta.caret_rect().expect("a caret");
    let at_end = painted(&mut f, 5);
    assert!((head.x - at_end.0).abs() < 0.5, "{head:?} vs {at_end:?}");
    assert!((head.y - at_end.1).abs() < 0.5, "{head:?} vs {at_end:?}");
    assert_eq!(head.width, 0.0);
    assert!(
        (head.height - 16.0 * 1.2).abs() < 0.01,
        "paint's caret height"
    );

    // Off the line start: offset 1 is right of offset 0, on line 1; offset 3
    // starts line 2, a line lower (paint lays the text out at the font's own
    // line height, not the CSS `line-height` — #320).
    let c0 = ta.caret_rect_at(0).unwrap();
    let c1 = ta.caret_rect_at(1).unwrap();
    let c3 = ta.caret_rect_at(3).unwrap();
    let p3 = painted(&mut f, 3);
    let p1 = painted(&mut f, 1);
    assert!(
        (c1.x - p1.0).abs() < 0.5 && (c1.y - p1.1).abs() < 0.5,
        "{c1:?} vs {p1:?}"
    );
    assert!(c1.x > c0.x + 3.0, "{c0:?} {c1:?}");
    assert!(
        (c3.x - p3.0).abs() < 0.5 && (c3.y - p3.1).abs() < 0.5,
        "{c3:?} vs {p3:?}"
    );
    assert!(
        (c3.x - c0.x).abs() < 0.5 && c3.y > c0.y + 15.0,
        "{c0:?} {c3:?}"
    );
    // The textarea's padding is in it: 10px page padding + 6px.
    let (bx, by) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        let (x, y, _, _) = painted_element_box(&d.tree, id);
        (x, y)
    };
    assert!((c0.x - (bx + 6.0)).abs() < 0.5, "{c0:?} box {bx}");
    // The top padding too (the caret's top is its line's, which Parley puts
    // a little above the content box for this face).
    assert!((c0.y - (by + 4.0)).abs() < 2.0, "{c0:?} box {by}");
    let p0 = painted(&mut f, 0);
    assert!((c0.y - p0.1).abs() < 0.5, "{c0:?} vs {p0:?}");

    // An empty single-line field with a placeholder: the caret at its start,
    // centred as paint centres the placeholder's line.
    let input = f.input.clone();
    let r = input.caret_rect().expect("an empty field has a caret");
    let (ix, iy, _, ih) = {
        let d = f.app.doc.as_ref().unwrap().borrow();
        painted_element_box(&d.tree, input.node_id().0)
    };
    assert!((r.x - (ix + 8.0)).abs() < 0.5, "{r:?}");
    assert!(
        (r.y + r.height / 2.0 - (iy + ih / 2.0)).abs() < 2.0,
        "centred: {r:?} in {iy}..{}",
        iy + ih
    );
    assert_eq!(f.tile.caret_rect(), None, "not a text control");
}

/// A password field's caret is measured over the bullets paint draws, not
/// the value.
#[test]
fn a_password_fields_caret_is_measured_over_its_bullets() {
    let ids: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let ids_in = ids.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        for ty in ["password", "text"] {
            let field = __scope.create_element("input");
            field.set_attribute("type", ty);
            field.set_attribute("value", "WWWWWW");
            field.set_attribute(
                "style",
                "width: 300px; height: 30px; font-size: 16px; padding: 0; border: 0",
            );
            root.append_child(&field);
        }
        let k = root.children();
        *ids_in.borrow_mut() = Some((k[0].clone(), k[1].clone()));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (pw, plain) = ids.borrow_mut().take().unwrap();
    let pw6 = pw.caret_rect_at(6).unwrap().x - pw.caret_rect_at(0).unwrap().x;
    let plain6 = plain.caret_rect_at(6).unwrap().x - plain.caret_rect_at(0).unwrap().x;
    assert!(
        pw6 > 1.0 && pw6 < plain6 * 0.8,
        "bullets are narrower than W: {pw6} vs {plain6}"
    );
}

/// Review 2 of #1686, G1/G6/N1: the selection a control shows when it gains
/// the keyboard is the one `selection_range()` read for it — after a range
/// set and then overridden by a `value` write (posted or stashed), typing
/// lands at the end, as in Chrome; an untouched textarea's caret is at its
/// start; an equal `value` write keeps the selection.
#[test]
fn focus_shows_the_selection_selection_range_reads() {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let o = out.clone();
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let root = __scope.create_element("div");
        for i in 0..4 {
            let tag = if i == 2 { "textarea" } else { "input" };
            let el = __scope.create_element(tag);
            el.set_attribute("style", "width: 200px; height: 30px; display: block");
            let id = __scope.register_input_handler(|_v: String| {});
            el.set_attribute("data-oninput", &id.0.to_string());
            if i == 2 {
                el.append_child(&__scope.create_text("hello"));
            } else {
                el.set_attribute("value", "hello");
            }
            root.append_child(&el);
            o.borrow_mut().push(el);
        }
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let h = out.borrow().clone();
    let fwd = SelectionDirection::Forward;
    let mut f = Fixture {
        app,
        outer: h[0].clone(),
        textarea: h[0].clone(),
        input: h[0].clone(),
        tile: h[0].clone(),
        log: Rc::new(RefCell::new(Vec::new())),
    };
    let focus_and_type = |f: &mut Fixture, n: &NodeHandle| {
        n.focus();
        f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
        f.type_str("Z");
        f.value(n)
    };

    // Posted, then overridden.
    h[0].set_selection_range(1, 2, fwd);
    h[0].set_attribute("value", "hello world");
    assert_eq!(h[0].selection_range(), Some((11, 11, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[0]), "hello worldZ");

    // Stashed, then overridden.
    h[1].set_selection_range(1, 2, fwd);
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    h[1].set_attribute("value", "hello world");
    assert_eq!(h[1].selection_range(), Some((11, 11, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[1]), "hello worldZ");

    // Untouched textarea: the caret starts where the read says.
    assert_eq!(h[2].selection_range(), Some((0, 0, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[2]), "Zhello");

    // An equal write keeps the selection (Chrome's value setter does too).
    h[3].set_selection_range(1, 2, fwd);
    f.app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    h[3].set_attribute("value", "hello");
    assert_eq!(h[3].selection_range(), Some((1, 2, fwd)));
    assert_eq!(focus_and_type(&mut f, &h[3]), "hZllo");
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
