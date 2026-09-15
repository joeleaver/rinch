//! Browser-driven tests for overlay focus move + restore (#695).
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test overlay_focus
//! ```
//!
//! **The desktop twin is `rinch/src/app/overlay_focus_tests.rs`**, fixture for
//! fixture. What is shared between the two backends is the *policy* — the
//! component's `overlay_focus::arm_overlay_focus` — and what differs is
//! everything it rests on: reading the current focus, blurring, testing
//! attachment, and picking "the first focusable". Each of those is one
//! `DomDocument` method per backend, so a twin is the only way to say that both
//! halves work.
//!
//! **Unlike desktop these assertions need no pumping.** The browser lays out on
//! demand, so `WebDocument::focus_into` answers on the spot where desktop has to
//! park the request until after the next layout pass. That asymmetry is the one
//! thing a reader should carry across from the desktop file.
//!
//! Focus here is real browser focus, moved by rinch's own `el.focus()` — no
//! synthetic event is involved, so nothing is weakened by the untrusted-event
//! caveat that `trap_focus.rs` has to carry.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch::components::{Modal, Popover};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, Signal};
use rinch_web::RootHandle;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-overlay-focus-test-host";

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
        // A test that left focus on one of its own controls would otherwise
        // hand the next test a stale `activeElement` to capture as an opener.
        if let Some(active) = document()
            .active_element()
            .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
        {
            active.blur().ok();
        }
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
        Self { root, host }
    }

    /// The `id` of whatever currently holds DOM focus, or `<body>`.
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

    fn focus(&self, id: &str) {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .focus()
            .unwrap();
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

/// An opener button, a `Modal` with two controls inside, and a second page
/// button to move focus to.
///
/// `autofocus_last` puts HTML's `autofocus` on the **last** control, so "the
/// first focusable" and "the `autofocus` one" are different nodes.
fn modal_page(open: Signal<bool>, trap_focus: bool, autofocus_last: bool) -> Fixture {
    Fixture::mount(move |scope| {
        let page = scope.create_element("div");
        page.append_child(&button(scope, "opener"));

        let in_first = button(scope, "in-first");
        let in_last = button(scope, "in-last");
        if autofocus_last {
            in_last.set_attribute("autofocus", "");
        }
        let modal = Modal {
            opened_fn: Some(Rc::new(move || open.get())),
            trap_focus,
            // Off so the only stops inside are the fixture's own two; the close
            // button is a `<button>` and would otherwise be the first.
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[in_first, in_last]);
        page.append_child(&modal);

        page.append_child(&button(scope, "elsewhere"));
        page
    })
}

// ── 1. moving focus in ──────────────────────────────────────────────────────

/// Opening a modal takes the keyboard off the opener and puts it on the first
/// control inside.
///
/// **Mutant: `WebDocument::focus_into` returning without focusing anything.**
/// Focus stays on `opener` — nothing else on this page would move it, since no
/// event is dispatched at all.
#[wasm_bindgen_test]
fn opening_a_modal_moves_focus_to_its_first_control() {
    let open = Signal::new(false);
    let f = modal_page(open, true, false);
    f.focus("opener");
    assert_eq!(f.active(), "opener", "precondition");

    open.set(true);
    assert_eq!(f.active(), "in-first");
    f.teardown();
}

/// An `autofocus` control inside wins over the first one.
///
/// **Mutant: the `autofocus` lookup in `WebDocument::focus_into` deleted.**
/// Focus lands on `in-first` instead.
#[wasm_bindgen_test]
fn an_autofocus_control_inside_wins_over_the_first_one() {
    let open = Signal::new(false);
    let f = modal_page(open, true, true);

    open.set(true);
    assert_eq!(f.active(), "in-last");
    f.teardown();
}

/// `trap_focus: false` moves nothing — rinch's spelling of a non-modal `show()`.
#[wasm_bindgen_test]
fn trap_focus_false_moves_no_focus() {
    let open = Signal::new(false);
    let f = modal_page(open, false, false);
    f.focus("opener");

    open.set(true);
    assert_eq!(f.active(), "opener");
    f.teardown();
}

// ── 2. giving it back ───────────────────────────────────────────────────────

/// Closing hands the keyboard back to whatever held it when the modal opened.
///
/// **Mutant: the effect's close arm deleted.** Focus stays on `in-first`, inside
/// a `display: none` subtree — which in a browser is worse than on desktop,
/// because the browser itself then drops focus to `<body>` at some later point
/// of its own choosing.
#[wasm_bindgen_test]
fn closing_a_modal_restores_the_opener() {
    let open = Signal::new(false);
    let f = modal_page(open, true, false);
    f.focus("opener");

    open.set(true);
    assert_eq!(f.active(), "in-first", "precondition: inside");

    open.set(false);
    assert_eq!(f.active(), "opener");
    f.teardown();
}

/// An opener the dialog itself removed is not focused back into.
///
/// **Mutant: `restore`'s `is_connected` guard deleted.** `HTMLElement.focus()`
/// on a detached element is a no-op in the browser, so the *visible* outcome
/// happens to be the same here — which is exactly why this fixture asserts the
/// blur half too: without the guard the `restore` returns early and the claim
/// is never released, leaving `activeElement` on the control inside the closed
/// overlay.
#[wasm_bindgen_test]
fn an_opener_removed_while_the_modal_was_open_is_not_restored() {
    let open = Signal::new(false);
    let f = modal_page(open, true, false);
    f.focus("opener");

    open.set(true);
    assert_eq!(f.active(), "in-first", "precondition: inside");

    document().get_element_by_id("opener").unwrap().remove();
    open.set(false);
    assert_eq!(
        f.active(),
        "<body>",
        "a detached opener must not be focused; the claim is released"
    );
    f.teardown();
}

/// Focus the user moved out of the overlay before it closed is theirs.
///
/// **Mutant: `restore`'s `is_self_or_descendant` guard deleted**, so the close
/// blurs whatever holds the keyboard — here a control on the page.
#[wasm_bindgen_test]
fn closing_does_not_release_a_claim_outside_the_overlay() {
    let open = Signal::new(false);
    let f = modal_page(open, true, false);

    open.set(true);
    assert_eq!(f.active(), "in-first", "precondition: inside");

    f.focus("elsewhere");
    open.set(false);
    assert_eq!(f.active(), "elsewhere");
    f.teardown();
}

/// Unmounting while still open restores too — `if show { Modal { … } }` is the
/// ordinary way to mount an overlay, and it disposes the component instead of
/// toggling `opened`.
///
/// **Mutant: the `on_cleanup` deleted.** `activeElement` stays on a control
/// inside a subtree that has left the document.
#[wasm_bindgen_test]
fn unmounting_an_open_modal_restores_the_opener() {
    let inner_scope: Rc<RefCell<Option<RenderScope>>> = Rc::new(RefCell::new(None));
    let modal_handle: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let open = Signal::new(false);

    let scope_out = inner_scope.clone();
    let modal_out = modal_handle.clone();
    let f = Fixture::mount(move |scope| {
        let page = scope.create_element("div");
        page.append_child(&button(scope, "opener"));

        // A child scope, which is what `show_dom` gives an `if` branch and the
        // only way to unmount the overlay and not the page.
        let doc = scope.doc_weak().upgrade().expect("document alive");
        let mut child = RenderScope::new(doc, page.node_id());
        let modal = {
            let _owner = child.push_owner();
            let in_first = button(&mut child, "in-first");
            Modal {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                with_close_button: false,
                ..Default::default()
            }
            .render(&mut child, &[in_first])
        };
        page.append_child(&modal);
        *modal_out.borrow_mut() = Some(modal);
        *scope_out.borrow_mut() = Some(child);
        page
    });

    f.focus("opener");
    open.set(true);
    assert_eq!(f.active(), "in-first", "precondition: inside");

    // `show_dom`'s own order: dispose first, remove after.
    inner_scope
        .borrow_mut()
        .take()
        .expect("mounted once")
        .dispose();
    modal_handle.borrow().as_ref().expect("the modal").remove();

    assert_eq!(f.active(), "opener");
    f.teardown();
}

// ── 3. nesting ──────────────────────────────────────────────────────────────

/// Closing an inner overlay restores **into the outer one**, and closing the
/// outer restores to the page.
///
/// **Mutant: one shared memory** instead of a cell per armed overlay. The
/// inner's capture overwrites the outer's, and closing the outer restores to
/// `inner-a` rather than to `opener`.
#[wasm_bindgen_test]
fn a_nested_overlay_restores_into_the_one_that_contains_it() {
    let outer = Signal::new(false);
    let inner = Signal::new(false);
    let f = Fixture::mount(move |scope| {
        let page = scope.create_element("div");
        page.append_child(&button(scope, "opener"));

        let inner_a = button(scope, "inner-a");
        let inner_modal = Modal {
            opened_fn: Some(Rc::new(move || inner.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[inner_a]);

        let outer_a = button(scope, "outer-a");
        let outer_modal = Modal {
            opened_fn: Some(Rc::new(move || outer.get())),
            trap_focus: true,
            with_close_button: false,
            ..Default::default()
        }
        .render(scope, &[outer_a, inner_modal]);
        page.append_child(&outer_modal);
        page
    });

    f.focus("opener");
    outer.set(true);
    assert_eq!(f.active(), "outer-a", "the outer took it");

    inner.set(true);
    assert_eq!(f.active(), "inner-a", "the inner took it");

    inner.set(false);
    assert_eq!(
        f.active(),
        "outer-a",
        "closing the inner restores into the outer"
    );

    outer.set(false);
    assert_eq!(f.active(), "opener", "and the outer restores to the page");
    f.teardown();
}

// ── 4. the popover policy ───────────────────────────────────────────────────

/// A `Popover` takes the keyboard on open **only** for an `autofocus` child —
/// the HTML popover API's rule, not the dialog's.
///
/// **Mutant: `Popover` passing `FocusIntoPolicy::FirstFocusable`.** The
/// no-autofocus half fails: a popover opening would yank the keyboard out of
/// whatever the user was typing in.
#[wasm_bindgen_test]
fn a_popover_takes_focus_only_for_an_autofocus_child() {
    for autofocus in [false, true] {
        let open = Signal::new(false);
        let f = Fixture::mount(move |scope| {
            let page = scope.create_element("div");
            page.append_child(&button(scope, "outside"));

            let inside = button(scope, "inside");
            if autofocus {
                inside.set_attribute("autofocus", "");
            }
            let popover = Popover {
                opened_fn: Some(Rc::new(move || open.get())),
                trap_focus: true,
                ..Default::default()
            }
            .render(scope, &[inside]);
            page.append_child(&popover);
            page
        });

        f.focus("outside");
        open.set(true);
        assert_eq!(
            f.active(),
            if autofocus { "inside" } else { "outside" },
            "autofocus: {autofocus}"
        );
        f.teardown();
    }
}
