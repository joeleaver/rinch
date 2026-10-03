//! `Textarea::max_rows` and `Textarea::autosize` (issue #715).
//!
//! `prop_wiring_707.rs` used to say `Textarea::max_rows` was not among the
//! props it checks, because it was wired once and reverted: a `max-height`
//! could not bind on a rinch `<textarea>`, whose used height was exactly a
//! `min-height` its `rows` and the stylesheet's size floor gave it (CSS
//! resolves `min-height` over `max-height` whenever the two conflict). #715
//! turned that `min-height` into a Taffy *measure* instead (`#297`/`#1152`),
//! which is what finally lets a `max-height` bind — these fixtures pin the
//! two-line component change that follows: `max_rows` writes the cap, and
//! `autosize` keeps `rows` tracking the controlled value's line count between
//! `min_rows` and `max_rows`.
//!
//! The pixel-level proof that the cap actually *lays out* smaller — not just
//! that the style string is written — lives in `rinch`'s
//! `app::textarea_max_rows_tests`, which mounts the real component under the
//! real stylesheets. This file is cheaper: it is a mock-backed check that the
//! prop reaches an attribute/style at all, so a future refactor that reads the
//! prop into a value it then discards (the #474 trap) fails here without a
//! full `RinchApp`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::textarea::Textarea;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_input_event};
use rinch_core::{Component, InputCallback, Signal};

mod common;
use common::find_by_class;

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    field: NodeHandle,
}

fn mount(textarea: Textarea) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = textarea.render(&mut scope, &[]);
    let field = find_by_class(&root, "rinch-textarea__input").expect("the control renders");
    Mounted {
        _doc: doc,
        _scope: scope,
        field,
    }
}

/// With no `max_rows`, nothing writes `max-height` — the prop stays silent
/// when unset, as every other optional style prop does.
#[test]
fn with_no_max_rows_nothing_caps_the_height() {
    let m = mount(Textarea::default());
    let style = m.field.get_attribute("style").unwrap_or_default();
    assert!(
        !style.contains("max-height"),
        "no max_rows was given: {style:?}"
    );
}

/// `max_rows` writes a `max-height` naming that many rows — the two-line
/// change issue #715 predicted, now that the DOM layer can honour it.
///
/// Mutant this kills: wiring `self.max_rows` into a local the render never
/// writes anywhere (read, then discarded) leaves `style` with no
/// `max-height` at all — the exact #474 shape this allowlist entry used to
/// flag.
#[test]
fn max_rows_writes_a_max_height_naming_the_row_count() {
    let m = mount(Textarea {
        max_rows: Some(5),
        ..Default::default()
    });
    let style = m.field.get_attribute("style").unwrap_or_default();
    assert!(
        style.contains("max-height") && style.contains("1.2em * 5"),
        "expected a 5-row max-height, got {style:?}"
    );
}

/// Without `autosize`, `rows` is `min_rows` (or absent) and the value never
/// moves it — the pre-#715 behaviour, unchanged.
#[test]
fn without_autosize_rows_does_not_follow_the_value() {
    let signal = Signal::new("one\ntwo\nthree".to_string());
    let value_fn: Rc<dyn Fn() -> String> = Rc::new(move || signal.get());
    let m = mount(Textarea {
        value_fn: Some(value_fn),
        ..Default::default()
    });
    assert_eq!(m.field.get_attribute("rows"), None);
}

/// `autosize` makes `rows` follow the controlled value's line count, clamped
/// to `[min_rows, max_rows]`, on every change — not just at mount.
///
/// Mutant this kills: computing the initial `rows` from the value but never
/// registering the effect (so only the *first* assertion here would pass) —
/// the second `dispatch_input_event` is what a "wired once, not reactively"
/// mistake fails on.
#[test]
fn autosize_tracks_the_value_between_min_and_max_rows() {
    let signal = Signal::new("one line".to_string());
    let value_fn: Rc<dyn Fn() -> String> = Rc::new(move || signal.get());
    let oninput = InputCallback::new(move |text: String| signal.set(text));
    let m = mount(Textarea {
        autosize: true,
        min_rows: Some(2),
        max_rows: Some(4),
        value_fn: Some(value_fn),
        oninput: Some(oninput),
        ..Default::default()
    });
    // One line, floored to min_rows.
    assert_eq!(m.field.get_attribute("rows"), Some("2".to_string()));

    let input_id = EventHandlerId(
        m.field
            .get_attribute("data-oninput")
            .expect("the field registers an input handler")
            .parse()
            .expect("a numeric handler id"),
    );

    // Three lines fits between the floor and the cap.
    assert!(dispatch_input_event(input_id, "a\nb\nc".to_string()));
    assert_eq!(m.field.get_attribute("rows"), Some("3".to_string()));

    // Six lines is capped at max_rows.
    assert!(dispatch_input_event(input_id, "1\n2\n3\n4\n5\n6".to_string()));
    assert_eq!(m.field.get_attribute("rows"), Some("4".to_string()));
}
