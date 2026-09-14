//! Browser-driven tests for `data-trap-focus` (#474).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! The invariant: while a live `data-trap-focus` region is on the page, Tab and
//! Shift+Tab cycle **inside it** and the keydown's default is prevented, so the
//! browser's own focus order never runs. With no live trap, rinch touches
//! neither — web focus order is the browser's job and stays that way.
//!
//! **What a synthetic `keydown` can and cannot say.** An untrusted event runs no
//! native default action, so the browser will not move focus for it either way.
//! That makes the *positive* assertions strong — only rinch's own `el.focus()`
//! could have moved the focus — and it makes the negative ones read as "rinch
//! did not move it", which is precisely the claim: the untrapped path must leave
//! Tab alone. `default_prevented()` is asserted alongside, because that is the
//! half a trusted Tab would actually notice.
//!
//! **The desktop twin is `rinch/src/app/trap_focus_tests.rs`**, and it is a
//! twin and not a shared body of code: the focusable set is a tree walk there
//! and a CSS selector plus a filter here. The fixtures are deliberately the same
//! shape so the two can be read side by side.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-trap-focus-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope) -> NodeHandle + 'static) -> Self {
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    fn el(&self, id: &str) -> web_sys::HtmlElement {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into()
            .unwrap()
    }

    /// The `id` of whatever currently holds DOM focus, or `"<body>"`.
    fn active(&self) -> String {
        document()
            .active_element()
            .map(|el| {
                let id = el.id();
                if id.is_empty() {
                    format!("<{}>", el.tag_name().to_lowercase())
                } else {
                    id
                }
            })
            .unwrap_or_else(|| "<none>".to_string())
    }

    fn teardown(self) {
        self.root.unmount();
        self.host.remove();
    }
}

/// A `<button>` with a declared box. Web focusability needs a rendered box, and
/// declaring it keeps the fixture off the host's font metrics.
fn button(scope: &mut RenderScope, id: &str) -> NodeHandle {
    let b = scope.create_element("button");
    b.set_attribute("id", id);
    b.set_attribute("style", "display: block; width: 120px; height: 28px");
    let label = scope.create_text("B");
    b.append_child(&label);
    b
}

/// One control outside a trap region and two inside it, with `trap` written as
/// the region's `data-trap-focus` value (`None` leaves the attribute off).
fn trap_fixture(trap: Option<&'static str>, region_style: &'static str) -> Fixture {
    Fixture::mount(move |scope| {
        let root = scope.create_element("div");
        let outside = button(scope, "outside");
        root.append_child(&outside);

        let region = scope.create_element("div");
        region.set_attribute("id", "region");
        region.set_attribute("style", region_style);
        if let Some(value) = trap {
            region.set_attribute("data-trap-focus", value);
        }
        region.append_child(&button(scope, "in-a"));
        region.append_child(&button(scope, "in-b"));
        root.append_child(&region);

        let after = button(scope, "after");
        root.append_child(&after);
        root
    })
}

const VISIBLE: &str = "display: block; width: 400px; height: 120px";

/// A bubbling, cancellable `keydown` for Tab, dispatched at the focused element
/// so it travels the same path a real one does.
fn tab(shift: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key("Tab");
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let target: web_sys::Element = document()
        .active_element()
        .unwrap_or_else(|| document().body().unwrap().unchecked_into());
    target.dispatch_event(&ev).unwrap();
    ev
}

// ── containment ─────────────────────────────────────────────────────────────

/// Tab from the last control inside a live trap wraps to its first, never out
/// to the page.
///
/// **Mutant: `handle_trapped_tab` returning `false` unconditionally** (i.e. the
/// branch never added). Focus then stays on `in-b`, because an untrusted keydown
/// moves nothing by itself — so the assertion cannot be satisfied by the
/// browser doing the work.
#[wasm_bindgen_test]
fn tab_wraps_from_the_last_control_inside_a_trap_to_the_first() {
    let f = trap_fixture(Some(""), VISIBLE);
    f.el("in-b").focus().unwrap();
    assert_eq!(f.active(), "in-b", "precondition");

    let ev = tab(false);
    assert_eq!(f.active(), "in-a", "Tab must wrap inside the trap");
    assert!(
        ev.default_prevented(),
        "and the browser's own Tab must be suppressed"
    );
    f.teardown();
}

/// Shift+Tab from the first control inside wraps to the last inside.
#[wasm_bindgen_test]
fn shift_tab_wraps_from_the_first_control_inside_a_trap_to_the_last() {
    let f = trap_fixture(Some(""), VISIBLE);
    f.el("in-a").focus().unwrap();
    assert_eq!(f.active(), "in-a", "precondition");

    let ev = tab(true);
    assert_eq!(f.active(), "in-b", "Shift+Tab must wrap inside the trap");
    assert!(ev.default_prevented());
    f.teardown();
}

/// Tab from *outside* a live trap enters it at its first control, matching
/// desktop's `tab_from_outside_an_open_trap_enters_it`.
#[wasm_bindgen_test]
fn tab_from_outside_a_trap_enters_it() {
    let f = trap_fixture(Some(""), VISIBLE);
    f.el("outside").focus().unwrap();
    assert_eq!(f.active(), "outside", "precondition");

    tab(false);
    assert_eq!(f.active(), "in-a");
    f.teardown();
}

// ── when there is no trap ───────────────────────────────────────────────────

/// With no `data-trap-focus` anywhere, rinch does not touch Tab: focus is
/// unmoved and the default is not prevented, so a trusted Tab would run the
/// browser's own focus order.
///
/// **Mutant: `trap_root` falling back to the document when it finds no trap.**
/// Every page would then be a trap, which is a far worse bug than the one #474
/// fixes — and nothing else in this file would notice.
#[wasm_bindgen_test]
fn without_a_trap_tab_is_left_to_the_browser() {
    let f = trap_fixture(None, VISIBLE);
    f.el("in-b").focus().unwrap();

    let ev = tab(false);
    assert_eq!(f.active(), "in-b", "rinch must not have moved focus");
    assert!(
        !ev.default_prevented(),
        "and must leave the browser's Tab to run"
    );
    f.teardown();
}

/// `data-trap-focus="false"` is not a trap — rinch's own `"false"` escape, read
/// here by the selector `[data-trap-focus]:not([data-trap-focus="false" i])`
/// and on desktop by `rinch_core::dom::data_attr_is_on`. One rule, two
/// spellings, and this is the value that tells them apart from a presence-only
/// read.
///
/// `"0"` goes in beside it because it is the one value where the escape and the
/// writer's rule `attr_is_truthy` disagree: `"0"` is **on**. The desktop twin
/// asserts the same pair.
#[wasm_bindgen_test]
fn the_false_escape_opts_out_and_zero_does_not() {
    for (value, traps) in [("false", false), ("FALSE", false), ("0", true)] {
        let f = trap_fixture(Some(value), VISIBLE);
        f.el("in-b").focus().unwrap();

        tab(false);
        let expected = if traps { "in-a" } else { "in-b" };
        assert_eq!(
            f.active(),
            expected,
            "data-trap-focus={value:?} should {} trap",
            if traps { "" } else { "not" }
        );
        f.teardown();
    }
}

/// A trap with no box is not a trap — the state a closed `Modal` is in, whose
/// root is `display: none`.
///
/// The component also *removes* the attribute when it closes, so a real closed
/// overlay is guarded twice; this fixture keeps the attribute on purpose so the
/// visibility guard is tested on its own. **Mutant: dropping `element_is_visible`
/// from `trap_root`.** Tab would then be swallowed by an invisible region for
/// the rest of the session.
#[wasm_bindgen_test]
fn an_invisible_trap_is_not_a_trap() {
    let f = trap_fixture(Some(""), "display: none");
    f.el("outside").focus().unwrap();

    let ev = tab(false);
    assert_eq!(f.active(), "outside", "a boxless trap must be skipped");
    assert!(
        !ev.default_prevented(),
        "and Tab left to the browser entirely"
    );
    f.teardown();
}

// ── membership of the focusable set ─────────────────────────────────────────

/// A `disabled` control and a `tabindex="-1"` one inside a trap are not stops,
/// and a `tabindex="0"` div is.
///
/// Membership is where the two backends' sets are computed by different code —
/// a selector plus a filter here, a tree walk on desktop — so it is worth
/// pinning on each. `disabled` is read by **presence** (issue #612), so
/// `disabled="false"` is still disabled.
#[wasm_bindgen_test]
fn the_focusable_set_inside_a_trap_honours_disabled_and_negative_tabindex() {
    let f = Fixture::mount(move |scope| {
        let root = scope.create_element("div");
        let region = scope.create_element("div");
        region.set_attribute("data-trap-focus", "");
        region.set_attribute("style", VISIBLE);

        let first = button(scope, "first");
        let off = button(scope, "off");
        off.set_attribute("disabled", "false");
        let untabbable = button(scope, "untabbable");
        untabbable.set_attribute("tabindex", "-1");
        let custom = scope.create_element("div");
        custom.set_attribute("id", "custom");
        custom.set_attribute("tabindex", "0");
        custom.set_attribute("style", "display: block; width: 100px; height: 20px");

        region.append_child(&first);
        region.append_child(&off);
        region.append_child(&untabbable);
        region.append_child(&custom);
        root.append_child(&region);
        root
    });

    f.el("first").focus().unwrap();
    tab(false);
    assert_eq!(
        f.active(),
        "custom",
        "disabled=\"false\" is still disabled, and tabindex=-1 is not a stop"
    );
    tab(false);
    assert_eq!(f.active(), "first", "and the cycle wraps");
    f.teardown();
}

// ── the browser is the authority on focusability ────────────────────────────

/// A `<fieldset disabled>` inside a trap does not wedge forward Tab.
///
/// **The defect this pins was a regression, and its symptom was the exact
/// failure containment exists to prevent.** `trap_focusables` decides membership
/// from an element's *own* attributes; a `<button>` inside a disabled
/// `<fieldset>` carries none, so it landed in the list — and `focus()` on it is
/// refused by the browser. With the default prevented anyway, focus did not
/// move, the next press recomputed the same index from the same unchanged
/// `activeElement`, and **forward Tab was dead for the rest of the session**
/// (Shift+Tab still moved). Before this feature existed the browser's own Tab
/// ran and skipped the control correctly.
///
/// `Fieldset { disabled: true }` ships in the component library and writes a
/// bare `disabled`, so a form section disabled while saving, inside a `Modal`,
/// is the ordinary shape of it.
///
/// **Mutant: taking `items[target]` on trust** (the single unchecked `focus()`
/// this replaced). Both presses then answer `first` and the second assertion
/// dies. The `:disabled` half of the filter covers *this* case without a retry,
/// so the fixture below is the one that pins the retry itself.
#[wasm_bindgen_test]
fn a_disabled_fieldset_inside_a_trap_does_not_wedge_tab() {
    let f = Fixture::mount(move |scope| {
        let root = scope.create_element("div");
        let region = scope.create_element("div");
        region.set_attribute("data-trap-focus", "");
        region.set_attribute("style", VISIBLE);

        let fieldset = scope.create_element("fieldset");
        fieldset.set_attribute("disabled", "");
        fieldset.append_child(&button(scope, "mid"));

        region.append_child(&button(scope, "first"));
        region.append_child(&fieldset);
        region.append_child(&button(scope, "last"));
        root.append_child(&region);
        root
    });

    f.el("first").focus().unwrap();
    tab(false);
    assert_eq!(
        f.active(),
        "last",
        "the browser refuses focus inside a disabled fieldset, so Tab must \
         step over `mid` rather than stall on it"
    );
    tab(false);
    assert_eq!(f.active(), "first", "and the cycle still wraps");
    f.teardown();
}

/// The same class, reached a different way: a `tabindex` the browser and rinch
/// parse differently.
///
/// `<div tabindex="abc">` matches `[tabindex]`, and `"abc".parse::<i32>()`
/// fails — so `is_ok_and(|n| n < 0)` is false and the filter *keeps* it. Chrome
/// reports `tabIndex === -1` for it and refuses `focus()`. No filter can be
/// closed over this class, which is why the caller verifies the move instead.
///
/// **Mutant: the unchecked `focus()`.** Nothing but the retry saves this one —
/// `:disabled` does not match a div, and no attribute test would have caught a
/// value whose meaning is the browser's own to decide.
#[wasm_bindgen_test]
fn an_unfocusable_tabindex_inside_a_trap_does_not_wedge_tab() {
    let f = Fixture::mount(move |scope| {
        let root = scope.create_element("div");
        let region = scope.create_element("div");
        region.set_attribute("data-trap-focus", "");
        region.set_attribute("style", VISIBLE);

        let odd = scope.create_element("div");
        odd.set_attribute("id", "odd");
        odd.set_attribute("tabindex", "abc");
        odd.set_attribute("style", "display: block; width: 100px; height: 20px");

        region.append_child(&button(scope, "first"));
        region.append_child(&odd);
        region.append_child(&button(scope, "last"));
        root.append_child(&region);
        root
    });

    f.el("first").focus().unwrap();
    tab(false);
    assert_eq!(
        f.active(),
        "last",
        "an element the browser will not focus must not stall the cycle"
    );
    tab(false);
    assert_eq!(f.active(), "first");
    f.teardown();
}

// ── nesting ─────────────────────────────────────────────────────────────────

/// A nested trap that closes hands containment back to the outer one, with
/// focus still inside the closed subtree.
///
/// **This is the only web coverage of nesting, and the only fixture that
/// reaches `trap_root`'s rule 1 at all.** Every other fixture here has focus
/// either outside the trap or inside a trap that stays visible, so rule 1 either
/// `break`s at `closest` or finds a visible ancestor on its first look — and a
/// mutant that drops `element_is_visible` from **rule 1 only** survives all of
/// them. Here the nearest trap ancestor of the focused control is the one that
/// just went `display: none`, so stepping over it to the outer trap is the whole
/// behaviour.
///
/// The desktop twin is
/// `trap_focus_tests::closing_a_nested_trap_hands_containment_back_to_the_outer_one`.
#[wasm_bindgen_test]
fn closing_a_nested_trap_hands_containment_back_to_the_outer_one() {
    let f = Fixture::mount(move |scope| {
        let root = scope.create_element("div");
        let outer = scope.create_element("div");
        outer.set_attribute("id", "outer");
        outer.set_attribute("data-trap-focus", "");
        outer.set_attribute("style", VISIBLE);

        let inner = scope.create_element("div");
        inner.set_attribute("id", "inner");
        inner.set_attribute("data-trap-focus", "");
        inner.set_attribute("style", "display: block; width: 200px; height: 60px");
        inner.append_child(&button(scope, "in-a"));
        inner.append_child(&button(scope, "in-b"));

        outer.append_child(&button(scope, "out-a"));
        outer.append_child(&inner);
        outer.append_child(&button(scope, "out-b"));
        root.append_child(&outer);
        root
    });

    f.el("in-a").focus().unwrap();
    tab(false);
    assert_eq!(f.active(), "in-b", "precondition: the inner trap is live");

    // Close the inner overlay the way its component would: no box any more,
    // attribute or not.
    f.el("inner")
        .set_attribute("style", "display: none")
        .unwrap();

    tab(false);
    assert_eq!(
        f.active(),
        "out-a",
        "the outer trap takes over; the inner's two controls are gone"
    );
    tab(false);
    assert_eq!(f.active(), "out-b");
    tab(false);
    assert_eq!(f.active(), "out-a", "and the outer cycle wraps");
    f.teardown();
}
