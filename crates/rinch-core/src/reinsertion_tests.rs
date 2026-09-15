//! The post-condition of [`NodeHandle::remove`] and [`NodeHandle::discard`]
//! across the reactive helpers, issue #719 — over [`MockDomDocument`], which is
//! the host-runnable oracle for both real backends.
//!
//! # Why the mock is the right oracle here
//!
//! It does not paint or lay anything out; what it models is exactly the
//! bookkeeping the two backends disagreed about. Before #719 it faithfully
//! emulated the **browser** rule — `remove_node` dropped the subtree from its
//! table — so a caller that toggled a captured handle failed here as well as in
//! Chrome, and every fixture below was red on the host. That is why the
//! divergence is testable without a browser at all.
//!
//! The rule, in one sentence: **`remove` detaches, `discard` retires**, and a
//! reactive helper picks whichever matches what it knows about the subtree's
//! future. The browser half is pinned in real Chrome by
//! `crates/rinch-web/tests/reinsertion.rs`; the desktop half against the real
//! `RinchDocument` by `crates/rinch-dom/tests/reinsertion_handle_tests.rs`.
//!
//! # Each fixture is a pair
//!
//! A suite that only ever asserts "the subtree came back" is passed by a
//! backend that retires nothing at all, which leaks without bound — the very
//! thing #184 added the prune for. So each *re-show* fixture below has a
//! *growth* twin asserting the opposite verb still retires. Only the pair
//! distinguishes the fix from either degenerate backend.
//!
//! The growth half is **not** an assertion about `discard` being called: it
//! counts nodes in the mock's table across many toggles, which is the same
//! quantity `rinch-web`'s `__node_registry_len` counts in a browser. A helper
//! that releases the wrong way fails here. PR #728's first round had exactly
//! this hole — every re-show direction was pinned and no growth direction was,
//! for `show_dom` and `match_dom`, and both leaked one subtree per toggle in
//! real Chrome while the board stayed green.
//!
//! # The rule these helpers apply
//!
//! **Ownership**, not a guess about ids: a branch closure runs inside its own
//! [`RenderScope`], and a node it *built* through that scope is discarded on the
//! way out while a node it was *handed* is only detached. Same rule #141 PR4
//! gave signals and effects, applied to nodes.

use crate::dom::mock::MockDomDocument;
use crate::dom::{DomDocument, NodeHandle, RenderScope};
use crate::reactive::Signal;
use crate::{for_each_dom_typed, match_dom, show_dom};
use std::cell::RefCell;
use std::rc::Rc;

fn doc() -> Rc<RefCell<MockDomDocument>> {
    Rc::new(RefCell::new(MockDomDocument::new()))
}

fn body_handle(doc: &Rc<RefCell<MockDomDocument>>) -> NodeHandle {
    let body = doc.borrow().body();
    NodeHandle::new(body, Rc::downgrade(doc) as _)
}

fn scope(doc: &Rc<RefCell<MockDomDocument>>) -> RenderScope {
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    RenderScope::new(dyn_doc, body)
}

/// Tag names of the body's children, so a fixture reads the way the page does.
fn body_tags(doc: &Rc<RefCell<MockDomDocument>>) -> Vec<String> {
    let d = doc.borrow();
    d.get_children(d.body())
        .into_iter()
        .filter_map(|c| d.tag_name(c))
        .collect()
}

// ── the primitive: remove detaches, discard retires ─────────────────────────

/// The whole of #719 in four lines, with no helper in the way.
#[test]
fn a_removed_node_can_be_inserted_again_with_its_subtree() {
    let doc = doc();
    let body = doc.borrow().body();
    let panel = doc.borrow_mut().create_element("section");
    let inner = doc.borrow_mut().create_element("p");
    doc.borrow_mut().append_child(panel, inner);
    doc.borrow_mut().append_child(body, panel);

    doc.borrow_mut().remove_node(panel);
    assert!(
        doc.borrow().get_children(body).is_empty(),
        "precondition: remove takes it out of the tree"
    );

    doc.borrow_mut().append_child(body, panel);
    assert_eq!(body_tags(&doc), ["section"], "#719: the node comes back");
    assert_eq!(
        doc.borrow().get_children(panel),
        vec![inner],
        "#719: and so does its subtree"
    );
}

/// A removed node is still writable — it has left the tree, not the document.
/// A branch that restyles a hidden subtree before showing it again rests on
/// this, and the browser prune used to swallow the write.
#[test]
fn a_removed_node_can_still_be_written_to() {
    let doc = doc();
    let body = doc.borrow().body();
    let node = doc.borrow_mut().create_element("div");
    doc.borrow_mut().append_child(body, node);

    doc.borrow_mut().remove_node(node);
    doc.borrow_mut().set_attribute(node, "class", "later");

    assert_eq!(
        doc.borrow().get_attribute(node, "class").as_deref(),
        Some("later"),
        "#719: a removed node is detached, not gone"
    );
}

/// The twin. On a backend that **does** retire — this mock, and `rinch-web` —
/// every operation on a discarded id does nothing: never a panic, and never a
/// write aimed at somebody else, since no backend re-issues an id it retired.
///
/// Scoped to a retiring backend deliberately. `rinch-dom` reclaims nothing on
/// this route (#723), so a discarded node there still re-inserts and still takes
/// writes — which is exactly why this fixture matters: the mock retires like the
/// browser, so re-attaching a discarded handle fails on the host instead of only
/// in Chrome.
#[test]
fn a_discarded_node_is_a_silent_no_op_on_a_retiring_backend() {
    let doc = doc();
    let body = doc.borrow().body();
    let node = doc.borrow_mut().create_element("div");
    let inner = doc.borrow_mut().create_element("span");
    doc.borrow_mut().append_child(node, inner);
    doc.borrow_mut().append_child(body, node);

    doc.borrow_mut().discard_node(node);

    // Every one of these must simply do nothing, and none may panic.
    doc.borrow_mut().append_child(body, node);
    doc.borrow_mut().set_attribute(node, "class", "zombie");
    doc.borrow_mut().set_text_content(inner, "zombie");
    doc.borrow_mut().remove_node(node);
    doc.borrow_mut().discard_node(node);

    assert!(
        doc.borrow().get_children(body).is_empty(),
        "#719: a discarded node must not be relisted"
    );
    assert!(
        doc.borrow().tag_name(node).is_none() && doc.borrow().tag_name(inner).is_none(),
        "#184: the whole subtree is retired"
    );
}

// ── the helpers that may RE-SHOW ────────────────────────────────────────────

/// `show_dom` over a branch closure that returns a **captured** handle — the
/// `rsx!` shape `if cond { {panel} }`, and the exact report in #654/#719.
#[test]
fn show_dom_can_re_show_a_captured_handle() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let inner = sc.create_element("p");
    panel.append_child(&inner);

    let visible = Signal::new(true);
    let captured = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |_: &mut RenderScope| captured.clone(),
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    assert_eq!(body_tags(&doc), ["section"], "precondition: shown");

    visible.set(false);
    assert_eq!(
        body_tags(&doc),
        Vec::<String>::new(),
        "precondition: hidden"
    );

    visible.set(true);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: re-showing a captured handle must put its subtree back"
    );
    assert_eq!(
        panel.children().len(),
        1,
        "#719: with its children still under it"
    );

    // Twice, so the fixture is not sitting on a single toggle: a backend that
    // retired on the *second* removal would pass a one-toggle test.
    visible.set(false);
    visible.set(true);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: and on every later toggle"
    );
}

/// The same for `match_dom`: switching away from an arm and back must bring a
/// captured subtree with it.
#[test]
fn match_dom_can_re_show_a_captured_arm() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let first = sc.create_element("section");
    let second = sc.create_element("aside");

    let arm = Signal::new(0usize);
    let (a, b) = (first.clone(), second.clone());
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![
            Box::new(move |_: &mut RenderScope| a.clone())
                as Box<dyn Fn(&mut RenderScope) -> NodeHandle>,
            Box::new(move |_: &mut RenderScope| b.clone()),
        ],
    );
    assert_eq!(body_tags(&doc), ["section"]);

    arm.set(1);
    assert_eq!(body_tags(&doc), ["aside"]);

    arm.set(0);
    assert_eq!(
        body_tags(&doc),
        ["section"],
        "#719: switching back to an arm must restore its captured subtree"
    );
}

// ── the helpers that DISCARD ────────────────────────────────────────────────

/// A dropped `for` row is gone for good, so the helper says so and the backend
/// lets go. This is the half that keeps #184's leak closed: without it, a `for`
/// churning a list would pin every row it ever rendered.
#[test]
fn a_dropped_for_row_is_discarded() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let rows = Signal::new(vec![1u32, 2u32]);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let node = s.create_element("div");
            node.set_attribute("data-n", &n.to_string());
            seen.borrow_mut().push(node.clone());
            node
        },
    );
    assert_eq!(built.borrow().len(), 2, "precondition: two rows rendered");
    let doomed = built.borrow()[1].clone();

    rows.set(vec![1u32]);

    assert_eq!(
        doc.borrow().tag_name(doomed.node_id()),
        None,
        "#719: a dropped `for` row is discarded, so the backend can release it"
    );
}

/// A `for` row whose *data* changed is re-rendered, and the node it replaces is
/// discarded too — a second site in the same helper, and the one with no
/// `ListOp::Remove` to make it obvious.
#[test]
fn a_rerendered_for_row_discards_the_node_it_replaces() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let rows = Signal::new(vec![(1u32, "a".to_string())]);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |r: &(u32, String)| r.0.to_string(),
        move |r: (u32, String), s: &mut RenderScope| {
            let node = s.create_element("div");
            node.set_attribute("data-v", &r.1);
            seen.borrow_mut().push(node.clone());
            node
        },
    );
    let first = built.borrow()[0].clone();

    // Same key, different data — the `Changed` arm, not `Remove`.
    rows.set(vec![(1u32, "b".to_string())]);

    assert_eq!(
        built.borrow().len(),
        2,
        "precondition: the row was re-rendered"
    );
    assert_eq!(
        doc.borrow().tag_name(first.node_id()),
        None,
        "#719: the replaced node is discarded, not merely detached"
    );
    assert_eq!(body_tags(&doc), ["div"], "exactly one row is mounted");
}

/// The component re-render effect throws its previous output away, so it
/// discards. Its `render_fn` builds afresh every run — the contract stated on
/// `reactive_component_dom` — and this is the fixture that holds it to it.
#[test]
fn a_rerendered_component_discards_its_previous_output() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let version = Signal::new(0u32);
    let built: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = built.clone();
    reactive_component_dom(&mut sc, &body, move |s: &mut RenderScope| {
        let node = s.create_element("div");
        node.set_attribute("data-v", &version.get().to_string());
        seen.borrow_mut().push(node.clone());
        node
    });
    let first = built.borrow()[0].clone();

    version.set(1);

    assert_eq!(built.borrow().len(), 2, "precondition: it re-rendered");
    assert_eq!(
        doc.borrow().tag_name(first.node_id()),
        None,
        "#719: the previous output is discarded, so the backend can release it"
    );
    assert_eq!(body_tags(&doc), ["div"], "exactly one output is mounted");
}

// ── growth: the half PR #728's first round was missing ──────────────────────

/// How many nodes a document holds after `toggles` rounds, and how many it held
/// after the first — the shape every growth fixture below asserts on.
///
/// Taking the baseline **after** the first toggle pair matters: the first show
/// mounts content that was not there before, so a baseline taken at zero would
/// count that as growth and hide a real leak behind an expected one.
fn growth(doc: &Rc<RefCell<MockDomDocument>>, drive: impl Fn(usize), toggles: usize) -> isize {
    drive(0);
    drive(1);
    let baseline = doc.borrow().__node_count() as isize;
    for i in 2..toggles {
        drive(i);
    }
    doc.borrow().__node_count() as isize - baseline
}

/// A `show_dom` branch that **builds** its markup — the ordinary
/// `if open.get() { p { "hi" } }` — must not grow the document without bound.
///
/// Every show mints a fresh subtree and every hide throws one away, so a helper
/// that merely detached would strand one per toggle for the life of the page.
#[test]
fn a_fresh_show_branch_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        |s: &mut RenderScope| {
            // Three nodes per show, so a leak is unmistakable.
            let wrap = s.create_element("section");
            let inner = s.create_element("p");
            let text = s.create_text("hi");
            inner.append_child(&text);
            wrap.append_child(&inner);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    let delta = growth(&doc, |i| visible.set(i % 2 == 0), 200);
    assert_eq!(
        delta, 0,
        "#719/#184: a fresh `show` branch must not grow the document — leaked {delta} nodes over 198 toggles"
    );
}

/// The same for `match_dom`, whose arms both build.
#[test]
fn fresh_match_arms_do_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let arm = Signal::new(0usize);
    let build = |tag: &'static str| {
        move |s: &mut RenderScope| {
            let node = s.create_element(tag);
            let text = s.create_text(tag);
            node.append_child(&text);
            node
        }
    };
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![
            Box::new(build("section")) as Box<dyn Fn(&mut RenderScope) -> NodeHandle>,
            Box::new(build("aside")),
        ],
    );

    let delta = growth(&doc, |i| arm.set(i % 2), 200);
    assert_eq!(
        delta, 0,
        "#719/#184: fresh `match` arms must not grow the document — leaked {delta} nodes over 198 switches"
    );
}

/// The same for `reactive_component_dom`, whose `render_fn` builds.
#[test]
fn a_rebuilding_component_does_not_grow_the_document() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let version = Signal::new(0u32);
    reactive_component_dom(&mut sc, &body, move |s: &mut RenderScope| {
        let node = s.create_element("div");
        let text = s.create_text(&version.get().to_string());
        node.append_child(&text);
        node
    });

    let delta = growth(&doc, |i| version.set(i as u32), 200);
    assert_eq!(
        delta, 0,
        "#719/#184: a re-rendering component must not grow the document — leaked {delta} nodes"
    );
}

/// The same for `for_each_dom_typed`, whose `view` builds. This one was already
/// green before the ownership rule (the row was discarded outright); it is here
/// so the four helpers are pinned by one shape rather than three plus an
/// exception.
#[test]
fn a_churning_for_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let rows = Signal::new(vec![0u32]);
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        |n: u32, s: &mut RenderScope| {
            let row = s.create_element("div");
            let text = s.create_text(&n.to_string());
            row.append_child(&text);
            row
        },
    );

    let delta = growth(&doc, |i| rows.set(vec![i as u32]), 200);
    assert_eq!(
        delta, 0,
        "#719/#184: a churning `for` must not grow the document — leaked {delta} nodes"
    );
}

// ── the memoised shapes, which ownership makes work again ───────────────────

/// A **memoising** `for` view — one that hands back a subtree it built once —
/// keeps its rows across a remove/re-insert cycle (issue #719).
///
/// #719's own text names "a `for` view that memoises rows" as in scope, and an
/// earlier round of PR #728 discarded the row outright, which broke it on
/// `rinch-web`. Ownership fixes it without a special case: the row was not built
/// through the row's scope, so it is detached rather than retired.
///
/// **Which flavour of memoisation this is matters.** The cached subtree here is
/// built on the *outer* scope, before the `for`, and the view only ever hands it
/// back — the supported shape. A view that builds **lazily through the row's own
/// scope** and caches afterwards owns its row by this rule and loses it on the
/// first removal; that is **#733**, and `rinch-dom`'s
/// `branch_helper_transition_tests::a_for_row_reinserted_under_the_same_key_can_still_transition`
/// is the fixture that models it and cannot see the loss.
#[test]
fn a_memoised_for_row_survives_leaving_the_list() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    // Built once, outside any row scope.
    let cached = sc.create_element("article");
    let inner = sc.create_text("CACHED");
    cached.append_child(&inner);
    let cached_id = cached.node_id();

    let rows = Signal::new(vec![1u32]);
    let memo = cached.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |_n: u32, _s: &mut RenderScope| memo.clone(),
    );
    assert_eq!(body_tags(&doc), ["article"], "precondition: mounted");

    rows.set(vec![]);
    assert_eq!(
        body_tags(&doc),
        Vec::<String>::new(),
        "precondition: removed"
    );

    rows.set(vec![1u32]);
    assert_eq!(
        doc.borrow().tag_name(cached_id).as_deref(),
        Some("article"),
        "#719: a memoised row must survive leaving the list"
    );
    assert_eq!(
        body_tags(&doc),
        ["article"],
        "#719: and be re-insertable when its key comes back"
    );
}

/// The same for `reactive_component_dom`: a `render_fn` that memoises is the
/// #654 shape and is supported.
#[test]
fn a_memoising_component_render_fn_keeps_its_subtree() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let cached = sc.create_element("article");
    let cached_id = cached.node_id();

    let version = Signal::new(0u32);
    let memo = cached.clone();
    reactive_component_dom(&mut sc, &body, move |_s: &mut RenderScope| {
        let _ = version.get();
        memo.clone()
    });
    assert_eq!(body_tags(&doc), ["article"], "precondition: mounted");

    version.set(1);
    assert_eq!(
        doc.borrow().tag_name(cached_id).as_deref(),
        Some("article"),
        "#719: a memoising render_fn's subtree must survive a re-render"
    );
    assert_eq!(body_tags(&doc), ["article"], "#719: and be re-inserted");
}

// ── the documented gap ──────────────────────────────────────────────────────

/// **Pins a known limitation, not a desired behaviour** (issue #732).
///
/// Ownership is asked of the **content root only**, because discarding is
/// recursive: a root the branch built takes its whole subtree with it, which is
/// what correctly reclaims a nested `for`'s rows and an inner branch's markup.
/// The cost is that a *captured* handle nested inside branch-built markup —
/// `if open { div { {panel} } }` — is inside that recursion and is retired with
/// the wrapper.
///
/// This is the behaviour on both backends before #719 as well as after, so it is
/// not a regression; the fixture exists so that closing #732 is a deliberate
/// change with a test to update, rather than a silent side effect. The
/// unwrapped form (`if open { {panel} }`) is the shape #654 reported and it
/// works — `show_dom_can_re_show_a_captured_handle` is the contrast.
#[test]
fn a_captured_handle_nested_inside_fresh_markup_is_still_lost() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let panel_id = panel.node_id();

    let visible = Signal::new(true);
    let captured = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            // The wrapper is the branch's; the panel is not.
            let wrap = s.create_element("div");
            wrap.append_child(&captured);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    assert_eq!(body_tags(&doc), ["div"], "precondition: shown");

    visible.set(false);

    assert_eq!(
        doc.borrow().tag_name(panel_id),
        None,
        "#732: a captured handle inside branch-built markup goes with the wrapper. \
         If this now answers Some(..), #732 is fixed — delete the fixture, do not relax it"
    );
}
