//! `onsubmit:` written in `rsx!` on a raw `<input>` runs on Enter (#1590).
//!
//! The macro used to map every `on*` name it did not list to `data-rid`, the
//! **click** handler, so `input { onsubmit: … }` ran when the field was clicked
//! and never on Enter: the runtime reads `data-onsubmit` for Enter, which that
//! route never wrote. These drive the real macro output through the real
//! click, type and Enter path.

// `rsx!` writes absolute `rinch::` paths, and this *is* the rinch crate.
use super::hit_testing::painted_element_box;
use super::*;
use crate as rinch;
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
