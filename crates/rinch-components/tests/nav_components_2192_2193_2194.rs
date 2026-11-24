//! Breadcrumbs (#2193), Pagination (#2194) and NavLink (#2192): the bugs the
//! configurability audit found, and the props it asked for.
//!
//! Each test renders the component into a `MockDomDocument` and reads the tree
//! back; a click is what the runtime does with one, `dispatch_event` on the
//! element's `data-rid`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_components::navlink::NavLink;
use rinch_components::{Breadcrumbs, BreadcrumbsItem, Pagination, SectionRenderer};
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_core::events::{EventHandlerId, dispatch_event};
use rinch_core::{Callback, Component, Signal, ValueCallback};
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
    assert_eq!(
        current.get_attribute("aria-current").as_deref(),
        Some("page")
    );
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
    assert_eq!(
        control.get_attribute("aria-expanded").as_deref(),
        Some("true")
    );
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

// ------------------------------------------------------- the new props

/// A renderer that draws `<i class="{class}">`.
fn marker(class: &'static str) -> SectionRenderer {
    Rc::new(move |scope: &mut RenderScope| {
        let i = scope.create_element("i");
        i.set_attribute("class", class);
        i
    })
}

#[test]
fn a_breadcrumb_separator_can_be_an_icon_or_anything_the_app_renders() {
    let tree = three_crumbs(Breadcrumbs {
        separator: "|".into(),
        separator_icon: Some(TablerIcon::ChevronRight),
        ..Default::default()
    });
    let seps = tree.all("rinch-breadcrumbs__separator");
    assert_eq!(seps.len(), 2);
    for sep in &seps {
        assert_eq!(glyph(sep), glyph_of(TablerIcon::ChevronRight));
        assert_eq!(text(sep), "", "the icon replaces the text");
    }

    let tree = three_crumbs(Breadcrumbs {
        separator_icon: Some(TablerIcon::ChevronRight),
        separator_fn: Some(marker("my-sep")),
        ..Default::default()
    });
    let seps = tree.all("rinch-breadcrumbs__separator");
    assert_eq!(seps.len(), 2, "one call per separator");
    for sep in &seps {
        assert!(find_by_class(sep, "my-sep").is_some(), "the renderer wins");
        assert!(glyph(sep).is_empty());
    }
}

#[test]
fn the_breadcrumb_navs_accessible_name_is_a_prop() {
    let tree = three_crumbs(Breadcrumbs::default());
    assert_eq!(
        tree.root.get_attribute("aria-label").as_deref(),
        Some("Breadcrumb")
    );
    let tree = three_crumbs(Breadcrumbs {
        aria_label: "Fil d'Ariane".into(),
        ..Default::default()
    });
    assert_eq!(
        tree.root.get_attribute("aria-label").as_deref(),
        Some("Fil d'Ariane")
    );
}

#[test]
fn a_breadcrumb_item_routes_by_callback() {
    let clicks = Rc::new(Cell::new(0));
    let sink = clicks.clone();
    let tree = crumbs(
        Breadcrumbs::default(),
        vec![
            (
                BreadcrumbsItem {
                    onclick: Some(Callback::new(move || sink.set(sink.get() + 1))),
                    ..Default::default()
                },
                "Home",
            ),
            (BreadcrumbsItem::default(), "Page"),
        ],
    );
    let link = tree.find("rinch-breadcrumbs__link");
    assert_eq!(link.tag_name().as_deref(), Some("button"));
    assert_eq!(link.get_attribute("type").as_deref(), Some("button"));
    assert_eq!(
        link.get_attribute("aria-current"),
        None,
        "an item with `onclick` is not the current page"
    );
    assert!(click(&link));
    assert_eq!(clicks.get(), 1);
    assert_eq!(text(&link), "Home");
}

#[test]
fn a_breadcrumb_items_active_overrides_the_guess() {
    let tree = crumbs(
        Breadcrumbs::default(),
        vec![
            (
                BreadcrumbsItem {
                    href: "/here".into(),
                    active: Some(true),
                    ..Default::default()
                },
                "Here",
            ),
            (
                BreadcrumbsItem {
                    active: Some(false),
                    ..Default::default()
                },
                "Inert",
            ),
        ],
    );
    let link = tree.find("rinch-breadcrumbs__link");
    assert_eq!(link.tag_name().as_deref(), Some("a"));
    assert!(has_class(&link, "rinch-breadcrumbs__item--active"));
    assert_eq!(link.get_attribute("aria-current").as_deref(), Some("page"));
    let items = tree.all("rinch-breadcrumbs__item");
    let inert = items[1].children()[0].clone();
    assert!(!has_class(&inert, "rinch-breadcrumbs__item--active"));
    assert_eq!(inert.get_attribute("aria-current"), None);
}

#[test]
fn breadcrumb_separators_follow_items_that_arrive_and_leave() {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let a = BreadcrumbsItem::default().render(&mut scope, &[]);
    let b = BreadcrumbsItem::default().render(&mut scope, &[]);
    let nav = Breadcrumbs {
        separator: "›".into(),
        ..Default::default()
    }
    .render(&mut scope, &[a.clone(), b.clone()]);
    scope.body_handle().append_child(&nav);
    let count = |nav: &NodeHandle| {
        let mut out = Vec::new();
        collect_by_class(nav, "rinch-breadcrumbs__separator", &mut out);
        out.len()
    };
    let has_sep = |item: &NodeHandle| find_by_class(item, "rinch-breadcrumbs__separator").is_some();
    assert_eq!(count(&nav), 1);

    // A crumb a `for` loop appends later: the old last item gains one.
    let c = BreadcrumbsItem::default().render(&mut scope, &[]);
    let list = find_by_class(&nav, "rinch-breadcrumbs__list").unwrap();
    list.append_child(&c);
    assert!(
        has_sep(&b),
        "the item that stopped being last gains a separator"
    );
    assert!(!has_sep(&c), "the new last item has none");
    assert_eq!(count(&nav), 2);

    // And it leaves again: the new last item loses its separator.
    c.remove();
    assert!(!has_sep(&b));
    assert_eq!(count(&nav), 1);
}

#[test]
fn pagination_labels_icons_and_dots_are_props() {
    let tree = pagination(Pagination {
        total: 20,
        value: 10,
        with_edges: true,
        aria_label: "Seiten".into(),
        first_label: "Erste Seite".into(),
        previous_label: "Vorherige Seite".into(),
        next_label: "Nächste Seite".into(),
        last_label: "Letzte Seite".into(),
        first_icon: Some(TablerIcon::ArrowBarToLeft),
        previous_icon: Some(TablerIcon::ArrowLeft),
        next_icon: Some(TablerIcon::ArrowRight),
        last_icon: Some(TablerIcon::ArrowBarToRight),
        dots: "…".into(),
        ..Default::default()
    });
    assert_eq!(
        tree.root.get_attribute("aria-label").as_deref(),
        Some("Seiten")
    );
    for (data, label, icon) in [
        ("data-first", "Erste Seite", TablerIcon::ArrowBarToLeft),
        ("data-prev", "Vorherige Seite", TablerIcon::ArrowLeft),
        ("data-next", "Nächste Seite", TablerIcon::ArrowRight),
        ("data-last", "Letzte Seite", TablerIcon::ArrowBarToRight),
    ] {
        let c = control(&tree, data);
        assert_eq!(c.get_attribute("aria-label").as_deref(), Some(label));
        assert_eq!(glyph(&c), glyph_of(icon), "{data}");
    }
    let dots = tree.all("rinch-pagination__dots");
    assert_eq!(dots.len(), 2, "both gaps");
    for d in dots {
        assert_eq!(text(&d), "…");
    }

    // The defaults are the old glyphs and the old dots.
    let tree = pagination(Pagination {
        total: 20,
        value: 10,
        ..Default::default()
    });
    for d in tree.all("rinch-pagination__dots") {
        assert_eq!(text(&d), "...");
    }
    assert_ne!(
        glyph(&control(&tree, "data-next")),
        glyph_of(TablerIcon::ArrowRight)
    );
}

#[test]
fn pagination_render_item_draws_each_pages_content_and_keeps_the_wiring() {
    let seen: Rc<RefCell<Vec<(u32, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    let log = seen.clone();
    let picked = Rc::new(Cell::new(0));
    let sink = picked.clone();
    let tree = pagination(Pagination {
        total: 3,
        value: 2,
        onchange: Some(ValueCallback::new(move |n: u32| sink.set(n))),
        render_item: Some(Rc::new(
            move |scope: &mut RenderScope, n: u32, active: bool| {
                log.borrow_mut().push((n, active));
                let b = scope.create_element("b");
                b.set_attribute("class", "my-page");
                let t = scope.create_text(&format!("p{n}"));
                b.append_child(&t);
                b
            },
        )),
        ..Default::default()
    });
    assert_eq!(
        seen.borrow().as_slice(),
        &[(1, false), (2, true), (3, false)]
    );
    let items = tree.all("rinch-pagination__item");
    assert_eq!(items.len(), 3);
    assert_eq!(text(&items[2]), "p3");
    assert!(find_by_class(&items[2], "my-page").is_some());
    assert!(click(&items[2]), "the button keeps its handler");
    assert_eq!(picked.get(), 3);
}

#[test]
fn the_active_page_text_colour_is_a_custom_property() {
    let css = strip_comments(&sheet());
    let decls = declarations_for(&css, ".rinch-pagination__item--active");
    assert!(
        decls
            .iter()
            .any(|d| d == "color: var(--rinch-pagination-active-text, var(--rinch-pagination-contrast, var(--rinch-primary-color-contrast, white)))"),
        "{decls:?}"
    );
}

#[test]
fn a_controlled_navlink_follows_opened_fn_and_reports_clicks() {
    let open = Signal::new(false);
    let asked: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        opened_fn: Some(Rc::new(move || open.get())),
        on_opened_change: Some(ValueCallback::new(move |v: bool| sink.borrow_mut().push(v))),
        ..Default::default()
    });
    assert!(collapsed(&tree));
    click(&parent_control(&tree));
    assert_eq!(asked.borrow().as_slice(), &[true], "the click asks to open");
    assert!(
        collapsed(&tree),
        "but a controlled navlink waits for the app"
    );
    open.set(true);
    assert!(!collapsed(&tree), "the getter opens it");
    assert_eq!(
        parent_control(&tree)
            .get_attribute("aria-expanded")
            .as_deref(),
        Some("true")
    );
    click(&parent_control(&tree));
    assert_eq!(asked.borrow().as_slice(), &[true, false]);
}

#[test]
fn an_uncontrolled_navlink_reports_each_toggle() {
    let asked: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = asked.clone();
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        opened: true,
        on_opened_change: Some(ValueCallback::new(move |v: bool| sink.borrow_mut().push(v))),
        ..Default::default()
    });
    assert!(!collapsed(&tree), "`opened` opens it");
    click(&parent_control(&tree));
    click(&parent_control(&tree));
    assert_eq!(asked.borrow().as_slice(), &[false, true]);
    assert!(!collapsed(&tree));
}

#[test]
fn a_disabled_navlink_with_children_does_not_toggle() {
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        disabled: true,
        ..Default::default()
    });
    assert!(!click(&parent_control(&tree)));
    assert!(collapsed(&tree));
}

#[test]
fn a_navlinks_chevron_is_an_icon_option_and_its_rotation_can_be_turned_off() {
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        chevron_icon: Some(TablerIcon::ChevronDown),
        default_opened: true,
        ..Default::default()
    });
    let right = tree.find("rinch-navlink__right");
    assert_eq!(glyph(&right), glyph_of(TablerIcon::ChevronDown));
    let chevron = tree.find("rinch-navlink__chevron");
    assert!(has_class(&chevron, "rinch-navlink__chevron--opened"));

    // A right section on a navlink with children turns with it by default...
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        right_section: Some(TablerIcon::Plus),
        ..Default::default()
    });
    click(&parent_control(&tree));
    assert!(
        find_by_class(&tree.root, "rinch-navlink__chevron--opened").is_some(),
        "the right section stands in for the chevron and turns"
    );

    // ...and not with `disable_right_section_rotation`.
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        disable_right_section_rotation: true,
        ..Default::default()
    });
    click(&parent_control(&tree));
    assert!(!collapsed(&tree), "it still opens");
    assert!(find_by_class(&tree.root, "rinch-navlink__chevron--opened").is_none());
}

#[test]
fn a_navlinks_label_slot_takes_a_renderer() {
    let tree = Tree::build(|scope| {
        NavLink {
            label: "ignored".into(),
            description: "desc".into(),
            label_fn: Some(marker("my-badge")),
            ..Default::default()
        }
        .render(scope, &[])
    });
    let label = tree.find("rinch-navlink__label");
    assert!(find_by_class(&label, "my-badge").is_some());
    assert_eq!(text(&label), "", "the renderer replaces the text");
    assert!(
        find_by_class(&tree.find("rinch-navlink__body"), "my-badge").is_some(),
        "beside the description, in the body"
    );
}

#[test]
fn collapsed_navlink_children_are_not_displayed_so_tab_skips_them() {
    let tree = navlink_with_children(NavLink {
        label: "Section".into(),
        ..Default::default()
    });
    let container = tree.find("rinch-navlink__children");
    // The inline `display`, read from the `style` attribute the mock writes.
    let display = |c: &NodeHandle| {
        c.get_attribute("style")
            .unwrap_or_default()
            .split(';')
            .filter_map(|d| d.split_once(':'))
            .find(|(k, _)| k.trim() == "display")
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    assert_eq!(
        display(&container),
        "none",
        "closed children must be `display: none`, as a closed <details> and \
         Accordion's panel are; clipped to height 0 they stayed in the Tab order"
    );
    click(&parent_control(&tree));
    assert_eq!(
        display(&container),
        "",
        "open, the inline display is cleared"
    );
    click(&parent_control(&tree));
    assert_eq!(display(&container), "none");
}

#[test]
fn a_disabled_navlink_anchor_is_out_of_the_tab_order() {
    let tree = Tree::build(|scope| {
        NavLink {
            label: "Off".into(),
            href: "/off".into(),
            disabled: true,
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(
        parent_control(&tree).get_attribute("tabindex").as_deref(),
        Some("-1")
    );
    let tree = Tree::build(|scope| {
        NavLink {
            label: "On".into(),
            href: "/on".into(),
            ..Default::default()
        }
        .render(scope, &[])
    });
    assert_eq!(parent_control(&tree).get_attribute("tabindex"), None);
}
