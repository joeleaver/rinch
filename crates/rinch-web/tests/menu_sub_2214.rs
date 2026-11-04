//! Issue #2214 in a real browser: cascading submenus (`DropdownMenuSub`) in
//! `DropdownMenu` and `ContextMenu`, and the menus' own keys and Escape
//! (#2169, #2170). Twin of `rinch/src/app/menu_sub_2214_tests.rs`.
//!
//! The pointer is a synthetic `pointermove` at the element under the point
//! (`elementFromPoint`), the keys a `keydown` at the focused element — where
//! the browser sends them. The delays are the browser's own `setTimeout`.
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test menu_sub_2214
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::{
    ContextMenu, DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuSub,
    DropdownMenuTarget,
};
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::{Callback, Component, Signal};
use rinch_web::RootHandle;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-2214";

const FIXTURE_CSS: &str = "
.rinch-dropdown-menu__item { height: 30px; padding: 0 8px; font-size: 12px; line-height: 30px; }
.rinch-dropdown-menu__dropdown { --rinch-dropdown-menu-width: 200px; }
.rinch-context-menu__dropdown { width: 200px; }
.fixture-2214 { --rinch-dropdown-menu-sub-width: 170px; }
";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn doc() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

fn viewport() -> (f64, f64) {
    let el = doc().document_element().unwrap();
    (el.client_width() as f64, el.client_height() as f64)
}

fn log() -> Vec<String> {
    LOG.with(|l| l.borrow().clone())
}

fn mount(build: impl Fn(&mut RenderScope) -> NodeHandle + 'static) {
    PREVIOUS.with(|p| {
        if let Some(h) = p.borrow_mut().take() {
            h.unmount();
        }
    });
    for sel in [
        format!("[{HOST_ATTR}]"),
        ".rinch-context-menu__portal".into(),
    ] {
        while let Ok(Some(el)) = doc().query_selector(&sel) {
            el.remove();
        }
    }
    LOG.with(|l| l.borrow_mut().clear());
    let host = doc().create_element("div").unwrap();
    host.set_attribute(HOST_ATTR, "").unwrap();
    let style = doc().create_element("style").unwrap();
    style.set_text_content(Some(FIXTURE_CSS));
    host.append_child(&style).unwrap();
    doc().body().unwrap().append_child(&host).unwrap();
    rinch_web::__force_trusted_clicks(true);
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
}

fn item(scope: &mut RenderScope, label: &str, id: &str) -> NodeHandle {
    let text = scope.create_text(label);
    let name = label.to_string();
    let node = DropdownMenuItem {
        onclick: Some(Callback::new(move || {
            LOG.with(|l| l.borrow_mut().push(name.clone()))
        })),
        ..Default::default()
    }
    .render(scope, &[text]);
    node.set_attribute("id", id);
    node
}

fn sub(
    scope: &mut RenderScope,
    label: &str,
    id: &str,
    disabled: bool,
    kids: &[NodeHandle],
) -> NodeHandle {
    let node = DropdownMenuSub {
        label: label.into(),
        disabled,
        ..Default::default()
    }
    .render(scope, kids);
    node.set_attribute("id", id);
    node
}

/// Alpha, New ▸ (File, Deep ▸ (Leaf, Lemma, Lx, Ly, Lz), Folder),
/// Off ▸ (disabled), Beta.
fn rows(scope: &mut RenderScope) -> Vec<NodeHandle> {
    let leaves: Vec<NodeHandle> = [
        ("Leaf", "i-leaf"),
        ("Lemma", "i-leaf2"),
        ("Lx", "i-lx"),
        ("Ly", "i-ly"),
        ("Lz", "i-lz"),
    ]
    .iter()
    .map(|(l, id)| item(scope, l, id))
    .collect();
    let deep = sub(scope, "Deep", "s-deep", false, &leaves);
    let file = item(scope, "File", "i-file");
    let folder = item(scope, "Folder", "i-folder");
    let new = sub(scope, "New", "s-new", false, &[file, deep, folder]);
    let inner = item(scope, "Inner", "i-inner");
    let off = sub(scope, "Off", "s-off", true, &[inner]);
    vec![
        item(scope, "Alpha", "i-alpha"),
        new,
        off,
        item(scope, "Beta", "i-beta"),
    ]
}

/// A `DropdownMenu`, open, its 40x20 trigger in a fixed box at `wrapper`.
fn dropdown(wrapper: String, panel_style: Option<&'static str>) -> Signal<bool> {
    let open = Signal::new(true);
    mount(move |scope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 40px; height: 20px;");
        let target = DropdownMenuTarget.render(scope, &[trigger]);
        let kids = rows(scope);
        let dd = DropdownMenuDropdown.render(scope, &kids);
        dd.set_attribute("id", "panel");
        if let Some(s) = panel_style {
            dd.set_attribute("style", s);
        }
        let menu = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            on_close: Some(Callback::new(move || {
                LOG.with(|l| l.borrow_mut().push("on_close".into()));
                open.set(false);
            })),
            ..Default::default()
        }
        .render(scope, &[target, dd]);
        let w = scope.create_element("div");
        w.set_attribute("class", "fixture-2214");
        w.set_attribute("style", &wrapper);
        w.append_child(&menu);
        w
    });
    open
}

fn el(id: &str) -> web_sys::HtmlElement {
    doc()
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("no #{id}"))
        .dyn_into()
        .unwrap()
}

/// A submenu's row and flyout.
fn row(sub_id: &str) -> web_sys::HtmlElement {
    el(sub_id)
        .first_element_child()
        .unwrap()
        .dyn_into()
        .unwrap()
}

fn flyout(sub_id: &str) -> web_sys::Element {
    el(sub_id)
        .query_selector(":scope > .rinch-dropdown-menu__sub-dropdown")
        .unwrap()
        .unwrap()
}

fn is_open(sub_id: &str) -> bool {
    el(sub_id)
        .get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == "rinch-dropdown-menu__sub--opened")
}

fn rect(e: &web_sys::Element) -> (f64, f64, f64, f64) {
    let r = e.get_bounding_client_rect();
    (r.left(), r.top(), r.right(), r.bottom())
}

fn center(e: &web_sys::Element) -> (f64, f64) {
    let (l, t, r, b) = rect(e);
    ((l + r) / 2.0, (t + b) / 2.0)
}

fn pointer(name: &str, (x, y): (f64, f64), button: i16, buttons: u16) {
    let target = doc()
        .element_from_point(x as f32, y as f32)
        .unwrap_or_else(|| doc().body().unwrap().into());
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_is_primary(true);
    init.set_pointer_type("mouse");
    init.set_button(button);
    init.set_buttons(buttons);
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    let ev = web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

fn move_to(p: (f64, f64)) {
    pointer("pointermove", p, -1, 0);
}

fn click(e: &web_sys::Element) {
    let p = center(e);
    move_to(p);
    pointer("pointerdown", p, 0, 1);
    pointer("pointerup", p, 0, 0);
    let target = doc().element_from_point(p.0 as f32, p.1 as f32).unwrap();
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(1);
    init.set_client_x(p.0 as i32);
    init.set_client_y(p.1 as i32);
    let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    target.dispatch_event(&ev).unwrap();
}

fn key(k: &str) -> bool {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(k);
    init.set_code(k);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    let target: web_sys::EventTarget = doc()
        .active_element()
        .unwrap_or_else(|| doc().body().unwrap().into())
        .into();
    target.dispatch_event(&ev).unwrap();
    ev.default_prevented()
}

fn focused_id() -> String {
    doc().active_element().map(|e| e.id()).unwrap_or_default()
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(resolve.unchecked_ref(), ms)
            .unwrap();
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
}

/// Hover opens after 400 ms, not at 150; the flyout overlaps the panel by
/// 3px on its right, its first row level with the row.
#[wasm_bindgen_test]
async fn hover_opens_after_the_delay_beside_the_panel() {
    dropdown(
        "position: fixed; left: 100px; top: 40px; line-height: 0;".into(),
        None,
    );
    move_to(center(&row("s-new")));
    sleep_ms(150).await;
    assert!(!is_open("s-new"), "not before the delay");
    sleep_ms(400).await;
    assert!(is_open("s-new"), "open after it");
    let panel = rect(&el("panel"));
    let fly = rect(&flyout("s-new"));
    assert!(
        (fly.0 - (panel.2 - 3.0)).abs() < 0.6,
        "3px overlap: {fly:?} {panel:?}"
    );
    assert!(
        (fly.2 - fly.0 - 170.0).abs() < 0.6,
        "declared width: {fly:?}"
    );
    assert!(
        (rect(&el("i-file")).1 - rect(&row("s-new")).1).abs() < 0.6,
        "first row level with its row"
    );
}

/// The diagonal across a sibling into the flyout keeps it; staying on the
/// sibling closes it after the delay.
#[wasm_bindgen_test]
async fn a_diagonal_move_keeps_the_submenu_and_a_sibling_closes_it() {
    dropdown(
        "position: fixed; left: 100px; top: 40px; line-height: 0;".into(),
        None,
    );
    click(&row("s-new"));
    assert!(is_open("s-new"), "a press opens at once");
    move_to(center(&el("i-alpha")));
    move_to(center(&el("i-folder")));
    sleep_ms(500).await;
    assert!(is_open("s-new"), "reached the flyout in time");
    move_to(center(&el("i-beta")));
    sleep_ms(500).await;
    assert!(
        !is_open("s-new"),
        "a pointer that stays on a sibling closes it"
    );
}

/// The keyboard, three levels, and Escape closing the innermost first and the
/// menu last, with the focus back where it was at each step.
#[wasm_bindgen_test]
fn the_keyboard_opens_walks_and_closes() {
    let open = dropdown(
        "position: fixed; left: 100px; top: 40px; line-height: 0;".into(),
        None,
    );
    assert!(
        key("ArrowDown"),
        "positive control: a genuine keydown reached the menu"
    );
    assert_eq!(focused_id(), "i-alpha");
    key("ArrowDown");
    let new_row = row("s-new");
    assert!(new_row.is_same_node(doc().active_element().as_ref().map(|e| e.as_ref())));
    assert!(key("ArrowRight"));
    assert!(is_open("s-new"));
    assert_eq!(focused_id(), "i-file", "first row focused");
    key("ArrowDown");
    assert!(row("s-deep").is_same_node(doc().active_element().as_ref().map(|e| e.as_ref())));
    key("Enter");
    assert!(is_open("s-deep"));
    assert_eq!(focused_id(), "i-leaf");
    key("ArrowUp");
    assert_eq!(focused_id(), "i-lz", "wraps");
    key("ArrowLeft");
    assert!(!is_open("s-deep") && is_open("s-new"));
    assert!(row("s-deep").is_same_node(doc().active_element().as_ref().map(|e| e.as_ref())));
    assert!(key("Escape"));
    assert!(!is_open("s-new") && open.get(), "Escape: only the submenu");
    assert!(new_row.is_same_node(doc().active_element().as_ref().map(|e| e.as_ref())));
    assert!(key("Escape"));
    assert!(!open.get(), "the next Escape closes the menu");
    assert_eq!(log(), vec!["on_close".to_string()]);
}

/// A clipping (capped, scrolling) panel does not clip the flyout.
#[wasm_bindgen_test]
fn a_clipping_panel_does_not_clip_the_flyout() {
    dropdown(
        "position: fixed; left: 100px; top: 40px; line-height: 0;".into(),
        Some("max-height: 100px; overflow-y: auto;"),
    );
    click(&row("s-new"));
    let p = center(&el("i-folder"));
    assert!(
        p.0 > rect(&el("panel")).2,
        "premise: outside the panel's clip"
    );
    let hit = doc().element_from_point(p.0 as f32, p.1 as f32).unwrap();
    assert!(
        el("i-folder").contains(Some(&hit)),
        "hit {:?}",
        hit.outer_html()
    );
}

/// At the viewport's right and bottom edges: flipped left, shifted up.
#[wasm_bindgen_test]
fn at_the_edge_it_flips_left_and_shifts_up() {
    let (vw, vh) = viewport();
    // Root panel fits below its trigger; Deep's five rows would not.
    let left = vw - 240.0;
    let top = vh - 180.0;
    dropdown(
        format!("position: fixed; left: {left}px; top: {top}px; line-height: 0;"),
        None,
    );
    click(&row("s-new"));
    click(&row("s-deep"));
    let panel = rect(&el("panel"));
    assert!(
        panel.3 <= vh,
        "premise: the root panel is below its trigger, inside"
    );
    let fly = rect(&flyout("s-new"));
    assert!(
        (fly.2 - (panel.0 + 3.0)).abs() < 0.6,
        "flipped left: {fly:?} {panel:?}"
    );
    let deep = rect(&flyout("s-deep"));
    assert!(
        deep.3 <= vh - 4.0 + 0.6,
        "shifted up inside: {deep:?} vh={vh}"
    );
}

/// A disabled submenu opens neither on hover nor on a press.
#[wasm_bindgen_test]
async fn a_disabled_submenu_does_not_open() {
    dropdown(
        "position: fixed; left: 100px; top: 40px; line-height: 0;".into(),
        None,
    );
    move_to(center(&row("s-off")));
    sleep_ms(500).await;
    click(&row("s-off"));
    assert!(!is_open("s-off"));
    move_to(center(&row("s-new")));
    sleep_ms(500).await;
    assert!(is_open("s-new"), "positive control");
}

/// `ContextMenu`: submenus three deep, a deep item closes the whole menu, and
/// the menu's keys work with the focus outside it.
#[wasm_bindgen_test]
fn a_context_menu_holds_submenus_and_answers_the_keyboard() {
    let opened: Rc<Cell<Option<Signal<bool>>>> = Rc::new(Cell::new(None));
    let opened_in = opened.clone();
    mount(move |scope| {
        let target = scope.create_element("div");
        target.set_attribute("id", "ctx-target");
        target.set_attribute(
            "style",
            "position: fixed; top: 0; left: 0; width: 100vw; height: 100vh",
        );
        let list = scope.create_element("div");
        for r in rows(scope) {
            list.append_child(&r);
        }
        let cm = ContextMenu::default();
        opened_in.set(Some(cm.opened));
        let w = scope.create_element("div");
        w.set_attribute("class", "fixture-2214");
        w.append_child(&cm.render(scope, &[target, list]));
        w
    });
    rinch_web::set_suppress_native_context_menu(false);
    let opened = opened.get().unwrap();
    let right = || {
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        init.set_button(2);
        init.set_buttons(2);
        init.set_client_x(100);
        init.set_client_y(60);
        let ev = web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap();
        el("ctx-target").dispatch_event(&ev).unwrap();
    };
    right();
    assert!(opened.get(), "positive control: the right press opened it");
    assert!(key("ArrowDown"));
    assert_eq!(focused_id(), "i-alpha");
    key("ArrowDown");
    key("ArrowRight");
    assert!(is_open("s-new"));
    assert_eq!(focused_id(), "i-file");
    key("Escape");
    assert!(!is_open("s-new") && opened.get(), "only the submenu");
    key("Escape");
    assert!(!opened.get(), "Escape closes the context menu");

    right();
    click(&row("s-new"));
    click(&row("s-deep"));
    assert!(is_open("s-deep"), "positive control: three deep");
    click(&el("i-leaf2"));
    assert_eq!(log(), vec!["Lemma".to_string()]);
    assert!(!opened.get(), "a deep item closes the context menu");
    assert!(!is_open("s-new") && !is_open("s-deep"));
}

// ---- Review of #2216 ----------------------------------------------------

/// Alpha, New ▸ (File, Deep ▸ (Leaf), Folder), Other ▸ (X), Beta.
fn two_subs() -> Signal<bool> {
    let open = Signal::new(true);
    mount(move |scope| {
        let trigger = scope.create_element("div");
        trigger.set_attribute("style", "width: 40px; height: 20px;");
        let target = DropdownMenuTarget.render(scope, &[trigger]);
        let leaf = item(scope, "Leaf", "i-leaf");
        let deep = sub(scope, "Deep", "s-deep", false, &[leaf]);
        let file = item(scope, "File", "i-file");
        let folder = item(scope, "Folder", "i-folder");
        let new = sub(scope, "New", "s-new", false, &[file, deep, folder]);
        let x = item(scope, "X", "i-x");
        let other = sub(scope, "Other", "s-other", false, &[x]);
        let kids = vec![
            item(scope, "Alpha", "i-alpha"),
            new,
            other,
            item(scope, "Beta", "i-beta"),
        ];
        let dd = DropdownMenuDropdown.render(scope, &kids);
        dd.set_attribute("id", "panel");
        let menu = DropdownMenu {
            opened_fn: Some(Rc::new(move || open.get())),
            on_close: Some(Callback::new(move || {
                LOG.with(|l| l.borrow_mut().push("on_close".into()));
                open.set(false);
            })),
            ..Default::default()
        }
        .render(scope, &[target, dd]);
        let w = scope.create_element("div");
        w.set_attribute("class", "fixture-2214");
        w.set_attribute(
            "style",
            "position: fixed; left: 60px; top: 40px; line-height: 0;",
        );
        w.append_child(&menu);
        w
    });
    open
}

/// Crossing a sibling submenu row into an open flyout, and out of the menu:
/// the sibling's pending open is dropped both times.
#[wasm_bindgen_test]
async fn a_crossed_sibling_submenu_does_not_open() {
    two_subs();
    click(&row("s-new"));
    click(&row("s-deep"));
    move_to(center(&el("i-file")));
    sleep_ms(500).await;
    assert!(is_open("s-new") && !is_open("s-deep"), "premise");
    move_to(center(&row("s-other")));
    move_to(center(&el("i-folder")));
    sleep_ms(500).await;
    assert!(!is_open("s-other"), "into the flyout: Other must not open");
    assert!(is_open("s-new"), "New stays");

    move_to(center(&row("s-other")));
    move_to((5.0, 5.0));
    sleep_ms(500).await;
    assert!(!is_open("s-other"), "out of the menu: Other must not open");
    // Positive control: resting on the row opens it.
    move_to(center(&row("s-other")));
    sleep_ms(500).await;
    assert!(is_open("s-other"));
}

/// A row that grows while the flyout is open: placed again, inside.
#[wasm_bindgen_test]
async fn a_flyout_whose_content_grows_is_placed_again() {
    two_subs();
    click(&row("s-new"));
    let style = doc().create_element("style").unwrap();
    style.set_text_content(Some(
        ".fixture-2214 { --rinch-dropdown-menu-sub-width: auto; } #i-file { width: 600px; }",
    ));
    doc().body().unwrap().append_child(&style).unwrap();
    sleep_ms(100).await;
    let fly = rect(&flyout("s-new"));
    let file = rect(&el("i-file"));
    style.remove();
    let (vw, _) = viewport();
    assert!(fly.2 >= file.2 - 0.6, "holds its row: {fly:?} {file:?}");
    assert!(fly.2 <= vw + 0.6, "inside the viewport: {fly:?} vw={vw}");
}

/// Type-ahead on the web: a single match is activated.
#[wasm_bindgen_test]
fn typeahead_activates_a_single_match() {
    two_subs();
    assert!(key("ArrowDown"), "positive control: the menu took a key");
    assert_eq!(focused_id(), "i-alpha");
    assert!(key("b"));
    assert_eq!(log(), vec!["Beta".to_string(), "on_close".to_string()]);
}

/// Menu roles.
#[wasm_bindgen_test]
fn rows_and_panels_carry_menu_roles() {
    two_subs();
    assert_eq!(el("panel").get_attribute("role").as_deref(), Some("menu"));
    assert_eq!(
        el("i-alpha").get_attribute("role").as_deref(),
        Some("menuitem")
    );
    assert_eq!(
        row("s-new").get_attribute("role").as_deref(),
        Some("menuitem")
    );
    assert_eq!(
        flyout("s-new").get_attribute("role").as_deref(),
        Some("menu")
    );
}
