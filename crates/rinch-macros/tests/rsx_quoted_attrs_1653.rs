//! A **quoted** attribute name — `"title": "Save"` — is always the HTML
//! attribute, on a component as on an element (issue #1653).
//!
//! #433 routes a hyphenated name on a component to the component's root,
//! because a hyphen cannot be a Rust field name and so cannot collide with a
//! prop. An unhyphenated global (`title`, `id`, `role`, `lang`, `dir`) has no
//! such guarantee: `Alert`, `Modal`, `Drawer`, `Notification`, `FloatingPanel`
//! and `BorderlessWindow` all declare a `title` field meaning a heading, and the
//! macro has no type information to tell the two apart. A string literal is not
//! an identifier either, so a quoted name cannot be a field: it takes #433's
//! root path, with #433's rules (the caller writes last, a `{|| …}` value is an
//! effect on the root and re-renders nothing).
//!
//! The mock is a pass-through (`set_attribute` inserts, `remove_attribute`
//! erases), so these observe the macro's output with no backend policy in
//! between.

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
use rinch_tabler_icons::TablerIcon;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<MockDomDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

thread_local! {
    /// How many times `Tagged` has rendered on this thread.
    static RENDERS: Cell<u32> = const { Cell::new(0) };
}

fn renders() -> u32 {
    RENDERS.with(Cell::get)
}

/// A `#[component]` PascalCase component that writes a `role` and `dir` of its
/// own on its root (so a caller's write has something to collide with), and
/// counts its renders.
#[component]
fn Tagged(label: String) -> NodeHandle {
    RENDERS.with(|r| r.set(r.get() + 1));
    let root = __scope.create_element("section");
    root.set_attribute("class", "tagged");
    root.set_attribute("role", "region");
    root.set_attribute("dir", "ltr");
    root.append_child(&__scope.create_text(&label));
    root
}

/// The `.tagged` element at or below `node`.
fn find_tagged(node: &NodeHandle) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some("tagged") {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_tagged)
}

/// The concatenated text below `node`.
fn text_of(node: &NodeHandle) -> String {
    node.text_content().unwrap_or_default()
}

// ── 1. a #[component] PascalCase component, static path ─────────────────────

#[component]
fn quoted_globals() -> NodeHandle {
    rsx! {
        Tagged {
            label: "Toolbar",
            "title": "Save file",
            "id": "main-bar",
            "role": "toolbar",
            "lang": "fr",
            "data-quoted": "q",
        }
    }
}

/// Every quoted name lands on the component's own root, the caller's write
/// winning a collision with the component's `role`, and an attribute the
/// caller does not name (`dir`) staying the component's.
#[test]
fn quoted_names_land_on_a_component_root() {
    let (_doc, _scope, root) = mount(quoted_globals);
    assert_eq!(
        root.get_attribute("class").as_deref(),
        Some("tagged"),
        "the static path returns the component's root itself"
    );
    assert_eq!(root.get_attribute("title").as_deref(), Some("Save file"));
    assert_eq!(root.get_attribute("id").as_deref(), Some("main-bar"));
    assert_eq!(
        root.get_attribute("role").as_deref(),
        Some("toolbar"),
        "the caller writes after the component's own `role`"
    );
    assert_eq!(root.get_attribute("lang").as_deref(), Some("fr"));
    assert_eq!(
        root.get_attribute("data-quoted").as_deref(),
        Some("q"),
        "a quoted hyphenated name is the same attribute as the unquoted one"
    );
    assert_eq!(root.get_attribute("dir").as_deref(), Some("ltr"));
    assert_eq!(
        text_of(&root),
        "Toolbar",
        "the struct prop still reached the struct"
    );
}

// ── 2. a hand-written `impl Component` whose `title` is a heading ───────────

/// A hand-written component with a `title` **field** that means a heading, as
/// `Alert` and `Modal` have.
#[derive(Debug, Default)]
struct Card {
    title: String,
}

impl Component for Card {
    fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let root = scope.create_element("article");
        root.set_attribute("class", "card");
        let h = scope.create_element("h3");
        h.append_child(&scope.create_text(&self.title));
        root.append_child(&h);
        for c in children {
            root.append_child(c);
        }
        root
    }
}

#[component]
fn card_both_titles() -> NodeHandle {
    rsx! {
        Card { title: "Heading", "title": "Tooltip", "dir": "rtl", "body" }
    }
}

/// The bare name is the field, the quoted name is the attribute: the two
/// meanings are told apart by spelling, not by guessing.
#[test]
fn bare_title_is_the_field_and_quoted_title_is_the_attribute() {
    let (_doc, _scope, root) = mount(card_both_titles);
    assert_eq!(root.get_attribute("class").as_deref(), Some("card"));
    assert_eq!(root.get_attribute("title").as_deref(), Some("Tooltip"));
    assert_eq!(root.get_attribute("dir").as_deref(), Some("rtl"));
    let heading = &root.children()[0];
    assert_eq!(heading.tag_name().as_deref(), Some("h3"));
    assert_eq!(text_of(heading), "Heading");
    assert_eq!(text_of(&root), "Headingbody");
}

// ── 3. reactive, surgical ───────────────────────────────────────────────────

#[component]
fn reactive_quoted(tip: Signal<String>) -> NodeHandle {
    rsx! {
        div {
            Tagged { label: "x", "title": {move || tip.get()} }
        }
    }
}

/// A `{|| …}` value is an effect on the root: it follows the signal, and the
/// component renders once.
#[test]
fn a_reactive_quoted_attribute_updates_without_a_re_render() {
    let tip = Signal::new("Save".to_string());
    let before = renders();
    let (_doc, _scope, root) = mount(|s| reactive_quoted(s, tip));
    let first = find_tagged(&root).expect("rendered");
    assert_eq!(first.get_attribute("title").as_deref(), Some("Save"));
    assert_eq!(renders() - before, 1);
    assert_eq!(
        root.children().len(),
        1,
        "a reactive quoted attribute alone is the static path: the component's \
         root is the only child, with no re-render marker beside it"
    );

    tip.set("Saving…".to_string());
    let second = find_tagged(&root).expect("still rendered");
    assert_eq!(second.get_attribute("title").as_deref(), Some("Saving…"));
    assert_eq!(first.node_id(), second.node_id(), "the same root");
    assert_eq!(
        renders() - before,
        1,
        "a quoted attribute change must not re-render the component"
    );
}

#[component]
fn rerendering_quoted(label: Signal<String>, tip: Signal<String>) -> NodeHandle {
    rsx! {
        div {
            Tagged { label: {move || label.get()}, "title": {move || tip.get()}, "id": "t" }
        }
    }
}

/// On a component that re-renders for a struct prop, the quoted attributes are
/// written on each new root, and a change to the attribute alone still
/// re-renders nothing (#1190's root bindings).
#[test]
fn a_re_rendered_component_gets_its_quoted_attributes_back() {
    let label = Signal::new("one".to_string());
    let tip = Signal::new("a".to_string());
    let before = renders();
    let (_doc, _scope, root) = mount(|s| rerendering_quoted(s, label, tip));
    let first = find_tagged(&root).expect("rendered");
    assert_eq!(first.get_attribute("title").as_deref(), Some("a"));
    assert_eq!(first.get_attribute("id").as_deref(), Some("t"));

    label.set("two".to_string());
    let second = find_tagged(&root).expect("re-rendered");
    assert_ne!(first.node_id(), second.node_id(), "positive control");
    assert_eq!(second.get_attribute("title").as_deref(), Some("a"));
    assert_eq!(second.get_attribute("id").as_deref(), Some("t"));
    let after_label = renders();
    assert_eq!(after_label - before, 2);

    tip.set("b".to_string());
    let third = find_tagged(&root).expect("still rendered");
    assert_eq!(third.get_attribute("title").as_deref(), Some("b"));
    assert_eq!(third.node_id(), second.node_id());
    assert_eq!(
        renders(),
        after_label,
        "the attribute alone re-renders nothing"
    );
}

// ── 4. on an HTML element ───────────────────────────────────────────────────

#[component]
fn quoted_on_element() -> NodeHandle {
    rsx! {
        div { "title": "Tip", "p": "literal", "data-2x": "yes", "aria-label": "Box", "value_fn": "x" }
    }
}

/// On an element a quoted name is the same attribute as the bare one, and it
/// is never a style shorthand: `"p"` is an attribute named `p`, where `p:` is
/// padding. It also reaches names a bare spelling cannot tokenize
/// (`data-2x`).
#[test]
fn a_quoted_name_on_an_element_is_the_literal_attribute() {
    let (_doc, _scope, root) = mount(quoted_on_element);
    assert_eq!(root.get_attribute("title").as_deref(), Some("Tip"));
    assert_eq!(root.get_attribute("p").as_deref(), Some("literal"));
    assert_eq!(
        root.get_attribute("style"),
        None,
        "not the padding shorthand"
    );
    assert_eq!(root.get_attribute("data-2x").as_deref(), Some("yes"));
    assert_eq!(root.get_attribute("aria-label").as_deref(), Some("Box"));
    assert_eq!(
        root.get_attribute("value_fn").as_deref(),
        Some("x"),
        "a quoted `_fn` name is an attribute, not a `_fn` binding"
    );
}

#[component]
fn quoted_shorthand_on_component() -> NodeHandle {
    rsx! { Tagged { label: "x", "p": "md" } }
}

/// From the #2003 review: on a component too, a quoted `"p"` is the attribute
/// `p` on the root, never the padding shorthand.
#[test]
fn a_quoted_shorthand_name_on_a_component_is_the_attribute() {
    let (_doc, _scope, root) = mount(quoted_shorthand_on_component);
    assert_eq!(root.get_attribute("p").as_deref(), Some("md"));
    assert!(
        !root
            .get_attribute("style")
            .unwrap_or_default()
            .contains("padding"),
        "not the padding shorthand"
    );
}

#[component]
fn quoted_boolean() -> NodeHandle {
    rsx! {
        Tagged { label: "x", "hidden": false, "inert": true }
    }
}

/// A quoted boolean attribute goes through `write_attribute` like any other
/// (#551): truthy is presence, falsey is an absence.
#[test]
fn a_quoted_boolean_attribute_is_a_presence() {
    let (_doc, _scope, root) = mount(quoted_boolean);
    assert_eq!(root.get_attribute("hidden"), None);
    assert_eq!(root.get_attribute("inert").as_deref(), Some(""));
}

// ── 5. library components ───────────────────────────────────────────────────

#[component]
fn alert_heading_and_tooltip() -> NodeHandle {
    rsx! {
        Alert { title: "Heads up", "title": "More detail", "Body" }
    }
}

/// `Alert`'s `title` heading prop is untouched: the bare name still renders the
/// heading, and the quoted name is the root's tooltip.
#[test]
fn alerts_title_heading_prop_is_untouched() {
    let (_doc, _scope, root) = mount(alert_heading_and_tooltip);
    assert!(
        root.get_attribute("class")
            .unwrap_or_default()
            .contains("rinch-alert--with-title"),
        "the heading prop reached the struct"
    );
    assert!(text_of(&root).contains("Heads up"));
    assert_eq!(root.get_attribute("title").as_deref(), Some("More detail"));
}

#[component]
fn alert_heading_only() -> NodeHandle {
    rsx! {
        Alert { title: "Heads up", "Body" }
    }
}

/// And a bare `title` on `Alert` writes no attribute.
#[test]
fn alerts_bare_title_writes_no_attribute() {
    let (_doc, _scope, root) = mount(alert_heading_only);
    assert!(text_of(&root).contains("Heads up"));
    assert_eq!(root.get_attribute("title"), None);
}

#[component]
fn icon_buttons() -> NodeHandle {
    rsx! {
        div {
            ActionIcon { icon: TablerIcon::DeviceFloppy, title: "Save" }
            CloseButton { title: "Dismiss" }
            ActionIcon { icon: TablerIcon::X }
            CloseButton {}
            Button { "title": "Delete the row", "Delete" }
        }
    }
}

/// `ActionIcon` and `CloseButton` take a bare `title` prop, written on their
/// root `<button>`; an empty one (the default) writes nothing. Any other
/// component takes the quoted spelling.
#[test]
fn action_icon_and_close_button_take_a_title_prop() {
    let (_doc, _scope, root) = mount(icon_buttons);
    let kids = root.children();
    assert_eq!(kids.len(), 5);
    for k in &kids {
        assert_eq!(k.tag_name().as_deref(), Some("button"), "each is its root");
    }
    assert_eq!(kids[0].get_attribute("title").as_deref(), Some("Save"));
    assert_eq!(kids[1].get_attribute("title").as_deref(), Some("Dismiss"));
    assert_eq!(kids[2].get_attribute("title"), None);
    assert_eq!(kids[3].get_attribute("title"), None);
    assert_eq!(
        kids[4].get_attribute("title").as_deref(),
        Some("Delete the row")
    );
}
