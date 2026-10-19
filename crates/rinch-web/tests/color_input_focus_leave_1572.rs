//! `ColorInput`'s dropdown closes when focus leaves the field and when the
//! window loses focus, in a real browser (issue #1572) — Mantine closes it in
//! the input's `onBlur`, which a window blur also fires.
//!
//! The web backend has no focus arbiter: the browser moves focus, and rinch's
//! document `focusin` / `focusout` listeners and its `window` `blur` listener
//! tell the key entries (`rinch_core::notify_focus_moved`,
//! `notify_window_blurred`). The dropdown holds such an entry while it is
//! open, owned by the `ColorInput`'s root, so a focus move *inside* it does
//! not close it.
//!
//! A test cannot take a trusted press, so "a press in the panel does not move
//! focus" is pinned at its mechanism: rinch `preventDefault`s the
//! `pointerdown` (`data-nofocus` on the dropdown), and the browser's focus
//! change on a press is that event's default action.
//!
//! The positive control of every fixture is the first assertion: a genuine
//! `click` through rinch's delegation opened the dropdown and put the focus on
//! the text field.
//!
//! **Mount through `rinch_web::mount_into`** — without the owner `mount_tree`
//! pushes, component effects are disposed at once.
#![cfg(target_arch = "wasm32")]

use rinch::components::ColorInput;
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Component, InputCallback};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-1572";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// A `ColorInput` between two buttons, with one preset swatch.
fn mount() -> (web_sys::Document, Rc<RefCell<Vec<String>>>) {
    let bdoc = browser_document();
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    while let Ok(Some(el)) = bdoc.query_selector(&format!("[{HOST_ATTR}]")) {
        el.remove();
    }
    let host = bdoc.create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "true").unwrap();
    host.set_attribute("style", "width: 240px; margin: 40px")
        .unwrap();
    bdoc.body().unwrap().append_child(&host).unwrap();
    let picks = Rc::new(RefCell::new(Vec::new()));
    let sink = picks.clone();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let shell = scope.create_element("div");
        let before = scope.create_element("button");
        before.set_attribute("class", "before-1572");
        before.append_child(&scope.create_text("before"));
        shell.append_child(&before);
        let sink = sink.clone();
        let field = ColorInput {
            value: "#000000".to_string(),
            swatches: vec!["#ff0000".into()],
            swatches_per_row: Some(1),
            onchange: Some(InputCallback::new(move |v: String| {
                sink.borrow_mut().push(v)
            })),
            ..Default::default()
        }
        .render(scope, &[]);
        shell.append_child(&field);
        let after = scope.create_element("button");
        after.set_attribute("class", "after-1572");
        after.append_child(&scope.create_text("after"));
        shell.append_child(&after);
        shell
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    (bdoc, picks)
}

fn one(doc: &web_sys::Document, selector: &str) -> web_sys::Element {
    doc.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn html(el: &web_sys::Element) -> &web_sys::HtmlElement {
    el.dyn_ref::<web_sys::HtmlElement>().unwrap()
}

fn is_open(doc: &web_sys::Document) -> bool {
    one(doc, ".rinch-color-input")
        .get_attribute("class")
        .is_some_and(|c| {
            c.split_whitespace()
                .any(|one| one == "rinch-color-input--opened")
        })
}

fn plain_event(target: &web_sys::EventTarget, name: &str, bubbles: bool) {
    let init = web_sys::EventInit::new();
    init.set_bubbles(bubbles);
    let ev = web_sys::Event::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

fn keydown(el: &web_sys::Element, key: &str) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn pointerdown(el: &web_sys::Element) -> web_sys::PointerEvent {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_is_primary(true);
    init.set_button(0);
    init.set_buttons(1);
    init.set_pointer_type("mouse");
    let down = web_sys::PointerEvent::new_with_event_init_dict("pointerdown", &init).unwrap();
    el.dispatch_event(&down).unwrap();
    // Release it, so no press is left pending for the next one.
    let up = web_sys::PointerEvent::new_with_event_init_dict("pointerup", &init).unwrap();
    el.dispatch_event(&up).unwrap();
    down
}

/// Open the dropdown with a click on the input group, through rinch's
/// delegation — the positive control — and check that opening put the focus on
/// the text field (Mantine: a press on the field's section focuses the field).
fn open(doc: &web_sys::Document) -> web_sys::Element {
    html(&one(doc, ".before-1572")).focus().unwrap();
    html(&one(doc, ".rinch-color-input__input-group")).click();
    assert!(
        is_open(doc),
        "positive control: the click opened the dropdown"
    );
    let input = one(doc, ".rinch-color-input__input");
    assert_eq!(
        doc.active_element().as_ref(),
        Some(&input),
        "opening put the focus on the text field"
    );
    input
}

#[wasm_bindgen_test]
fn focus_leaving_the_field_closes_the_dropdown_in_chrome_1572() {
    let (doc, picks) = mount();
    let input = open(&doc);

    // Control: an element blur/focusout with the focus staying put closes
    // nothing, since the same events fire for every focus move in the page.
    plain_event(input.as_ref(), "blur", false);
    plain_event(input.as_ref(), "focusout", true);
    assert!(
        is_open(&doc),
        "a focusout that leaves the focus in place closes nothing"
    );

    let after = one(&doc, ".after-1572");
    html(&after).focus().unwrap();
    assert_eq!(doc.active_element().as_ref(), Some(&after));
    assert!(
        !is_open(&doc),
        "focus leaving the field closes the dropdown"
    );
    assert!(picks.borrow().is_empty(), "and reports no colour");
}

#[wasm_bindgen_test]
fn a_window_blur_closes_the_dropdown_in_chrome_1572() {
    let (doc, _) = mount();
    let input = open(&doc);

    let window: web_sys::EventTarget = web_sys::window().unwrap().into();
    plain_event(&window, "blur", false);
    assert!(!is_open(&doc), "the window's blur closes the dropdown");
    assert_eq!(
        doc.active_element().as_ref(),
        Some(&input),
        "the field keeps the focus"
    );
    plain_event(&window, "focus", false);
    assert!(!is_open(&doc), "coming back reopens nothing");
}

#[wasm_bindgen_test]
fn a_press_in_the_panel_takes_no_focus_and_keeps_it_open_in_chrome_1572() {
    let (doc, picks) = mount();
    let input = open(&doc);

    let swatch = one(&doc, ".rinch-color-picker__swatches")
        .first_element_child()
        .expect("one preset swatch");
    // A press on the panel itself — its padding, between the controls —
    // reaches no click handler, so nothing else prevents its default: that is
    // `data-nofocus`'s alone. (A press on a swatch is prevented by the click
    // dispatch whatever the attribute says.)
    let panel = one(&doc, ".rinch-color-input__dropdown");
    assert!(
        pointerdown(&panel).default_prevented(),
        "rinch prevents a press in the panel's default, which is the browser \
         moving focus"
    );
    assert!(is_open(&doc), "the press on the panel closes nothing");
    assert!(pointerdown(&swatch).default_prevented());
    assert!(!picks.borrow().is_empty(), "the click reached the swatch");
    assert!(is_open(&doc), "and the dropdown stays open");
    assert_eq!(doc.active_element().as_ref(), Some(&input));

    // Escape still closes it.
    keydown(&input, "Escape");
    assert!(!is_open(&doc), "Escape closes the dropdown");
}

/// A focus move from the field into the panel is not a leave: the entry is
/// owned by the `ColorInput`'s root. The picker ships no focusable control, so
/// one is made — a swatch given a `tabindex` — and focused by code.
#[wasm_bindgen_test]
fn a_focus_move_into_the_panel_keeps_the_dropdown_open_in_chrome_1572() {
    let (doc, _) = mount();
    open(&doc);

    let swatch = one(&doc, ".rinch-color-picker__swatches")
        .first_element_child()
        .expect("one preset swatch");
    swatch.set_attribute("tabindex", "0").unwrap();
    html(&swatch).focus().unwrap();
    assert_eq!(
        doc.active_element().as_ref(),
        Some(&swatch),
        "precondition: the swatch holds the focus"
    );
    assert!(
        is_open(&doc),
        "a focus move inside the ColorInput keeps it open"
    );

    html(&one(&doc, ".after-1572")).focus().unwrap();
    assert!(!is_open(&doc), "and leaving from there closes it");
}
