//! The four text components with a `value_fn` do not write back the echo of
//! the user's own keystroke (issue #238, part 2).
//!
//! The documented controlled pattern is `value_fn` + `oninput` on one signal,
//! so every keystroke runs keystroke → `oninput` → signal → `value_fn` effect →
//! `set_attribute("value")` with the very text the field already shows. The
//! effect now asks the field first (`NodeHandle::live_value`) and writes only
//! a value the field does not show — a normalising handler's rewrite, or a
//! programmatic change.
//!
//! Harness: the mock models the web's split through
//! `MockDomDocument::__type_into` — typing moves the live text and leaves the
//! `value` attribute at the last programmatic write. So the attribute is the
//! witness: it moves only when a write lands.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::number_input::NumberInput;
use rinch_components::password_input::PasswordInput;
use rinch_components::text_input::TextInput;
use rinch_components::textarea::Textarea;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_input_event};
use rinch_core::{Component, InputCallback, Signal};

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Password,
    Textarea,
    Number,
}

const KINDS: [Kind; 4] = [Kind::Text, Kind::Password, Kind::Textarea, Kind::Number];

struct Mounted {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    field: NodeHandle,
    signal: Signal<String>,
}

impl Mounted {
    /// A controlled component of `kind` bound to a signal holding `initial`,
    /// whose `oninput` stores `normalise(text)`.
    fn mount(kind: Kind, initial: &str, normalise: fn(&str) -> String) -> Self {
        let signal = Signal::new(initial.to_string());
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);

        let value_fn: Rc<dyn Fn() -> String> = Rc::new(move || signal.get());
        let oninput = InputCallback::new(move |text: String| signal.set(normalise(&text)));
        let (component, class): (Box<dyn Component>, &str) = match kind {
            Kind::Text => (
                Box::new(TextInput {
                    value_fn: Some(value_fn),
                    oninput: Some(oninput),
                    ..Default::default()
                }),
                "rinch-text-input__input",
            ),
            Kind::Password => (
                Box::new(PasswordInput {
                    value_fn: Some(value_fn),
                    oninput: Some(oninput),
                    ..Default::default()
                }),
                "rinch-password-input__input",
            ),
            Kind::Textarea => (
                Box::new(Textarea {
                    value_fn: Some(value_fn),
                    oninput: Some(oninput),
                    ..Default::default()
                }),
                "rinch-textarea__input",
            ),
            Kind::Number => (
                Box::new(NumberInput {
                    value_fn: Some(value_fn),
                    oninput: Some(oninput),
                    ..Default::default()
                }),
                "rinch-number-input__input",
            ),
        };
        let root = component.render(&mut scope, &[]);
        let field = find_by_class(&root, class).expect("the component's field");
        Self {
            doc,
            _scope: scope,
            field,
            signal,
        }
    }

    /// One keystroke the way a browser delivers it: the live text moves, the
    /// attribute does not, and `oninput` fires with the live text.
    fn type_text(&self, text: &str) {
        self.doc
            .borrow_mut()
            .__type_into(self.field.node_id(), text);
        let handler = EventHandlerId(
            self.field
                .get_attribute("data-oninput")
                .expect("the field routes input")
                .parse()
                .expect("numeric handler id"),
        );
        dispatch_input_event(handler, text.to_string());
    }

    fn attribute(&self) -> Option<String> {
        self.field.get_attribute("value")
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

fn as_typed(text: &str) -> String {
    text.to_string()
}

fn upper(text: &str) -> String {
    text.to_uppercase()
}

/// The echo: the signal ends up holding exactly what the field shows, so
/// nothing is written. The mount value is deliberately non-empty and different
/// from what is typed, so "the attribute did not move" cannot be confused
/// with "the attribute happens to equal the typed text".
#[test]
fn the_echo_of_a_keystroke_is_not_written_back() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "10", as_typed);
        m.type_text("12");

        assert_eq!(
            m.signal.get(),
            "12",
            "control: the keystroke reached oninput"
        );
        assert_eq!(m.field.live_value().as_deref(), Some("12"));
        assert_eq!(
            m.attribute().as_deref(),
            Some("10"),
            "the field already shows the signal's value; no write should land"
        );
    }
}

/// Not a freeze: a normalising handler's rewrite is a value the field does
/// not show, and it is written.
#[test]
fn a_normalised_rewrite_is_still_written() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "", upper);
        m.type_text("ab");

        assert_eq!(m.field.live_value().as_deref(), Some("AB"));
        assert_eq!(m.attribute().as_deref(), Some("AB"));
    }
}

/// Not a freeze either: a programmatic change the field does not show is
/// written, including one back to the attribute's own stale value — the
/// comparison is against the live text, never the attribute.
#[test]
fn a_programmatic_change_is_written_even_back_to_the_stale_attribute() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "10", as_typed);
        m.type_text("12");
        assert_eq!(m.attribute().as_deref(), Some("10"));

        m.signal.set("10".to_string());

        assert_eq!(
            m.field.live_value().as_deref(),
            Some("10"),
            "the field showed 12; a value_fn of 10 must reach it"
        );
    }
}

// ---- from the review of PR #863 ----

fn digits_only_keep(text: &str) -> String {
    // A rejecting handler cannot see the old value through `fn`, so this
    // variant keys on the test's mount value: anything non-digit collapses to
    // the digits it contains — "12a" -> "12", the value the signal held.
    text.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// The classic reject: the handler stores the SAME value the signal already
/// held (`Signal::set` notifies on an equal write). The field shows "12a",
/// so the effect must write "12" back — on every kind.
#[test]
fn a_rejected_keystroke_is_rewritten_to_the_unchanged_value() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "12", digits_only_keep);
        m.type_text("12a");
        assert_eq!(m.signal.get(), "12", "control: the handler rejected it");
        assert_eq!(
            m.field.live_value().as_deref(),
            Some("12"),
            "the field still shows the rejected keystroke"
        );
    }
}

/// A focused-desktop-shaped run: no `__type_into` split at all (the attribute
/// IS the live text, as on desktop), programmatic change is written.
#[test]
fn desktop_shape_programmatic_change_lands() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "10", as_typed);
        // desktop mirrors the text into the attribute before oninput
        m.field.set_attribute("value", "12");
        let handler = EventHandlerId(
            m.field
                .get_attribute("data-oninput")
                .unwrap()
                .parse()
                .unwrap(),
        );
        dispatch_input_event(handler, "12".to_string());
        assert_eq!(m.signal.get(), "12");
        m.signal.set("7".to_string());
        assert_eq!(m.attribute().as_deref(), Some("7"));
    }
}

fn trimmed(text: &str) -> String {
    text.trim().to_string()
}

/// A whitespace-only difference is a difference: a trimming handler's rewrite
/// must reach the field. Kills a guard that compares trimmed text.
#[test]
fn a_whitespace_only_rewrite_is_written() {
    for kind in KINDS {
        let m = Mounted::mount(kind, "", trimmed);
        m.type_text("ab ");
        assert_eq!(m.signal.get(), "ab");
        assert_eq!(m.field.live_value().as_deref(), Some("ab"));
    }
}
