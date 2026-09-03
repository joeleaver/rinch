//! NumberInput: `value_fn` makes the component controllable (#264).
//!
//! Pre-fix `NumberInput` had no `value_fn` and no reactive effect at all —
//! `set_attribute("value", …)` ran once at mount, and the stepper buttons only
//! invoked the app's callbacks without touching the field. An app owning the
//! number in a `Signal` (the documented `value_fn` + `oninput` pattern every
//! other text component supports) had no way to push a value into the field:
//! `signal.set(0.0)` from a Reset button, a stepper click the app clamps to
//! `max`, or a value loaded after mount all left the displayed text stale.
//!
//! The fix is `TextInput`'s exact shape: `value_fn: Option<ReactiveString>`,
//! an initial write at mount, and a scope-owned effect re-writing the `value`
//! attribute whenever the signals the closure reads change. The steppers stay
//! callback-only — in the controlled shape they write the *signal* and the
//! effect carries it to the DOM, so there is no second write path to desync.
//!
//! Harness note: as in `color_input_dropdown_sync.rs`, the desktop runtime
//! mirrors typed text into the `value` attribute before dispatching `oninput`,
//! so `type_text` does the same.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::number_input::NumberInput;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event, dispatch_input_event};
use rinch_core::{Callback, Component, InputCallback, Signal};

struct Input {
    // Kept alive for the test's duration: the document owns the nodes the
    // effect patches, the scope owns the effect and handlers.
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    signal: Signal<f64>,
}

impl Input {
    /// A NumberInput bound to a fresh signal holding `value`, with steppers
    /// that move the signal by `step` and clamp to `[min, max]` — the
    /// controlled call site. `clamp_input` adds an `oninput` that parses,
    /// clamps and writes back on every keystroke (the #238 shape).
    fn controlled(value: f64, step: f64, min: f64, max: f64, clamp_input: bool) -> Self {
        let signal = Signal::new(value);

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let input = NumberInput {
            min: Some(min),
            max: Some(max),
            step: Some(step),
            value_fn: Some(Rc::new(move || signal.get().to_string())),
            onincrement: Some(Callback::new(move || {
                signal.set((signal.get() + step).min(max));
            })),
            ondecrement: Some(Callback::new(move || {
                signal.set((signal.get() - step).max(min));
            })),
            oninput: clamp_input.then(|| {
                InputCallback::new(move |text: String| {
                    if let Ok(n) = text.parse::<f64>() {
                        signal.set(n.clamp(min, max));
                    }
                })
            }),
            ..Default::default()
        };
        let root = input.render(&mut scope, &[]);

        Self {
            _doc: doc,
            _scope: scope,
            root,
            signal,
        }
    }

    fn field(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-number-input__input").expect("the input field exists")
    }

    fn field_text(&self) -> String {
        self.field()
            .get_attribute("value")
            .expect("the field has a value")
    }

    fn handler(&self, class: &str, attr: &str) -> EventHandlerId {
        let node = find_by_class(&self.root, class).expect("element exists");
        EventHandlerId(
            node.get_attribute(attr)
                .expect("element carries a handler id")
                .parse()
                .expect("handler id is numeric"),
        )
    }

    /// Click a stepper button (`--up` or `--down`).
    fn click_stepper(&self, direction: &str) {
        dispatch_event(self.handler(
            &format!("rinch-number-input__control--{direction}"),
            "data-rid",
        ));
    }

    /// One keystroke, desktop-shaped: the runtime mirrors the field's text
    /// into the `value` attribute, then dispatches `oninput` with it.
    fn type_text(&self, text: &str) {
        self.field().set_attribute("value", text);
        dispatch_input_event(
            self.handler("rinch-number-input__input", "data-oninput"),
            text.to_string(),
        );
    }
}

fn find_by_class(node: &NodeHandle, class: &str) -> Option<NodeHandle> {
    let matches = node
        .get_attribute("class")
        .is_some_and(|attr| attr.split_whitespace().any(|c| c == class));
    if matches {
        return Some(node.clone());
    }
    node.children().iter().find_map(|c| find_by_class(c, class))
}

/// A programmatic `signal.set()` with no user interaction reaches the field —
/// the exact complaint of #264. The signal moves to a value the mount never
/// saw, so a field still reading its mount text cannot pass.
#[test]
fn a_programmatic_set_reaches_the_field() {
    let input = Input::controlled(5.0, 1.0, 0.0, 100.0, false);
    assert_eq!(input.field_text(), "5", "mount shows the signal's value");

    input.signal.set(42.0);

    assert_eq!(
        input.field_text(),
        "42",
        "a programmatic set must be reflected visually (#264)"
    );
}

/// `value_fn` takes precedence over the static `value` prop, exactly as in
/// `TextInput`: when both are given, the reactive binding owns the field.
#[test]
fn value_fn_wins_over_the_static_value_prop() {
    let signal = Signal::new(5.0);

    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);

    let input = NumberInput {
        value: Some(7.0),
        value_fn: Some(Rc::new(move || signal.get().to_string())),
        ..Default::default()
    };
    let root = input.render(&mut scope, &[]);

    let field = find_by_class(&root, "rinch-number-input__input").expect("the input field exists");
    assert_eq!(
        field.get_attribute("value").as_deref(),
        Some("5"),
        "the reactive binding owns the field when both props are given"
    );
}

/// A stepper click keeps the signal and the field in agreement: the callback
/// writes the signal and the `value_fn` effect carries it to the DOM. The
/// steppers themselves never touch the field, so this is the whole write path
/// — pre-fix the signal moved and the field stayed at its mount text.
#[test]
fn a_stepper_click_keeps_signal_and_field_in_agreement() {
    let input = Input::controlled(5.0, 1.0, 0.0, 6.0, false);

    input.click_stepper("up");
    assert_eq!(input.signal.get(), 6.0);
    assert_eq!(
        input.field_text(),
        "6",
        "the field follows the stepped signal"
    );

    // At max, the app clamps: the signal stays put and so does the field.
    input.click_stepper("up");
    assert_eq!(input.signal.get(), 6.0);
    assert_eq!(input.field_text(), "6", "a clamped step leaves both at max");

    input.click_stepper("down");
    assert_eq!(input.signal.get(), 5.0);
    assert_eq!(input.field_text(), "5", "the field follows a decrement too");
}

/// A clamping `oninput` that writes back on every keystroke — the case #238's
/// focused-write adoption exists for. Typing "42" mirrors the raw text into
/// the field; the handler clamps to 10 and sets the signal; the effect must
/// rewrite the field to the clamped value. The second "42" is the stronger
/// half: the signal is *already* 10, and `Signal::set` (unlike
/// `set_if_changed`) still notifies, so the write-back must fire again and
/// replace the freshly mirrored "42".
#[test]
fn a_clamping_oninput_writes_back_on_every_keystroke() {
    let input = Input::controlled(5.0, 1.0, 0.0, 10.0, true);

    input.type_text("42");
    assert_eq!(input.signal.get(), 10.0);
    assert_eq!(
        input.field_text(),
        "10",
        "the clamped value must replace the typed text"
    );

    input.type_text("42");
    assert_eq!(
        input.field_text(),
        "10",
        "an equal-value set still rewrites the mirrored text (#238)"
    );
}
