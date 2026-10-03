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
use crate::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
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

/// The one **element** child of `parent` — every marker-based helper also
/// leaves its own comment marker as a permanent sibling, so "the mounted
/// wrapper" is not simply `get_children(parent)[0]`. Panics if there isn't
/// exactly one.
fn mounted_element(doc: &Rc<RefCell<MockDomDocument>>, parent: NodeId) -> NodeId {
    let d = doc.borrow();
    let elements: Vec<NodeId> = d
        .get_children(parent)
        .into_iter()
        .filter(|&c| d.tag_name(c).is_some())
        .collect();
    assert_eq!(
        elements.len(),
        1,
        "expected exactly one mounted element child, found {elements:?}"
    );
    elements[0]
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

// ── #732: a captured handle nested inside branch-built markup ──────────────

/// A captured handle nested inside branch-built markup — `if open { div {
/// {panel} } }` — is detached (not discarded) when the wrapper hides, and the
/// next show puts it back (issue #732).
///
/// Before the fix, `discard`ing the wrapper was recursive and reached straight
/// through to `panel`, retiring it with the wrapper; this is the fixture that
/// used to pin that as a known limitation
/// (`a_captured_handle_nested_inside_fresh_markup_is_still_lost`, now deleted
/// per its own instruction). The unwrapped form (`if open { {panel} }`) is the
/// shape #654 reported and already worked —
/// `show_dom_can_re_show_a_captured_handle` is the contrast.
#[test]
fn show_dom_can_re_show_a_captured_handle_nested_inside_fresh_markup() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let panel_id = panel.node_id();
    let inner = sc.create_element("p");
    panel.append_child(&inner);

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
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "#732: the nested captured handle must survive the wrapper's discard — \
         detached, not retired"
    );
    assert_eq!(
        body_tags(&doc),
        Vec::<String>::new(),
        "precondition: hidden — the wrapper itself IS discarded"
    );

    visible.set(true);
    assert_eq!(
        body_tags(&doc),
        ["div"],
        "#732: re-showing rebuilds a fresh wrapper"
    );
    let new_wrap = mounted_element(&doc, body.node_id());
    assert_eq!(
        doc.borrow().get_children(new_wrap),
        vec![panel_id],
        "#732: the fresh wrapper holds the SAME captured panel, with its \
         own children intact"
    );
    assert_eq!(
        panel.children().len(),
        1,
        "#732: the captured panel's own subtree was never touched"
    );

    // Twice, so the fixture is not sitting on a single toggle.
    visible.set(false);
    visible.set(true);
    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "#732: and on every later toggle"
    );
}

/// The nested-wrapper shape must not grow the document either (issue #732):
/// every hide discards a *fresh* wrapper and detaches the *same* captured
/// panel, so nothing accumulates on either side.
#[test]
fn a_nested_captured_handle_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let visible = Signal::new(false);
    let captured = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let text = s.create_text("hi");
            wrap.append_child(&text);
            wrap.append_child(&captured);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    let delta = growth(&doc, |i| visible.set(i % 2 == 0), 200);
    assert_eq!(
        delta, 0,
        "#732: a nested captured handle must not grow the document — leaked {delta} nodes"
    );
}

/// `match_dom`'s twin: an arm's own markup wraps a captured handle.
#[test]
fn match_dom_can_re_show_a_captured_arm_nested_inside_fresh_markup() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let panel_id = panel.node_id();
    let other = sc.create_element("aside");

    let arm = Signal::new(0usize);
    let captured = panel.clone();
    let other_arm = other.clone();
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![
            Box::new(move |s: &mut RenderScope| {
                let wrap = s.create_element("div");
                wrap.append_child(&captured);
                wrap
            }) as Box<dyn Fn(&mut RenderScope) -> NodeHandle>,
            Box::new(move |_: &mut RenderScope| other_arm.clone()),
        ],
    );
    assert_eq!(body_tags(&doc), ["div"]);

    arm.set(1);
    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "#732: switching away must detach the nested captured panel, not retire it"
    );
    assert_eq!(body_tags(&doc), ["aside"]);

    arm.set(0);
    assert_eq!(
        body_tags(&doc),
        ["div"],
        "#732: switching back rebuilds the wrapper"
    );
    let wrap_id = mounted_element(&doc, body.node_id());
    assert_eq!(
        doc.borrow().get_children(wrap_id),
        vec![panel_id],
        "#732: holding the SAME captured panel"
    );
}

/// `reactive_component_dom`'s twin: a re-rendering `render_fn` wraps a
/// captured handle in fresh markup.
#[test]
fn a_rerendering_component_preserves_a_nested_captured_handle() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let panel_id = panel.node_id();

    let version = Signal::new(0u32);
    let captured = panel.clone();
    reactive_component_dom(&mut sc, &body, move |s: &mut RenderScope| {
        let _ = version.get();
        let wrap = s.create_element("div");
        wrap.append_child(&captured);
        wrap
    });
    assert_eq!(body_tags(&doc), ["div"], "precondition: mounted");

    version.set(1);
    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "#732: a re-render must detach the nested captured panel, not retire it \
         with the previous output's wrapper"
    );
    assert_eq!(body_tags(&doc), ["div"], "exactly one output is mounted");
    let wrap_id = mounted_element(&doc, body.node_id());
    assert_eq!(
        doc.borrow().get_children(wrap_id),
        vec![panel_id],
        "#732: the fresh wrapper holds the SAME captured panel"
    );
}

/// `for_each_dom_typed`'s twin: a `view` that builds fresh markup around a
/// handle it was handed (not one it built itself), per row.
#[test]
fn a_for_row_preserves_a_nested_captured_handle_across_removal_and_reinsertion() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    // Built once, outside any row scope — the row's `view` wraps it in fresh
    // per-row markup rather than handing it straight back.
    let panel = sc.create_element("article");
    let panel_id = panel.node_id();

    let rows = Signal::new(vec![1u32]);
    let captured = panel.clone();
    for_each_dom_typed(
        &mut sc,
        &body,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |_n: u32, s: &mut RenderScope| {
            let wrap = s.create_element("div");
            wrap.append_child(&captured);
            wrap
        },
    );
    assert_eq!(body_tags(&doc), ["div"], "precondition: mounted");

    rows.set(vec![]);
    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("article"),
        "#732: dropping the row must detach its nested captured panel, not \
         retire it with the row's own wrapper"
    );
    assert_eq!(
        body_tags(&doc),
        Vec::<String>::new(),
        "precondition: removed"
    );

    rows.set(vec![1u32]);
    assert_eq!(
        body_tags(&doc),
        ["div"],
        "#732: the row comes back with a fresh wrapper"
    );
    let wrap_id = mounted_element(&doc, body.node_id());
    assert_eq!(
        doc.borrow().get_children(wrap_id),
        vec![panel_id],
        "#732: holding the SAME captured panel"
    );
}

// ── #732 cost: the walk must be bounded by the discarded subtree ───────────

/// `collect_captured_descendants`' walk must cost proportionally to the
/// subtree being discarded, not to the whole document (issue #732's own
/// design note 2 called out "a full subtree walk ... paid by every app
/// whether or not it ever captures a handle" as the risk to avoid).
///
/// Measured via [`MockDomDocument::__get_children_calls`]: an unrelated
/// sibling subtree, never touched by the branch, must not move the call
/// count at all when it grows from 10 nodes to 2000.
#[test]
fn the_capture_walk_is_bounded_by_the_discarded_subtree_not_the_document() {
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
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            wrap.append_child(&captured);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    // An unrelated sibling subtree, sized by `width` — not part of the
    // branch, so a bounded walk must never reach it.
    let add_elsewhere = |sc: &mut RenderScope, width: usize| {
        for _ in 0..width {
            let leaf = sc.create_element("i");
            body.append_child(&leaf);
        }
    };

    add_elsewhere(&mut sc, 10);
    let before_small = doc.borrow().__get_children_calls();
    visible.set(false);
    visible.set(true);
    let small_delta = doc.borrow().__get_children_calls() - before_small;

    add_elsewhere(&mut sc, 2000);
    let before_large = doc.borrow().__get_children_calls();
    visible.set(false);
    visible.set(true);
    let large_delta = doc.borrow().__get_children_calls() - before_large;

    assert_eq!(
        small_delta, large_delta,
        "#732: the capture walk's cost must depend only on the discarded \
         subtree, not on an unrelated sibling subtree elsewhere in the \
         document — {small_delta} get_children calls with a 10-node sibling \
         subtree vs {large_delta} with a 2000-node one"
    );
    // A positive control: the count really does move with the DISCARDED
    // subtree's own size, so a walk that silently did nothing (and so could
    // never fail the equality above either) is caught too.
    assert!(
        small_delta > 0,
        "positive control: the walk must call get_children at least once"
    );
}

// ── adversarial review of PR #1360: a nested `for`'s rows are NOT captured ──

/// A `for` loop nested inside a `show_dom` branch builds its rows through a
/// *child* `RenderScope` the `for` machinery creates internally
/// (`for_loop.rs`'s `RenderScope::new(doc, parent_id)` per item) — not through
/// the branch's own scope. `collect_captured_descendants` (PR #1360, issue
/// #732) asks only the branch's own `scope.created(..)` of each descendant,
/// so every row built by that child scope answers `false` and is classified
/// as "captured" even though nothing outside the branch is holding it. The
/// walk stops descending there (by design, for genuine captures) and the row
/// is DETACHED instead of discarded. Each hide->show cycle creates a fresh
/// set of rows (through a fresh for_each_dom_typed/child_scope) and detaches
/// (not discards) the old set, so the old rows are never freed: a leak on a
/// retiring backend (rinch-web), exactly the #719 shape PR #1360 was
/// supposed to have fully closed for nested reactive content.
#[test]
fn a_nested_for_inside_a_branch_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            for_each_dom_typed(
                s,
                &wrap,
                || vec![1u32, 2u32, 3u32],
                |n: &u32| n.to_string(),
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("li");
                    let text = rs.create_text(&n.to_string());
                    row.append_child(&text);
                    row
                },
            );
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    let delta = growth(&doc, |i| visible.set(i % 2 == 0), 200);
    assert_eq!(
        delta, 0,
        "#732 regression: a `for` nested inside a branch must not grow the \
         document when the branch hides and re-shows — leaked {delta} nodes \
         over 198 toggles (its rows were detached as 'captured' rather than \
         discarded with the rest of the branch)"
    );
}

/// Same mechanism as the `for` case above, but with a nested
/// `reactive_component_dom` instead of a `for` loop — `rsx!`'s PascalCase
/// component sites build through their own `RenderScope` the same way
/// (`reactive_component_dom`'s `render_fn` runs inside a fresh
/// `current_scope`), so a plain `if open { Card {} }`-shaped nesting leaks
/// the same way.
#[test]
fn a_nested_component_inside_a_branch_does_not_grow_the_document() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            reactive_component_dom(s, &wrap, |inner: &mut RenderScope| {
                let node = inner.create_element("article");
                let text = inner.create_text("hi");
                node.append_child(&text);
                node
            });
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    let delta = growth(&doc, |i| visible.set(i % 2 == 0), 200);
    assert_eq!(
        delta, 0,
        "#732 regression: a component nested inside a branch must not grow \
         the document when the branch hides and re-shows — leaked {delta} \
         nodes over 198 toggles"
    );
}

// ── #732 review: match_dom twin, release_scratch_container, ordering ───────

/// `match_dom`'s twin of the two leaks above: an arm nested inside the match
/// builds a `for` loop (and, separately, a re-rendering component) through
/// its own child scope.
#[test]
fn a_nested_for_inside_a_match_arm_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let arm = Signal::new(0usize);
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![Box::new(|s: &mut RenderScope| {
            let wrap = s.create_element("div");
            for_each_dom_typed(
                s,
                &wrap,
                || vec![1u32, 2u32, 3u32],
                |n: &u32| n.to_string(),
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("li");
                    let text = rs.create_text(&n.to_string());
                    row.append_child(&text);
                    row
                },
            );
            wrap
        }) as Box<dyn Fn(&mut RenderScope) -> NodeHandle>],
    );

    let delta = growth(&doc, |i| arm.set(i % 2), 200);
    assert_eq!(
        delta, 0,
        "#732 regression: a `for` nested inside a match arm must not grow \
         the document — leaked {delta} nodes over 198 switches"
    );
}

/// The `reactive_component_dom` twin of the same shape, nested inside a
/// `match` arm.
#[test]
fn a_nested_component_inside_a_match_arm_does_not_grow_the_document() {
    use crate::dom::reactive_component_dom;

    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let arm = Signal::new(0usize);
    match_dom(
        &mut sc,
        &body,
        move || arm.get(),
        vec![Box::new(|s: &mut RenderScope| {
            let wrap = s.create_element("div");
            reactive_component_dom(s, &wrap, |inner: &mut RenderScope| {
                let node = inner.create_element("article");
                let text = inner.create_text("hi");
                node.append_child(&text);
                node
            });
            wrap
        }) as Box<dyn Fn(&mut RenderScope) -> NodeHandle>],
    );

    let delta = growth(&doc, |i| arm.set(i % 2), 200);
    assert_eq!(
        delta, 0,
        "#732 regression: a component nested inside a match arm must not \
         grow the document — leaked {delta} nodes over 198 switches"
    );
}

/// `release_scratch_container` — the fifth discard site, not covered by any
/// of the above — with a **genuinely captured** handle nested inside an
/// unadopted leftover, the exact shape `if open { Card { {panel} } } }`
/// produces when `Card` ignores its children. `panel` is built by an
/// entirely separate, earlier `RenderScope` (`sc`), never by the component
/// site's own scope (`site_scope`), so it must be detached, not discarded —
/// and the leftover wrapper plus the scratch `<template>` container must
/// still be fully discarded (retired).
///
/// This is also the fixture the review's mutation matrix found missing:
/// it kills both (2) reordering `discard_owned_preserving_captured`'s two
/// statements (`root.discard()` before detaching `captured` would retire
/// `panel` along with `leftover`, since discard is recursive and nothing
/// would have pulled `panel` out first) and (4) replacing
/// `discard_owned_preserving_captured` with a bare `leftover.discard()` in
/// `release_scratch_container` (same effect).
#[test]
fn release_scratch_container_detaches_a_nested_captured_leftover_rather_than_discarding_it() {
    use crate::dom::release_scratch_container;

    let doc = doc();
    let mut sc = scope(&doc); // the OUTER scope that built `panel`, earlier.
    let panel = sc.create_element("section");
    let panel_id = panel.node_id();

    // A SEPARATE, later scope — the component call site's own `__scope`
    // (`component_codegen.rs`'s call), as `if open { Card { {panel} } } }`
    // would build it.
    let mut site_scope = scope(&doc);
    let container = site_scope.create_element("template");
    let leftover = site_scope.create_element("div");
    leftover.append_child(&panel);
    container.append_child(&leftover);
    let leftover_id = leftover.node_id();
    let container_id = container.node_id();

    release_scratch_container(&site_scope, &container);

    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "#732: a captured handle nested inside an unadopted scratch-container \
         leftover must be detached, not discarded with it"
    );
    assert_eq!(
        doc.borrow().tag_name(leftover_id),
        None,
        "the unadopted leftover itself must still be discarded"
    );
    assert_eq!(
        doc.borrow().tag_name(container_id),
        None,
        "the scratch container itself is always discarded"
    );
}

// ── review2-1360: the watermark is an id-ordering proxy for ownership, and ──
// ── id ordering is not ownership ────────────────────────────────────────────

/// A genuinely independent node — minted via raw document access (no scope at
/// all, standing in for anything built by code other than this branch's own
/// `s`: a sibling render, a portal, a helper that calls `RenderScope::new`
/// directly) **chronologically after** the branch's wrapper — is appended as
/// a child of the branch's markup, exactly the `{captured}`-nested-in-markup
/// shape of #732. The watermark rule (`node.id >= scope's smallest minted
/// id`) says this node is "owned" by the branch purely because its id is
/// higher, even though the branch's own scope `s` never created it and holds
/// no relationship to it at all. If the watermark is unsound, hiding the
/// branch retires this independent node instead of merely detaching it.
#[test]
fn a_node_minted_by_unrelated_code_after_the_wrapper_is_not_owned_by_the_branch() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let doc_weak = Rc::downgrade(&doc) as std::rc::Weak<RefCell<dyn DomDocument>>;

    let visible = Signal::new(true);
    let captured_slot: Rc<RefCell<Option<NodeId>>> = Rc::new(RefCell::new(None));
    let slot_for_closure = captured_slot.clone();
    let doc_weak_for_closure = doc_weak.clone();

    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            // The branch's own wrapper -- this fixes `s`'s watermark.
            let wrap = s.create_element("div");

            // A node minted by something that is NOT `s` -- raw document
            // access, standing in for any code path that does not route
            // through `s` (a sibling scope, a portal, a cache that lazily
            // builds its content on first need via its own fresh
            // `RenderScope`). It is minted strictly AFTER `wrap`, so its id
            // is higher than `s`'s watermark -- but `s` never created it.
            let indep_doc = doc_weak_for_closure.upgrade().unwrap();
            let indep_id = indep_doc.borrow_mut().create_element("section");
            *slot_for_closure.borrow_mut() = Some(indep_id);

            // The app threads it into the branch's markup, exactly like
            // `{panel}` nested inside `div { .. }` in the #732 fixtures above
            // -- except this handle was never built by `s`.
            wrap.append_child(&NodeHandle::new(indep_id, doc_weak_for_closure.clone()));
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    assert_eq!(body_tags(&doc), ["div"], "precondition: shown");
    let indep_id = captured_slot.borrow().unwrap();
    assert_eq!(
        doc.borrow().tag_name(indep_id).as_deref(),
        Some("section"),
        "precondition: the independent node exists"
    );

    visible.set(false);

    assert_eq!(
        doc.borrow().tag_name(indep_id).as_deref(),
        Some("section"),
        "the watermark misclassified a node minted by UNRELATED code (not by \
         `s`, not by any scope nested inside `s`'s render) as owned, purely \
         because its id happened to be minted after `s`'s watermark -- it \
         was retired with the wrapper instead of merely detached"
    );
}

/// Same shape, but the independent node is minted BEFORE the branch's first
/// render even starts, and the branch is toggled through several renders so
/// its watermark keeps climbing -- a sanity check that ordinary monotonic
/// captures (the shape every other #732 fixture above already covers) still
/// work, so the preceding fixture is attacking the watermark specifically via
/// *post*-watermark minting, not via some other mistake in the harness.
#[test]
fn a_node_minted_before_the_branch_first_shows_is_still_captured_after_many_toggles() {
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
            let wrap = s.create_element("div");
            wrap.append_child(&captured);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    for _ in 0..5 {
        visible.set(false);
        visible.set(true);
    }
    visible.set(false);
    assert_eq!(
        doc.borrow().tag_name(panel_id).as_deref(),
        Some("section"),
        "sanity: an ordinarily-captured (pre-existing) handle still survives \
         many toggles"
    );
}

/// `collect_captured_descendants`/`discard_owned_preserving_captured` called
/// directly with a scope whose `created` is **empty** (it minted nothing of
/// its own) -- the `watermark()` is `None`, and `watermark_or_newer` answers
/// `false` for every id, unconditionally. Every real call site guards this by
/// checking `scope.created(root_id)` before calling (which can only be true
/// if the scope minted *something*, guaranteeing a watermark) -- this probes
/// whether the two functions are safe to call WITHOUT that guard, since
/// nothing in their own signature enforces it and both are `pub(crate)`
/// (reachable from anywhere else in this crate, now or in a future call
/// site).
#[test]
fn collect_captured_descendants_with_an_empty_watermark_treats_every_child_as_captured() {
    use crate::dom::collect_captured_descendants;

    let doc = doc();
    let empty_scope = scope(&doc); // minted nothing: created == [], watermark == None
    let body = body_handle(&doc);

    // Build a root with real content using a DIFFERENT scope that mints
    // plenty, so the root's children are unambiguously "owned by *someone*"
    // -- just not by `empty_scope`.
    let mut builder = scope(&doc);
    let root = builder.create_element("div");
    let child_a = builder.create_element("p");
    let child_b = builder.create_element("span");
    root.append_child(&child_a);
    root.append_child(&child_b);
    body.append_child(&root);

    let mut out = Vec::new();
    collect_captured_descendants(&root, &empty_scope, &mut out);

    // With no watermark, `watermark_or_newer` is `false` for every id, so
    // EVERY direct child is collected as "captured" and the walk never
    // recurses into either -- including into grandchildren that might
    // themselves be genuinely, unambiguously owned by `empty_scope` (none
    // here, but the point is the walk stops at the first level regardless).
    assert_eq!(
        out.len(),
        2,
        "an empty-watermark scope must not silently treat `root`'s whole \
         subtree as unowned-and-therefore-safe-to-walk-through; every real \
         call site avoids this by checking `scope.created(root_id)` first, \
         which this test deliberately skips to probe the function's own \
         contract"
    );
}

// ── review2-1360: scope-ancestry tables must not leak either ───────────────

/// The scope-ancestry tables this round introduced (`MINTED_BY`,
/// `SCOPE_PARENTS`) must not grow without bound across many toggles — the
/// coordinator's own cost requirement for real ownership tracking. Every
/// toggle mints a fresh wrapper, a fresh `for` marker and three fresh rows
/// (and their own per-row scopes); if discarding the old set did not purge
/// their `MINTED_BY` entries and dispose their scopes' `SCOPE_PARENTS`
/// entries, both tables would grow by a fixed amount every toggle, forever.
#[test]
fn the_scope_ancestry_tables_do_not_grow_with_a_nested_for_inside_a_branch() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            for_each_dom_typed(
                s,
                &wrap,
                || vec![1u32, 2u32, 3u32],
                |n: &u32| n.to_string(),
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("li");
                    let text = rs.create_text(&n.to_string());
                    row.append_child(&text);
                    row
                },
            );
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    // Baseline AFTER the first show/hide pair, same reasoning as `growth`
    // above: the very first show mints content that was not there before,
    // which is not growth.
    visible.set(true);
    visible.set(false);
    let minted_by_baseline = crate::dom::__minted_by_len();
    let scope_parents_baseline = crate::dom::__scope_parents_len();

    for i in 2..200 {
        visible.set(i % 2 == 0);
    }

    let minted_by_delta = crate::dom::__minted_by_len() as isize - minted_by_baseline as isize;
    let scope_parents_delta =
        crate::dom::__scope_parents_len() as isize - scope_parents_baseline as isize;

    assert_eq!(
        minted_by_delta, 0,
        "#732 round 3: MINTED_BY must not grow across toggles of a nested \
         `for` inside a branch — baseline {minted_by_baseline}, delta {minted_by_delta} \
         over 198 toggles"
    );
    assert_eq!(
        scope_parents_delta, 0,
        "#732 round 3: SCOPE_PARENTS must not grow across toggles of a \
         nested `for` inside a branch — baseline {scope_parents_baseline}, \
         delta {scope_parents_delta} over 198 toggles"
    );
}

/// Same measurement, for the simple captured-handle shape (no nested
/// reactive helper) — the scope-ancestry tables must not grow even when
/// nothing nested is involved.
#[test]
fn the_scope_ancestry_tables_do_not_grow_with_a_plain_captured_handle() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let panel = sc.create_element("section");
    let visible = Signal::new(true);
    let captured = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            wrap.append_child(&captured);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    // Three toggles, ending HIDDEN — matching the parity the loop below ends
    // on (its last iteration, i=199, is odd) — so the baseline and the final
    // measurement are taken in the same state and the only thing a
    // non-zero delta can mean is growth, not "the branch happens to be
    // showing at one end and not the other."
    visible.set(false);
    visible.set(true);
    visible.set(false);
    let minted_by_baseline = crate::dom::__minted_by_len();
    let scope_parents_baseline = crate::dom::__scope_parents_len();

    for i in 2..200 {
        visible.set(i % 2 == 0);
    }

    let minted_by_delta = crate::dom::__minted_by_len() as isize - minted_by_baseline as isize;
    let scope_parents_delta =
        crate::dom::__scope_parents_len() as isize - scope_parents_baseline as isize;

    assert_eq!(
        minted_by_delta, 0,
        "#732 round 3: MINTED_BY must not grow across toggles of a plain \
         captured handle — baseline {minted_by_baseline}, delta {minted_by_delta}"
    );
    assert_eq!(
        scope_parents_delta, 0,
        "#732 round 3: SCOPE_PARENTS must not grow across toggles of a plain \
         captured handle — baseline {scope_parents_baseline}, delta {scope_parents_delta}"
    );
}
