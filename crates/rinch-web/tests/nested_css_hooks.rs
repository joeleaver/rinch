//! One component instance's open or active state does not reach an instance
//! nested inside it — in a real browser (issue #760, review of #774).
//!
//! #760 moved the `DropdownMenu` and `Tooltip` reveals, and the `Tabs` active
//! styling, from inline writes on each instance's own nodes onto sheet rules
//! keyed off a class or attribute. `rinch-components` ships **one** stylesheet
//! to both backends, so the combinator those rules use is a question Chrome
//! answers exactly as desktop does: spelled as a *descendant* combinator, an
//! open outer menu opened every closed menu inside its panel (in Chrome as on
//! desktop, measured), and its nested backdrop covered the outer items. The
//! rules are child combinators now, except the DropdownMenu panel's, which is a
//! descendant rule with an exclusion so that it still reaches a panel behind an
//! rsx wrapper (the last fixture here). This is the browser twin of the nesting
//! and wrapper fixtures in `rinch/src/app/css_hook_760_tests.rs`.
//!
//! Every nesting fixture nests two instances in **different** states: in the
//! same state a leaking rule and a correct one agree, and the fixture would pass
//! against the leak.
//!
//! **Mount through `rinch_web::mount_into`, not a hand-rolled `RenderScope`** —
//! without the owner `mount_tree` pushes, component effects are disposed at
//! once and the `opened_fn` class is never written.
//!
//! Run with a chromedriver matching the installed Chrome:
//!
//! ```text
//! CHROMEDRIVER=/path/to/chromedriver \
//!   cargo test -p rinch-web --target wasm32-unknown-unknown --test nested_css_hooks
//! ```
#![cfg(target_arch = "wasm32")]

use rinch::components::{
    DropdownMenu, DropdownMenuDropdown, DropdownMenuItem, DropdownMenuTarget, Tab, Tabs, TabsList,
    TabsPanel, Tooltip,
};
use rinch::prelude::rsx;
use rinch_core::element::{IntoEventHandler, ThemeProviderProps};
use rinch_core::{Callback, Component, Signal};
use rinch_web::RootHandle;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

const HOST_ATTR: &str = "data-test-host-774";

thread_local! {
    static PREVIOUS: RefCell<Option<RootHandle>> = const { RefCell::new(None) };
}

fn browser_document() -> web_sys::Document {
    web_sys::window().unwrap().document().unwrap()
}

/// Mount `build` in a fresh host, unmounting the previous fixture's tree.
fn mount(
    build: impl Fn(&mut rinch_core::dom::RenderScope) -> rinch_core::dom::NodeHandle + 'static,
) -> web_sys::Document {
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
    bdoc.body().unwrap().append_child(&host).unwrap();
    let handle = rinch_web::mount_into(&host, ThemeProviderProps::default(), build);
    PREVIOUS.with(|p| *p.borrow_mut() = Some(handle));
    bdoc
}

fn one(doc: &web_sys::Document, selector: &str) -> web_sys::Element {
    doc.query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("nothing matches `{selector}`"))
}

fn computed(el: &web_sys::Element, prop: &str) -> String {
    web_sys::window()
        .unwrap()
        .get_computed_style(el)
        .unwrap()
        .unwrap()
        .get_property_value(prop)
        .unwrap()
}

/// A closed `DropdownMenu` inside an open one's panel stays closed, and the
/// outer menu's item — not the inner menu's backdrop — is what sits under a
/// pointer aimed at that item.
#[wasm_bindgen_test]
fn a_closed_dropdown_menu_nested_in_an_open_one_stays_closed_in_chrome() {
    let doc = mount(|scope| {
        let text = scope.create_text("Outer item");
        let item = DropdownMenuItem::default().render(scope, &[text]);
        item.set_attribute("data-probe", "outer-item");

        let inner_target = DropdownMenuTarget.render(scope, &[]);
        let inner_dropdown = DropdownMenuDropdown.render(scope, &[]);
        let inner = DropdownMenu {
            opened_fn: Some(Rc::new(|| false)),
            on_close: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[inner_target, inner_dropdown]);
        inner.set_attribute("data-probe", "inner");

        let outer_target = DropdownMenuTarget.render(scope, &[]);
        let outer_dropdown = DropdownMenuDropdown.render(scope, &[item, inner]);
        let outer = DropdownMenu {
            opened_fn: Some(Rc::new(|| true)),
            on_close: Some(Callback::new(|| {})),
            ..Default::default()
        }
        .render(scope, &[outer_target, outer_dropdown]);
        outer.set_attribute("data-probe", "outer");
        outer
    });

    let outer_panel = one(&doc, "[data-probe=outer] > .rinch-dropdown-menu__dropdown");
    let outer_backdrop = one(&doc, "[data-probe=outer] > .rinch-dropdown-menu__backdrop");
    let inner_panel = one(&doc, "[data-probe=inner] > .rinch-dropdown-menu__dropdown");
    let inner_backdrop = one(&doc, "[data-probe=inner] > .rinch-dropdown-menu__backdrop");

    assert_eq!(
        (
            computed(&outer_panel, "display"),
            computed(&outer_backdrop, "display")
        ),
        ("block".to_string(), "block".to_string()),
        "control: the outer menu is open"
    );
    assert_eq!(
        computed(&inner_panel, "display"),
        "none",
        "the outer menu's `--opened` does not reach the inner menu's panel"
    );
    assert_eq!(
        computed(&inner_backdrop, "display"),
        "none",
        "nor its backdrop"
    );

    let item = one(&doc, "[data-probe=outer-item]");
    let r = item.get_bounding_client_rect();
    assert!(
        r.width() > 0.0 && r.height() > 0.0,
        "precondition: the outer item has a box"
    );
    let hit = doc
        .element_from_point(
            (r.left() + r.width() / 2.0) as f32,
            (r.top() + r.height() / 2.0) as f32,
        )
        .expect("something is under the item's centre");
    assert!(
        item.contains(Some(hit.unchecked_ref())),
        "the outer item is what a pointer aimed at it hits, not the closed inner \
         menu's backdrop — got <{} class={:?}>",
        hit.tag_name(),
        hit.get_attribute("class")
    );
}

/// An open `Tooltip` does not open an un-hovered one inside its target, and a
/// `disabled` one does not hide an opened one inside its target.
#[wasm_bindgen_test]
fn nested_tooltips_keep_their_own_state_in_chrome() {
    for (outer_opened, outer_disabled, inner_opened, outer_display, inner_display) in [
        (true, false, false, "block", "none"),
        (false, true, true, "none", "block"),
    ] {
        let doc = mount(move |scope| {
            let inner_target = scope.create_element("span");
            let inner = Tooltip {
                label: "inner".to_string(),
                opened: inner_opened,
                ..Default::default()
            }
            .render(scope, &[inner_target]);
            inner.set_attribute("data-probe", "inner");
            let outer = Tooltip {
                label: "outer".to_string(),
                opened: outer_opened,
                disabled: outer_disabled,
                ..Default::default()
            }
            .render(scope, &[inner]);
            outer.set_attribute("data-probe", "outer");
            outer
        });
        let outer_content = one(&doc, "[data-probe=outer] > .rinch-tooltip__content");
        let inner_content = one(&doc, "[data-probe=inner] > .rinch-tooltip__content");
        let case = format!(
            "outer opened={outer_opened} disabled={outer_disabled}, inner opened={inner_opened}"
        );
        assert_eq!(
            computed(&outer_content, "display"),
            outer_display,
            "control ({case}): the outer tooltip's own content"
        );
        assert_eq!(
            computed(&inner_content, "display"),
            inner_display,
            "{case}: the inner tooltip's content follows its own state, not the outer one's"
        );
    }
}

/// A `default` Tabs inside a `pills` Tabs' panel does not take the pill fill on
/// its active tab.
#[wasm_bindgen_test]
fn nested_default_tabs_do_not_take_the_pill_fill_in_chrome() {
    fn tabs(
        scope: &mut rinch_core::dom::RenderScope,
        variant: &str,
        marker: &str,
        panel_a_kids: &[rinch_core::dom::NodeHandle],
    ) -> rinch_core::dom::NodeHandle {
        let mut tab_nodes = Vec::new();
        for value in ["a", "b"] {
            let label = scope.create_text(value);
            tab_nodes.push(
                Tab {
                    value: value.to_string(),
                    ..Default::default()
                }
                .render(scope, &[label]),
            );
        }
        let list = TabsList::default().render(scope, &tab_nodes);
        let panel_a = TabsPanel {
            value: "a".to_string(),
        }
        .render(scope, panel_a_kids);
        let panel_b = TabsPanel {
            value: "b".to_string(),
        }
        .render(scope, &[]);
        let t = Tabs {
            value: "a".to_string(),
            variant: variant.to_string(),
            ..Default::default()
        }
        .render(scope, &[list, panel_a, panel_b]);
        t.set_attribute("data-probe", marker);
        t
    }

    let doc = mount(|scope| {
        let inner = tabs(scope, "default", "inner", &[]);
        tabs(scope, "pills", "outer", &[inner])
    });
    let outer_active = one(
        &doc,
        "[data-probe=outer] > .rinch-tabs__list > .rinch-tabs__tab[data-active=\"true\"]",
    );
    let inner_active = one(
        &doc,
        "[data-probe=inner] > .rinch-tabs__list > .rinch-tabs__tab[data-active=\"true\"]",
    );
    let pill = computed(&outer_active, "background-color");
    assert_ne!(
        pill, "rgba(0, 0, 0, 0)",
        "control: the outer `pills` active tab is filled"
    );
    assert_eq!(
        computed(&inner_active, "background-color"),
        "rgba(0, 0, 0, 0)",
        "the nested `default` active tab is not pill-filled (outer fill was {pill})"
    );
    assert_ne!(
        computed(&inner_active, "color"),
        "rgb(255, 255, 255)",
        "nor given the pill's white text"
    );
}

/// The panel opens when `rsx!` has put a wrapper in front of it — the other half
/// of the trade the nesting fixture above makes (second review of #774).
///
/// Spelled `.rinch-dropdown-menu--opened > .rinch-dropdown-menu__dropdown`, the
/// open rule kept a nested closed menu closed and also never opened a menu whose
/// panel reached the root through the `display: contents` wrapper `rsx!` inserts
/// for an `{Option<NodeHandle>}` child or an `else if` branch. Written with real
/// `rsx!` for that reason: a hand-built `Component::render` tree has no wrapper.
#[wasm_bindgen_test]
fn a_dropdown_menu_opens_behind_an_rsx_wrapper_in_chrome() {
    fn panel_display(doc: &web_sys::Document) -> String {
        computed(&one(doc, ".rinch-dropdown-menu__dropdown"), "display")
    }

    // `{Option<NodeHandle>}`.
    let opened = Signal::new(false);
    let doc = mount(move |__scope| {
        let dropdown: Option<rinch_core::dom::NodeHandle> =
            Some(rsx! { DropdownMenuDropdown { DropdownMenuItem { "One" } } });
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                {dropdown}
            }
        }
    });
    let wrapper_is_contents = |doc: &web_sys::Document, shape: &str| {
        let parent = one(doc, ".rinch-dropdown-menu__dropdown")
            .parent_element()
            .unwrap();
        assert!(
            !parent
                .get_attribute("class")
                .is_some_and(|c| c.split_whitespace().any(|w| w == "rinch-dropdown-menu")),
            "precondition ({shape}): the panel is not a direct child of the menu root"
        );
        assert_eq!(
            computed(&parent, "display"),
            "contents",
            "precondition ({shape}): rsx's own `display: contents` wrapper"
        );
    };
    wrapper_is_contents(&doc, "{Option}");
    assert_eq!(panel_display(&doc), "none", "{{Option}}: closed");
    opened.set(true);
    assert_eq!(
        panel_display(&doc),
        "block",
        "{{Option}}: the open menu's panel is shown"
    );
    opened.set(false);
    assert_eq!(panel_display(&doc), "none", "{{Option}}: closed again");

    // An `else if` branch.
    let opened = Signal::new(false);
    let mode = Signal::new(2u32);
    let doc = mount(move |__scope| {
        rsx! {
            DropdownMenu { opened_fn: move || opened.get(), on_close: || {},
                DropdownMenuTarget { button { "t" } }
                if mode.get() == 1 {
                    span { "loading" }
                } else if mode.get() == 2 {
                    DropdownMenuDropdown { DropdownMenuItem { "One" } }
                }
            }
        }
    });
    wrapper_is_contents(&doc, "else if");
    assert_eq!(panel_display(&doc), "none", "else if: closed");
    opened.set(true);
    assert_eq!(
        panel_display(&doc),
        "block",
        "else if: the open menu's panel is shown"
    );
}
