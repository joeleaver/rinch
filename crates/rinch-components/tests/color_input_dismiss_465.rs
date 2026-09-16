//! `ColorInput` dismisses its dropdown on an outside click and on Escape
//! (issue #465).
//!
//! Until this change the picker was dismissed **only** by clicking the field
//! again: the component mounted no backdrop and registered nothing on the
//! dismiss stack, so a click anywhere else on the page left it open. #263 had
//! declared a `close_on_click_outside` prop for exactly this and *removed* it
//! rather than grow a new interaction inside a cleanup, which is why the prop
//! comes back here together with the behaviour it names.
//!
//! What these fixtures pin, and what each is here to stop:
//!
//! - **The backdrop's handler closes; it never toggles.** A toggle passes every
//!   "outside click closes it" assertion and then *opens* the dropdown on an
//!   outside click while it is closed — the state the backdrop is in for most
//!   of the component's life. `an_outside_click_on_a_closed_dropdown_is_inert`
//!   is the one that can tell them apart.
//! - **The field keeps its own toggle.** The backdrop is one z-level under the
//!   panel and the field is lifted one *above* it, so clicking into the text to
//!   place a caret still reaches the field. Losing that would be invisible here
//!   — the mock has no CSS — so the desktop twin
//!   (`rinch/src/app/color_input_dismiss_465_tests.rs`) is what measures it.
//! - **Off means off.** `close_on_click_outside: false` mounts no backdrop at
//!   all, which is the only spelling that preserves the pre-#465 behaviour.
//! - **The classes reach a rule that declares something.** #263's lesson: a
//!   prop that reaches the DOM and no stylesheet is the same defect wearing a
//!   different hat.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::color_input::ColorInput;
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dismiss_handler_count, dispatch_dismiss, dispatch_event};

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Mounted {
    fn new(input: ColorInput) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = input.render(&mut scope, &[]);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    fn find(&self, class: &str) -> Option<NodeHandle> {
        find_by_class(&self.root, class)
    }

    /// Fire the handler on the node carrying `class`.
    fn click(&self, class: &str) {
        let node = self.find(class).expect("the element exists");
        let id: usize = node
            .get_attribute("data-rid")
            .expect("the element carries a handler id")
            .parse()
            .expect("the handler id is numeric");
        dispatch_event(EventHandlerId(id));
    }

    fn is_open(&self) -> bool {
        self.root.get_attribute("class").is_some_and(|c| {
            c.split_whitespace()
                .any(|c| c == "rinch-color-input--opened")
        })
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

const BACKDROP: &str = "rinch-color-input__backdrop";
const FIELD: &str = "rinch-color-input__input-group";

// ------------------------------------------------------- the interaction

/// The fault: with the dropdown open, a click outside it closed nothing.
#[test]
fn an_outside_click_closes_the_dropdown() {
    let m = Mounted::new(ColorInput::default());

    m.click(FIELD);
    assert!(m.is_open(), "precondition: the field opened it");

    m.click(BACKDROP);
    assert!(
        !m.is_open(),
        "a click on the backdrop must close the dropdown"
    );
}

/// The field keeps the toggle it has always had — opening *and* closing.
///
/// This is the test #263 wrote as
/// `the_field_toggles_the_dropdown_and_is_the_only_way_to_dismiss_it`, minus
/// the clause that is no longer true.
#[test]
fn the_field_still_toggles_the_dropdown() {
    let m = Mounted::new(ColorInput::default());

    m.click(FIELD);
    assert!(m.is_open());
    m.click(FIELD);
    assert!(!m.is_open(), "the field still closes it too");
}

/// **The backdrop closes, it never toggles.**
///
/// The backdrop is mounted for the component's whole life and is only
/// *revealed* while the dropdown is open, so on the mock — which has no CSS and
/// therefore no `display: none` — its handler is reachable while closed. A
/// handler spelled `opened.update(|v| *v = !*v)`, copied from the field's, would
/// satisfy every other fixture in this file and open the picker here.
///
/// On a real backend the hidden backdrop takes no clicks, so this is a claim
/// about the handler rather than about a gesture a user can make — which is
/// exactly why it needs saying: nothing else in the suite would notice.
#[test]
fn an_outside_click_on_a_closed_dropdown_is_inert() {
    let m = Mounted::new(ColorInput::default());
    assert!(!m.is_open(), "precondition: it starts closed");

    m.click(BACKDROP);
    assert!(
        !m.is_open(),
        "the backdrop must set `opened` to false, not toggle it"
    );

    // And still inert on the second one — a handler that toggled would be back
    // to closed here, agreeing with the assertion above for the wrong reason.
    m.click(BACKDROP);
    assert!(!m.is_open());
}

/// Escape closes the dropdown, through the dismiss stack (#474/#671).
///
/// LIFO puts this input's handler on top of whatever an enclosing `Modal` or
/// `Drawer` registered earlier, so the key reaches the picker first.
#[test]
fn escape_closes_the_dropdown_and_a_closed_one_leaves_the_key_alone() {
    let m = Mounted::new(ColorInput::default());

    assert!(
        !dispatch_dismiss(),
        "a closed ColorInput must not swallow Escape — the app, or a modal \
         behind it, still gets the key"
    );

    m.click(FIELD);
    assert!(m.is_open(), "precondition: the field opened it");

    assert!(dispatch_dismiss(), "an open one consumes the key");
    assert!(!m.is_open(), "and closes");
}

/// **The dismiss entry is pushed when the dropdown opens, not when the input
/// mounts** — the #671 shape, and the whole of `Modal { ColorInput { … } }`
/// working.
///
/// A component renders *after* its children, so a mount-time registration would
/// sit under the modal's and the modal would answer Escape while the picker was
/// the thing on screen. That ordering is measured for real in
/// `rinch/src/app/color_input_dismiss_465_tests.rs`; what this fixture adds is
/// the mechanism it rests on, which nothing else here can see — with a
/// mount-time registration `escape_closes_the_dropdown_and_a_closed_one_leaves_the_key_alone`
/// passes unchanged, because a closed overlay declines the key either way.
///
/// Counted as **deltas**: the stack is one thread-local shared by every test in
/// this binary, so an absolute count would pin whatever ran before this.
#[test]
fn the_dismiss_entry_is_pushed_on_open_and_released_on_close() {
    let before = dismiss_handler_count();
    let m = Mounted::new(ColorInput::default());
    assert_eq!(
        dismiss_handler_count(),
        before,
        "mounting a ColorInput must register nothing — a closed picker is not \
         on the dismiss stack at all"
    );

    m.click(FIELD);
    assert_eq!(
        dismiss_handler_count(),
        before + 1,
        "opening the dropdown pushes exactly one entry, on top of whatever \
         overlay is already there"
    );

    m.click(FIELD);
    assert_eq!(
        dismiss_handler_count(),
        before,
        "and closing releases it, so the key falls back to whatever is behind"
    );
}

// ------------------------------------------------------------- the prop

/// `close_on_click_outside` defaults to `true`: the library's other popovers
/// (`DropdownMenu`, `Popover`, `Modal`, `Drawer`) all dismiss on an outside
/// click, and an input that does not is the odd one out.
#[test]
fn close_on_click_outside_is_on_by_default() {
    assert!(ColorInput::default().close_on_click_outside);
    assert!(
        Mounted::new(ColorInput::default()).find(BACKDROP).is_some(),
        "the default mounts a backdrop"
    );
}

/// Off mounts no backdrop at all, which is the spelling that preserves the
/// pre-#465 behaviour: the field is the only way in and out.
///
/// Not decoration — without it the suite passes against a fix that ignores the
/// prop and dismisses unconditionally, which is a different bug with the same
/// headline (`overlay_dismiss_tests`' phrasing of the same rule).
#[test]
fn close_on_click_outside_false_mounts_no_backdrop() {
    let m = Mounted::new(ColorInput {
        close_on_click_outside: false,
        ..Default::default()
    });
    assert!(
        m.find(BACKDROP).is_none(),
        "close_on_click_outside: false must mount no backdrop"
    );

    m.click(FIELD);
    assert!(m.is_open());
    m.click(FIELD);
    assert!(!m.is_open(), "the field is still the way in and out");
}

/// The backdrop is a child of the *wrapper*, not of the root, and it is a
/// **direct** child of it.
///
/// Both halves are load-bearing for the reveal rule, which is spelled with a
/// child combinator (`… .rinch-color-input__wrapper > .…__backdrop`). The
/// component appends it itself, so unlike a caller's child it can never grow a
/// `display: contents` wrapper in front of it — the #774 hazard — and the `>`
/// costs nothing while keeping the rule from reaching any backdrop but this
/// input's own.
#[test]
fn the_backdrop_is_a_direct_child_of_the_wrapper() {
    let m = Mounted::new(ColorInput::default());
    let wrapper = m
        .find("rinch-color-input__wrapper")
        .expect("the wrapper exists");
    assert!(
        wrapper.children().iter().any(|c| c
            .get_attribute("class")
            .is_some_and(|a| a.split_whitespace().any(|c| c == BACKDROP))),
        "the backdrop must be a direct child of the wrapper"
    );
}

// -------------------------------------- the classes reach the stylesheet

/// The backdrop must be hidden by default, revealed by the `--opened` class,
/// and ordered *under* the dropdown panel — otherwise it covers the picker and
/// every click inside the dropdown dismisses instead of picking a colour.
///
/// Declarations rather than mere selector presence, for the reason
/// `color_input_props.rs` gives: a class can be named by a rule that gives it
/// nothing, and a presence-only check stays green through that.
#[test]
fn the_backdrop_rules_are_in_the_shipped_stylesheet() {
    let css = strip_comments(&rinch_components::styles::generate_all_component_styles());

    let base = declarations_for(&css, &format!(".{BACKDROP}"));
    assert!(
        base.iter().any(|d| d == "display: none"),
        "the backdrop must start hidden; got {base:?}"
    );
    assert!(
        base.iter().any(|d| d == "position: fixed"),
        "the backdrop must be `fixed`, so \"outside\" means the window and not \
         whatever clips the field — the reason is in the long note above \
         `.rinch-dropdown-menu__backdrop`. Got {base:?}"
    );

    let opened = declarations_for(
        &css,
        &format!(".rinch-color-input--opened > .rinch-color-input__wrapper > .{BACKDROP}"),
    );
    assert!(
        opened.iter().any(|d| d == "display: block"),
        "the `--opened` class must reveal the backdrop; got {opened:?}"
    );

    // Ordering, read off the sheet: backdrop < panel < field.
    let panel_z = z_index_of(&css, ".rinch-color-input__dropdown");
    let backdrop_z = z_index_of(&css, &format!(".{BACKDROP}"));
    let field_z = z_index_of(&css, &format!(".rinch-color-input--opened .{FIELD}"));
    assert!(
        backdrop_z < panel_z,
        "the backdrop ({backdrop_z}) must sit under the panel ({panel_z}), or a \
         click inside the dropdown dismisses instead of picking"
    );
    assert!(
        field_z > backdrop_z,
        "the field ({field_z}) must sit above the backdrop ({backdrop_z}), or \
         clicking into the text to place a caret is swallowed by the dismissal"
    );
}

/// The one `z-index` an exactly-`selector` rule declares, as an integer.
fn z_index_of(css: &str, selector: &str) -> i32 {
    let decls = declarations_for(css, selector);
    let z = decls
        .iter()
        .find_map(|d| d.strip_prefix("z-index: "))
        .unwrap_or_else(|| panic!("`{selector}` declares no z-index; got {decls:?}"));
    z.trim()
        .parse()
        .unwrap_or_else(|_| panic!("`{selector}` has a non-integer z-index {z:?}"))
}

/// Every declaration in every rule whose selector list contains exactly
/// `selector`, whitespace-normalized and lowercased.
fn declarations_for(css: &str, selector: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let prelude = rest[..open].rsplit('}').next().unwrap_or("").trim();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + close];
        let matches = prelude
            .split(',')
            .any(|s| s.split_whitespace().collect::<Vec<_>>().join(" ") == selector);
        if matches {
            found.extend(
                body.split(';')
                    .map(|d| {
                        d.split_whitespace()
                            .collect::<Vec<_>>()
                            .join(" ")
                            .to_lowercase()
                    })
                    .filter(|d| !d.is_empty()),
            );
        }
        rest = &rest[open + close + 1..];
    }
    found
}

/// Strip `/* ... */` comments before parsing. Load-bearing: without it the
/// comment introducing a rule is swept into that rule's prelude and stops the
/// selector matching.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find("*/") {
            Some(end) => rest = &rest[start + 2 + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}
