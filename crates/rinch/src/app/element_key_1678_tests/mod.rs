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

mod caret_rect;
mod key_handlers;
mod selection_range;
