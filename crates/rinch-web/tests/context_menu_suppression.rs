//! Browser-driven tests for the `contextmenu` delegation.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown
//! ```
//!
//! Two questions that used to be one. **Dispatch** is "does some ancestor carry
//! a live `data-oncontextmenu`"; **suppression** is "does the browser get to
//! open its own menu". The listener answered both on the same branch, so a
//! right-click that found no handler kept the page's default — including on
//! `ContextMenu`'s overlay, which is portalled to `body` and therefore outside
//! the subtree the handler is on, which is exactly where an app rendering its
//! own menu did *not* want the browser's.
//!
//! The first four cases are the handler matrix: a live handler, no handler with
//! the flag off, no handler with the flag on, and a **stale** handler — an
//! attribute outliving the scope that registered it (issue #141), which is not a
//! handler and must not suppress anything on its own.
//!
//! The rest are **what the flag leaves alone** (issue #812). With the flag on, a
//! right-click in an editable context — a `<textarea>`, a text-like `<input>`,
//! or content whose `isContentEditable` is true — keeps the browser's menu,
//! because that menu is the user's cut, copy, paste and spelling suggestions and
//! a page cannot rebuild it. A live handler still wins there, and a stale one
//! is no handler there either. Outside editable content, a checkbox, a
//! `<select>`, a label beside a field, the rich-text editor's surface and
//! selected text are still suppressed, and so is a `contenteditable="false"`
//! island.
//!
//! `defaultPrevented` on a synthetic, cancelable `contextmenu` event is the
//! observable: it is what decides whether the browser goes on to open its menu.
//! A *not prevented* answer is also what a page with no rinch listener at all
//! gives, so every fixture that expects one right-clicks a plain control first,
//! with the flag on, and asserts it **was** prevented — the proof that the
//! delegation is installed in this binary and reached by the event.
#![cfg(target_arch = "wasm32")]

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events::{EventHandlerId, has_click_handler};
use rinch_web::RootHandle;
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Marks a fixture's host so a later test can purge one a failed test left in
/// the body (a failed assertion never reaches its own `teardown`).
const HOST_MARKER: &str = "data-ctxmenu-test-host";

struct Fixture {
    root: RootHandle,
    host: web_sys::Element,
    count: Rc<Cell<u32>>,
}

impl Fixture {
    fn mount(build: impl FnOnce(&mut RenderScope, Rc<Cell<u32>>) -> NodeHandle + 'static) -> Self {
        // The flag is page-global and every wasm test shares one page, so each
        // fixture states the default rather than inheriting whatever the last
        // test left.
        rinch_web::set_suppress_native_context_menu(false);
        if let Ok(stale) = document().query_selector_all(&format!("[{HOST_MARKER}]")) {
            for i in 0..stale.length() {
                if let Some(node) = stale.item(i)
                    && let Ok(el) = node.dyn_into::<web_sys::Element>()
                {
                    el.remove();
                }
            }
        }
        let count = Rc::new(Cell::new(0u32));
        let counter = count.clone();
        let host = document().create_element("div").unwrap();
        host.set_attribute(HOST_MARKER, "").unwrap();
        document().body().unwrap().append_child(&host).unwrap();
        let root = rinch_web::mount_into(
            &host,
            ThemeProviderProps::default(),
            move |scope: &mut RenderScope| build(scope, counter),
        );
        Self { root, host, count }
    }

    fn dispatches(&self) -> u32 {
        self.count.get()
    }

    fn el(&self, id: &str) -> web_sys::Element {
        document()
            .get_element_by_id(id)
            .unwrap_or_else(|| panic!("no element #{id}"))
    }

    fn teardown(self) {
        rinch_web::set_suppress_native_context_menu(false);
        self.root.unmount();
        self.host.remove();
    }
}

/// A `<div id=..>` with the given attributes and some text to right-click on.
fn div(scope: &mut RenderScope, id: &str, attrs: &[(&str, &str)]) -> NodeHandle {
    let el = scope.create_element("div");
    el.set_attribute("id", id);
    for (k, v) in attrs {
        el.set_attribute(k, v);
    }
    let text = scope.create_text("target");
    el.append_child(&text);
    el
}

/// Right-click `el`, returning whether the browser's own menu was suppressed.
fn right_click(el: &web_sys::Element) -> bool {
    let rect = el.get_bounding_client_rect();
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    init.set_buttons(2);
    init.set_client_x((rect.x() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.y() + rect.height() / 2.0) as i32);
    let event = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
    el.dispatch_event(&event).unwrap();
    event.default_prevented()
}

/// The case that always worked, kept so the restructuring cannot quietly lose
/// it: a live handler both fires and takes the browser's menu.
#[wasm_bindgen_test]
fn a_live_handler_fires_and_suppresses_the_native_menu() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        div(
            scope,
            "ctx-live",
            &[("data-oncontextmenu", &id.0.to_string())],
        )
    });

    let target = fixture.el("ctx-live");
    assert!(
        right_click(&target),
        "a handled right-click must not also open the browser's menu"
    );
    assert_eq!(fixture.dispatches(), 1);
    fixture.teardown();
}

/// The default, and why the flag exists: an island hydrated into somebody
/// else's page must leave the right-click alone everywhere it has no handler.
#[wasm_bindgen_test]
fn without_the_flag_an_unhandled_right_click_keeps_the_browsers_menu() {
    let fixture = Fixture::mount(|scope, _| div(scope, "ctx-plain", &[]));

    let target = fixture.el("ctx-plain");
    assert!(
        !right_click(&target),
        "with no handler and the flag off, the right-click belongs to the page"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// The defect: a whole-page app rendering its own menus wore the browser's on
/// top of them, because `ContextMenu` portals its overlay to `body` — outside
/// the subtree the handler sits on, so nothing there carries the attribute.
/// This fixture is that shape: a sibling of the handler's element, standing in
/// for the portalled overlay.
#[wasm_bindgen_test]
fn with_the_flag_an_unhandled_right_click_is_suppressed_and_still_dispatches_nothing() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(
            scope,
            "ctx-handled",
            &[("data-oncontextmenu", &id.0.to_string())],
        ));
        wrapper.append_child(&div(scope, "ctx-overlay", &[]));
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert!(rinch_web::suppresses_native_context_menu());

    let overlay = fixture.el("ctx-overlay");
    assert!(
        right_click(&overlay),
        "the flag suppresses the browser's menu wherever the click lands"
    );
    assert_eq!(
        fixture.dispatches(),
        0,
        "suppressing is not dispatching: nothing here carries a handler"
    );

    // And the handler beside it still fires, so the flag does not shadow it.
    let handled = fixture.el("ctx-handled");
    assert!(right_click(&handled));
    assert_eq!(fixture.dispatches(), 1);

    fixture.teardown();
}

/// The id of a handler that was registered and is now dead: the real shape
/// rather than an invented id. A root registers a handler and is then
/// unmounted, which deregisters it, while an attribute naming that id can live
/// on somewhere else.
fn dead_handler_id() -> EventHandlerId {
    let dead_id = Rc::new(Cell::new(0usize));
    let sink = dead_id.clone();
    let doomed_host = document().create_element("div").unwrap();
    doomed_host.set_attribute(HOST_MARKER, "").unwrap();
    document()
        .body()
        .unwrap()
        .append_child(&doomed_host)
        .unwrap();
    let doomed = rinch_web::mount_into(
        &doomed_host,
        ThemeProviderProps::default(),
        move |scope: &mut RenderScope| {
            let id = scope.register_handler(|| unreachable!("this handler is dead"));
            sink.set(id.0);
            div(scope, "ctx-doomed", &[])
        },
    );
    doomed.unmount();
    doomed_host.remove();

    let dead = EventHandlerId(dead_id.get());
    assert!(
        !has_click_handler(dead),
        "precondition: unmounting the root must have deregistered the handler"
    );
    dead
}

/// A stale `data-oncontextmenu` is not a handler. The attribute outlives the
/// scope that registered it (issue #141), and before the liveness check it both
/// swallowed the browser's menu and dispatched nothing — a right-click that did
/// precisely nothing. The desktop's `dispatch_oncontextmenu` has always filtered
/// on `has_click_handler`; this is the web catching up.
#[wasm_bindgen_test]
fn a_stale_handler_attribute_neither_fires_nor_suppresses() {
    let dead = dead_handler_id();

    let fixture = Fixture::mount(move |scope, _| {
        div(
            scope,
            "ctx-stale",
            &[("data-oncontextmenu", &dead.0.to_string())],
        )
    });

    let target = fixture.el("ctx-stale");
    assert!(
        !right_click(&target),
        "a dead handler must not swallow the browser's menu — the desktop \
         answers `false` here and lets the click path run"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// An element `<tag id=..>` with the given attributes and no children.
fn element(scope: &mut RenderScope, tag: &str, id: &str, attrs: &[(&str, &str)]) -> NodeHandle {
    let el = scope.create_element(tag);
    el.set_attribute("id", id);
    for (k, v) in attrs {
        el.set_attribute(k, v);
    }
    el
}

/// The positive control for a *not prevented* assertion, which a page with no
/// rinch listener would also give: with the flag on, a right-click on the plain
/// `#ctx-control` div must be suppressed, so the listener exists and runs.
fn assert_control_is_suppressed(fixture: &Fixture) {
    assert!(rinch_web::suppresses_native_context_menu());
    assert!(
        right_click(&fixture.el("ctx-control")),
        "positive control: with the flag on, a plain div must be suppressed — \
         otherwise nothing below proves the delegation ran at all"
    );
}

/// The case the issue is about. A whole-page app turns the flag on so its own
/// menus do not wear the browser's, and every text field on the page lost cut,
/// copy, paste and spelling with it.
///
/// Every text-like type is listed, and three spellings of `type` that only the
/// browser's normalisation resolves: none at all, an unknown one and one in the
/// wrong case all *are* a text field, which reading the attribute instead of
/// the `type` IDL property gets wrong.
#[wasm_bindgen_test]
fn with_the_flag_a_text_field_keeps_the_browsers_menu() {
    const TYPES: &[&str] = &[
        "text", "search", "url", "tel", "email", "password", "number", "bogus", "Email",
    ];
    let fixture = Fixture::mount(|scope, _| {
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(scope, "ctx-control", &[]));
        for ty in TYPES {
            wrapper.append_child(&element(
                scope,
                "input",
                &format!("ctx-input-{ty}"),
                &[("type", ty)],
            ));
        }
        wrapper.append_child(&element(scope, "input", "ctx-input-untyped", &[]));
        wrapper.append_child(&element(scope, "textarea", "ctx-textarea", &[]));
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert_control_is_suppressed(&fixture);

    for ty in TYPES {
        assert!(
            !right_click(&fixture.el(&format!("ctx-input-{ty}"))),
            "an <input type={ty:?}> is a text field: its right-click must keep \
             the browser's editing menu with the flag on"
        );
    }
    assert!(
        !right_click(&fixture.el("ctx-input-untyped")),
        "an <input> with no type is a text field"
    );
    assert!(
        !right_click(&fixture.el("ctx-textarea")),
        "a <textarea> must keep the browser's editing menu with the flag on"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// `readonly` and `disabled` do not take the carve-out away. A read-only
/// field's native menu still offers Copy, and what a disabled one shows is the
/// browser's business rather than rinch's.
///
/// A disabled control is the case where the event might never reach the
/// document at all, which would make the assertion pass for nothing — so a
/// disabled input under a live handler goes first and must dispatch it.
#[wasm_bindgen_test]
fn with_the_flag_a_read_only_or_disabled_field_keeps_the_browsers_menu() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(scope, "ctx-control", &[]));
        let handled = element(
            scope,
            "div",
            "ctx-disabled-handled",
            &[("data-oncontextmenu", &id.0.to_string())],
        );
        handled.append_child(&element(
            scope,
            "input",
            "ctx-disabled-under-handler",
            &[("disabled", "")],
        ));
        wrapper.append_child(&handled);
        wrapper.append_child(&element(
            scope,
            "input",
            "ctx-readonly-input",
            &[("readonly", "")],
        ));
        wrapper.append_child(&element(
            scope,
            "textarea",
            "ctx-readonly-textarea",
            &[("readonly", "")],
        ));
        wrapper.append_child(&element(
            scope,
            "input",
            "ctx-disabled-input",
            &[("disabled", "")],
        ));
        wrapper.append_child(&element(
            scope,
            "textarea",
            "ctx-disabled-textarea",
            &[("disabled", "")],
        ));
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert_control_is_suppressed(&fixture);

    assert!(right_click(&fixture.el("ctx-disabled-under-handler")));
    assert_eq!(
        fixture.dispatches(),
        1,
        "positive control: a right-click on a disabled input must reach rinch's \
         listener, or the disabled assertions below prove nothing"
    );

    for id in [
        "ctx-readonly-input",
        "ctx-readonly-textarea",
        "ctx-disabled-input",
        "ctx-disabled-textarea",
    ] {
        assert!(
            !right_click(&fixture.el(id)),
            "#{id} is still a text field: readonly and disabled keep the carve-out"
        );
    }
    assert_eq!(fixture.dispatches(), 1);
    fixture.teardown();
}

/// Only a *text-like* input is exempt. The native menu on a checkbox or a
/// button has nothing to edit, so the flag takes it as it takes a plain div.
#[wasm_bindgen_test]
fn with_the_flag_a_non_text_input_is_still_suppressed() {
    const TYPES: &[&str] = &[
        "checkbox", "radio", "button", "submit", "reset", "range", "color", "file",
    ];
    let fixture = Fixture::mount(|scope, _| {
        let wrapper = scope.create_element("div");
        for ty in TYPES {
            wrapper.append_child(&element(
                scope,
                "input",
                &format!("ctx-input-{ty}"),
                &[("type", ty)],
            ));
        }
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    for ty in TYPES {
        assert!(
            right_click(&fixture.el(&format!("ctx-input-{ty}"))),
            "an <input type={ty:?}> is not a text field: the flag suppresses it"
        );
    }
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// `contenteditable` is read as the browser computes it, through
/// `isContentEditable`: it is inherited, `""` and `plaintext-only` turn it on,
/// a `contenteditable="false"` island turns it off for its own subtree, and an
/// editable host inside that island turns it back on. An attribute selector
/// gets the island wrong in one direction or the other.
///
/// An SVG element has no `isContentEditable`, so an icon inside an editable
/// host is answered by its nearest HTML ancestor.
#[wasm_bindgen_test]
fn with_the_flag_editable_content_keeps_the_browsers_menu_and_a_false_island_does_not() {
    let fixture = Fixture::mount(|scope, _| {
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(scope, "ctx-control", &[]));

        let host = element(scope, "div", "ctx-ce", &[("contenteditable", "true")]);
        host.append_child(&element(scope, "span", "ctx-ce-span", &[]));
        host.append_child(&element(scope, "svg", "ctx-ce-svg", &[]));
        let island = element(
            scope,
            "div",
            "ctx-ce-island",
            &[("contenteditable", "false")],
        );
        island.append_child(&element(scope, "span", "ctx-ce-island-span", &[]));
        let reopened = element(
            scope,
            "div",
            "ctx-ce-reopened",
            &[("contenteditable", "true")],
        );
        reopened.append_child(&element(scope, "span", "ctx-ce-reopened-span", &[]));
        island.append_child(&reopened);
        host.append_child(&island);
        wrapper.append_child(&host);

        let empty = element(scope, "div", "ctx-ce-empty", &[("contenteditable", "")]);
        empty.append_child(&element(scope, "span", "ctx-ce-empty-span", &[]));
        wrapper.append_child(&empty);
        let plain = element(
            scope,
            "div",
            "ctx-ce-plaintext",
            &[("contenteditable", "plaintext-only")],
        );
        plain.append_child(&element(scope, "span", "ctx-ce-plaintext-span", &[]));
        wrapper.append_child(&plain);
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert_control_is_suppressed(&fixture);

    for id in [
        "ctx-ce",
        "ctx-ce-span",
        "ctx-ce-svg",
        "ctx-ce-reopened",
        "ctx-ce-reopened-span",
        "ctx-ce-empty-span",
        "ctx-ce-plaintext-span",
    ] {
        assert!(
            !right_click(&fixture.el(id)),
            "#{id} is editable content: it must keep the browser's editing menu"
        );
    }
    for id in ["ctx-ce-island", "ctx-ce-island-span"] {
        assert!(
            right_click(&fixture.el(id)),
            "#{id} is inside a contenteditable=\"false\" island, which is not \
             editable: the flag suppresses it"
        );
    }
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// The field is the `<input>` itself, not whatever wraps it: a label, an icon
/// section and the wrapper around a text field are not editable, so the flag
/// still suppresses a right-click on them — which is where an app's own menu
/// for the row would be.
#[wasm_bindgen_test]
fn with_the_flag_a_label_or_icon_beside_a_text_field_is_still_suppressed() {
    let fixture = Fixture::mount(|scope, _| {
        let wrapper = element(scope, "div", "ctx-field-wrapper", &[]);
        let label = element(scope, "label", "ctx-field-label", &[]);
        label.append_child(&scope.create_text("Name"));
        wrapper.append_child(&label);
        let row = element(scope, "div", "ctx-field-row", &[]);
        let icon = element(scope, "span", "ctx-field-icon", &[]);
        icon.append_child(&scope.create_text("@"));
        row.append_child(&icon);
        row.append_child(&element(scope, "input", "ctx-field-input", &[]));
        wrapper.append_child(&row);
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    for id in [
        "ctx-field-wrapper",
        "ctx-field-label",
        "ctx-field-row",
        "ctx-field-icon",
    ] {
        assert!(
            right_click(&fixture.el(id)),
            "#{id} is beside the text field, not in it: the flag suppresses it"
        );
    }
    assert!(
        !right_click(&fixture.el("ctx-field-input")),
        "and the field itself keeps the browser's menu"
    );
    fixture.teardown();
}

/// A live `data-oncontextmenu` still wins inside an editable context, whether
/// or not the flag is on: an app that wants its own menu in a field says so by
/// attaching one, and the browser's must not open on top of it.
#[wasm_bindgen_test]
fn a_live_handler_still_takes_a_right_click_in_an_editable_context() {
    let fixture = Fixture::mount(|scope, count| {
        let id = scope.register_handler(move || count.set(count.get() + 1));
        let handled = element(
            scope,
            "div",
            "ctx-handled-fields",
            &[("data-oncontextmenu", &id.0.to_string())],
        );
        handled.append_child(&element(scope, "input", "ctx-handled-input", &[]));
        handled.append_child(&element(scope, "textarea", "ctx-handled-textarea", &[]));
        let editable = element(
            scope,
            "div",
            "ctx-handled-ce",
            &[("contenteditable", "true")],
        );
        editable.append_child(&element(scope, "span", "ctx-handled-ce-span", &[]));
        handled.append_child(&editable);
        handled
    });

    let targets = [
        "ctx-handled-input",
        "ctx-handled-textarea",
        "ctx-handled-ce-span",
    ];
    let mut expected = 0;
    for flag in [false, true] {
        rinch_web::set_suppress_native_context_menu(flag);
        for id in targets {
            assert!(
                right_click(&fixture.el(id)),
                "#{id} sits under a live handler: it must take the browser's \
                 menu (flag {flag})"
            );
            expected += 1;
            assert_eq!(
                fixture.dispatches(),
                expected,
                "#{id}'s right-click must dispatch the handler (flag {flag})"
            );
        }
    }
    fixture.teardown();
}

/// The carve-out is for editing, not for text. Selected text outside any field
/// has a native menu too (Copy, Search), but an app that turned the flag on
/// page-wide said it wanted that one gone, so a selection changes nothing.
#[wasm_bindgen_test]
fn with_the_flag_selected_text_outside_a_field_is_still_suppressed() {
    let fixture = Fixture::mount(|scope, _| div(scope, "ctx-selected", &[]));

    let target = fixture.el("ctx-selected");
    let selection = web_sys::window().unwrap().get_selection().unwrap().unwrap();
    let range = document().create_range().unwrap();
    range.select_node_contents(&target).unwrap();
    selection.remove_all_ranges().unwrap();
    selection.add_range(&range).unwrap();
    assert!(
        !selection.is_collapsed() && selection.to_string() == "target",
        "precondition: the div's text must be selected"
    );

    rinch_web::set_suppress_native_context_menu(true);
    assert!(
        right_click(&target),
        "a selection outside any field is not an editing context"
    );
    selection.remove_all_ranges().unwrap();
    fixture.teardown();
}

/// A stale handler is no handler inside an editing context either. With the
/// flag on, an editable target under an ancestor whose `data-oncontextmenu`
/// names a dead handler keeps the browser's menu, and a plain div beside it is
/// suppressed by the flag. The existing stale fixture runs with the flag off,
/// where the carve-out is never asked, so it cannot tell whether the carve-out
/// reads the *live* handler or merely the nearest attribute — this one does.
#[wasm_bindgen_test]
fn with_the_flag_a_stale_handler_does_not_take_an_editable_targets_menu() {
    let dead = dead_handler_id();
    let fixture = Fixture::mount(move |scope, _| {
        let wrapper = scope.create_element("div");
        wrapper.append_child(&div(scope, "ctx-control", &[]));
        let stale = element(
            scope,
            "div",
            "ctx-stale-fields",
            &[("data-oncontextmenu", &dead.0.to_string())],
        );
        stale.append_child(&element(scope, "input", "ctx-stale-input", &[]));
        stale.append_child(&element(scope, "textarea", "ctx-stale-textarea", &[]));
        let editable = element(scope, "div", "ctx-stale-ce", &[("contenteditable", "true")]);
        editable.append_child(&element(scope, "span", "ctx-stale-ce-span", &[]));
        stale.append_child(&editable);
        stale.append_child(&div(scope, "ctx-stale-plain", &[]));
        wrapper.append_child(&stale);
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    assert_control_is_suppressed(&fixture);

    for id in ["ctx-stale-input", "ctx-stale-textarea", "ctx-stale-ce-span"] {
        assert!(
            !right_click(&fixture.el(id)),
            "#{id} is editable and its only `data-oncontextmenu` is dead: a stale \
             handler is no handler, so the field keeps the browser's menu"
        );
    }
    assert!(
        right_click(&fixture.el("ctx-stale-plain")),
        "a plain div under the same stale handler is suppressed by the flag"
    );
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}

/// Neither a `<select>` nor the rich-text editor's surface has an editing menu,
/// so the flag suppresses both — the predicate's `<textarea>` and `<input>`
/// branches must not grow to take them in.
///
/// The surface here is a hand-built `[data-pm-editor]` div, and the event is
/// dispatched straight on it and on its `<p>`, with no pointer press first.
/// Issue #814 plans to move the editor's hidden capture `<textarea>` under the
/// pointer on a right-button press, so that the browser's own hit test targets
/// the textarea — which this predicate already carves out. That changes what a
/// real right-click hits, not what the surface element itself is, so #814
/// should leave this fixture passing; if #814 instead carves the surface out in
/// the predicate, this is the fixture to revisit.
#[wasm_bindgen_test]
fn with_the_flag_a_select_and_the_editor_surface_are_still_suppressed() {
    let fixture = Fixture::mount(|scope, _| {
        let wrapper = scope.create_element("div");
        let select = element(scope, "select", "ctx-select", &[]);
        let option = element(scope, "option", "ctx-select-option", &[]);
        option.append_child(&scope.create_text("one"));
        select.append_child(&option);
        wrapper.append_child(&select);
        let surface = element(scope, "div", "ctx-pm-surface", &[("data-pm-editor", "")]);
        let paragraph = element(scope, "p", "ctx-pm-paragraph", &[]);
        paragraph.append_child(&scope.create_text("text"));
        surface.append_child(&paragraph);
        wrapper.append_child(&surface);
        wrapper
    });

    rinch_web::set_suppress_native_context_menu(true);
    for id in [
        "ctx-select",
        "ctx-select-option",
        "ctx-pm-surface",
        "ctx-pm-paragraph",
    ] {
        assert!(
            right_click(&fixture.el(id)),
            "#{id} has no editing menu: the flag suppresses it"
        );
    }
    assert_eq!(fixture.dispatches(), 0);
    fixture.teardown();
}
