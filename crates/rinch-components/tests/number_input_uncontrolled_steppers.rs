//! NumberInput: an uncontrolled instance's steppers write the field (#501).
//!
//! Pre-fix the stepper buttons only invoked the app's callbacks — with no
//! `value_fn`, clicking + or − ran `onincrement`/`ondecrement` and the number
//! on screen never moved. #264 fixed the controlled half (signal → effect →
//! DOM); this suite pins the uncontrolled half: the component owns the field
//! text, so a stepper click computes the next value from what the field shows,
//! clamps it to `[min, max]`, writes it, and reports the written text through
//! `oninput` — the same channel a keystroke reports through.
//!
//! The invariant #264 established is protected here too: when a `value_fn` IS
//! supplied it stays the field's single write path, and the stepper's internal
//! write is inert (`controlled_steppers_stay_callback_only`).
//!
//! Fixture note: mount values are chosen so a stepped field can never coincide
//! with its mount text — a fixture sitting on the one value where correct and
//! broken code agree passes vacuously. Every stepped assertion is on a value
//! the mount never held.
//!
//! Harness note: as in `number_input_value_fn.rs`, the desktop runtime mirrors
//! typed text into the `value` attribute before dispatching `oninput`, so
//! `type_text` does the same.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::number_input::NumberInput;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event, dispatch_input_event};
use rinch_core::{Callback, Component, InputCallback, Signal};

struct Fixture {
    // Kept alive for the test's duration: the document owns the nodes, the
    // scope owns the effects and handlers.
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
    /// Every callback invocation, in order: "increment", "decrement",
    /// "input:<text>".
    log: Rc<RefCell<Vec<String>>>,
}

impl Fixture {
    /// Mount the NumberInput the builder returns. The builder receives the
    /// shared log so callbacks can record into it.
    fn mount(build: impl FnOnce(Rc<RefCell<Vec<String>>>) -> NumberInput) -> Self {
        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(log.clone()).render(&mut scope, &[]);

        Self {
            _doc: doc,
            _scope: scope,
            root,
            log,
        }
    }

    /// The standard uncontrolled fixture: value 5, step 1, clamped to [0, 6],
    /// with all three callbacks recording into the log.
    fn uncontrolled() -> Self {
        Self::mount(|log| {
            let inc = log.clone();
            let dec = log.clone();
            let inp = log.clone();
            NumberInput {
                value: Some(5.0),
                min: Some(0.0),
                max: Some(6.0),
                step: Some(1.0),
                onincrement: Some(Callback::new(move || {
                    inc.borrow_mut().push("increment".into());
                })),
                ondecrement: Some(Callback::new(move || {
                    dec.borrow_mut().push("decrement".into());
                })),
                oninput: Some(InputCallback::new(move |text: String| {
                    inp.borrow_mut().push(format!("input:{text}"));
                })),
                ..Default::default()
            }
        })
    }

    fn field(&self) -> NodeHandle {
        find_by_class(&self.root, "rinch-number-input__input").expect("the input field exists")
    }

    fn field_text(&self) -> String {
        self.field().get_attribute("value").unwrap_or_default()
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

/// The headline fix: with no `value_fn`, a stepper click moves the displayed
/// number. Kills the pre-#501 mutant where the stepper handler only invokes
/// the callback and never touches the field — there the field still reads its
/// mount text "5" after every click.
#[test]
fn an_uncontrolled_stepper_click_moves_the_field() {
    let f = Fixture::uncontrolled();
    assert_eq!(f.field_text(), "5", "mount shows the static value");

    f.click_stepper("up");
    assert_eq!(
        f.field_text(),
        "6",
        "increment writes a value the mount never held"
    );

    f.click_stepper("down");
    f.click_stepper("down");
    assert_eq!(
        f.field_text(),
        "4",
        "decrement steps back down through the mount value"
    );
}

/// The write is reported through `oninput` — after the `onincrement`
/// notification, and carrying exactly the text written. Kills three mutants:
/// no report at all (an app tracking the number in a signal via `oninput`
/// silently diverges from the field), the report firing before the
/// notification (an app doing arithmetic in `onincrement` and syncing in
/// `oninput` would end on the pre-click value), and reporting an unclamped or
/// unformatted text that differs from the field.
#[test]
fn an_uncontrolled_step_reports_the_written_text_through_oninput() {
    let f = Fixture::uncontrolled();

    f.click_stepper("up");

    assert_eq!(
        f.log.borrow().as_slice(),
        ["increment".to_string(), "input:6".to_string()],
        "notification first, then the written text through oninput"
    );
}

/// A stepper click beyond `max` (and `min`) is a clamped no-op: the field
/// keeps the boundary value, and no `oninput` fires for a write that did not
/// happen — HTML's input event fires only when the value changes.
/// `onincrement` still fires (it always has; it is a notification, not a
/// value). Kills the mutant that writes unclamped arithmetic ("7") and the
/// one that reports an unchanged value.
#[test]
fn stepping_clamps_at_max_and_min() {
    let f = Fixture::uncontrolled();

    f.click_stepper("up");
    f.click_stepper("up");
    assert_eq!(f.field_text(), "6", "the second click at max=6 is clamped");
    let inputs: Vec<String> = f
        .log
        .borrow()
        .iter()
        .filter(|e| e.starts_with("input:"))
        .cloned()
        .collect();
    assert_eq!(
        inputs,
        ["input:6".to_string()],
        "the clamped click writes nothing, so it reports nothing"
    );

    for _ in 0..7 {
        f.click_stepper("down");
    }
    assert_eq!(
        f.field_text(),
        "0",
        "the seventh decrement from 6 is clamped at min=0"
    );
}

/// An uncontrolled NumberInput with NO callbacks at all still steps — the
/// docs' own bare example (`NumberInput { label: "Quantity" }`). Kills the
/// mutant that keeps handler registration gated on `onincrement`/
/// `ondecrement` being present: there the buttons carry no `data-rid` and
/// this test panics looking one up.
#[test]
fn a_stepper_with_no_callbacks_still_steps() {
    let f = Fixture::mount(|_| NumberInput {
        value: Some(5.0),
        step: Some(1.0),
        ..Default::default()
    });

    f.click_stepper("up");
    assert_eq!(f.field_text(), "6");
}

/// Typing moves the stepping base: after the user types "3", + must write
/// "4", not step the mount value to "6". An unparseable partial ("3x") leaves
/// the base where it was. Kills the mutant whose internal record is seeded at
/// mount and never hears about typed edits.
#[test]
fn a_typed_value_becomes_the_stepping_base() {
    let f = Fixture::uncontrolled();

    f.type_text("3");
    f.click_stepper("up");
    assert_eq!(
        f.field_text(),
        "4",
        "steps from the typed 3, not the mount 5"
    );

    // The base is now the stepped 4. "2x" must not move it — not to its
    // parsed prefix 2 (down would write 1), not to an empty base (down would
    // clamp to 0), not back to the mount 5 (down would write 4).
    f.type_text("2x");
    f.click_stepper("down");
    assert_eq!(
        f.field_text(),
        "3",
        "an unparseable partial does not move the base: 4 - 1"
    );
}

/// THE invariant (#264, re-pinned for #501): when a `value_fn` is supplied it
/// stays the field's single write path, and the stepper's internal write is
/// inert. Both are wired here, and the callback deliberately sets the signal
/// to 50 — NOT mount+step — so the assertion distinguishes "the field follows
/// the signal" (50) from "the stepper's own arithmetic reached the field"
/// (6, or a post-callback overwrite of 50). Proven to fail against the mutant
/// with the `controlled` gate removed from the stepper write.
#[test]
fn controlled_steppers_stay_callback_only() {
    let signal = Signal::new(5.0);

    let f = Fixture::mount(|log| {
        let inp = log.clone();
        NumberInput {
            value_fn: Some(Rc::new(move || signal.get().to_string())),
            min: Some(0.0),
            max: Some(100.0),
            step: Some(1.0),
            onincrement: Some(Callback::new(move || signal.set(50.0))),
            oninput: Some(InputCallback::new(move |text: String| {
                inp.borrow_mut().push(format!("input:{text}"));
            })),
            ..Default::default()
        }
    });

    f.click_stepper("up");

    assert_eq!(signal.get(), 50.0);
    assert_eq!(
        f.field_text(),
        "50",
        "the field follows the signal, not the stepper's own arithmetic"
    );
    assert!(
        f.log.borrow().is_empty(),
        "a controlled stepper writes nothing, so it reports nothing through oninput"
    );
}

/// A disabled NumberInput's steppers do not write the field. (The callbacks
/// still fire, as they always have — changing that is #315's territory, not
/// this test's.) Proven to fail against the mutant with the `disabled` gate
/// removed from the stepper write.
#[test]
fn a_disabled_uncontrolled_stepper_does_not_write() {
    let f = Fixture::mount(|log| {
        let inc = log.clone();
        NumberInput {
            value: Some(5.0),
            step: Some(1.0),
            disabled: true,
            onincrement: Some(Callback::new(move || {
                inc.borrow_mut().push("increment".into());
            })),
            ..Default::default()
        }
    });

    f.click_stepper("up");
    assert_eq!(
        f.field_text(),
        "5",
        "a disabled field's number does not move"
    );
}

/// `default_value` seeds an uncontrolled field whose `value` is absent — both
/// the mount text and the stepping base. Kills the mutant that leaves the
/// prop dead (mount shows nothing, and the first step writes "1" from an
/// empty base instead of "8").
#[test]
fn default_value_seeds_the_uncontrolled_field() {
    let f = Fixture::mount(|_| NumberInput {
        default_value: Some(7.0),
        step: Some(1.0),
        ..Default::default()
    });

    assert_eq!(f.field_text(), "7", "default_value reaches the mount text");
    f.click_stepper("up");
    assert_eq!(f.field_text(), "8", "and is the stepping base");
}

/// An empty field (no `value`, no `default_value`) steps from 0, clamped into
/// range — matching the browser's spinner on an empty `<input type=number>`:
/// with min=5, the first + lands on 5, not 6. Kills the mutant seeding the
/// base from `min` (which would write 6).
#[test]
fn an_empty_field_steps_from_zero_clamped_into_range() {
    let f = Fixture::mount(|_| NumberInput {
        min: Some(5.0),
        step: Some(1.0),
        ..Default::default()
    });

    assert_eq!(
        f.field_text(),
        "",
        "no value, no default: the field mounts empty"
    );
    f.click_stepper("up");
    assert_eq!(f.field_text(), "5", "0 + 1 clamped up to min");
}

/// `decimal_scale` fixes what the component itself writes — the mount text
/// and every stepper write. Kills the raw `to_string` mutant, whose step from
/// 9.99 by 0.01 writes the float dust "10.000000000000002".
#[test]
fn decimal_scale_formats_mount_and_stepped_writes() {
    let f = Fixture::mount(|_| NumberInput {
        value: Some(10.0),
        min: Some(0.0),
        step: Some(0.01),
        decimal_scale: Some(2),
        ..Default::default()
    });

    assert_eq!(f.field_text(), "10.00", "the mount text carries the scale");
    f.click_stepper("down");
    assert_eq!(f.field_text(), "9.99");
    f.click_stepper("up");
    assert_eq!(
        f.field_text(),
        "10.00",
        "9.99 + 0.01 is written scaled, not as float dust"
    );
}

/// Without `decimal_scale`, repeated ±step arithmetic still cannot surface
/// float dust: 0.2 + 0.1 is written "0.3", not "0.30000000000000004". Kills
/// the mutant that writes the raw f64 text.
#[test]
fn stepped_writes_do_not_surface_float_dust() {
    let f = Fixture::mount(|_| NumberInput {
        value: Some(0.2),
        step: Some(0.1),
        ..Default::default()
    });

    f.click_stepper("up");
    assert_eq!(f.field_text(), "0.3");
}
