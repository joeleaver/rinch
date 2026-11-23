//! #2207: configurability gaps in the display components — Text, Title,
//! Highlight, Blockquote, Divider, List, Skeleton and SimpleGrid — and the
//! bugs found beside them.
//!
//! Each new prop is checked on a rendered tree (the mock document), and each
//! default is checked to render what the component rendered before. What only
//! layout can show (SimpleGrid's template yielding to an app stylesheet, the
//! loaded Skeleton's wrapper generating no box, Text's line clamp) is pinned in
//! `crates/rinch/src/app/display_props_2207_tests.rs`.

use rinch_components as rinch;
use std::cell::RefCell;
use std::rc::Rc;

use rinch_components::{
    Blockquote, Divider, Highlight, List, ListItem, SectionRenderer, SimpleGrid, Skeleton, Text,
    Title,
};
use rinch_core::Component;
use rinch_core::Signal;
use rinch_core::dom::traits::DomDocument;
use rinch_core::dom::{NodeHandle, RenderScope, mock::MockDomDocument};
use rinch_macros::rsx;
use rinch_tabler_icons::TablerIcon;

mod common;
use common::{collect_by_class, find_by_class, has_class, sheet};

/// A rendered component, with its document and scope kept alive around it.
struct Mounted {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    root: NodeHandle,
}

fn mount_with<C: Component>(component: C, children: &[&str]) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let children: Vec<NodeHandle> = children.iter().map(|t| scope.create_text(t)).collect();
    let root = component.render(&mut scope, &children);
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
    }
}

/// Build a tree with `rsx!`, as an app writes it.
fn build(f: impl FnOnce(&mut RenderScope) -> NodeHandle) -> Mounted {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = f(&mut scope);
    Mounted {
        _doc: doc,
        _scope: scope,
        root,
    }
}

/// A renderer drawing `<i class="{class}">{text}</i>`.
fn node(class: &'static str, text: &'static str) -> SectionRenderer {
    Rc::new(move |scope: &mut RenderScope| {
        let i = scope.create_element("i");
        i.set_attribute("class", class);
        let t = scope.create_text(text);
        i.append_child(&t);
        i
    })
}

/// The inline style's declarations, as `(name, value)`.
fn style_of(node: &NodeHandle) -> Vec<(String, String)> {
    node.get_attribute("style")
        .unwrap_or_default()
        .split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            Some((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

fn style_value(node: &NodeHandle, name: &str) -> Option<String> {
    style_of(node)
        .into_iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v)
}

/// The declarations of the first rule in the component sheet whose selector
/// is exactly `selector`.
fn rule(selector: &str) -> String {
    let css = common::strip_comments(&sheet());
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let sel = rest[..open].trim();
        let close = rest[open..].find('}').expect("closed rule") + open;
        if sel == selector {
            return rest[open + 1..close].to_string();
        }
        rest = &rest[close + 1..];
    }
    panic!("no rule `{selector}`");
}

fn mount<C: Component>(component: C) -> Mounted {
    mount_with(component, &[])
}

/// The text of every `<mark>` a Highlight rendered, in order.
fn marks(root: &NodeHandle) -> Vec<String> {
    let mut out = Vec::new();
    collect_by_class(root, "rinch-highlight__match", &mut out);
    out.iter()
        .map(|m| m.text_content().unwrap_or_default())
        .collect()
}

/// All of a node's text, its children's included.
fn all_text(node: &NodeHandle) -> String {
    let mut s = node.text_content().unwrap_or_default();
    if node.tag_name().is_some() {
        s.clear();
        for c in node.children() {
            s.push_str(&all_text(&c));
        }
    }
    s
}

// ------------------------------------------------------- Highlight (bug)

/// `İ` (U+0130, 2 bytes) lowercases to `i̇` (3 bytes). Matching on the
/// lowercased copy and slicing the original with its offsets highlighted the
/// wrong letters, or panicked on a slice past the end.
#[test]
fn a_case_insensitive_match_after_a_length_changing_letter_marks_the_right_text() {
    let m = mount(Highlight {
        text: "İab".into(),
        highlight: "a".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["a".to_string()]);
    assert_eq!(all_text(&m.root), "İab");
}

#[test]
fn a_case_insensitive_match_at_the_end_after_a_length_changing_letter_does_not_panic() {
    let m = mount(Highlight {
        text: "İx".into(),
        highlight: "x".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["x".to_string()]);
    assert_eq!(all_text(&m.root), "İx");
}

#[test]
fn a_case_insensitive_term_matches_the_original_case() {
    let m = mount(Highlight {
        text: "Find FIND find".into(),
        highlight: "find".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["Find", "FIND", "find"]);
    assert_eq!(all_text(&m.root), "Find FIND find");

    let m = mount(Highlight {
        text: "Find FIND find".into(),
        highlight: "find".into(),
        ignore_case: false,
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["find"]);
}

// ------------------------------------------------------ Blockquote (bug)

/// The body was a `<p>`, so a block child was invalid HTML (and the web
/// parser would close the paragraph before it).
#[test]
fn a_blockquotes_body_is_a_div_so_block_children_are_valid() {
    let m = mount_with(Blockquote::default(), &["quote"]);
    let body = find_by_class(&m.root, "rinch-blockquote__body").expect("body");
    assert_eq!(body.tag_name().as_deref(), Some("div"));
    assert_eq!(all_text(&body), "quote");
}

// -------------------------------------------------------- Skeleton (bug)

/// The loaded wrapper was a bare inline `<span>`, so block children sat in
/// an inline box and a flex or grid parent saw one item. It is now a span that
/// generates no box (`display: contents`).
#[test]
fn a_loaded_skeletons_wrapper_generates_no_box() {
    let m = mount_with(
        Skeleton {
            visible: false,
            ..Default::default()
        },
        &["loaded"],
    );
    assert!(has_class(&m.root, "rinch-skeleton__content"));
    assert!(!has_class(&m.root, "rinch-skeleton"));
    assert_eq!(all_text(&m.root), "loaded");
    let css = sheet();
    let rule = css
        .split(".rinch-skeleton__content")
        .nth(1)
        .expect("a rule for the wrapper");
    let body = &rule[..rule.find('}').unwrap()];
    assert!(body.contains("display: contents"), "{body}");
}

// ------------------------------------------------------------------ Text

#[test]
fn a_named_text_colour_is_still_a_class_and_writes_no_variable() {
    let m = mount(Text {
        color: "blue".into(),
        ..Default::default()
    });
    assert!(has_class(&m.root, "rinch-text--blue"));
    assert!(!has_class(&m.root, "rinch-text--custom-color"));
    assert_eq!(m.root.get_attribute("style"), None);
}

#[test]
fn any_other_text_colour_goes_through_the_variable() {
    for (color, want) in [
        ("#ff8800", "#ff8800"),
        ("rgb(1, 2, 3)", "rgb(1, 2, 3)"),
        ("var(--my-ink)", "var(--my-ink)"),
        ("blue.3", "var(--rinch-color-blue-3)"),
        ("brand", "var(--rinch-color-brand-6, brand)"),
        ("crimson", "var(--rinch-color-crimson-6, crimson)"),
    ] {
        let m = mount(Text {
            color: color.into(),
            ..Default::default()
        });
        assert!(has_class(&m.root, "rinch-text--custom-color"), "{color}");
        assert_eq!(
            style_value(&m.root, "--rinch-text-color").as_deref(),
            Some(want),
            "{color}"
        );
    }
    assert!(rule(".rinch-text--custom-color").contains("color: var(--rinch-text-color)"));
}

#[test]
fn text_truncate_and_line_clamp_are_props() {
    let m = mount(Text::default());
    assert!(!has_class(&m.root, "rinch-text--truncate"));
    assert!(!has_class(&m.root, "rinch-text--line-clamp"));
    assert_eq!(m.root.get_attribute("style"), None);

    let m = build(|__scope| rsx! { Text { truncate: true, line_clamp: 3, "long" } });
    assert!(has_class(&m.root, "rinch-text--truncate"));
    assert!(has_class(&m.root, "rinch-text--line-clamp"));
    assert_eq!(
        style_value(&m.root, "--rinch-text-line-clamp").as_deref(),
        Some("3")
    );

    let truncate = rule(".rinch-text--truncate");
    for decl in [
        "display: block",
        "white-space: nowrap",
        "text-overflow: ellipsis",
    ] {
        assert!(truncate.contains(decl), "{decl} in {truncate}");
    }
    assert!(
        rule(".rinch-text--line-clamp")
            .contains("-webkit-line-clamp: var(--rinch-text-line-clamp)")
    );
}

// ----------------------------------------------------------------- Title

#[test]
fn title_size_takes_levels_theme_sizes_and_css_sizes() {
    let cls = |size: &str| {
        let m = mount(Title {
            order: Some(2),
            size: size.into(),
            ..Default::default()
        });
        let level = (1..=6)
            .find(|n| has_class(&m.root, &format!("rinch-title--{n}")))
            .expect("a level class");
        (
            m.root.tag_name().unwrap(),
            level,
            has_class(&m.root, "rinch-title--custom-size"),
            style_value(&m.root, "--rinch-title-size"),
        )
    };
    // Unchanged: empty and "1"-"6".
    assert_eq!(cls(""), ("h2".into(), 2, false, None));
    assert_eq!(cls("4"), ("h2".into(), 4, false, None));
    // New: "h1"-"h6", theme sizes, CSS sizes.
    assert_eq!(cls("h5"), ("h2".into(), 5, false, None));
    assert_eq!(
        cls("lg"),
        (
            "h2".into(),
            2,
            true,
            Some("var(--rinch-font-size-lg)".into())
        )
    );
    assert_eq!(cls("40px"), ("h2".into(), 2, true, Some("40px".into())));
    // A bare number outside 1-6 keeps the order's size.
    assert_eq!(cls("9"), ("h2".into(), 2, false, None));
    assert!(rule(".rinch-title--custom-size").contains("font-size: var(--rinch-title-size)"));
}

// ------------------------------------------------------------- Highlight

#[test]
fn highlight_takes_several_terms_and_the_longest_wins() {
    let m = build(|__scope| {
        rsx! {
            Highlight {
                text: "New York and new things",
                highlight: "new",
                highlights: vec!["new york".to_string(), "thing".to_string()],
            }
        }
    });
    assert_eq!(marks(&m.root), vec!["New York", "new", "thing"]);
    assert_eq!(all_text(&m.root), "New York and new things");
}

#[test]
fn highlight_match_class_is_added_to_each_mark() {
    let m = mount(Highlight {
        text: "a b a".into(),
        highlight: "a".into(),
        match_class: "my-hit".into(),
        ..Default::default()
    });
    let mut hits = Vec::new();
    collect_by_class(&m.root, "my-hit", &mut hits);
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| has_class(h, "rinch-highlight__match")));

    // Default: the base class alone.
    let m = mount(Highlight {
        text: "a".into(),
        highlight: "a".into(),
        ..Default::default()
    });
    let mark = find_by_class(&m.root, "rinch-highlight__match").unwrap();
    assert_eq!(
        mark.get_attribute("class").as_deref(),
        Some("rinch-highlight__match")
    );
}

#[test]
fn a_term_ending_inside_a_letters_lowercase_expansion_does_not_match_it() {
    // "İ" folds to "i" + U+0307; the term "i" stops halfway through it.
    let m = mount(Highlight {
        text: "İ i".into(),
        highlight: "i".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["i"]);
    // The whole fold matches the whole letter.
    let m = mount(Highlight {
        text: "İ i".into(),
        highlight: "i\u{307}".into(),
        ..Default::default()
    });
    assert_eq!(marks(&m.root), vec!["İ"]);
}

// ------------------------------------------------- Blockquote and Divider

#[test]
fn a_cite_node_replaces_the_cite_text() {
    let m = mount_with(Blockquote::default(), &["q"]);
    assert!(find_by_class(&m.root, "rinch-blockquote__cite").is_none());

    let m = build(|__scope| {
        rsx! {
            Blockquote { cite: "ignored", cite_node: Some(node("who", "Ada")), "q" }
        }
    });
    let cite = find_by_class(&m.root, "rinch-blockquote__cite").expect("cite");
    assert_eq!(cite.tag_name().as_deref(), Some("cite"));
    assert!(find_by_class(&cite, "who").is_some());
    assert_eq!(all_text(&cite), "Ada");
}

#[test]
fn a_divider_label_node_draws_in_the_label_span() {
    let m = mount(Divider::default());
    assert_eq!(m.root.tag_name().as_deref(), Some("hr"));

    let m = build(|__scope| {
        rsx! { Divider { label_node: Some(node("glyph", "*")), label_position: "left" } }
    });
    assert_eq!(m.root.tag_name().as_deref(), Some("div"));
    assert!(has_class(&m.root, "rinch-divider--with-label"));
    assert!(has_class(&m.root, "rinch-divider--label-left"));
    let label = find_by_class(&m.root, "rinch-divider__label").expect("label");
    assert!(find_by_class(&label, "glyph").is_some());
}

// ------------------------------------------------------------------ List

fn markers(root: &NodeHandle, class: &str) -> usize {
    let mut icons = Vec::new();
    collect_by_class(root, "rinch-list__item-icon", &mut icons);
    icons
        .iter()
        .filter(|i| find_by_class(i, class).is_some())
        .count()
}

#[test]
fn a_list_icon_node_marks_every_item_and_an_items_own_wins() {
    let items = Signal::new(vec!["one"]);
    let m = build(move |__scope| {
        rsx! {
            div {
                List { icon: TablerIcon::Check, icon_node: Some(node("dot", "")),
                    ListItem { "plain" }
                    ListItem { icon_node: Some(node("star", "")), "own node" }
                    ListItem { icon: TablerIcon::X, "own icon" }
                    for it in items.get() { ListItem { key: it, {it} } }
                }
            }
        }
    });
    // "plain" and "one" take the list's node; the two with their own keep them.
    assert_eq!(markers(&m.root, "dot"), 2);
    assert_eq!(markers(&m.root, "star"), 1);
    let mut all = Vec::new();
    collect_by_class(&m.root, "rinch-list__item-icon", &mut all);
    assert_eq!(all.len(), 4);

    // A late row takes the list's node too.
    items.update(|v| v.push("two"));
    assert_eq!(markers(&m.root, "dot"), 3);
}

// -------------------------------------------------------------- Skeleton

#[test]
fn skeleton_duration_is_a_variable_the_animation_reads() {
    let m = mount(Skeleton::default());
    assert_eq!(style_value(&m.root, "--rinch-skeleton-duration"), None);

    let m = mount(Skeleton {
        duration: "800ms".into(),
        ..Default::default()
    });
    assert_eq!(
        style_value(&m.root, "--rinch-skeleton-duration").as_deref(),
        Some("800ms")
    );
    assert!(
        rule(".rinch-skeleton--animate")
            .contains("rinch-skeleton-pulse var(--rinch-skeleton-duration, 1.5s)")
    );
}

// ------------------------------------------------------------ SimpleGrid

#[test]
fn simple_grid_writes_only_variables_inline() {
    // Defaults: nothing inline at all.
    let m = mount(SimpleGrid::default());
    assert_eq!(m.root.get_attribute("style"), None);

    let m = mount(SimpleGrid {
        cols: Some(3),
        spacing: "lg".into(),
        vertical_spacing: "4px".into(),
        ..Default::default()
    });
    let mut names: Vec<String> = style_of(&m.root).into_iter().map(|(k, _)| k).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "--rinch-simple-grid-cols",
            "--rinch-simple-grid-gap",
            "--rinch-simple-grid-row-gap"
        ]
    );
    assert_eq!(
        style_value(&m.root, "--rinch-simple-grid-gap").as_deref(),
        Some("var(--rinch-spacing-lg)")
    );

    let m = mount(SimpleGrid {
        cols: Some(3),
        min_child_width: "200px".into(),
        ..Default::default()
    });
    assert_eq!(
        style_of(&m.root),
        [(
            "--rinch-simple-grid-template".to_string(),
            "repeat(auto-fill, minmax(200px, 1fr))".to_string()
        )]
    );

    let grid = rule(".rinch-simple-grid");
    assert!(grid.contains("display: grid"), "{grid}");
    assert!(grid.contains("grid-template-columns"), "{grid}");
}
