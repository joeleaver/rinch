//! On a component that re-renders for a reactive **struct** prop, the caller's
//! reactive `style:`, `class:`, style shorthands and hyphenated attributes are
//! effects on the rendered root, not reads of the re-render (issue #1190).
//!
//! `reactive_component_dom` runs the macro's render closure tracked: the struct
//! prop closures are called inside it and their signal reads schedule the next
//! render. The caller's root bindings used to be invoked in that same tracked
//! region, so a change to the signal a `style: {|| …}` reads rebuilt the whole
//! component — a new root, and every component-local signal reset — which is
//! the cost #390 removed for the body. Each binding is now an effect owned by
//! the per-render child scope: disposed with the render it belongs to, and
//! created afresh (with a fresh reactive `style:`/`class:` memory) on the new
//! root.
//!
//! Both codegen sites have fixtures: `element_to_dom_component_reactive` (the
//! component as the rsx root) and `generate_reactive_component_stmt` (the
//! component as a child of an element).

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_core::dom::mock::MockDomDocument;
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
    /// How many times `Probe` has rendered.
    static RENDERS: Cell<u32> = const { Cell::new(0) };
    /// The component-local signal of the most recent `Probe` render.
    static LOCAL: RefCell<Option<Signal<i32>>> = const { RefCell::new(None) };
}

fn renders() -> u32 {
    RENDERS.with(Cell::get)
}

fn local() -> Signal<i32> {
    LOCAL.with(|l| l.borrow().expect("Probe rendered"))
}

/// A component with a struct prop, component-local state, and inline
/// declarations of its own on its root (the #647 channel a caller's `style:`
/// must merge over, not replace).
#[component]
fn Probe(label: String) -> NodeHandle {
    RENDERS.with(|r| r.set(r.get() + 1));
    let count = Signal::new(0);
    LOCAL.with(|l| *l.borrow_mut() = Some(count));
    let root = __scope.create_element("section");
    root.set_attribute("class", "probe");
    root.set_attribute("style", "--probe: 1");
    root.set_attribute("aria-label", &label);
    let text = __scope.create_text("0");
    root.append_child(&text);
    __scope.create_effect(move || text.set_text(&count.get().to_string()));
    root
}

fn find_probe(node: &NodeHandle) -> Option<NodeHandle> {
    let class = node.get_attribute("class").unwrap_or_default();
    if class.split_whitespace().any(|c| c == "probe") {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_probe)
}

fn decl(node: &NodeHandle, property: &str) -> Option<String> {
    rinch_core::split_declarations(&node.get_attribute("style").unwrap_or_default())
        .into_iter()
        .find(|(k, _)| k == property)
        .map(|(_, v)| v)
}

fn classes(node: &NodeHandle) -> Vec<String> {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn text(node: &NodeHandle) -> String {
    node.children()
        .iter()
        .map(|c| c.text_content().unwrap_or_default())
        .collect()
}

/// The signals one fixture drives, and a counter of every caller-binding
/// evaluation (so a leaked effect from an earlier render shows up as an extra
/// evaluation).
#[derive(Clone, Copy)]
struct Knobs {
    label: Signal<String>,
    css: Signal<String>,
    cls: Signal<String>,
    margin: Signal<String>,
    state: Signal<String>,
}

impl Knobs {
    fn new() -> Self {
        Self {
            label: Signal::new("one".into()),
            css: Signal::new("color: red; padding: 3px".into()),
            cls: Signal::new("alpha".into()),
            margin: Signal::new("7px".into()),
            state: Signal::new("a".into()),
        }
    }
}

thread_local! {
    static EVALS: Cell<u32> = const { Cell::new(0) };
}

fn evals() -> u32 {
    EVALS.with(Cell::get)
}

fn bump() {
    EVALS.with(|e| e.set(e.get() + 1));
}

#[component]
fn as_root(k: Knobs) -> NodeHandle {
    rsx! {
        Probe {
            label: {move || k.label.get()},
            style: {move || { bump(); k.css.get() }},
            class: {move || { bump(); k.cls.get() }},
            mt: {move || { bump(); k.margin.get() }},
            data-state: {move || { bump(); k.state.get() }},
        }
    }
}

#[component]
fn as_child(k: Knobs) -> NodeHandle {
    rsx! {
        div {
            Probe {
                label: {move || k.label.get()},
                style: {move || { bump(); k.css.get() }},
                class: {move || { bump(); k.cls.get() }},
                mt: {move || { bump(); k.margin.get() }},
                data-state: {move || { bump(); k.state.get() }},
            }
        }
    }
}

/// A named change to one binding's signal.
type Step = (&'static str, Box<dyn Fn()>);

/// Every caller binding, changed on its own, updates the root in place: same
/// node, no render, the component-local signal keeps its value, and exactly one
/// evaluation of that binding.
fn a_binding_change_does_not_re_render(build: fn(&mut RenderScope, Knobs) -> NodeHandle) {
    let k = Knobs::new();
    let (_doc, _scope, root) = mount(|s| build(s, k));
    let first = find_probe(&root).expect("rendered");
    let renders0 = renders();
    local().set(5);
    assert_eq!(
        text(&first),
        "5",
        "positive control: the local signal drives the text"
    );

    let steps: [Step; 4] = [
        ("style", Box::new(move || k.css.set("color: blue".into()))),
        ("class", Box::new(move || k.cls.set("beta".into()))),
        ("shorthand", Box::new(move || k.margin.set("9px".into()))),
        ("attribute", Box::new(move || k.state.set("b".into()))),
    ];
    for (what, step) in steps {
        let before = evals();
        step();
        let now = find_probe(&root).expect("still rendered");
        assert_eq!(
            now.node_id(),
            first.node_id(),
            "a {what}-only change must not rebuild the component"
        );
        assert_eq!(
            renders(),
            renders0,
            "a {what}-only change must not re-render"
        );
        assert_eq!(
            evals(),
            before + 1,
            "a {what}-only change evaluates that binding once"
        );
        assert_eq!(
            text(&now),
            "5",
            "a {what}-only change must not reset local state"
        );
    }

    assert_eq!(decl(&first, "color").as_deref(), Some("blue"));
    assert_eq!(
        decl(&first, "padding"),
        None,
        "the style memory took padding back"
    );
    assert_eq!(decl(&first, "--probe").as_deref(), Some("1"));
    assert_eq!(decl(&first, "margin-top").as_deref(), Some("9px"));
    assert_eq!(
        classes(&first),
        ["probe", "beta"],
        "the class memory took alpha back"
    );
    assert_eq!(first.get_attribute("data-state").as_deref(), Some("b"));
}

#[test]
fn a_binding_change_on_the_root_path_does_not_re_render() {
    a_binding_change_does_not_re_render(as_root);
}

#[test]
fn a_binding_change_on_the_child_path_does_not_re_render() {
    a_binding_change_does_not_re_render(as_child);
}

/// A struct-prop change still re-renders, and every binding is applied to the
/// new root — once, with a fresh memory — and keeps working there, with no
/// effect left behind on the discarded root.
fn a_struct_prop_change_re_renders_and_re_applies(
    build: fn(&mut RenderScope, Knobs) -> NodeHandle,
) {
    let k = Knobs::new();
    let (_doc, _scope, root) = mount(|s| build(s, k));
    let first = find_probe(&root).expect("rendered");
    let renders0 = renders();

    // Move every binding off its initial value first, so the memories of the
    // first render hold something the second render must not inherit.
    k.css.set("color: green; border: 1px".into());
    k.cls.set("gamma".into());

    k.label.set("two".into());
    let second = find_probe(&root).expect("re-rendered");
    assert_ne!(
        second.node_id(),
        first.node_id(),
        "positive control: the struct prop really re-rendered into a new root"
    );
    assert_eq!(renders(), renders0 + 1, "exactly one re-render");
    assert_eq!(second.get_attribute("aria-label").as_deref(), Some("two"));
    assert_eq!(decl(&second, "--probe").as_deref(), Some("1"));
    assert_eq!(decl(&second, "color").as_deref(), Some("green"));
    assert_eq!(decl(&second, "border").as_deref(), Some("1px"));
    assert_eq!(decl(&second, "margin-top").as_deref(), Some("7px"));
    assert_eq!(classes(&second), ["probe", "gamma"]);
    assert_eq!(second.get_attribute("data-state").as_deref(), Some("a"));

    // Each binding, changed after the re-render, runs once — the first
    // render's effects went with its scope — and the fresh memory takes back
    // what *this* root's effect wrote.
    for _ in 0..3 {
        k.label.set(format!("{}!", k.label.get()));
    }
    let current = find_probe(&root).expect("re-rendered");
    let before = evals();
    k.css.set("color: black".into());
    assert_eq!(evals(), before + 1, "one live style effect, none leaked");
    assert_eq!(decl(&current, "border"), None);
    assert_eq!(decl(&current, "color").as_deref(), Some("black"));
    assert_eq!(decl(&current, "--probe").as_deref(), Some("1"));

    let before = evals();
    k.cls.set("delta".into());
    assert_eq!(evals(), before + 1, "one live class effect, none leaked");
    assert_eq!(classes(&current), ["probe", "delta"]);

    let before = evals();
    k.state.set("z".into());
    assert_eq!(
        evals(),
        before + 1,
        "one live attribute effect, none leaked"
    );
    assert_eq!(current.get_attribute("data-state").as_deref(), Some("z"));
    assert_eq!(
        find_probe(&root).unwrap().node_id(),
        current.node_id(),
        "and still no re-render"
    );
}

#[test]
fn a_struct_prop_change_on_the_root_path_re_renders_and_re_applies() {
    a_struct_prop_change_re_renders_and_re_applies(as_root);
}

#[test]
fn a_struct_prop_change_on_the_child_path_re_renders_and_re_applies() {
    a_struct_prop_change_re_renders_and_re_applies(as_child);
}

// ── captures ────────────────────────────────────────────────────────────────

/// The render closure runs once per render and builds a `'static` effect for
/// each binding, so a binding that captures a non-`Copy` value — by `move` or
/// by reference — must be handed a clone per render (E0507 otherwise). These
/// compile, and keep working across re-renders.
#[component]
fn non_copy_captures(label: Signal<String>, css: String, cls: String, name: String) -> NodeHandle {
    rsx! {
        div {
            Probe {
                label: {move || label.get()},
                style: {move || css.clone()},
                class: {|| cls.clone()},
                mt: {move || name.len().to_string() + "px"},
                data-name: {|| name.clone()},
            }
        }
    }
}

#[test]
fn bindings_capturing_non_copy_values_survive_re_renders() {
    let label = Signal::new("one".to_string());
    let (_doc, _scope, root) = mount(|s| {
        non_copy_captures(
            s,
            label,
            "color: teal".to_string(),
            "cap".to_string(),
            "nm".to_string(),
        )
    });
    label.set("two".to_string());
    label.set("three".to_string());
    let p = find_probe(&root).expect("rendered");
    assert_eq!(p.get_attribute("aria-label").as_deref(), Some("three"));
    assert_eq!(decl(&p, "color").as_deref(), Some("teal"));
    assert_eq!(classes(&p), ["probe", "cap"]);
    assert_eq!(decl(&p, "margin-top").as_deref(), Some("2px"));
    assert_eq!(p.get_attribute("data-name").as_deref(), Some("nm"));
}
