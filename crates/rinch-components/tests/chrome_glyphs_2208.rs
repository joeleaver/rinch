//! #2208: the built-in chrome glyphs (close X, chevrons, check) are
//! replaceable app-wide through an `IconSet` context, and a component's
//! `color` prop carries the contrast its filled text paints.
//!
//! Without an `IconSet` (or with a slot left `None`) each component renders
//! the glyph it always rendered.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::accordion::AccordionControl;
use rinch_components::action_icon::ActionIcon;
use rinch_components::alert::Alert;
use rinch_components::close_button::CloseButton;
use rinch_components::{
    Avatar, IconSet, NavLink, Pagination, SectionRenderer, Stepper, Tabs, icons,
};
use rinch_core::Component;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_tabler_icons::TablerIcon;

mod common;
use common::{find_by_class, glyph, glyph_of};

struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    scope: RenderScope,
}

fn mounted() -> Mounted {
    rinch_core::clear_context();
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let scope = RenderScope::new(doc.clone(), body);
    Mounted { _doc: doc, scope }
}

/// A glyph that draws `<svg data-glyph="{name}">`.
fn marker(name: &'static str) -> SectionRenderer {
    Rc::new(move |scope: &mut RenderScope| {
        let svg = scope.create_element("svg");
        svg.set_attribute("data-glyph", name);
        svg
    })
}

/// Every `data-glyph` in `node`'s subtree, in document order.
fn markers(node: &NodeHandle) -> Vec<String> {
    fn walk(node: &NodeHandle, out: &mut Vec<String>) {
        if let Some(g) = node.get_attribute("data-glyph") {
            out.push(g);
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn full_set() -> IconSet {
    IconSet::default()
        .close(marker("close"))
        .chevron_down(marker("chevron_down"))
        .chevron_left(marker("chevron_left"))
        .chevron_right(marker("chevron_right"))
        .chevrons_left(marker("chevrons_left"))
        .chevrons_right(marker("chevrons_right"))
        .check(marker("check"))
}

fn pagination() -> Pagination {
    Pagination {
        total: 5,
        value: 2,
        with_edges: true,
        with_controls: true,
        ..Default::default()
    }
}

fn alert() -> Alert {
    Alert {
        with_close_button: true,
        ..Default::default()
    }
}

#[test]
fn without_a_set_the_built_in_glyphs_render() {
    let mut m = mounted();
    let s = &mut m.scope;
    assert!(markers(&CloseButton::default().render(s, &[])).is_empty());
    let close = CloseButton::default().render(s, &[]);
    assert_eq!(
        glyph(&close),
        vec!["M18 6L6 18M6 6l12 12".to_string()],
        "the built-in X"
    );
    assert!(markers(&alert().render(s, &[])).is_empty());
    assert!(markers(&AccordionControl::default().render(s, &[])).is_empty());
    assert!(markers(&pagination().render(s, &[])).is_empty());
    assert!(markers(&icons::check_dom(s)).is_empty());

    // An empty set changes nothing either.
    rinch_core::create_context(IconSet::default());
    assert_eq!(
        glyph(&CloseButton::default().render(s, &[])),
        vec!["M18 6L6 18M6 6l12 12".to_string()]
    );
    assert!(markers(&pagination().render(s, &[])).is_empty());
}

#[test]
fn the_set_replaces_each_listed_glyph() {
    let mut m = mounted();
    rinch_core::create_context(full_set());
    let s = &mut m.scope;

    let close = CloseButton::default().render(s, &[]);
    assert_eq!(markers(&close), ["close"]);

    let alert = alert().render(s, &[]);
    let alert_close = find_by_class(&alert, "rinch-alert__close").unwrap();
    assert_eq!(markers(&alert_close), ["close"]);
    let svg = &alert_close.children()[0];
    assert_eq!(svg.get_attribute("width").as_deref(), Some("14"), "sized");
    assert_eq!(svg.get_attribute("height").as_deref(), Some("14"));

    let control = AccordionControl::default().render(s, &[]);
    assert_eq!(markers(&control), ["chevron_down"]);
    assert!(
        find_by_class(&control, "rinch-accordion__chevron").is_some(),
        "the replacement carries the chevron's class, which the CSS rotates"
    );

    let pages = pagination().render(s, &[]);
    assert_eq!(
        markers(&pages),
        [
            "chevrons_left",
            "chevron_left",
            "chevron_right",
            "chevrons_right"
        ]
    );

    assert_eq!(markers(&icons::check_dom(s)), ["check"]);
    assert_eq!(markers(&icons::close_icon_lines_dom(s)), ["close"]);
}

#[test]
fn a_components_own_icon_still_wins() {
    let mut m = mounted();
    rinch_core::create_context(full_set());
    let s = &mut m.scope;
    let close = CloseButton {
        icon: Some(TablerIcon::Trash),
        ..Default::default()
    }
    .render(s, &[]);
    assert!(markers(&close).is_empty());
    assert_eq!(glyph(&close), glyph_of(TablerIcon::Trash));
}

#[test]
fn a_tabler_glyph_is_sized_by_the_component() {
    let mut m = mounted();
    rinch_core::create_context(IconSet::default().close(IconSet::tabler(TablerIcon::SquareX)));
    let s = &mut m.scope;
    let close = icons::close_icon_dom(s, "18");
    assert_eq!(glyph(&close), glyph_of(TablerIcon::SquareX));
    assert_eq!(close.get_attribute("style"), None, "no inline 1em size");
    assert_eq!(close.get_attribute("width").as_deref(), Some("18"));
}

/// Each component whose `color` prop fills it writes the contrast for that
/// colour beside the colour, palette name or raw colour.
#[test]
fn a_colour_prop_writes_its_contrast() {
    fn style_in(node: &NodeHandle) -> String {
        fn walk(node: &NodeHandle, out: &mut String) {
            if let Some(s) = node.get_attribute("style") {
                out.push_str(&s);
                out.push(';');
            }
            for child in node.children() {
                walk(&child, out);
            }
        }
        let mut out = String::new();
        walk(node, &mut out);
        out
    }
    let mut m = mounted();
    let s = &mut m.scope;
    for (color, want) in [
        ("yellow", "var(--rinch-color-yellow-contrast)"),
        ("#ffe066", "black"),
        ("#1864ab", "white"),
    ] {
        let c = color.to_string();
        let rendered = [
            (
                "--rinch-action-icon",
                ActionIcon {
                    color: c.clone(),
                    ..Default::default()
                }
                .render(s, &[]),
            ),
            (
                "--rinch-avatar",
                Avatar {
                    color: c.clone(),
                    ..Default::default()
                }
                .render(s, &[]),
            ),
            (
                "--rinch-pagination",
                Pagination {
                    color: c.clone(),
                    ..pagination()
                }
                .render(s, &[]),
            ),
            (
                "--rinch-navlink",
                NavLink {
                    color: c.clone(),
                    ..Default::default()
                }
                .render(s, &[]),
            ),
            (
                "--rinch-tabs",
                Tabs {
                    color: c.clone(),
                    ..Default::default()
                }
                .render(s, &[]),
            ),
            (
                "--rinch-stepper",
                Stepper {
                    color: c.clone(),
                    ..Default::default()
                }
                .render(s, &[]),
            ),
        ];
        for (prefix, node) in rendered {
            let style = style_in(&node);
            assert!(
                style.contains(&format!("{prefix}-contrast: {want}")),
                "{prefix} {color}: {style}"
            );
        }
        let button = rinch_components::Button {
            color: c.clone(),
            ..Default::default()
        }
        .style_string()
        .unwrap();
        assert!(
            button.contains(&format!("--rinch-button-color-contrast: {want}")),
            "button {color}: {button}"
        );
    }
}
