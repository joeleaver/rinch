//! A `style:` prop is laid **over** the element's own inline style, not in
//! place of it (issue #647).
//!
//! `rsx!` applied a component-level `style:` by writing the root's whole
//! `style` attribute, *after* `Component::render` had already written its own
//! declarations there. Everything the component put in was erased — including
//! every CSS custom property it publishes to its own stylesheet, which is how
//! `Modal`, `Drawer`, `Popover`, `DropdownMenu`, `Notification` and
//! `LoadingOverlay` carry `z_index`, `offset`, `overlay_blur` and the rest
//! (#474). The prop kept compiling and silently stopped working:
//! `Modal { z_index: 517 }` computes 517, and `Modal { z_index: 517, style:
//! "margin: 0" }` computed the pre-#474 default of 200.
//!
//! The same write is emitted from four codegen sites, which is why there are
//! four groups of fixtures below:
//!
//! | site | reached by |
//! |---|---|
//! | `html::generate_style_code` | a component with no reactive prop |
//! | `component_codegen::element_to_dom_component_reactive` | a component with a reactive prop, used as an expression |
//! | `component_codegen::generate_reactive_component_stmt` | the same, as a child of an element |
//! | `html::element_to_dom_html`'s attribute loop | a plain HTML element |
//!
//! Every fixture uses a component that writes **two** declarations of its own,
//! one of which the caller also declares. One alone would sit on a fixed point:
//! a caller declaration that collides with nothing cannot tell "merged" from
//! "merged in the wrong order", and a component that writes nothing cannot tell
//! "merged" from "replaced".
//!
//! The end-to-end half — that a custom property surviving this merge really
//! does still move the computed `z-index` through Stylo — is
//! `rinch`'s `app::overlay_z_index_tests::a_caller_style_prop_does_not_cost_the_
//! overlay_its_level`.

use std::cell::RefCell;
use std::rc::Rc;

use rinch::prelude::*;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// Mount a component into a real desktop document.
///
/// `RinchDocument` rather than the mock on purpose: a style shorthand reaches
/// the node through `set_style`, whose merge is a *backend* behaviour, and the
/// mock only approximates it.
fn mount(
    build: impl FnOnce(&mut RenderScope) -> NodeHandle,
) -> (Rc<RefCell<RinchDocument>>, RenderScope, NodeHandle) {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = build(&mut scope);
    (doc, scope, root)
}

/// The node's inline declarations, as `(property, value)` pairs.
fn decls(node: &NodeHandle) -> Vec<(String, String)> {
    rinch_core::split_declarations(&node.get_attribute("style").unwrap_or_default())
}

/// The value the node's inline style gives `property`, if any.
fn decl(node: &NodeHandle, property: &str) -> Option<String> {
    decls(node)
        .into_iter()
        .find(|(k, _)| k == property)
        .map(|(_, v)| v)
}

// ── the component under test ────────────────────────────────────────────────

/// Stands in for the overlay family: publishes one custom property its
/// stylesheet would read, plus one ordinary declaration the caller is going to
/// collide with.
#[component]
fn Overlay(level: i32) -> NodeHandle {
    let root = __scope.create_element("div");
    root.set_attribute("class", "overlay");
    root.set_attribute("style", &format!("--overlay-z: {}; margin: 8px", level));
    root
}

/// The overlay somewhere under `node`, found by its class rather than by what
/// is in its style — the thing under test must not also be the locator.
fn find_overlay(node: &NodeHandle) -> Option<NodeHandle> {
    if node.get_attribute("class").as_deref() == Some("overlay") {
        return Some(node.clone());
    }
    node.children().iter().find_map(find_overlay)
}

// ── 1. the static component path ────────────────────────────────────────────

#[component]
fn static_style() -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: "margin: 0" }
    }
}

/// A literal `style:` keeps the component's own declarations and wins where it
/// collides with them.
#[test]
fn a_literal_style_prop_keeps_the_components_custom_property() {
    let (_doc, _scope, root) = mount(static_style);
    assert_eq!(
        decl(&root, "--overlay-z").as_deref(),
        Some("517"),
        "the published custom property is what the stylesheet reads; replacing \
         the attribute makes the prop inert with nothing warning"
    );
    assert_eq!(
        decl(&root, "margin").as_deref(),
        Some("0"),
        "the caller is the later author, so the caller wins the collision"
    );
}

#[component]
fn reactive_style(css: Signal<String>) -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: {move || css.get()} }
    }
}

/// A reactive `style:` merges on the first run **and** on every re-run.
#[test]
fn a_reactive_style_prop_keeps_the_components_custom_property() {
    let css = Signal::new(String::from("margin: 0"));
    let (_doc, _scope, root) = mount(|s| reactive_style(s, css));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&root, "margin").as_deref(), Some("0"));

    css.set(String::from("margin: 4px"));
    assert_eq!(
        decl(&root, "--overlay-z").as_deref(),
        Some("517"),
        "a re-run must not eat the component's declarations either"
    );
    assert_eq!(decl(&root, "margin").as_deref(), Some("4px"));
}

/// A re-run replaces the **caller's** previous declarations rather than
/// stacking onto them, and hands back what they displaced.
///
/// Three properties, each with a different fate, because a single one cannot
/// tell the three apart: `padding` is dropped by the caller (pure addition, so
/// it goes), `margin` is dropped by the caller but the component had declared
/// it (so the component's value comes back), and `--overlay-z` was never the
/// caller's at all (so it is untouched throughout).
#[test]
fn a_reactive_style_prop_takes_its_own_previous_declarations_off() {
    let css = Signal::new(String::from("margin: 0; padding: 4px"));
    let (_doc, _scope, root) = mount(|s| reactive_style(s, css));
    assert_eq!(decl(&root, "padding").as_deref(), Some("4px"));

    css.set(String::from("color: red"));
    assert_eq!(
        decl(&root, "padding"),
        None,
        "a declaration the caller no longer makes must not stay behind"
    );
    assert_eq!(
        decl(&root, "margin").as_deref(),
        Some("8px"),
        "the component's own value comes back when the caller stops overriding it"
    );
    assert_eq!(decl(&root, "color").as_deref(), Some("red"));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
}

// ── 2/3. the reactive-component paths ───────────────────────────────────────

/// A reactive *non-style* prop is what routes a component through
/// `element_to_dom_component_reactive` — as an expression here…
#[component]
fn reactive_prop_expression(level: Signal<i32>) -> NodeHandle {
    rsx! {
        Overlay { level: {move || level.get()}, style: "margin: 0" }
    }
}

/// …and through `generate_reactive_component_stmt` here, by sitting inside an
/// element.
#[component]
fn reactive_prop_child(level: Signal<i32>) -> NodeHandle {
    rsx! {
        div {
            Overlay { level: {move || level.get()}, style: "margin: 0" }
        }
    }
}

#[test]
fn a_style_prop_beside_a_reactive_component_prop_still_merges() {
    let level = Signal::new(517);
    let (_doc, _scope, root) = mount(|s| reactive_prop_expression(s, level));
    // The reactive wrapper re-renders into a fresh element, so the node has to
    // be looked up again after every change.
    let overlay = || find_overlay(&root).expect("the overlay is mounted");
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));

    level.set(742);
    assert_eq!(
        decl(&overlay(), "--overlay-z").as_deref(),
        Some("742"),
        "the re-render publishes the new level…"
    );
    assert_eq!(
        decl(&overlay(), "margin").as_deref(),
        Some("0"),
        "…and the caller's style is laid over it again"
    );
}

#[test]
fn a_style_prop_on_a_reactive_component_child_still_merges() {
    let level = Signal::new(517);
    let (_doc, _scope, root) = mount(|s| reactive_prop_child(s, level));
    // The reactive wrapper re-renders into a fresh element, so the node has to
    // be looked up again after every change.
    let overlay = || find_overlay(&root).expect("the overlay is mounted");
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));

    level.set(742);
    assert_eq!(decl(&overlay(), "--overlay-z").as_deref(), Some("742"));
    assert_eq!(decl(&overlay(), "margin").as_deref(), Some("0"));
}

// ── 4. a plain HTML element ─────────────────────────────────────────────────

/// The identical write on an HTML element, where the second author is a style
/// **shorthand** rather than a component.
#[component]
fn html_style_and_shorthand(css: Signal<String>) -> NodeHandle {
    rsx! {
        div { style: {move || css.get()}, p: "12px" }
    }
}

#[test]
fn a_reactive_style_attribute_keeps_a_shorthand_prop() {
    let css = Signal::new(String::from("color: red"));
    let (_doc, _scope, div) = mount(|s| html_style_and_shorthand(s, css));
    assert_eq!(decl(&div, "padding").as_deref(), Some("12px"));
    assert_eq!(decl(&div, "color").as_deref(), Some("red"));

    css.set(String::from("color: blue"));
    assert_eq!(
        decl(&div, "padding").as_deref(),
        Some("12px"),
        "the shorthand prop is a second author on this attribute and the \
         re-firing style effect must not erase it"
    );
    assert_eq!(decl(&div, "color").as_deref(), Some("blue"));
}

/// A shorthand prop still wins a collision with `style:`, which is the order
/// the guide documents. Without this the two could be emitted either way round
/// and every other fixture here would stay green: none of them collides.
#[component]
fn html_style_collides_with_shorthand() -> NodeHandle {
    rsx! {
        div { style: "padding: 0; color: red", p: "12px" }
    }
}

#[test]
fn a_shorthand_prop_wins_a_collision_with_the_style_prop() {
    let (_doc, _scope, div) = mount(html_style_collides_with_shorthand);
    assert_eq!(decl(&div, "padding").as_deref(), Some("12px"));
    assert_eq!(decl(&div, "color").as_deref(), Some("red"));
}

/// An element with a `style:` and nothing else keeps the author's string
/// verbatim — the merge must not reformat what it has no reason to touch.
#[component]
fn html_style_only() -> NodeHandle {
    rsx! {
        div { style: "color:red;gap:4px" }
    }
}

#[test]
fn a_style_prop_with_no_second_author_is_written_through_untouched() {
    let (_doc, _scope, div) = mount(html_style_only);
    assert_eq!(
        div.get_attribute("style").as_deref(),
        Some("color:red;gap:4px")
    );
}

// ── 5. what a re-run must not do to a second author ─────────────────────────

/// The shape that bites without a same-property collision: the caller declares
/// a shorthand, a shorthand prop declares one of its longhands.
#[component]
fn html_reactive_margin_and_mt(css: Signal<String>) -> NodeHandle {
    rsx! {
        div { style: {move || css.get()}, mt: "8px" }
    }
}

/// A reactive `style:` re-run must not demote a shorthand prop's declaration.
///
/// `margin-top: 8px` only beats `margin: 0` while it stays *after* it in the
/// block. The first version of this fix removed the caller's `margin` during
/// the undo and re-appended it, which put it last and silently killed the top
/// margin from the first signal change onward. Neither existing fixture saw it:
/// one uses properties that do not interact, the other a literal `style:` that
/// never re-runs.
///
/// The signal is toggled **twice**, and the two fires are different cases: the
/// first keeps declaring `margin` (the property must hold its slot), the second
/// stops (the declaration must go, and the shorthand's must not go with it).
#[test]
fn a_reactive_style_prop_does_not_demote_a_shorthand() {
    let css = Signal::new(String::from("margin: 0"));
    let (_doc, _scope, div) = mount(|s| html_reactive_margin_and_mt(s, css));
    let order =
        |node: &NodeHandle| -> Vec<String> { decls(node).into_iter().map(|(k, _)| k).collect() };
    assert_eq!(order(&div), ["margin", "margin-top"]);
    assert_eq!(decl(&div, "margin-top").as_deref(), Some("8px"));

    css.set(String::from("margin: 0; color: red"));
    assert_eq!(
        order(&div),
        ["margin", "margin-top", "color"],
        "`margin` must be overwritten where it stands: moved to the end it \
         would beat the shorthand's `margin-top` and the element would lose \
         its top margin"
    );
    assert_eq!(decl(&div, "margin").as_deref(), Some("0"));
    assert_eq!(decl(&div, "margin-top").as_deref(), Some("8px"));

    css.set(String::from("color: red"));
    assert_eq!(
        decl(&div, "margin"),
        None,
        "the caller stopped declaring it, so it goes"
    );
    assert_eq!(
        decl(&div, "margin-top").as_deref(),
        Some("8px"),
        "…and the shorthand prop's declaration is not collateral"
    );
}

/// The same claim as a **cascade** result rather than a declaration order: the
/// element's computed `margin-top` is the shorthand's 8px after the re-run, not
/// the caller's 0.
///
/// Declaration order is the mechanism; this is what a user sees. Both are here
/// because the order assertion alone would pass against a fix that preserved
/// the order and broke the parse.
#[test]
fn the_shorthands_longhand_still_wins_the_cascade_after_a_re_run() {
    let css = Signal::new(String::from("margin: 0"));
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let div = html_reactive_margin_and_mt(&mut scope, css);
    doc.borrow_mut().append_child(body, div.node_id());

    css.set(String::from("margin: 0; color: red"));

    doc.borrow_mut().recompute_all_styles_full();
    doc.borrow_mut().resolve_layout(800.0, 600.0);
    let d = doc.borrow();
    let style = &d
        .tree
        .get(div.node_id().0)
        .expect("the div is in the tree")
        .computed_style;
    assert_eq!(
        format!("{:?}", style.margin_top),
        "Length(8.0)",
        "computed margin-top, after the style: closure re-fired"
    );
}

/// The one case the merge deliberately does **not** settle, pinned because the
/// guide states it: a genuine same-property collision between a *reactive*
/// `style:` and a shorthand prop goes to the shorthand at mount and to the
/// closure from its first re-fire onward.
///
/// The shorthand is applied once, after the style prop, and nothing re-asserts
/// it; making shorthands reactive is separate work. If that ever changes, this
/// fixture is what says the guide has to change with it.
#[component]
fn html_reactive_style_collides_with_shorthand(css: Signal<String>) -> NodeHandle {
    rsx! {
        div { style: {move || css.get()}, p: "12px" }
    }
}

#[test]
fn a_reactive_style_prop_wins_a_collision_from_its_first_re_fire() {
    let css = Signal::new(String::from("padding: 0"));
    let (_doc, _scope, div) = mount(|s| html_reactive_style_collides_with_shorthand(s, css));
    assert_eq!(
        decl(&div, "padding").as_deref(),
        Some("12px"),
        "at mount the shorthand is applied last, so it wins"
    );

    css.set(String::from("padding: 0; color: red"));
    assert_eq!(
        decl(&div, "padding").as_deref(),
        Some("0"),
        "from the first re-fire the closure wins, because nothing re-asserts \
         the shorthand"
    );
}

// ── 6. the arms and orders nothing else reaches ─────────────────────────────

/// `style:` given a **non-closure, non-literal** expression is its own codegen
/// arm, and nothing else here reaches it. There is a real caller:
/// `examples/tree-demo/src/main.rs` passes `style: {icon_style.as_str()}`.
///
/// Note what it takes to make this discriminate. The bare expression is wrapped
/// in an effect that *is* reactive — it tracks whatever it reads — so the
/// signal has to be **set** before the fixture can tell a merge from an assign.
/// Asserting only on the mounted element passes either way: at mount the style
/// prop is written before the shorthand, so the shorthand lands on top of it
/// whichever call made the write.
#[component]
fn html_style_dynamic_expr(css: Signal<String>) -> NodeHandle {
    rsx! {
        div { style: {css.get()}, p: "12px" }
    }
}

#[test]
fn the_non_closure_dynamic_style_arm_merges_too() {
    let css = Signal::new(String::from("color: red"));
    let (_doc, _scope, div) = mount(|s| html_style_dynamic_expr(s, css));
    assert_eq!(decl(&div, "padding").as_deref(), Some("12px"));
    assert_eq!(decl(&div, "color").as_deref(), Some("red"));

    css.set(String::from("color: blue"));
    assert_eq!(
        decl(&div, "padding").as_deref(),
        Some("12px"),
        "the shorthand survives a re-fire of the bare-expression arm too"
    );
    assert_eq!(decl(&div, "color").as_deref(), Some("blue"));
}

/// The same arm on the **component** path, where the second author is the
/// component's own `render` and the erasure is #647's own shape.
#[component]
fn component_style_dynamic_expr(css: Signal<String>) -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: {css.get()} }
    }
}

#[test]
fn the_non_closure_dynamic_style_arm_keeps_the_components_declarations() {
    let css = Signal::new(String::from("color: red"));
    let (_doc, _scope, root) = mount(|s| component_style_dynamic_expr(s, css));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
    assert_eq!(decl(&root, "color").as_deref(), Some("red"));
}

/// The component path emits `style:` and the shorthands in its own order, and
/// `a_shorthand_prop_wins_a_collision_with_the_style_prop` only pins the HTML
/// one — it uses a `div`. Swapping the two on the component path used to kill
/// no test at all.
#[component]
fn component_style_collides_with_shorthand() -> NodeHandle {
    rsx! {
        Overlay { level: 517, style: "padding: 0; color: red", p: "12px" }
    }
}

#[test]
fn a_shorthand_prop_wins_a_collision_on_the_component_path_too() {
    let (_doc, _scope, root) = mount(component_style_collides_with_shorthand);
    assert_eq!(decl(&root, "padding").as_deref(), Some("12px"));
    assert_eq!(decl(&root, "color").as_deref(), Some("red"));
    assert_eq!(decl(&root, "--overlay-z").as_deref(), Some("517"));
}

/// A component whose root is positioned, so a repeated `inset`/`left` in a
/// caller's `style:` decides something the cascade can be asked about.
#[component]
fn PositionedBox() -> NodeHandle {
    let root = __scope.create_element("div");
    root.set_attribute("class", "positioned");
    root.set_attribute("style", "position: absolute; width: 10px; height: 10px");
    root
}

#[component]
fn repeated_property_through_a_style_prop() -> NodeHandle {
    rsx! {
        PositionedBox { style: "inset: 0px; left: 25px; inset: 4px" }
    }
}

/// A property declared twice in one `style:` collapses at the **last**
/// position, which is where CSS puts it, and the difference is a computed
/// value rather than a formatting preference.
///
/// Chrome 150 gives `inset: 0px; left: 25px; inset: 4px` a computed `left` of
/// `4px`: the surviving `inset` sits after the `left` it overrides. Collapsing
/// at the first position instead yields `inset: 4px; left: 25px` and a computed
/// `left` of `25px` — the longhand wins, and the element is 21px out.
///
/// **The component root has to carry its own inline style for this to test
/// anything.** On a bare element the merge takes its verbatim path and hands
/// the author's string to Stylo untouched, and Stylo collapses it correctly by
/// itself — a fixed point where both position rules agree. A second author is
/// what forces the string through `split_declarations`.
#[test]
fn a_repeated_property_collapses_where_css_says_it_does() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = repeated_property_through_a_style_prop(&mut scope);
    doc.borrow_mut().append_child(body, root.node_id());

    doc.borrow_mut().recompute_all_styles_full();
    doc.borrow_mut().resolve_layout(800.0, 600.0);

    let d = doc.borrow();
    let style = &d
        .tree
        .get(root.node_id().0)
        .expect("the box is in the tree")
        .computed_style;
    assert_eq!(
        format!("{:?}", style.left),
        "Length(4.0)",
        "the later `inset` must survive at its own position, after the `left` \
         it overrides"
    );
}

/// A `style:` that itself declares a property **twice** is collapsed the same
/// way whichever author parses it — because since #670 there is only one
/// parser.
///
/// The merge's own splitter gets the position right — that is
/// `a_repeated_property_collapses_where_css_says_it_does` — but only on the
/// path that parses. The **verbatim** path writes the author's string
/// byte-for-byte, duplicate included, and a second author is what then hands
/// it to whatever `rinch-dom` parses attributes with. `p: "12px"` is that
/// second author here: its `set_style` re-serialises the block, and this
/// fixture is what says the re-serialisation agrees with the merge.
///
/// It did not until #670. `rinch-dom` had its own `parse_style_string`, which
/// kept the *first* `inset` position and so let the `left` longhand win —
/// pinned here at `Length(25.0)` as a named deviation, against Chrome 150's
/// measured `4px`. Both parsers are `rinch_core::dom::split_declarations` now,
/// so the two authors agree and so does the browser.
///
/// Desktop only in the sense that matters for the *mechanism*: on the web the
/// verbatim `setAttribute` hands the duplicate to the browser, which has
/// always collapsed it correctly.
#[component]
fn duplicate_through_the_verbatim_path() -> NodeHandle {
    rsx! {
        div {
            style: "position: absolute; width: 10px; height: 10px; \
                    inset: 0px; left: 25px; inset: 4px",
            p: "12px",
        }
    }
}

#[test]
fn a_duplicate_in_a_style_prop_collapses_the_same_way_for_the_other_author() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = duplicate_through_the_verbatim_path(&mut scope);
    doc.borrow_mut().append_child(body, root.node_id());

    assert_eq!(
        decl(&root, "padding").as_deref(),
        Some("12px"),
        "the second author has to have written, or nothing hands the duplicate on"
    );

    doc.borrow_mut().recompute_all_styles_full();
    doc.borrow_mut().resolve_layout(800.0, 600.0);

    let d = doc.borrow();
    let style = &d
        .tree
        .get(root.node_id().0)
        .expect("the box is in the tree")
        .computed_style;
    assert_eq!(
        format!("{:?}", style.left),
        "Length(4.0)",
        "#670: the duplicate must collapse at its *last* position through the \
         other author's `set_style` too, so the surviving `inset` still sits \
         after the `left` it overrides — Chrome gives 4px"
    );
}

// ── 7. the issue's own repro, end to end through `rsx!` ─────────────────────

/// `Modal { z_index: 517, style: "margin: 0" }` computes 517.
///
/// This is the measurement issue #647 reported, run through the real macro, the
/// real component, the real stylesheet and the real cascade — it computed 200,
/// the pre-#474 default. `rinch`'s
/// `app::overlay_z_index_tests::a_caller_style_prop_does_not_cost_the_overlay_its_level`
/// is the same claim one layer down: it applies the merge by hand because
/// `rsx!` cannot expand inside the `rinch` crate, so it pins the merge and
/// **not** the codegen — with both codegen files reverted it stays green. This
/// one does not.
#[component]
fn modal_with_a_caller_style() -> NodeHandle {
    rsx! {
        Modal { opened: true, z_index: 517, style: "margin: 0" }
    }
}

#[test]
fn the_issues_own_repro_still_computes_the_level_it_asked_for() {
    let doc = Rc::new(RefCell::new(RinchDocument::new()));
    let body = doc.borrow().body();
    let mut scope = RenderScope::new(doc.clone(), body);
    let root = modal_with_a_caller_style(&mut scope);
    doc.borrow_mut().append_child(body, root.node_id());

    doc.borrow_mut()
        .load_css(&rinch::components::generate_component_css());
    doc.borrow_mut().recompute_all_styles_full();
    doc.borrow_mut().resolve_layout(800.0, 600.0);

    let d = doc.borrow();
    let overlay: Vec<usize> = d
        .tree
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.attributes
                .get("class")
                .is_some_and(|c| c.split_whitespace().any(|one| one == "rinch-modal__root"))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(overlay.len(), 1, "exactly one modal overlay is mounted");
    assert_eq!(
        d.tree
            .get(overlay[0])
            .expect("the overlay is in the tree")
            .computed_style
            .z_index,
        Some(517),
        "the caller's `style:` must not erase the custom property the modal \
         publishes its level through"
    );
}
