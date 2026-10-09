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
/// back. A view that builds **lazily through the row's own scope** and caches
/// afterwards owns its row by this rule and loses it on the first removal
/// (issue #733); a view that has to build lazily builds through
/// [`RenderScope::cache_scope`] instead, which `lazy_memo_733` below pins.
/// `rinch-dom`'s
/// `branch_helper_transition_tests::a_for_row_reinserted_under_the_same_key_can_still_transition`
/// models the losing shape and cannot see the loss.
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

/// The discard walk (`sweep_for_discard`) costs in proportion to the subtree
/// being discarded, not to the whole document. Measured via
/// [`MockDomDocument::__get_children_calls`]: an unrelated sibling subtree,
/// never touched by the branch, must not move the call count at all when it
/// grows from 10 nodes to 2000.
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

// ── nested helpers inside a branch are the branch's, not captured ─────────

/// A `for` nested in a branch builds its rows through scopes of its own, not
/// the branch's: `created` answers `false` for them. They are still the
/// branch's render's (their scopes name the branch's scope as parent), so a
/// hide discards them rather than detaching them as captured.
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

// ── match_dom twin, release_scratch_container ────────────────────────────────

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
/// still be fully discarded (retired). Discarding the leftover before
/// detaching `panel`, or with no capture walk at all, retires `panel` too.
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

// ── id order is not ownership ───────────────────────────────────────────────

/// A node minted outside any scope (raw document access, standing in for any
/// code other than the branch's render) *after* the branch's wrapper, and put
/// inside it, is not the branch's: a hide detaches it. Ownership is ancestry,
/// not id order.
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
            // The branch's own wrapper.
            let wrap = s.create_element("div");

            // A node minted by something that is NOT `s` -- raw document
            // access, standing in for any code path that does not route
            // through `s` (a sibling scope, a portal, a cache that lazily
            // builds its content on first need via its own fresh
            // `RenderScope`). It is minted strictly AFTER `wrap`, so its id
            // is higher -- but `s` never created it.
            let indep_doc = doc_weak_for_closure.upgrade().unwrap();
            let body_id = indep_doc.borrow().body();
            let mut sibling = RenderScope::new(indep_doc, body_id);
            let indep_id = sibling.create_element("section").node_id();
            std::mem::forget(sibling);
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
        "a node minted by unrelated code (not by `s`, not by any scope \
         descended from it) was read as the branch's because its id is \
         newer, and retired with the wrapper instead of detached"
    );
}

/// Same shape, the node minted before the branch first renders, over several
/// renders: an ordinary capture, as a control for the fixture above.
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

/// The discard walk asked for an owner that minted nothing under `root`:
/// every direct child is someone else's, so each is captured and none is
/// entered. Every real call site first checks `created(root)`; this pins the
/// walk's own contract without that guard.
#[test]
fn a_sweep_for_an_owner_that_built_nothing_captures_every_child() {
    use crate::dom::sweep_for_discard;

    let doc = doc();
    let empty_scope = scope(&doc);
    let body = body_handle(&doc);

    let mut builder = scope(&doc);
    let root = builder.create_element("div");
    let child_a = builder.create_element("p");
    let child_b = builder.create_element("span");
    child_a.append_child(&builder.create_element("i"));
    root.append_child(&child_a);
    root.append_child(&child_b);
    body.append_child(&root);

    let mut out = Vec::new();
    sweep_for_discard(&root, Some(empty_scope.id()), &mut out);
    let mut ids: Vec<_> = out.iter().map(NodeHandle::node_id).collect();
    ids.sort_by_key(|id| id.0);
    assert_eq!(ids, [child_a.node_id(), child_b.node_id()]);
}

// ── the ancestry tables are bounded ─────────────────────────────────────────

/// The ancestry tables (minting records, scope entries) do not grow across
/// toggles of a branch holding a `for`: each hide drops the records of what it
/// discards, and the scope entries go with their scopes.
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
        "#732: the minting records must not grow across toggles of a nested \
         `for` inside a branch — baseline {minted_by_baseline}, delta {minted_by_delta} \
         over 198 toggles"
    );
    assert_eq!(
        scope_parents_delta, 0,
        "#732: the scope entries must not grow across toggles of a \
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
        "#732: the minting records must not grow across toggles of a plain \
         captured handle — baseline {minted_by_baseline}, delta {minted_by_delta}"
    );
    assert_eq!(
        scope_parents_delta, 0,
        "#732: the scope entries must not grow across toggles of a plain \
         captured handle — baseline {scope_parents_baseline}, delta {scope_parents_delta}"
    );
}

// ── content built later, by an effect, is still the branch's ──────────────

/// A row a `for` inserts later, from its reconcile effect (the list grows while
/// the branch is open), is built with nothing of the branch's render on the
/// stack. Its scope still names the branch's scope as parent, so the hide
/// discards it.
#[test]
fn a_row_added_to_a_growing_list_after_the_branch_first_shows_leaks_on_hide() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    let items = Signal::new(vec![1u32]);

    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            for_each_dom_typed(
                s,
                &wrap,
                move || items.get(),
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

    // Initial show: one row, minted during the branch's own render.
    visible.set(true);
    // Grow the list WHILE the branch stays visible: this is the for-loop's
    // reconcile Effect's `Insert` path, firing from the reactive flush, not
    // from inside show_dom's push_owner call.
    items.set(vec![1, 2, 3]);

    // End on a known, stable state (shown) before measuring, same as the
    // existing #732 growth fixtures, to avoid measuring across a parity
    // artifact rather than real growth.
    visible.set(false);
    visible.set(true);
    let baseline = doc.borrow().__node_count() as isize;

    for i in 0..200 {
        visible.set(i % 2 == 0);
        // Keep the list at 3 items on every re-show so the per-cycle cost is
        // identical across iterations (no further growth/shrink noise).
        if i % 2 == 0 {
            items.set(vec![1, 2, 3]);
        }
    }
    visible.set(true);

    let after = doc.borrow().__node_count() as isize;
    let delta = after - baseline;
    assert_eq!(
        delta, 0,
        "#732: a row the for-loop's reconcile effect added after the \
         branch's first render leaked {delta} nodes over 200 toggles: its \
         scope does not chain to the branch's, so it was read as captured \
         and only detached"
    );
}

/// The single hide right after one insert, measured on its own: a growth
/// fixture over many toggles bakes a leak on the first hide into its baseline
/// (every later show re-renders the list at its new size).
#[test]
fn a_single_hide_after_a_reconcile_insert_does_not_strand_the_inserted_rows() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let visible = Signal::new(false);
    let items = Signal::new(vec![1u32]);

    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            for_each_dom_typed(
                s,
                &wrap,
                move || items.get(),
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

    let before_anything = doc.borrow().__node_count() as isize;

    // Initial show: ONE row, minted synchronously inside the branch's
    // push_owner window.
    visible.set(true);

    // Grow the list while visible: for_loop's reconcile Effect inserts TWO
    // more rows (4 nodes: 2 <li> + 2 text) via its own `RenderScope::new`
    // call, made from inside the Effect's later run -- not nested inside
    // show_dom's push_owner call.
    items.set(vec![1, 2, 3]);

    // Hide ONCE. If every row the for-loop built is correctly owned by the
    // branch's scope, this discards everything the branch holds (wrap, all
    // three rows, their text, the for-loop's own marker) and node count
    // returns to the pre-show baseline. If the two reconcile-inserted rows
    // were misclassified as captured, they are only detached -- still
    // present in the mock's table, stranded with no parent.
    visible.set(false);

    let after_hide = doc.borrow().__node_count() as isize;
    assert_eq!(
        after_hide,
        before_anything,
        "#732: hiding the branch once, right after the reconcile effect \
         inserted two rows, left {} extra node(s): the inserted rows' scope \
         does not chain to the branch's, so they were only detached",
        after_hide - before_anything
    );
}

/// A document whose scopes are dropped without anything being discarded (an
/// embed context dropped, a window closed) leaves no minting records: the
/// tables belong to the document's scopes and go with the last one.
#[test]
fn a_document_dropped_without_discarding_its_tree_leaks_minted_by_forever() {
    let minted_by_baseline = crate::dom::__minted_by_len();

    {
        let doc = doc();
        let mut sc = scope(&doc);
        // Mint 50 nodes through the scope -- a realistic small page.
        let parent = body_handle(&doc);
        for i in 0..50 {
            let el = sc.create_element("div");
            let text = sc.create_text(&i.to_string());
            el.append_child(&text);
            parent.append_child(&el);
        }
        // `doc` and `sc` are dropped here at the end of the block -- NOT
        // via `NodeHandle::discard()` on anything, which is exactly what
        // happens when an embed `RinchContext` (or a desktop window) is torn
        // down directly rather than walked node-by-node.
    }

    let minted_by_after = crate::dom::__minted_by_len();
    let leaked = minted_by_after as isize - minted_by_baseline as isize;
    assert_eq!(
        leaked, 0,
        "#732: dropping a document's scopes without discarding anything \
         left {leaked} minting records behind"
    );
}

/// A nested branch flipped later, from its own effect, builds content whose
/// scope names the scope the nested `show_dom` was called from: the outer
/// hide reclaims it.
#[test]
fn a_nested_branch_flipped_later_is_still_discarded_when_the_outer_branch_hides() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let outer_visible = Signal::new(false);
    let inner_visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || outer_visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            show_dom(
                s,
                &wrap,
                move || inner_visible.get(),
                |inner_s: &mut RenderScope| inner_s.create_element("p"),
                None::<fn(&mut RenderScope) -> NodeHandle>,
            );
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    // Show the outer branch (builds `wrap` + the inner `show_dom`'s own
    // marker, inner hidden). Then flip the INNER branch on, LATER, from its
    // own Effect -- not from the outer branch's initial render, which
    // already returned.
    outer_visible.set(true);
    inner_visible.set(true);

    let baseline = doc.borrow().__node_count() as isize;

    // Hide the outer branch ONCE. If the inner `<p>` (minted by the inner
    // show_dom's later-firing Effect) is correctly linked as a descendant of
    // the outer branch's scope, discarding `wrap` takes it with it.
    // Misclassified as captured, it is only detached and stranded.
    outer_visible.set(false);

    let after = doc.borrow().__node_count() as isize;
    assert_eq!(
        after,
        baseline - 3, // wrap + inner show_dom's marker + the inner <p>
        "#732: a nested branch flipped later must be discarded with the \
         outer branch -- {} node(s) were stranded instead",
        after - (baseline - 3)
    );
}

/// The scope-ancestry tables must not leak either, across many toggles of
/// this exact "inner branch flips later" shape.
#[test]
fn the_scope_ancestry_tables_do_not_grow_with_a_branch_flipped_later() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);

    let outer_visible = Signal::new(false);
    let inner_visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || outer_visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            show_dom(
                s,
                &wrap,
                move || inner_visible.get(),
                |inner_s: &mut RenderScope| inner_s.create_element("p"),
                None::<fn(&mut RenderScope) -> NodeHandle>,
            );
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );

    // End on a stable, parity-matched baseline: outer hidden, inner hidden.
    outer_visible.set(true);
    inner_visible.set(true);
    outer_visible.set(false);
    let minted_by_baseline = crate::dom::__minted_by_len();
    let scope_parents_baseline = crate::dom::__scope_parents_len();

    for _ in 0..100 {
        outer_visible.set(true);
        inner_visible.set(true);
        outer_visible.set(false);
        inner_visible.set(false);
    }

    let minted_by_delta = crate::dom::__minted_by_len() as isize - minted_by_baseline as isize;
    let scope_parents_delta =
        crate::dom::__scope_parents_len() as isize - scope_parents_baseline as isize;
    assert_eq!(
        minted_by_delta, 0,
        "#732: the minting records must not grow across a branch-flipped-later \
         cycle -- baseline {minted_by_baseline}, delta {minted_by_delta}"
    );
    assert_eq!(
        scope_parents_delta, 0,
        "#732: the scope entries must not grow across a \
         branch-flipped-later cycle -- baseline {scope_parents_baseline}, \
         delta {scope_parents_delta}"
    );
}

// ── A branch reclaims what nested helpers and late patches built (issue #732) ──

fn node_count(doc: &Rc<RefCell<MockDomDocument>>) -> isize {
    doc.borrow().__node_count() as isize
}

/// A `virtual_list` inside a branch: its rows are built by scopes of its own,
/// which must name the scope `virtual_list` was called from as their parent,
/// or the branch's hide reads them as captured and only detaches them.
#[test]
fn a_virtual_list_inside_a_branch_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let list = crate::virtual_list(
                s,
                20.0,
                || (0u32..50).collect::<Vec<_>>(),
                |n: &u32| *n,
                2,
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("div");
                    row.append_child(&rs.create_text(&n.to_string()));
                    row
                },
            );
            wrap.append_child(&list);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let base = node_count(&doc);
    for _ in 0..100 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(
        node_count(&doc) - base,
        0,
        "virtual_list rows leak per toggle"
    );
}

/// The `late_children` shape (List / Stepper / RadioGroup): an
/// `on_child_inserted` observer patches a late-arriving row through a
/// throwaway scope naming the container's scope as parent. The scope is gone
/// by the hide; its nodes must still be the branch's.
#[test]
fn a_late_child_patch_inside_a_branch_does_not_grow_the_document() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let visible = Signal::new(false);
    let items = Signal::new(vec![1u32]);
    let dw = Rc::downgrade(&doc);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let ul = s.create_element("ul");
            let dw = dw.clone();
            let ul_id = ul.node_id();
            // What `late_children` does: the patch scope names the scope the
            // container rendered in.
            let owner = s.id();
            crate::dom::on_child_inserted(&ul, move |inserted| {
                if inserted.tag_name().as_deref() != Some("li") {
                    return;
                }
                let Some(d) = dw.upgrade() else { return };
                let mut patch =
                    RenderScope::with_parent(d as Rc<RefCell<dyn DomDocument>>, ul_id, Some(owner));
                let icon = patch.create_element("i");
                inserted.append_child(&icon);
            });
            for_each_dom_typed(
                s,
                &ul,
                move || items.get(),
                |n: &u32| n.to_string(),
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("li");
                    row.append_child(&rs.create_text(&n.to_string()));
                    row
                },
            );
            ul
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    items.set(vec![1, 2]);
    visible.set(false);
    items.set(vec![1]);
    let base = node_count(&doc);
    for i in 0..100u32 {
        visible.set(true);
        items.set(vec![1, 2 + i]);
        visible.set(false);
        items.set(vec![1]);
    }
    assert_eq!(
        node_count(&doc) - base,
        0,
        "late-child patch nodes leak per toggle"
    );
}

/// Depth 3, every helper re-running LATER: outer show → match (arm swapped
/// later) → inner show (flipped later) → for (rows inserted later) +
/// reactive_component_dom (re-rendered later). One outer hide must return the
/// document to the baseline, and the tables must not grow over many cycles.
#[test]
fn depth_three_later_runs_of_every_helper_are_all_discarded() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let outer = Signal::new(false);
    let arm = Signal::new(0usize);
    let inner = Signal::new(false);
    let items = Signal::new(vec![1u32]);
    let rev = Signal::new(0u32);
    show_dom(
        &mut sc,
        &body,
        move || outer.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("section");
            match_dom(
                s,
                &wrap,
                move || arm.get(),
                vec![
                    Box::new(|a: &mut RenderScope| a.create_element("em")),
                    Box::new(move |a: &mut RenderScope| {
                        let d = a.create_element("div");
                        show_dom(
                            a,
                            &d,
                            move || inner.get(),
                            move |b: &mut RenderScope| {
                                let ul = b.create_element("ul");
                                for_each_dom_typed(
                                    b,
                                    &ul,
                                    move || items.get(),
                                    |n: &u32| n.to_string(),
                                    move |n: u32, rs: &mut RenderScope| {
                                        let li = rs.create_element("li");
                                        crate::dom::reactive_component_dom(
                                            rs,
                                            &li,
                                            move |c: &mut RenderScope| {
                                                let _ = rev.get();
                                                let x = c.create_element("b");
                                                x.append_child(&c.create_text(&n.to_string()));
                                                x
                                            },
                                        );
                                        li
                                    },
                                );
                                ul
                            },
                            None::<fn(&mut RenderScope) -> NodeHandle>,
                        );
                        d
                    }),
                ],
            );
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let base = node_count(&doc);
    let mb0 = crate::dom::__minted_by_len();
    let sp0 = crate::dom::__scope_parents_len();
    for i in 0..1000u32 {
        outer.set(true);
        arm.set(1); // later arm swap
        inner.set(true); // later flip
        items.set(vec![1, 2 + i, 3 + i]); // later inserts
        rev.set(i); // later re-render of every row's component
        outer.set(false); // one hide
        assert_eq!(node_count(&doc), base, "iteration {i}: stranded nodes");
        arm.set(0);
        inner.set(false);
        items.set(vec![1]);
    }
    assert_eq!(crate::dom::__minted_by_len(), mb0, "minting table grew");
    assert_eq!(crate::dom::__scope_parents_len(), sp0, "scope entries grew");
}

/// A captured handle that itself CONTAINS live helpers (built by the root
/// scope), nested inside branch markup at depth 2. Hide detaches it intact;
/// its own for loop keeps working while hidden and after re-show; the outer
/// branch's own markup is still reclaimed.
#[test]
fn a_captured_handle_containing_live_helpers_survives_and_stays_reactive() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let items = Signal::new(vec![1u32]);
    let panel = sc.create_element("aside");
    for_each_dom_typed(
        &mut sc,
        &panel,
        move || items.get(),
        |n: &u32| n.to_string(),
        |n: u32, rs: &mut RenderScope| {
            let p = rs.create_element("p");
            p.append_child(&rs.create_text(&n.to_string()));
            p
        },
    );
    let visible = Signal::new(false);
    let pc = panel.clone();
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let a = s.create_element("div");
            let b = s.create_element("div");
            a.append_child(&b);
            b.append_child(&pc);
            a
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let rows = |doc: &Rc<RefCell<MockDomDocument>>, panel: &NodeHandle| {
        doc.borrow()
            .get_children(panel.node_id())
            .into_iter()
            .filter(|c| doc.borrow().tag_name(*c).as_deref() == Some("p"))
            .count()
    };
    visible.set(true);
    items.set(vec![1, 2, 3]);
    visible.set(false);
    assert_eq!(rows(&doc, &panel), 3, "panel lost rows on hide");
    items.set(vec![1, 2, 3, 4]); // while hidden
    assert_eq!(rows(&doc, &panel), 4);
    let base = node_count(&doc);
    for _ in 0..200 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(node_count(&doc) - base, 0);
    visible.set(true);
    items.set(vec![9]);
    assert_eq!(rows(&doc, &panel), 1);
    assert!(panel.parent_node().is_some(), "panel re-shown");
}

/// A handle created by a nested LATER run (a row inserted later builds a
/// panel through its own scope) and captured by a closure that a nested
/// LATER-flipped branch returns inside its own markup. Inner hide keeps it
/// (it is the row's, not the inner branch's); outer hide reclaims everything.
#[test]
fn a_handle_captured_inside_a_nested_later_run() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let outer = Signal::new(false);
    let inner = Signal::new(false);
    let items = Signal::new(Vec::<u32>::new());
    let kept: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let kept2 = kept.clone();
    show_dom(
        &mut sc,
        &body,
        move || outer.get(),
        move |s: &mut RenderScope| {
            let ul = s.create_element("ul");
            let kept = kept2.clone();
            for_each_dom_typed(
                s,
                &ul,
                move || items.get(),
                |n: &u32| n.to_string(),
                move |n: u32, rs: &mut RenderScope| {
                    let li = rs.create_element("li");
                    let panel = rs.create_element("aside");
                    panel.append_child(&rs.create_text(&n.to_string()));
                    kept.borrow_mut().push(panel.clone());
                    show_dom(
                        rs,
                        &li,
                        move || inner.get(),
                        move |b: &mut RenderScope| {
                            let w = b.create_element("span");
                            w.append_child(&panel);
                            w
                        },
                        None::<fn(&mut RenderScope) -> NodeHandle>,
                    );
                    li
                },
            );
            ul
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let base = node_count(&doc);
    for i in 0..100u32 {
        kept.borrow_mut().clear();
        outer.set(true);
        items.set(vec![i, i + 1000]); // rows minted by a LATER reconcile run
        inner.set(true); // nested branches flipped LATER
        inner.set(false); // inner hide: panel must be detached, not retired
        for p in kept.borrow().iter() {
            assert!(
                doc.borrow().tag_name(p.node_id()).is_some(),
                "panel retired by inner hide"
            );
        }
        inner.set(true);
        outer.set(false);
        inner.set(false);
        items.set(vec![]);
        assert_eq!(node_count(&doc), base, "iteration {i}");
    }
}

/// Two documents on one thread, interleaved, colliding NodeIds.
#[test]
fn two_documents_on_one_thread_do_not_cross_classify() {
    let d1 = doc();
    let d2 = doc();
    let mut s1 = scope(&d1);
    let mut s2 = scope(&d2);
    let b1 = body_handle(&d1);
    let b2 = body_handle(&d2);
    let v1 = Signal::new(false);
    let v2 = Signal::new(false);
    // d2's captured panel; same NodeId as some d1 branch node very likely.
    let p2 = s2.create_element("aside");
    let p2c = p2.clone();
    show_dom(
        &mut s1,
        &b1,
        move || v1.get(),
        |s: &mut RenderScope| {
            let d = s.create_element("div");
            d.append_child(&s.create_element("i"));
            d
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    show_dom(
        &mut s2,
        &b2,
        move || v2.get(),
        move |s: &mut RenderScope| {
            let d = s.create_element("div");
            d.append_child(&p2c);
            d
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let (c1, c2) = (node_count(&d1), node_count(&d2));
    for _ in 0..200 {
        v1.set(true);
        v2.set(true);
        v1.set(false);
        v2.set(false);
    }
    assert_eq!((node_count(&d1), node_count(&d2)), (c1, c2));
    v2.set(true);
    assert!(p2.parent_node().is_some());
}

/// Dropping 1000 documents (each with a live branch) leaves no live table, but
/// how many per-document slots stay behind?
#[test]
fn dropping_many_documents_leaves_no_table_slots_behind() {
    let slots0 = crate::dom::__doc_table_slots();
    let len0 = crate::dom::__minted_by_len();
    for _ in 0..1000 {
        let d = doc();
        let mut s = scope(&d);
        let b = body_handle(&d);
        let v = Signal::new(true);
        show_dom(
            &mut s,
            &b,
            move || v.get(),
            |s: &mut RenderScope| {
                let x = s.create_element("div");
                x.append_child(&s.create_text("t"));
                x
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        drop(s);
        drop(d);
    }
    assert_eq!(crate::dom::__minted_by_len(), len0, "live entries leaked");
    let slots = crate::dom::__doc_table_slots() - slots0;
    assert_eq!(
        slots, 0,
        "{slots} dead ancestry-table slots left behind after 1000 documents"
    );
}

/// A row whose DATA changed (same key) is re-rendered by the reconcile
/// effect's `Changed` arm — a third later-run site. Outer hide must reclaim it.
#[test]
fn a_row_rerendered_for_changed_data_is_discarded_with_the_branch() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let visible = Signal::new(false);
    let items = Signal::new(vec![(1u32, 0u32)]);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let ul = s.create_element("ul");
            for_each_dom_typed(
                s,
                &ul,
                move || items.get(),
                |t: &(u32, u32)| t.0.to_string(),
                |t: (u32, u32), rs: &mut RenderScope| {
                    let li = rs.create_element("li");
                    li.append_child(&rs.create_text(&t.1.to_string()));
                    li
                },
            );
            ul
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let base = node_count(&doc);
    for i in 0..50u32 {
        visible.set(true);
        items.set(vec![(1, i + 1)]); // same key, new data -> Changed arm
        visible.set(false);
        items.set(vec![(1, 0)]);
        assert_eq!(node_count(&doc), base, "iteration {i}");
    }
}

/// The spacers `virtual_list` pools for a slot a duplicate key left empty are
/// minted by a scope that is dropped as soon as the spacer exists. The node
/// outlives its minting scope, and still belongs to the branch: its ancestry
/// must outlive the scope too, or the hide leaks every spacer.
#[test]
fn virtual_list_gap_spacers_inside_a_branch_are_reclaimed() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let visible = Signal::new(false);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let list = crate::virtual_list(
                s,
                20.0,
                || (0u32..50).collect::<Vec<_>>(),
                // Every other key repeats, so the window carries spacers.
                |n: &u32| *n / 2,
                2,
                |n: u32, rs: &mut RenderScope| {
                    let row = rs.create_element("div");
                    row.append_child(&rs.create_text(&n.to_string()));
                    row
                },
            );
            wrap.append_child(&list);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    fn gaps(n: &NodeHandle) -> usize {
        let own = usize::from(n.get_attribute("class").as_deref() == Some("rinch-vlist__gap"));
        own + n.children().iter().map(gaps).sum::<usize>()
    }
    assert!(gaps(&body) > 0, "precondition: the window carries spacers");
    visible.set(false);
    let base = node_count(&doc);
    for _ in 0..50 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(
        node_count(&doc) - base,
        0,
        "virtual_list spacers leak per toggle"
    );
}

/// `set_inner_html` replaces children the backend then forgets; their entries
/// in the minting table must go with them, or the table grows by every
/// scope-built child ever replaced this way (and on a backend that reuses ids,
/// a recycled id would inherit a stale owner).
#[test]
fn set_inner_html_over_scope_built_children_purges_their_minting_entries() {
    let doc = doc();
    let mut sc = scope(&doc);
    let body = body_handle(&doc);
    let host = sc.create_element("div");
    body.append_child(&host);
    let before = crate::dom::__minted_by_len();
    for i in 0..10 {
        let c = sc.create_element("p");
        c.append_child(&sc.create_text(&i.to_string()));
        host.append_child(&c);
    }
    assert_eq!(crate::dom::__minted_by_len(), before + 20, "precondition");
    host.set_inner_html("");
    assert_eq!(
        crate::dom::__minted_by_len(),
        before,
        "set_inner_html left the replaced children's minting entries behind"
    );
}

// ── #733: a `for` view that builds lazily and memoises ──────────────────────

/// Issue #733. A view is only ever handed its row's scope, so a cache that is
/// filled *from inside the view* cannot build through anything that outlives
/// the row — unless it asks for one. [`RenderScope::cache_scope`] is that: a
/// parentless scope on the same document, which the cache keeps beside the
/// node it built.
mod lazy_memo_733 {
    use super::*;
    use crate::dom::{__retired_view_returns, reactive_component_dom};
    use std::cell::Cell;
    use std::collections::HashMap;

    type Cache = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;

    struct Probe {
        /// Runs of the cached row's effect.
        runs: Rc<Cell<u32>>,
        /// A signal created *inside* the cached build, read by that effect.
        inner: Rc<Cell<Option<Signal<u32>>>>,
    }

    /// A list whose view builds each row once, through a cache scope, inside
    /// a wrapper the row's own scope builds when `wrapped`.
    fn lazy_list(
        sc: &mut RenderScope,
        body: &NodeHandle,
        rows: Signal<Vec<u32>>,
        cache: &Cache,
        wrapped: bool,
    ) -> Probe {
        let runs = Rc::new(Cell::new(0u32));
        let inner: Rc<Cell<Option<Signal<u32>>>> = Rc::new(Cell::new(None));
        let (c, r, i) = (cache.clone(), runs.clone(), inner.clone());
        for_each_dom_typed(
            sc,
            body,
            move || rows.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                let cached = c.borrow().get(&n).map(|(row, _)| row.clone());
                let row = cached.unwrap_or_else(|| {
                    let mut keep = s.cache_scope();
                    let (r, i) = (r.clone(), i.clone());
                    let row = keep.build(|k| {
                        let row = k.create_element("article");
                        let text = k.create_text("ROW");
                        row.append_child(&text);
                        let local = Signal::new(0u32);
                        i.set(Some(local));
                        let target = row.clone();
                        k.create_effect(move || {
                            r.set(r.get() + 1);
                            target.set_attribute("data-n", &local.get().to_string());
                        });
                        row
                    });
                    c.borrow_mut().insert(n, (row.clone(), keep));
                    row
                });
                if wrapped {
                    let wrap = s.create_element("section");
                    wrap.append_child(&row);
                    wrap
                } else {
                    row
                }
            },
        );
        Probe { runs, inner }
    }

    /// The row comes back with its subtree, and everything it built is still
    /// live while it is out of the list: its effect runs, and a signal created
    /// during the build can still be written and read.
    ///
    /// The signal half is what [`RenderScope::build`] is for. The view runs
    /// with the *row* as the ambient owner, so a `Signal::new` made there
    /// without it belongs to the row and is freed when the row leaves.
    #[test]
    fn a_row_built_through_a_cache_scope_survives_leaving_the_list() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let cache: Cache = Rc::default();
        let rows = Signal::new(vec![7u32]);
        let probe = lazy_list(&mut sc, &body, rows, &cache, false);
        let row = cache.borrow()[&7].0.node_id();
        assert_eq!(body_tags(&doc), ["article"], "precondition: mounted");
        assert_eq!(probe.runs.get(), 1);

        rows.set(vec![]);
        assert_eq!(body_tags(&doc), Vec::<String>::new(), "precondition: out");
        probe.inner.get().unwrap().set(3);
        assert_eq!(
            probe.runs.get(),
            2,
            "#733: the cached row's signal and effect outlive the row"
        );
        assert_eq!(
            doc.borrow().get_attribute(row, "data-n").as_deref(),
            Some("3"),
            "#733: and the effect still reaches its node while it is out"
        );

        rows.set(vec![7]);
        assert_eq!(body_tags(&doc), ["article"], "#733: the row comes back");
        assert_eq!(
            doc.borrow().get_children(row).len(),
            1,
            "#733: with its subtree"
        );
        probe.inner.get().unwrap().set(4);
        assert_eq!(
            doc.borrow().get_attribute(row, "data-n").as_deref(),
            Some("4")
        );
        assert_eq!(
            __retired_view_returns(),
            (0, 0),
            "nothing retired came back"
        );
    }

    /// A cached node *inside* markup the row builds: the wrapper is the row's
    /// and is discarded, the cached node is not the row's and is detached
    /// first (the #732 walk). This is what "parentless" buys — a cache scope
    /// that named the row's scope as its parent would be swept with the row.
    #[test]
    fn a_cached_node_inside_row_built_markup_survives_the_rows_discard() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let cache: Cache = Rc::default();
        let rows = Signal::new(vec![7u32]);
        let _probe = lazy_list(&mut sc, &body, rows, &cache, true);
        let row = cache.borrow()[&7].0.node_id();

        rows.set(vec![]);
        rows.set(vec![7]);
        assert_eq!(body_tags(&doc), ["section"]);
        let wrap = mounted_element(&doc, doc.borrow().body());
        assert_eq!(
            doc.borrow().get_children(wrap),
            [row],
            "#733: the cached node is under the fresh wrapper"
        );
        assert_eq!(
            doc.borrow().tag_name(row).as_deref(),
            Some("article"),
            "#733: and was not retired with the old one"
        );
    }

    /// The other direction: the cache must not turn the list into a leak. A
    /// key toggled 200 times holds one row, and the wrappers the row scope
    /// builds around it are still reclaimed.
    #[test]
    fn a_cache_scoped_list_does_not_grow_the_document() {
        for wrapped in [false, true] {
            let doc = doc();
            let mut sc = scope(&doc);
            let body = body_handle(&doc);
            let cache: Cache = Rc::default();
            let rows = Signal::new(vec![]);
            let _probe = lazy_list(&mut sc, &body, rows, &cache, wrapped);
            let delta = growth(
                &doc,
                |i| rows.set(if i % 2 == 0 { vec![] } else { vec![7] }),
                200,
            );
            assert_eq!(delta, 0, "#733 (wrapped: {wrapped}): leaked {delta} nodes");
        }
    }

    /// What the cache owes when it evicts: dispose the scope, discard the
    /// node. After that nothing of the row is left — no node, no effect.
    #[test]
    fn evicting_a_cache_entry_releases_its_nodes_and_effects() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let cache: Cache = Rc::default();
        let rows = Signal::new(vec![]);
        let probe = lazy_list(&mut sc, &body, rows, &cache, false);
        let empty = doc.borrow().__node_count();

        rows.set(vec![7]);
        rows.set(vec![]);
        assert_eq!(
            doc.borrow().__node_count(),
            empty + 2,
            "precondition: the cache holds the row and its text"
        );
        let local = probe.inner.get().unwrap();
        local.set(1);
        let before = probe.runs.get();

        let (row, keep) = cache.borrow_mut().remove(&7).unwrap();
        keep.dispose();
        row.discard();
        assert_eq!(doc.borrow().__node_count(), empty, "#733: nodes released");
        local.set(2);
        assert_eq!(probe.runs.get(), before, "#733: effects released");
    }

    /// Dropping the kept scope is a dispose (and leaves the node to the
    /// caller): a cache that keeps the node and drops the scope gets a row
    /// whose bindings are dead.
    #[test]
    fn dropping_the_cache_scope_stops_the_rows_effects() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let cache: Cache = Rc::default();
        let rows = Signal::new(vec![7u32]);
        let probe = lazy_list(&mut sc, &body, rows, &cache, false);
        let trigger = Signal::new(0u32);
        let (row, keep) = cache.borrow_mut().remove(&7).unwrap();
        let mut keep = keep;
        let runs = probe.runs.clone();
        keep.create_effect(move || {
            let _ = trigger.get();
            runs.set(runs.get() + 100);
        });
        let before = probe.runs.get();
        drop(keep);
        trigger.set(1);
        assert_eq!(probe.runs.get(), before);
        assert_eq!(
            doc.borrow().tag_name(row.node_id()).as_deref(),
            Some("article"),
            "the node is the caller's to discard"
        );
    }

    // ── the shape that is still lost, and now says so ───────────────────────

    /// The issue's own shape: built through the **row's** scope, then cached.
    /// The row owns it, the first removal retires it, and the key coming back
    /// shows nothing. That is unchanged — what changed is that the helper
    /// notices the retired node it was handed and warns, once per node.
    #[test]
    fn a_row_built_through_its_own_scope_is_still_lost_and_reported_once() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::default();
        let rows = Signal::new(vec![1u32]);
        let c = cache.clone();
        for_each_dom_typed(
            &mut sc,
            &body,
            move || rows.get(),
            |n: &u32| n.to_string(),
            move |n: u32, s: &mut RenderScope| {
                let hit = c.borrow().get(&n).cloned();
                hit.unwrap_or_else(|| {
                    let row = s.create_element("article");
                    c.borrow_mut().insert(n, row.clone());
                    row
                })
            },
        );
        assert_eq!(__retired_view_returns(), (0, 0));
        for _ in 0..3 {
            rows.set(vec![]);
            rows.set(vec![1]);
        }
        assert_eq!(body_tags(&doc), Vec::<String>::new(), "#733: still lost");
        assert_eq!(
            __retired_view_returns(),
            (3, 1),
            "#733: every return is seen, one warning per node"
        );
    }

    /// A node discarded before the test's helper is handed it — the same
    /// defect reached without a cache, so each call site can be driven alone.
    fn dead(sc: &mut RenderScope) -> NodeHandle {
        let node = sc.create_element("aside");
        node.discard();
        node
    }

    #[test]
    fn a_for_reports_a_retired_node_on_its_first_render_and_on_a_changed_row() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let gone = dead(&mut sc);
        let rows = Signal::new(vec![(1u32, 0u32)]);
        for_each_dom_typed(
            &mut sc,
            &body,
            move || rows.get(),
            |n: &(u32, u32)| n.0.to_string(),
            move |_n: (u32, u32), _s: &mut RenderScope| gone.clone(),
        );
        assert_eq!(__retired_view_returns().0, 1, "the initial render");
        rows.set(vec![(1, 1)]);
        assert_eq!(__retired_view_returns().0, 2, "a row whose data changed");
        rows.set(vec![(1, 1), (2, 0)]);
        assert_eq!(__retired_view_returns(), (3, 1), "an inserted row");
    }

    #[test]
    fn a_show_branch_reports_a_retired_node() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let gone = dead(&mut sc);
        let visible = Signal::new(true);
        show_dom(
            &mut sc,
            &body,
            move || visible.get(),
            move |_s: &mut RenderScope| gone.clone(),
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        assert_eq!(__retired_view_returns(), (1, 1));
    }

    #[test]
    fn a_match_arm_reports_a_retired_node() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let gone = dead(&mut sc);
        let arm: Box<dyn Fn(&mut RenderScope) -> NodeHandle> = Box::new(move |_s| gone.clone());
        match_dom(&mut sc, &body, || 0, vec![arm]);
        assert_eq!(__retired_view_returns(), (1, 1));
    }

    #[test]
    fn a_component_render_reports_a_retired_node() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let gone = dead(&mut sc);
        reactive_component_dom(&mut sc, &body, move |_s: &mut RenderScope| gone.clone());
        assert_eq!(__retired_view_returns(), (1, 1));
    }

    #[test]
    fn a_virtual_list_row_reports_a_retired_node() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let gone = dead(&mut sc);
        let list = crate::virtual_list(
            &mut sc,
            20.0,
            || vec![0u32],
            |n: &u32| *n,
            2,
            move |_n: u32, _rs: &mut RenderScope| gone.clone(),
        );
        body.append_child(&list);
        assert!(__retired_view_returns().0 >= 1);
        assert_eq!(__retired_view_returns().1, 1);
    }

    /// A live node — fresh, or handed back from outside — is never reported.
    #[test]
    fn a_live_node_is_not_reported() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let kept = sc.create_element("article");
        let visible = Signal::new(true);
        show_dom(
            &mut sc,
            &body,
            move || visible.get(),
            move |_s: &mut RenderScope| kept.clone(),
            Some(|s: &mut RenderScope| s.create_element("p")),
        );
        for _ in 0..4 {
            visible.update(|v| *v = !*v);
        }
        assert_eq!(__retired_view_returns(), (0, 0));
    }
}

// ── #1487: text written over an element's children ──────────────────────────

/// Issue #1487. `NodeHandle::set_text` on an element orphans every child, and
/// an orphan is under no root: the hide of the branch that built it walks the
/// branch's subtree and never meets it, so it stayed in the backend's node
/// table and in the minting table for the life of the document. The write
/// itself now decides, by the rule a hide uses: a child the written element's
/// render built is discarded, a child that render was handed is only detached.
mod text_over_children_1487 {
    use super::*;
    use crate::dom::__minted_by_len;

    /// `(nodes, minting records)` grown over `cycles` show/hide pairs, after a
    /// warm-up pair.
    fn cycle_growth(
        doc: &Rc<RefCell<MockDomDocument>>,
        visible: Signal<bool>,
        cycles: usize,
    ) -> (isize, isize) {
        visible.set(true);
        visible.set(false);
        let base = (node_count(doc), __minted_by_len() as isize);
        for _ in 0..cycles {
            visible.set(true);
            visible.set(false);
        }
        (node_count(doc) - base.0, __minted_by_len() as isize - base.1)
    }

    /// The issue's table, row two: `div > (span > text, span)` built by the
    /// branch, then written over.
    #[test]
    fn text_over_branch_built_children_does_not_grow_the_document() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let visible = Signal::new(false);
        let shown = Rc::new(RefCell::new(0usize));
        let seen = shown.clone();
        let probe = doc.clone();
        show_dom(
            &mut sc,
            &body,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let before = probe.borrow().__node_count();
                let wrap = s.create_element("div");
                let a = s.create_element("span");
                a.append_child(&s.create_text("a"));
                wrap.append_child(&a);
                wrap.append_child(&s.create_element("span"));
                // Positive control for the count below: the branch really
                // mints four nodes a show.
                *seen.borrow_mut() = probe.borrow().__node_count() - before;
                wrap.set_text("over");
                wrap
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        let grown = cycle_growth(&doc, visible, 200);
        assert_eq!(
            *shown.borrow(),
            4,
            "control: a show mints the wrapper and three nodes under it"
        );
        assert_eq!(
            grown,
            (0, 0),
            "#1487: the children a text write orphaned were built by the same \
             render as the element written to, and nothing can show them \
             again — (nodes, minting records) grown over 200 cycles"
        );
    }

    /// Outside any branch too: a long-lived element whose scope-built children
    /// are replaced by text, over and over.
    #[test]
    fn text_over_scope_built_children_leaves_nothing_behind() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let host = sc.create_element("div");
        body.append_child(&host);
        let base = (node_count(&doc), __minted_by_len());
        for i in 0..50 {
            let c = sc.create_element("p");
            c.append_child(&sc.create_text(&i.to_string()));
            host.append_child(&c);
            if i == 0 {
                assert_eq!(node_count(&doc), base.0 + 2, "control: two nodes minted");
            }
            host.set_text("cleared");
        }
        assert_eq!((node_count(&doc), __minted_by_len()), base);
    }

    /// The other direction: a handle the branch was **handed** is the
    /// caller's. Written over, it is detached with its subtree and comes back
    /// when it is appended again.
    #[test]
    fn text_over_a_captured_handle_only_detaches_it() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let panel = sc.create_element("article");
        let panel_text = sc.create_text("kept");
        panel.append_child(&panel_text);

        let visible = Signal::new(false);
        let fresh: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
        let (handed, built) = (panel.clone(), fresh.clone());
        show_dom(
            &mut sc,
            &body,
            move || visible.get(),
            move |s: &mut RenderScope| {
                let wrap = s.create_element("div");
                let own = s.create_element("span");
                wrap.append_child(&own);
                wrap.append_child(&handed);
                *built.borrow_mut() = Some(own);
                wrap.set_text("over");
                wrap
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        visible.set(true);

        let own = fresh.borrow().clone().expect("the branch rendered");
        {
            let d = doc.borrow();
            assert!(
                d.is_retired(own.node_id()),
                "control: the span the branch built beside it was discarded"
            );
            assert!(
                !d.is_retired(panel.node_id()) && !d.is_retired(panel_text.node_id()),
                "the captured panel and its text are still the caller's"
            );
            assert_eq!(d.parent_node(panel.node_id()), None, "detached by the write");
            assert_eq!(d.get_children(panel.node_id()), vec![panel_text.node_id()]);
        }
        body.append_child(&panel);
        assert!(
            doc.borrow()
                .get_children(doc.borrow().body())
                .contains(&panel.node_id()),
            "and it re-inserts"
        );
        panel.remove();

        let grown = cycle_growth(&doc, visible, 200);
        assert_eq!(grown, (0, 0), "with the panel kept, nothing else accumulates");
        assert!(!doc.borrow().is_retired(panel.node_id()));
    }

    /// A captured handle **nested** in markup the write discards comes out
    /// first, as it does when a branch hides (#732).
    #[test]
    fn a_captured_handle_nested_under_a_written_over_child_survives() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let panel = sc.create_element("article");

        let mut branch = RenderScope::with_parent(doc.clone(), body.node_id(), Some(sc.id()));
        let wrap = branch.create_element("div");
        let inner = branch.create_element("section");
        inner.append_child(&panel);
        wrap.append_child(&inner);
        body.append_child(&wrap);

        wrap.set_text("over");

        let d = doc.borrow();
        assert!(d.is_retired(inner.node_id()), "the branch built `inner`");
        assert!(
            !d.is_retired(panel.node_id()),
            "the panel was handed in: detached from the discarded wrapper, kept"
        );
        assert_eq!(d.parent_node(panel.node_id()), None);
    }

    /// The owner is the scope that built the element written to. A child built
    /// by a scope that is not that one, nor descended from it, was handed in —
    /// here by a parentless cache scope (#733) — and is kept; one built by a
    /// descendant scope (a row, a nested branch) goes.
    #[test]
    fn ownership_is_asked_of_the_written_elements_scope() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let wrap = sc.create_element("div");
        body.append_child(&wrap);

        let mut row = RenderScope::with_parent(doc.clone(), body.node_id(), Some(sc.id()));
        let from_row = row.create_element("li");
        let mut cache = sc.cache_scope();
        let cached = cache.create_element("aside");
        wrap.append_child(&from_row);
        wrap.append_child(&cached);

        wrap.set_text("over");

        let d = doc.borrow();
        assert!(d.is_retired(from_row.node_id()), "a descendant scope's node");
        assert!(!d.is_retired(cached.node_id()), "a cache scope's node");
    }

    /// A node minted by raw backend access has no record, so the write cannot
    /// say whose it is: it is detached, as before, and still re-inserts.
    #[test]
    fn a_child_with_no_minting_record_is_only_detached() {
        let doc = doc();
        let mut sc = scope(&doc);
        let body = body_handle(&doc);
        let wrap = sc.create_element("div");
        body.append_child(&wrap);
        let raw = doc.borrow_mut().create_element("i");
        let raw = NodeHandle::new(raw, Rc::downgrade(&doc) as _);
        wrap.append_child(&raw);

        wrap.set_text("over");
        assert!(!doc.borrow().is_retired(raw.node_id()));
        body.append_child(&raw);
        assert_eq!(doc.borrow().parent_node(raw.node_id()), Some(body.node_id()));
    }
}
