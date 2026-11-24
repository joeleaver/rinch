//! `onsubmit:` written in `rsx!` on a raw `<input>` runs on Enter (#1590).
//!
//! The macro used to map every `on*` name it did not list to `data-rid`, the
//! **click** handler, so `input { onsubmit: … }` ran when the field was clicked
//! and never on Enter: the runtime reads `data-onsubmit` for Enter, which that
//! route never wrote. These drive the real macro output through the real
//! click, type and Enter path.
//!
//! The second half is the component-level twin (#3048): Enter in a
//! `PasswordInput` submits as it does in a `TextInput`. `PasswordInput` had no
//! `onsubmit`, so an app that wanted Enter-to-submit on a password field wrote
//! `TextInput { input_type: "password" }`. Both now wire `onsubmit` through one
//! helper (`rinch_components::entry_attrs::bind_submit`); the tests drive a
//! real click, typing and Enter and check the two fields answer the same:
//! `oninput` per key, the `onchange` commit, then `onsubmit`, once.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
use rinch_components::password_input::PasswordInput;
use rinch_components::text_input::TextInput;
use rinch_core::{Callback, Component, InputCallback};
use rinch_macros::rsx;
use std::cell::Cell;

const VP: (u32, u32) = (800, 600);

struct Fixture {
    app: RinchApp,
    input: u64,
    wrapper: u64,
    log: Rc<RefCell<Vec<String>>>,
}

/// The three shapes: `onsubmit` alone on the `<input>`, beside an `oninput`,
/// and on a wrapper `div` around a field that has only an `oninput`.
#[derive(Clone, Copy)]
enum Shape {
    SubmitOnly,
    WithInput,
    OnWrapper,
}

fn mount(shape: Shape) -> Fixture {
    let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let ids: Rc<Cell<Option<(u64, u64)>>> = Rc::new(Cell::new(None));
    let (log_in, ids_in) = (log.clone(), ids.clone());
    let mut app = RinchApp::new(move |__scope: &mut RenderScope| {
        let (l1, l2) = (log_in.clone(), log_in.clone());
        let root = match shape {
            Shape::SubmitOnly => rsx! {
                div {
                    input {
                        style: "width: 200px; height: 30px",
                        onsubmit: move || l1.borrow_mut().push("submit".into()),
                    }
                }
            },
            Shape::WithInput => rsx! {
                div {
                    input {
                        style: "width: 200px; height: 30px",
                        oninput: move |v: String| l2.borrow_mut().push(format!("input:{v}")),
                        onsubmit: move || l1.borrow_mut().push("submit".into()),
                    }
                }
            },
            Shape::OnWrapper => rsx! {
                div {
                    onsubmit: move || l1.borrow_mut().push("submit".into()),
                    input {
                        style: "width: 200px; height: 30px",
                        oninput: move |v: String| l2.borrow_mut().push(format!("input:{v}")),
                    }
                }
            },
        };
        let input = root.children()[0].clone();
        assert_eq!(input.tag_name().as_deref(), Some("input"), "fixture shape");
        ids_in.set(Some((input.node_id().0, root.node_id().0)));
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let (input, wrapper) = ids.get().expect("ids captured at mount");
    Fixture {
        app,
        input,
        wrapper,
        log,
    }
}

impl Fixture {
    fn attr(&self, id: u64, name: &str) -> Option<String> {
        let d = self.app.doc.as_ref().unwrap().borrow();
        d.tree.nodes[id].attributes.get(name).cloned()
    }

    fn key(&mut self, key: KeyCode, text: Option<&str>) {
        self.app.handle_event(
            PlatformEvent::KeyDown {
                key,
                logical_key: None,
                text: text.map(str::to_string),
                modifiers: Modifiers::default(),
                repeat: KeyRepeat::Unknown,
            },
            VP,
            1.0,
        );
    }

    fn click_input(&mut self) {
        let (x, y) = {
            let d = self.app.doc.as_ref().unwrap().borrow();
            let (x, y, w, h) = painted_element_box(&d.tree, self.input);
            (x + w / 2.0, y + h / 2.0)
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
    }

    fn submits(&self) -> usize {
        self.log.borrow().iter().filter(|e| *e == "submit").count()
    }
}

/// The issue's shape: an `onsubmit` with no `oninput`. A click must not run
/// it (it used to: it was the click handler), the field must take typed text
/// (desktop recognises a text field by `data-oninput`, so the macro gives an
/// `onsubmit`-only `<input>` the same no-op one an `onchange`-only one gets),
/// and Enter runs it once.
#[test]
fn onsubmit_alone_on_an_input_runs_on_enter_and_not_on_click() {
    let mut f = mount(Shape::SubmitOnly);
    assert!(
        f.attr(f.input, "data-onsubmit").is_some(),
        "onsubmit must be written as data-onsubmit"
    );
    assert!(
        f.attr(f.input, "data-rid").is_none(),
        "onsubmit must not be wired as a click handler"
    );

    f.click_input();
    assert_eq!(f.submits(), 0, "a click is not a submit: {:?}", f.log);

    f.key(KeyCode::KeyA, Some("h"));
    f.key(KeyCode::KeyA, Some("i"));
    assert_eq!(
        f.attr(f.input, "value").as_deref(),
        Some("hi"),
        "the field takes typed text"
    );

    f.key(KeyCode::Enter, None);
    assert_eq!(f.submits(), 1, "Enter submits once: {:?}", f.log);
}

/// Beside an `oninput`, the field's own handler is the one that stays.
#[test]
fn onsubmit_beside_oninput_runs_on_enter() {
    let mut f = mount(Shape::WithInput);
    f.click_input();
    f.key(KeyCode::KeyA, Some("x"));
    f.key(KeyCode::Enter, None);
    assert_eq!(
        *f.log.borrow(),
        vec!["input:x".to_string(), "submit".to_string()],
        "typing fires oninput, Enter fires onsubmit, a click fires neither"
    );
}

/// On a wrapper the submit is read up the ancestors, as the guide says — and
/// the wrapper must NOT be given a text-field shim: a `div` carrying
/// `data-oninput` becomes a text field on desktop and would take the press.
#[test]
fn onsubmit_on_a_wrapper_submits_its_field_and_adds_no_shim() {
    let mut f = mount(Shape::OnWrapper);
    assert!(
        f.attr(f.wrapper, "data-oninput").is_none(),
        "a non-control wrapper must not become a text field"
    );
    f.click_input();
    // A click on the field bubbles to the wrapper: it must not submit (at the
    // bug the wrapper's `onsubmit` was its click handler).
    assert_eq!(f.submits(), 0, "a click is not a submit: {:?}", f.log);
    f.key(KeyCode::KeyA, Some("x"));
    f.key(KeyCode::Enter, None);
    assert_eq!(f.submits(), 1, "{:?}", f.log);
}

// ---------------------------------------------- PasswordInput onsubmit (#3048)

type Log = Rc<RefCell<Vec<String>>>;

/// The callbacks a field is given, each logging `<what>:<value>`.
fn callbacks(log: &Log) -> (InputCallback, InputCallback, Callback) {
    let (i, c, s) = (log.clone(), log.clone(), log.clone());
    (
        InputCallback::new(move |v: String| i.borrow_mut().push(format!("input:{v}"))),
        InputCallback::new(move |v: String| c.borrow_mut().push(format!("change:{v}"))),
        Callback::new(move || s.borrow_mut().push("submit".into())),
    )
}

/// Which field the fixture mounts.
#[derive(Clone, Copy, Debug)]
enum Field {
    Text,
    /// Masked: `type="password"`.
    Password,
    /// Shown: the toggle's `type="text"`.
    PasswordVisible,
}

/// Mount `field` with every callback logging into the returned log, under the
/// component stylesheet. Returns the app and the `<input>`'s node id.
fn mount_field(field: Field) -> (RinchApp, u64, Log) {
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    let (oninput, onchange, onsubmit) = callbacks(&log);
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "padding: 20px; width: 300px");
        let (oninput, onchange, onsubmit) = (
            Some(oninput.clone()),
            Some(onchange.clone()),
            Some(onsubmit.clone()),
        );
        let part = match field {
            Field::Text => TextInput {
                oninput,
                onchange,
                onsubmit,
                ..Default::default()
            }
            .render(scope, &[]),
            Field::Password | Field::PasswordVisible => PasswordInput {
                visible: matches!(field, Field::PasswordVisible),
                oninput,
                onchange,
                onsubmit,
                ..Default::default()
            }
            .render(scope, &[]),
        };
        root.append_child(&part);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    {
        let doc = app.doc.as_ref().unwrap();
        let mut d = doc.borrow_mut();
        d.load_css(&rinch_theme::generate_theme_css(
            &rinch_theme::Theme::default(),
        ));
        d.load_css(&rinch_components::generate_component_css());
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let class = match field {
        Field::Text => "rinch-text-input__input",
        Field::Password | Field::PasswordVisible => "rinch-password-input__input",
    };
    let input = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| {
                n.attributes
                    .get("class")
                    .is_some_and(|c| c.split_whitespace().any(|one| one == class))
            })
            .map(|(id, _)| id)
            .expect("the field's <input>")
    };
    (app, input, log)
}

fn press(app: &mut RinchApp, key: KeyCode, text: Option<&str>) {
    app.handle_event(
        PlatformEvent::KeyDown {
            key,
            logical_key: None,
            text: text.map(str::to_string),
            modifiers: Modifiers::default(),
            repeat: KeyRepeat::Unknown,
        },
        VP,
        1.0,
    );
}

fn click_field(app: &mut RinchApp, id: u64) {
    let (x, y) = {
        let d = app.doc.as_ref().unwrap().borrow();
        let (ax, ay, w, h) = painted_element_box(&d.tree, id);
        assert!(w > 8.0 && h > 8.0, "the field has a box: {w}x{h}");
        (ax + w / 2.0, ay + h / 2.0)
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
        app.handle_event(ev, VP, 1.0);
    }
}

/// Click the field, type "ab", press Enter, and return what was logged.
fn click_type_enter(field: Field) -> Vec<String> {
    let (mut app, input, log) = mount_field(field);
    let ty = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.nodes[input].attributes.get("type").cloned()
    };
    let want_type = match field {
        Field::Password => "password",
        Field::Text | Field::PasswordVisible => "text",
    };
    assert_eq!(ty.as_deref(), Some(want_type), "{field:?} fixture");

    click_field(&mut app, input);
    assert!(log.borrow().is_empty(), "a click submits nothing: {log:?}");
    press(&mut app, KeyCode::KeyA, Some("a"));
    press(&mut app, KeyCode::KeyB, Some("b"));
    press(&mut app, KeyCode::Enter, None);
    log.take()
}

#[test]
fn enter_in_a_password_input_submits_as_in_a_text_input() {
    let text = click_type_enter(Field::Text);
    assert_eq!(
        text,
        ["input:a", "input:ab", "change:ab", "submit"],
        "the TextInput reference"
    );
    for field in [Field::Password, Field::PasswordVisible] {
        assert_eq!(click_type_enter(field), text, "{field:?}");
    }
}
