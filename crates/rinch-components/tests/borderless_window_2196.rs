//! `BorderlessWindow`: a plain title, window controls placed per platform or
//! by the app, and a configurable titlebar height (issue #2196).
//!
//! Before: in the inline-menu layout the title was a fixed six-colour rainbow
//! of inline-styled spans no stylesheet could override; the controls were
//! minimize, maximize, close on the right on every platform; and the titlebar
//! height was a 36px constant in both the Rust inset and the CSS. The real
//! stylesheet's answer to the height is read in
//! `crates/rinch/src/app/stepper_window_2195_tests.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::borderless_window::{WindowControl, WindowControlsLayout};
use rinch_components::{BorderlessWindow, SectionRenderer};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::{Component, MenuBarContext, MenuBarLayout, MenuBarRenderer};

mod common;
use common::{collect_by_class, find_by_class, has_class};

fn new_scope() -> (Rc<RefCell<MockDomDocument>>, RenderScope) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let scope = RenderScope::new(doc.clone(), body);
    (doc, scope)
}

fn empty_renderer() -> MenuBarRenderer {
    Rc::new(|scope: &mut RenderScope| scope.create_element("div"))
}

/// Install the menu-bar context the shell sets up for a borderless window.
fn menu_context(layout: MenuBarLayout, bar_height: u32) {
    let inline = layout == MenuBarLayout::InlineTitlebar;
    rinch_core::create_context(MenuBarContext {
        renderer: empty_renderer(),
        bar_height,
        layout,
        items_renderer: inline.then(empty_renderer),
        overlay_renderer: inline.then(empty_renderer),
        spacer_width: 0,
    });
}

/// The controls under `node`, as `(side, [control modifier…])` in order.
fn controls(node: &NodeHandle) -> Vec<(String, Vec<String>)> {
    let mut boxes = Vec::new();
    collect_by_class(node, "rinch-borderlesswindow__controls", &mut boxes);
    boxes
        .iter()
        .map(|b| {
            let side = if has_class(b, "rinch-borderlesswindow__controls--left") {
                "left"
            } else {
                "right"
            };
            let names = b
                .children()
                .iter()
                .map(|c| {
                    let class = c.get_attribute("class").unwrap_or_default();
                    class
                        .split_whitespace()
                        .find_map(|k| k.strip_prefix("rinch-borderlesswindow__control--"))
                        .unwrap_or("?")
                        .to_string()
                })
                .collect();
            (side.to_string(), names)
        })
        .filter(|(_, names): &(String, Vec<String>)| !names.is_empty())
        .collect()
}

fn titlebar(root: &NodeHandle) -> NodeHandle {
    find_by_class(root, "rinch-borderlesswindow__titlebar").expect("a titlebar")
}

#[test]
fn the_inline_title_is_plain_text() {
    let (_doc, mut scope) = new_scope();
    menu_context(MenuBarLayout::InlineTitlebar, 0);
    let root = BorderlessWindow {
        title: "My App".into(),
        ..Default::default()
    }
    .render(&mut scope, &[]);

    let brand = find_by_class(&root, "rinch-borderlesswindow__brand").expect("a title");
    assert_eq!(brand.text_content().as_deref(), Some("My App"));
    fn styled(n: &NodeHandle) -> bool {
        n.get_attribute("style").is_some() || n.children().iter().any(styled)
    }
    assert!(
        !styled(&brand),
        "no inline style under the title — a stylesheet can colour it"
    );
}

#[test]
fn title_section_replaces_the_title_in_both_layouts() {
    let section: SectionRenderer = Rc::new(|scope: &mut RenderScope| {
        let logo = scope.create_element("img");
        logo.set_attribute("class", "my-logo");
        logo
    });
    for inline in [false, true] {
        let (_doc, mut scope) = new_scope();
        if inline {
            menu_context(MenuBarLayout::InlineTitlebar, 0);
        }
        let root = BorderlessWindow {
            title: "ignored".into(),
            title_section: Some(section.clone()),
            ..Default::default()
        }
        .render(&mut scope, &[]);
        let holder = if inline {
            "rinch-borderlesswindow__brand"
        } else {
            "rinch-borderlesswindow__title"
        };
        let holder = find_by_class(&root, holder).expect("the title's element");
        assert!(
            find_by_class(&holder, "my-logo").is_some(),
            "inline {inline}"
        );
        assert_eq!(
            holder.text_content().unwrap_or_default(),
            "",
            "inline {inline}"
        );
    }
}

#[test]
fn controls_follow_the_layout_prop() {
    // macOS: the traffic lights, first in the titlebar.
    let (_doc, mut scope) = new_scope();
    let root = BorderlessWindow {
        controls_layout: Some(WindowControlsLayout::macos()),
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let tb = titlebar(&root);
    assert_eq!(
        controls(&tb),
        [(
            "left".to_string(),
            vec!["close".into(), "minimize".into(), "maximize".into()]
        )]
    );
    assert!(
        has_class(&tb.children()[0], "rinch-borderlesswindow__controls--left"),
        "left controls open the titlebar row"
    );
    let kids = tb.children();
    assert!(
        has_class(
            kids.last().unwrap(),
            "rinch-borderlesswindow__controls--right"
        ),
        "the right box still closes the row"
    );

    // A split layout in GTK syntax, with `show_*` still hiding one.
    let (_doc, mut scope) = new_scope();
    let root = BorderlessWindow {
        controls_layout: Some("close:maximize,minimize".parse().unwrap()),
        show_minimize: false,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    assert_eq!(
        controls(&titlebar(&root)),
        [
            ("left".to_string(), vec!["close".to_string()]),
            ("right".to_string(), vec!["maximize".to_string()]),
        ]
    );
}

#[test]
fn inline_left_controls_lead_the_menu_row() {
    let (_doc, mut scope) = new_scope();
    menu_context(MenuBarLayout::InlineTitlebar, 0);
    let root = BorderlessWindow {
        title: "App".into(),
        controls_layout: Some(WindowControlsLayout::macos()),
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let row = find_by_class(&root, "rinch-app-menu-bar__inline-row").expect("the menu row");
    assert!(
        has_class(&row.children()[0], "rinch-borderlesswindow__controls--left"),
        "drawn over the titlebar's left end, the row is where they show"
    );
    assert_eq!(
        controls(&titlebar(&root)),
        Vec::<(String, Vec<String>)>::new()
    );
}

#[test]
fn gtk_layout_strings_parse() {
    use WindowControl::*;
    let l = WindowControlsLayout::from_gtk("'appmenu:minimize,maximize,close'\n");
    assert_eq!((l.left, l.right), (vec![], vec![Minimize, Maximize, Close]));
    let l = WindowControlsLayout::from_gtk("close,minimize,maximize:appmenu");
    assert_eq!((l.left, l.right), (vec![Close, Minimize, Maximize], vec![]));
    let l = WindowControlsLayout::from_gtk("icon,spacer,close");
    assert_eq!(
        (l.left, l.right),
        (vec![Close], vec![]),
        "no colon: all on the left"
    );
    let l = WindowControlsLayout::from_gtk("appmenu:close");
    assert_eq!(
        (l.left, l.right),
        (vec![], vec![Close]),
        "GNOME's stock layout"
    );
}

#[test]
fn titlebar_height_drives_the_variable_the_inset_and_the_menu_bar() {
    let (_doc, mut scope) = new_scope();
    menu_context(MenuBarLayout::BelowTitlebar, 28);
    let root = BorderlessWindow {
        titlebar_height: 48,
        ..Default::default()
    }
    .render(&mut scope, &[]);
    let style = root.get_attribute("style").unwrap_or_default();
    assert!(style.contains("--rinch-titlebar-height: 48px"), "{style}");
    assert!(
        style.contains("--rinch-window-top-inset: 76px"),
        "48 of titlebar and 28 of menu bar: {style}"
    );
    // The menu bar the shell rendered is the window's last child.
    let bar = root.children().last().cloned().unwrap();
    assert!(
        bar.get_attribute("style")
            .unwrap_or_default()
            .contains("top: 48px"),
        "the menu bar sits under this titlebar: {:?}",
        bar.get_attribute("style")
    );
}

#[test]
fn the_default_titlebar_is_36px() {
    let (_doc, mut scope) = new_scope();
    let root = BorderlessWindow::default().render(&mut scope, &[]);
    let style = root.get_attribute("style").unwrap_or_default();
    assert!(style.contains("--rinch-titlebar-height: 36px"), "{style}");
    assert!(style.contains("--rinch-window-top-inset: 36px"), "{style}");
}
