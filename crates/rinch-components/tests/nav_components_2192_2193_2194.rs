//! Breadcrumbs (#2193), Pagination (#2194) and NavLink (#2192): the bugs the
//! configurability audit found, and the props it asked for.
//!
//! Each test renders the component into a `MockDomDocument` and reads the tree
//! back; a click is what the runtime does with one, `dispatch_event` on the
//! element's `data-rid`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_components::navlink::NavLink;
use rinch_components::{Breadcrumbs, BreadcrumbsItem, Pagination};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Callback, Component, ValueCallback};
use rinch_tabler_icons::TablerIcon;

mod common;
use common::{collect_by_class, declarations_for, find_by_class, glyph, glyph_of, has_class};
use common::{sheet, strip_comments};

// ------------------------------------------------------------------ helpers

/// A rendered tree, with the document and scope that own it kept alive.
struct Tree {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

impl Tree {
    fn build(build: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Self {
        let doc = Rc::new(RefCell::new(MockDomDocument::new()));
        let body = doc.borrow().body();
        let mut scope = RenderScope::new(doc.clone(), body);
        let root = build(&mut scope);
        scope.body_handle().append_child(&root);
        Self {
            _doc: doc,
            _scope: scope,
            root,
        }
    }

    fn find(&self, class: &str) -> NodeHandle {
        find_by_class(&self.root, class).unwrap_or_else(|| panic!("no .{class} in the tree"))
    }

    fn all(&self, class: &str) -> Vec<NodeHandle> {
        let mut out = Vec::new();
        collect_by_class(&self.root, class, &mut out);
        out
    }
}

/// Click `node` the way the runtime does. `false` when it carries no handler.
fn click(node: &NodeHandle) -> bool {
    match node.get_attribute("data-rid") {
        Some(rid) => dispatch_event(EventHandlerId(rid.parse().expect("numeric data-rid"))),
        None => false,
    }
}

/// The text in `node`'s subtree.
fn text(node: &NodeHandle) -> String {
    node.text_content().unwrap_or_default()
}

fn crumbs(crumbs: Breadcrumbs, items: Vec<(BreadcrumbsItem, &'static str)>) -> Tree {
    Tree::build(move |scope| {
        let children: Vec<NodeHandle> = items
            .into_iter()
            .map(|(item, label)| {
                let t = scope.create_text(label);
                item.render(scope, &[t])
            })
            .collect();
        crumbs.render(scope, &children)
    })
}

fn three_crumbs(b: Breadcrumbs) -> Tree {
    crumbs(
        b,
        vec![
            (
                BreadcrumbsItem {
                    href: "/".into(),
                    ..Default::default()
                },
                "Home",
            ),
            (
                BreadcrumbsItem {
                    href: "/docs".into(),
                    ..Default::default()
                },
                "Docs",
            ),
            (BreadcrumbsItem::default(), "Page"),
        ],
    )
}

fn separators(tree: &Tree) -> Vec<String> {
    tree.all("rinch-breadcrumbs__separator")
        .iter()
        .map(text)
        .collect()
}

// ------------------------------------------------------- Breadcrumbs (#2193)

#[test]
fn any_breadcrumb_separator_is_drawn_as_given() {
    for sep in ["›", "|", "/", "&", "<>"] {
        let tree = three_crumbs(Breadcrumbs {
            separator: sep.into(),
            ..Default::default()
        });
        assert_eq!(
            separators(&tree),
            vec![sep.to_string(), sep.to_string()],
            "#2193: `separator: {sep:?}` must be drawn as itself between every \
             pair of items — it used to fall to `/` unless it was one of five \
             whitelisted strings, and `&` was HTML-escaped into `&amp;`"
        );
    }
}

#[test]
fn the_default_breadcrumb_separator_is_a_slash_and_the_last_item_has_none() {
    let tree = three_crumbs(Breadcrumbs::default());
    assert_eq!(separators(&tree), vec!["/", "/"]);
    let items = tree.all("rinch-breadcrumbs__item");
    assert_eq!(items.len(), 3);
    assert!(
        find_by_class(&items[2], "rinch-breadcrumbs__separator").is_none(),
        "the last item has no trailing separator"
    );
    for sep in tree.all("rinch-breadcrumbs__separator") {
        assert_eq!(
            sep.get_attribute("aria-hidden").as_deref(),
            Some("true"),
            "a separator is decoration (WAI-ARIA breadcrumb pattern)"
        );
    }
}

#[test]
fn the_breadcrumb_stylesheet_has_no_slash_fallback_left() {
    let css = strip_comments(&sheet());
    assert!(
        !css.contains("content: '/'"),
        "#2193: the `[data-separator]` fallback rule that drew `/` for every \
         unlisted separator must be gone"
    );
}

#[test]
fn the_current_breadcrumb_carries_aria_current() {
    let tree = three_crumbs(Breadcrumbs::default());
    let current = tree.find("rinch-breadcrumbs__item--active");
    assert_eq!(current.get_attribute("aria-current").as_deref(), Some("page"));
    for link in tree.all("rinch-breadcrumbs__link") {
        assert_eq!(link.get_attribute("aria-current"), None);
    }
}

// --------------------------------------------------------- Pagination (#2194)

fn pagination(p: Pagination) -> Tree {
    Tree::build(move |scope| p.render(scope, &[]))
}

fn control(tree: &Tree, data: &str) -> NodeHandle {
    tree.all("rinch-pagination__control")
        .into_iter()
        .find(|n| n.get_attribute(data).is_some())
        .unwrap_or_else(|| panic!("no control with {data}"))
}

#[test]
fn pagination_controls_have_accessible_names_by_default() {
    let tree = pagination(Pagination {
        total: 10,
        value: 5,
        with_edges: true,
        ..Default::default()
    });
    for (data, label) in [
        ("data-first", "First page"),
        ("data-prev", "Previous page"),
        ("data-next", "Next page"),
        ("data-last", "Last page"),
    ] {
        assert_eq!(
            control(&tree, data).get_attribute("aria-label").as_deref(),
            Some(label),
            "#2194: an icon-only control needs an accessible name"
        );
    }
    assert_eq!(
        tree.root.get_attribute("aria-label").as_deref(),
        Some("Pagination")
    );
}

#[test]
fn the_active_page_carries_aria_current() {
    let tree = pagination(Pagination {
        total: 10,
        value: 5,
        ..Default::default()
    });
    for item in tree.all("rinch-pagination__item") {
        let active = item.get_attribute("data-page").as_deref() == Some("5");
        assert_eq!(
            item.get_attribute("aria-current").as_deref(),
            active.then_some("page"),
            "page {:?}",
            item.get_attribute("data-page")
        );
    }
}

// ------------------------------------------------------------ NavLink (#2192)

fn navlink_with_children(nav: NavLink) -> Tree {
    Tree::build(move |scope| {
        let a = NavLink {
            label: "Child A".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        let b = NavLink {
            label: "Child B".into(),
            ..Default::default()
        }
        .render(scope, &[]);
        nav.render(scope, &[a, b])
    })
}

/// The parent's own control: the first `.rinch-navlink` in the tree.
fn parent_control(tree: &Tree) -> NodeHandle {
    tree.find("rinch-navlink")
}

fn collapsed(tree: &Tree) -> bool {
    has_class(
        &tree.find("rinch-navlink__children"),
        "rinch-navlink__children--collapsed",
    )
}

#[test]
fn clicking_a_navlink_with_children_toggles_them() {
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        ..Default::default()
    });
    assert!(collapsed(&tree), "closed by default");
    let control = parent_control(&tree);
    assert!(
        click(&control),
        "#2192: a NavLink with children must toggle on click — it used to \
         carry no handler at all unless the app passed `onclick`"
    );
    assert!(!collapsed(&tree), "one click opens it");
    assert!(has_class(
        &tree.find("rinch-navlink__chevron"),
        "rinch-navlink__chevron--opened"
    ));
    assert_eq!(control.get_attribute("aria-expanded").as_deref(), Some("true"));
    click(&control);
    assert!(collapsed(&tree), "a second click closes it again");
    assert_eq!(
        control.get_attribute("aria-expanded").as_deref(),
        Some("false")
    );
}

#[test]
fn a_navlink_toggle_still_calls_the_apps_onclick() {
    let clicks = Rc::new(Cell::new(0));
    let sink = clicks.clone();
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        default_opened: true,
        onclick: Some(Callback::new(move || sink.set(sink.get() + 1))),
        ..Default::default()
    });
    assert!(!collapsed(&tree), "`default_opened` opens it");
    click(&parent_control(&tree));
    assert_eq!(clicks.get(), 1);
    assert!(collapsed(&tree), "and the click closes it");
}

#[test]
fn a_disabled_navlink_anchor_registers_no_click() {
    let clicks = Rc::new(Cell::new(0));
    let sink = clicks.clone();
    let tree = Tree::build(move |scope| {
        NavLink {
            label: "Off".into(),
            href: "/off".into(),
            disabled: true,
            onclick: Some(Callback::new(move || sink.set(sink.get() + 1))),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let a = parent_control(&tree);
    assert!(
        !click(&a),
        "#2192: a disabled `<a>` NavLink must not be clickable — keyboard \
         activation reached its handler, where `pointer-events: none` stopped \
         only the mouse"
    );
    assert_eq!(clicks.get(), 0);
    assert_eq!(a.get_attribute("aria-disabled").as_deref(), Some("true"));
}

#[test]
fn a_navlink_with_children_keeps_its_right_section() {
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        right_section: Some(TablerIcon::Plus),
        ..Default::default()
    });
    let right = tree.find("rinch-navlink__right");
    assert_eq!(
        glyph(&right),
        glyph_of(TablerIcon::Plus),
        "#2192: the app's `right_section` used to be replaced silently by the \
         hand-drawn chevron when the link had children"
    );
}

#[test]
fn open_navlink_children_are_not_clipped_at_1000px() {
    let css = strip_comments(&sheet());
    let decls = declarations_for(&css, ".rinch-navlink__children");
    assert!(
        !decls.iter().any(|d| d == "max-height: 1000px"),
        "#2192: open children taller than 1000px were clipped: {decls:?}"
    );
}

#[allow(dead_code)]
fn _uses(_: ValueCallback<u32>) {}
