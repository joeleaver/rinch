//! Review of PR #1601: the Tree's keyboard in a real browser. The PR has no
//! rinch-web test; this is the positive control that the key entry, the
//! cursor and the focus move work through rinch-web's delegation, plus R1
//! (`*` with Shift) on the web.
#![cfg(target_arch = "wasm32")]

use rinch::components::{Tree, TreeNodeData, UseTreeOptions, UseTreeReturn};
use rinch_core::Component;
use rinch_core::element::ThemeProviderProps;
use rinch_web::RootHandle;
use std::cell::RefCell;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-1601";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn mount() -> (web_sys::Document, UseTreeReturn) {
    let bdoc = web_sys::window().unwrap().document().unwrap();
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
    let state = UseTreeReturn::new(UseTreeOptions {
        initial_expanded: ["a".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), move |scope| {
        let shell = scope.create_element("div");
        let tree = Tree {
            data: vec![
                TreeNodeData::new("a", "Apples").with_children(vec![
                    TreeNodeData::new("a1", "Avocado"),
                    TreeNodeData::new("a2", "Banana"),
                ]),
                TreeNodeData::new("b", "Beets"),
                TreeNodeData::new("c", "Carrots").with_child(TreeNodeData::new("c1", "Celery")),
            ],
            tree: Some(state),
            select_on_click: true,
            keyboard: true,
            cursor_delay_ms: 0,
            ..Default::default()
        }
        .render(scope, &[]);
        shell.append_child(&tree);
        let after = scope.create_element("button");
        after.set_attribute("class", "after-1601");
        after.append_child(&scope.create_text("after"));
        shell.append_child(&after);
        shell
    });
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    (bdoc, state)
}

fn row(doc: &web_sys::Document, value: &str) -> web_sys::HtmlElement {
    doc.query_selector(&format!(
        "[{HOST_ATTR}] li[data-value='{value}'] > .rinch-tree__node-content"
    ))
    .unwrap()
    .unwrap_or_else(|| panic!("no row {value}"))
    .dyn_into::<web_sys::HtmlElement>()
    .unwrap()
}

fn keydown(el: &web_sys::Element, key: &str, shift: bool) -> web_sys::KeyboardEvent {
    let init = web_sys::KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    init.set_shift_key(shift);
    let ev = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap();
    el.dispatch_event(&ev).unwrap();
    ev
}

fn active_value(doc: &web_sys::Document) -> Option<String> {
    doc.active_element()
        .and_then(|e| e.parent_element())
        .and_then(|li| li.get_attribute("data-value"))
}

#[wasm_bindgen_test]
fn arrows_move_the_cursor_and_the_focus() {
    let (doc, state) = mount();
    let a1 = row(&doc, "a1");
    a1.focus().unwrap();
    a1.click();
    assert_eq!(
        state.cursor.get().as_deref(),
        Some("a1"),
        "control: the click"
    );
    let ev = keydown(&doc.active_element().unwrap(), "ArrowDown", false);
    assert!(ev.default_prevented(), "the tree consumed ArrowDown");
    assert_eq!(state.cursor.get().as_deref(), Some("a2"));
    assert_eq!(
        active_value(&doc).as_deref(),
        Some("a2"),
        "the focus moved too"
    );
    let ev = keydown(&doc.active_element().unwrap(), "c", false);
    assert!(ev.default_prevented());
    assert_eq!(state.cursor.get().as_deref(), Some("c"), "type-ahead");
}

/// R1 on the web: a `*` typed on the main keyboard carries shiftKey.
#[wasm_bindgen_test]
fn shift_star_expands_the_siblings() {
    let (doc, state) = mount();
    let b = row(&doc, "b");
    b.focus().unwrap();
    b.click();
    assert_eq!(
        state.cursor.get().as_deref(),
        Some("b"),
        "control: the click"
    );
    keydown(&doc.active_element().unwrap(), "*", true);
    assert!(
        state.expanded.get().contains("c"),
        "Shift+8 `*` expands the siblings: {:?}",
        state.expanded.get()
    );
}

/// One Tab stop (roving tabindex), and the focused row is the treeitem,
/// carrying its state, in a real browser.
#[wasm_bindgen_test]
fn one_tab_stop_and_the_focused_row_is_the_treeitem() {
    let (doc, state) = mount();
    let stops = doc
        .query_selector_all(&format!(
            "[{HOST_ATTR}] .rinch-tree__node-content[tabindex='0']"
        ))
        .unwrap()
        .length();
    assert_eq!(stops, 1, "one Tab stop");
    let a1 = row(&doc, "a1");
    a1.focus().unwrap();
    a1.click();
    keydown(&doc.active_element().unwrap(), "ArrowDown", false);
    assert_eq!(state.cursor.get().as_deref(), Some("a2"));
    let active = doc.active_element().unwrap();
    assert_eq!(active.get_attribute("role").as_deref(), Some("treeitem"));
    assert_eq!(
        active.get_attribute("aria-selected").as_deref(),
        Some("true")
    );
    assert_eq!(active.get_attribute("aria-level").as_deref(), Some("2"));
    assert_eq!(active.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(
        row(&doc, "a1").get_attribute("tabindex").as_deref(),
        Some("-1")
    );
}
