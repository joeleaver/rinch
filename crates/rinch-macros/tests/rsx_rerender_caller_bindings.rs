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

// ── captures that compile on main (review of PR #1193, F1/F2) ───────────────

/// Not `Clone`: a binding may borrow it, as it could on main, but nothing may
/// clone it.
pub struct NoClone(pub String);

impl NoClone {
    pub fn css(&self) -> String {
        self.0.clone()
    }
}

#[component]
fn no_clone_style(label: Signal<String>, nc: NoClone) -> NodeHandle {
    rsx! { Probe { label: {move || label.get()}, style: {|| nc.css()} } }
}

#[component]
fn no_clone_class_expr(label: Signal<String>, nc: NoClone) -> NodeHandle {
    rsx! { Probe { label: {move || label.get()}, class: nc.css() } }
}

#[component]
fn no_clone_attr_child(label: Signal<String>, nc: NoClone) -> NodeHandle {
    rsx! { div { Probe { label: {move || label.get()}, data-x: {|| nc.css()} } } }
}

#[component]
fn no_clone_shorthand(label: Signal<String>, nc: NoClone) -> NodeHandle {
    rsx! { Probe { label: {move || label.get()}, mt: {|| nc.css()} } }
}

/// A binding that borrows a non-`Clone` value compiled on main (it was
/// evaluated inside the render closure). It still compiles — the binding
/// closure is built once, outside the render closure — and still works across
/// re-renders.
#[test]
fn a_binding_borrowing_a_non_clone_value_compiles_and_survives_re_renders() {
    let label = Signal::new("one".to_string());
    let nc = || NoClone("4px".to_string());
    let (_d1, _s1, r1) = mount(|s| no_clone_style(s, label, NoClone("color: navy".into())));
    let (_d2, _s2, r2) = mount(|s| no_clone_class_expr(s, label, NoClone("kept".into())));
    let (_d3, _s3, r3) = mount(|s| no_clone_attr_child(s, label, nc()));
    let (_d4, _s4, r4) = mount(|s| no_clone_shorthand(s, label, nc()));
    label.set("two".to_string());
    let p = |r: &NodeHandle| find_probe(r).expect("rendered");
    assert_eq!(p(&r1).get_attribute("aria-label").as_deref(), Some("two"));
    assert_eq!(decl(&p(&r1), "color").as_deref(), Some("navy"));
    assert_eq!(classes(&p(&r2)), ["probe", "kept"]);
    assert_eq!(p(&r3).get_attribute("data-x").as_deref(), Some("4px"));
    assert_eq!(decl(&p(&r4), "margin-top").as_deref(), Some("4px"));
}

#[component]
fn cell_state(label: Signal<String>) -> NodeHandle {
    let n = Cell::new(0);
    rsx! {
        Probe {
            label: {move || label.get()},
            data-n: {|| { n.set(n.get() + 1); n.get().to_string() }},
        }
    }
}

/// State a non-`move` binding keeps in a captured cell lives as long as the
/// component site, not one render: one binding closure serves every render, as
/// on main (mount + two re-renders = 3).
#[test]
fn a_bindings_captured_state_survives_re_renders() {
    let label = Signal::new("a".to_string());
    let (_d, _s, root) = mount(|s| cell_state(s, label));
    label.set("b".into());
    label.set("c".into());
    let p = find_probe(&root).expect("rendered");
    assert_eq!(p.get_attribute("aria-label").as_deref(), Some("c"));
    assert_eq!(p.get_attribute("data-n").as_deref(), Some("3"));
}

#[component]
fn shared_with_a_prop(label: Signal<String>, name: String) -> NodeHandle {
    rsx! {
        div {
            Probe {
                label: {|| name.clone() + "-" + &label.get()},
                data-name: {|| name.clone()},
            }
        }
    }
}

/// A value named by a struct prop **and** a binding is captured by two
/// closures now (the render closure and the binding fn), so it is shadow-cloned
/// for each: it must be `Clone`. The name has to be visible to the capture
/// analysis — one inside a macro invocation (`format!("{}", name)`) is not, and
/// is then moved twice (E0382), the limitation every `rsx!` capture site has.
#[test]
fn a_value_shared_by_a_prop_and_a_binding_compiles() {
    let label = Signal::new("a".to_string());
    let (_d, _s, root) = mount(|s| shared_with_a_prop(s, label, "nm".into()));
    label.set("b".into());
    let p = find_probe(&root).expect("rendered");
    assert_eq!(p.get_attribute("aria-label").as_deref(), Some("nm-b"));
    assert_eq!(p.get_attribute("data-name").as_deref(), Some("nm"));
}

// ── a value shared by the render and a binding (review of PR #1193, round 2) ─

// Each of these compiled on main, where the bindings were evaluated inside the
// render closure: whatever a struct prop, a child and a binding name is
// captured once. The site bundle keeps that — one closure per site renders and
// evaluates the bindings — so neither `Clone` nor the capture analysis seeing
// the name (it cannot see inside `format!`) is needed.

#[component]
fn r2_e(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! { Probe { label: {move || { let _ = sig.get(); s.clone() }}, data-s: {|| s.clone()} } }
}

#[component]
fn r2_g(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! { Probe { label: {move || { let _ = sig.get(); s.clone() }}, data-s: {|| format!("x-{s}")} } }
}

#[component]
fn r2_i(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! { div { Probe { label: {move || sig.get()}, class: {|| format!("k-{}", s)}, {s.clone()} } } }
}

#[component]
fn r2_j(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! { Probe { label: {move || sig.get()}, style: s.clone() } }
}

#[component]
fn r2_k(sig: Signal<String>, s: NoClone) -> NodeHandle {
    rsx! { Probe { label: {|| { let _ = sig.get(); s.css() }}, data-s: {|| s.css()} } }
}

#[component]
fn r2_l(sig: Signal<String>, s: String) -> NodeHandle {
    rsx! { Probe { label: {|| format!("{}{s}", sig.get())}, data-s: {|| s.clone()} } }
}

#[test]
fn a_value_shared_by_the_render_and_a_binding_compiles_as_on_main() {
    let sig = Signal::new("a".to_string());
    let s = || "v".to_string();
    let (_d1, _s1, e) = mount(|sc| r2_e(sc, sig, s()));
    let (_d2, _s2, g) = mount(|sc| r2_g(sc, sig, s()));
    let (_d3, _s3, i) = mount(|sc| r2_i(sc, sig, s()));
    let (_d4, _s4, j) = mount(|sc| r2_j(sc, sig, "color: plum".into()));
    let (_d5, _s5, k) = mount(|sc| r2_k(sc, sig, NoClone("nc".into())));
    let (_d6, _s6, l) = mount(|sc| r2_l(sc, sig, s()));
    sig.set("b".to_string());
    let p = |r: &NodeHandle| find_probe(r).expect("rendered");
    assert_eq!(p(&e).get_attribute("data-s").as_deref(), Some("v"));
    assert_eq!(p(&g).get_attribute("data-s").as_deref(), Some("x-v"));
    assert_eq!(classes(&p(&i)), ["probe", "k-v"]);
    assert_eq!(decl(&p(&j), "color").as_deref(), Some("plum"));
    assert_eq!(p(&k).get_attribute("data-s").as_deref(), Some("nc"));
    assert_eq!(p(&l).get_attribute("aria-label").as_deref(), Some("bv"));
    assert_eq!(p(&l).get_attribute("data-s").as_deref(), Some("v"));
}

#[component]
fn spacing_token(label: Signal<String>, margin: Signal<String>) -> NodeHandle {
    rsx! { Probe { label: {move || label.get()}, mt: {move || margin.get()} } }
}

/// A reactive shorthand resolves a spacing token at runtime on this path too,
/// at mount, on its own change and after a re-render.
#[test]
fn a_reactive_shorthand_resolves_a_spacing_token_across_re_renders() {
    let label = Signal::new("a".to_string());
    let margin = Signal::new("md".to_string());
    let (_d, _s, root) = mount(|s| spacing_token(s, label, margin));
    let mt = || decl(&find_probe(&root).unwrap(), "margin-top");
    assert_eq!(mt().as_deref(), Some("var(--rinch-spacing-md)"));
    label.set("b".into());
    assert_eq!(mt().as_deref(), Some("var(--rinch-spacing-md)"));
    margin.set("xl".into());
    assert_eq!(mt().as_deref(), Some("var(--rinch-spacing-xl)"));
}
