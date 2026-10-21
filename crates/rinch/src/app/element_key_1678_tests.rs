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
    // Never focused: a caret at the start, as a browser answers.
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
    f.app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    assert_eq!(
        ta.selection_range(),
        Some((1, 5, SelectionDirection::Forward))
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
        r.y > iy && r.y + r.height / 2.0 < iy + ih,
        "{r:?} in {iy}..{}",
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
