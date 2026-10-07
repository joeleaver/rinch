//! More shapes of a cache filled from inside a closure (issue #733:
//! `RenderScope::cache_scope` / `build`), from the review of PR #1452. Each
//! test states what it measures; the ones named `finding_*` pin a behaviour
//! that is documented rather than fixed, so they pass and say so.

use crate::dom::mock::MockDomDocument;
use crate::dom::{
    __minted_by_len, __retired_view_returns, __scope_parents_len, DomDocument, NodeHandle, NodeId,
    RenderScope, reactive_component_dom,
};
use crate::reactive::Signal;
use crate::{for_each_dom_typed, match_dom, show_dom};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type Doc = Rc<RefCell<MockDomDocument>>;
type Cache<K> = Rc<RefCell<HashMap<K, (NodeHandle, RenderScope)>>>;

fn doc() -> Doc {
    Rc::new(RefCell::new(MockDomDocument::new()))
}
fn body(doc: &Doc) -> NodeHandle {
    let b = doc.borrow().body();
    NodeHandle::new(b, Rc::downgrade(doc) as _)
}
fn scope(doc: &Doc) -> RenderScope {
    let b = doc.borrow().body();
    RenderScope::new(doc.clone(), b)
}
fn tags(doc: &Doc, parent: NodeId) -> Vec<String> {
    let d = doc.borrow();
    d.get_children(parent)
        .into_iter()
        .filter_map(|c| d.tag_name(c))
        .collect()
}
fn body_tags(doc: &Doc) -> Vec<String> {
    let b = doc.borrow().body();
    tags(doc, b)
}
fn count(doc: &Doc) -> isize {
    doc.borrow().__node_count() as isize
}

/// Build one cached `article` with a reactive attribute, through a cache scope.
fn cached<K: std::hash::Hash + Eq + Clone>(
    s: &mut RenderScope,
    cache: &Cache<K>,
    key: K,
    label: Signal<u32>,
    runs: &Rc<Cell<u32>>,
) -> NodeHandle {
    if let Some((row, _)) = cache.borrow().get(&key) {
        return row.clone();
    }
    let mut keep = s.cache_scope();
    let runs = runs.clone();
    let row = keep.build(|k| {
        let row = k.create_element("article");
        let t = k.create_text("ROW");
        row.append_child(&t);
        let target = row.clone();
        k.create_effect(move || {
            runs.set(runs.get() + 1);
            target.set_attribute("data-l", &label.get().to_string());
        });
        row
    });
    cache.borrow_mut().insert(key, (row.clone(), keep));
    row
}

// ── R1: the `for` Changed arm, with a cached row keyed by id ────────────────

/// A cached row whose *item data* changes while its key stays: the view is
/// called again and hands back the SAME node. Is it still on screen?
#[test]
fn r1_a_cached_row_whose_item_data_changes_stays_shown() {
    for wrapped in [false, true] {
        let doc = doc();
        let mut sc = scope(&doc);
        let b = body(&doc);
        let cache: Cache<u32> = Rc::default();
        let rows = Signal::new(vec![(1u32, 0u32), (2, 0)]);
        let label = Signal::new(0u32);
        let runs = Rc::new(Cell::new(0));
        let (c, r) = (cache.clone(), runs.clone());
        for_each_dom_typed(
            &mut sc,
            &b,
            move || rows.get(),
            |n: &(u32, u32)| n.0.to_string(),
            move |n: (u32, u32), s: &mut RenderScope| {
                let row = cached(s, &c, n.0, label, &r);
                if wrapped {
                    let w = s.create_element("section");
                    w.append_child(&row);
                    w
                } else {
                    row
                }
            },
        );
        let tag = if wrapped { "section" } else { "article" };
        assert_eq!(body_tags(&doc), [tag, tag], "precondition");
        let before = count(&doc);
        for i in 1..=5 {
            rows.set(vec![(1, i), (2, 0)]);
            assert_eq!(
                body_tags(&doc),
                [tag, tag],
                "wrapped={wrapped}: row 1 after data change {i}"
            );
            let first = doc.borrow().get_children(doc.borrow().body())[1];
            let row1 = cache.borrow()[&1].0.node_id();
            if wrapped {
                assert_eq!(doc.borrow().get_children(first), [row1]);
            } else {
                assert_eq!(first, row1, "order kept");
            }
        }
        assert_eq!(count(&doc), before, "wrapped={wrapped}: no growth");
        label.set(9);
        let row1 = cache.borrow()[&1].0.node_id();
        assert_eq!(
            doc.borrow().get_attribute(row1, "data-l").as_deref(),
            Some("9")
        );
        assert_eq!(__retired_view_returns(), (0, 0));
    }
}

// ── R2: forgetting `build` ──────────────────────────────────────────────────

/// Built through the cache scope's `&mut` but WITHOUT `build`: nodes are the
/// cache scope's, the `Signal::new` is the row's. What does the user see when
/// the row has left and the signal is touched?
#[test]
fn r2_forgetting_build_frees_the_rows_signal_under_the_cached_effect() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let cache: Cache<u32> = Rc::default();
    let rows = Signal::new(vec![7u32]);
    let inner: Rc<Cell<Option<Signal<u32>>>> = Rc::default();
    let (c, i) = (cache.clone(), inner.clone());
    for_each_dom_typed(
        &mut sc,
        &b,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            if let Some((row, _)) = c.borrow().get(&n) {
                return row.clone();
            }
            let mut keep = s.cache_scope();
            // no `build`
            let row = keep.create_element("article");
            let local = Signal::new(0u32);
            i.set(Some(local));
            let target = row.clone();
            keep.create_effect(move || {
                target.set_attribute("data-n", &local.get().to_string());
            });
            c.borrow_mut().insert(n, (row.clone(), keep));
            row
        },
    );
    let row = cache.borrow()[&7].0.node_id();
    let local = inner.get().unwrap();
    local.set(1);
    assert_eq!(
        doc.borrow().get_attribute(row, "data-n").as_deref(),
        Some("1")
    );
    rows.set(vec![]);
    rows.set(vec![7]);
    assert_eq!(body_tags(&doc), ["article"], "the node does come back");
    // The write is a warn-once no-op on a freed signal: silent.
    local.set(2);
    let after = doc.borrow().get_attribute(row, "data-n");
    eprintln!("R2: data-n after a write to the row-owned signal: {after:?}");
    assert_eq!(
        after.as_deref(),
        Some("1"),
        "finding: without `build` the binding is silently dead (no panic, no #733 warning)"
    );
    assert_eq!(__retired_view_returns(), (0, 0), "and D says nothing");
}

// ── R3: the guide's "the same works in an `if` or `match` body" ─────────────

#[test]
fn r3_a_lazily_cached_if_branch_comes_back_live_and_does_not_grow() {
    for wrapped in [false, true] {
        let doc = doc();
        let mut sc = scope(&doc);
        let b = body(&doc);
        let cache: Cache<u8> = Rc::default();
        let open = Signal::new(true);
        let label = Signal::new(0u32);
        let runs = Rc::new(Cell::new(0));
        let (c, r) = (cache.clone(), runs.clone());
        show_dom(
            &mut sc,
            &b,
            move || open.get(),
            move |s: &mut RenderScope| {
                let row = cached(s, &c, 0u8, label, &r);
                if wrapped {
                    let w = s.create_element("section");
                    w.append_child(&row);
                    w
                } else {
                    row
                }
            },
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
        let tag = if wrapped { "section" } else { "article" };
        assert_eq!(body_tags(&doc), [tag]);
        open.set(false);
        label.set(4);
        open.set(true);
        assert_eq!(body_tags(&doc), [tag], "wrapped={wrapped}");
        let row = cache.borrow()[&0].0.node_id();
        assert_eq!(
            doc.borrow().get_attribute(row, "data-l").as_deref(),
            Some("4")
        );
        let base = count(&doc);
        for i in 0..200 {
            open.set(i % 2 == 1);
        }
        assert_eq!(count(&doc), base, "wrapped={wrapped}: growth");
        assert_eq!(runs.get(), 2, "built once, one re-run for the label");
        assert_eq!(__retired_view_returns(), (0, 0));
    }
}

#[test]
fn r3_lazily_cached_match_arms_and_component_output_come_back() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let cache: Cache<usize> = Rc::default();
    let which = Signal::new(0usize);
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    type Arm = Box<dyn Fn(&mut RenderScope) -> NodeHandle>;
    let arms: Vec<Arm> = (0..2usize)
        .map(|i| {
            let (c, r) = (cache.clone(), runs.clone());
            Box::new(move |s: &mut RenderScope| cached(s, &c, i, label, &r)) as Arm
        })
        .collect();
    match_dom(&mut sc, &b, move || which.get(), arms);
    which.set(1);
    let base = count(&doc);
    for i in 0..200 {
        which.set(i % 2);
    }
    assert_eq!(count(&doc), base);
    label.set(3);
    for k in 0..2 {
        let n = cache.borrow()[&k].0.node_id();
        assert_eq!(
            doc.borrow().get_attribute(n, "data-l").as_deref(),
            Some("3")
        );
    }
    assert_eq!(runs.get(), 4);

    // component re-render: the render_fn re-runs for `tick`, hands back the cache
    let cache2: Cache<u8> = Rc::default();
    let tick = Signal::new(0u32);
    let (c, r) = (cache2.clone(), runs.clone());
    reactive_component_dom(&mut sc, &b, move |s: &mut RenderScope| {
        let _ = tick.get();
        let row = cached(s, &c, 0u8, label, &r);
        let w = s.create_element("section");
        w.append_child(&row);
        w
    });
    let base = count(&doc);
    for i in 0..200 {
        tick.set(i);
    }
    assert_eq!(count(&doc), base);
    let n = cache2.borrow()[&0].0.node_id();
    let parent = doc.borrow().parent_node(n).expect("still mounted");
    assert_eq!(doc.borrow().tag_name(parent).as_deref(), Some("section"));
    assert_eq!(__retired_view_returns(), (0, 0));
}

// ── R4: the cache's owner unmounts ──────────────────────────────────────────

/// The recipe as the guide writes it, inside a component that itself comes and
/// goes (`if page == Tabs { Tabs {} }`), with the cache owned by that
/// component. Dropping the cache drops the scopes (effects stop) — and the
/// nodes?
#[test]
fn r4_finding_a_dropped_cache_leaks_every_cached_node() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let open = Signal::new(false);
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    show_dom(
        &mut sc,
        &b,
        move || open.get(),
        move |s: &mut RenderScope| {
            // the component: owns its cache, shows two cached rows
            let cache: Cache<u32> = Rc::default();
            let host = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            let (c, r) = (cache.clone(), r.clone());
            for_each_dom_typed(
                s,
                &host,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| cached(s, &c, n, label, &r),
            );
            host
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    open.set(false);
    let base = count(&doc);
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    let grew = count(&doc) - base;
    eprintln!("R4: 50 mounts of a 2-row cached list grew the mock by {grew} nodes");
    assert_eq!(
        grew,
        50 * 2 * 2,
        "finding: each unmount strands every cached row (article + text); \
         nothing discards them and nothing warns"
    );
    let before = runs.get();
    label.set(1);
    assert_eq!(runs.get(), before, "the effects at least are stopped");
}

// ── R5: build + evict churn leaves no table entries ─────────────────────────

#[test]
fn r5_evict_churn_leaves_the_ancestry_tables_and_the_document_flat() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let cache: Cache<u32> = Rc::default();
    let rows = Signal::new(vec![]);
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    let (c, r) = (cache.clone(), runs.clone());
    for_each_dom_typed(
        &mut sc,
        &b,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| cached(s, &c, n, label, &r),
    );
    let cycle = |k: u32| {
        rows.set(vec![k]);
        rows.set(vec![]);
        let (row, keep) = cache.borrow_mut().remove(&k).unwrap();
        keep.dispose();
        row.discard();
    };
    cycle(0);
    cycle(1);
    let (n, m, p) = (count(&doc), __minted_by_len(), __scope_parents_len());
    for k in 2..200 {
        cycle(k);
    }
    assert_eq!(
        (count(&doc), __minted_by_len(), __scope_parents_len()),
        (n, m, p)
    );
}

// ── R6: the two half-evictions ──────────────────────────────────────────────

/// Discard the node, keep the scope: the effect goes on running against a
/// retired node for as long as the scope lives.
#[test]
fn r6_finding_a_discarded_node_with_a_kept_scope_keeps_its_effects_running() {
    let doc = doc();
    let mut sc = scope(&doc);
    let cache: Cache<u32> = Rc::default();
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    let row = cached(&mut sc, &cache, 1, label, &runs);
    row.discard();
    let before = runs.get();
    label.set(1);
    label.set(2);
    assert_eq!(runs.get(), before + 2, "finding: silent, runs for good");
    assert_eq!(__retired_view_returns(), (0, 0));
}

// ── R7: a cached row that itself holds a `for` ──────────────────────────────

#[test]
fn r7_a_cached_row_holding_its_own_for_survives_and_evicts_clean() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    type C = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;
    let cache: C = Rc::default();
    let rows = Signal::new(vec![]);
    let inner = Signal::new(vec![1u32, 2, 3]);
    let c = cache.clone();
    for_each_dom_typed(
        &mut sc,
        &b,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            if let Some((row, _)) = c.borrow().get(&n) {
                return row.clone();
            }
            let mut keep = s.cache_scope();
            let row = keep.build(|k| {
                let row = k.create_element("article");
                for_each_dom_typed(
                    k,
                    &row,
                    move || inner.get(),
                    |n: &u32| n.to_string(),
                    |_n: u32, s: &mut RenderScope| s.create_element("li"),
                );
                row
            });
            c.borrow_mut().insert(n, (row.clone(), keep));
            // wrapped in row-built markup, so the row's discard sweeps over it
            let w = s.create_element("section");
            w.append_child(&row);
            w
        },
    );
    let empty = count(&doc);
    let (m0, p0) = (__minted_by_len(), __scope_parents_len());
    rows.set(vec![9]);
    let row = cache.borrow()[&9].0.node_id();
    assert_eq!(tags(&doc, row), ["li", "li", "li"]);
    rows.set(vec![]);
    inner.set(vec![1, 2, 3, 4]); // inner list reconciles while detached
    rows.set(vec![9]);
    assert_eq!(tags(&doc, row), ["li", "li", "li", "li"]);
    inner.set(vec![4]);
    assert_eq!(tags(&doc, row), ["li"]);
    rows.set(vec![]);
    let (node, keep) = cache.borrow_mut().remove(&9).unwrap();
    keep.dispose();
    node.discard();
    assert_eq!(count(&doc), empty, "eviction releases the inner rows too");
    assert_eq!((__minted_by_len(), __scope_parents_len()), (m0, p0));
}

// ── R8: D's reach ───────────────────────────────────────────────────────────

/// The warning's memory is 32 nodes per THREAD for the life of the thread:
/// the 33rd distinct lost row is counted and not reported.
#[test]
fn r8_the_33rd_lost_row_on_a_thread_is_not_reported() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let dead: Vec<NodeHandle> = (0..40)
        .map(|_| {
            let n = sc.create_element("aside");
            n.discard();
            n
        })
        .collect();
    let rows = Signal::new(Vec::<usize>::new());
    for_each_dom_typed(
        &mut sc,
        &b,
        move || rows.get(),
        |n: &usize| n.to_string(),
        move |n: usize, _s: &mut RenderScope| dead[n].clone(),
    );
    rows.set((0..40).collect());
    assert_eq!(__retired_view_returns(), (40, 32));
}

/// D sees a retired node only when it is the closure's RETURN value. The
/// issue's shape one level down — the cached node inside a wrapper the row
/// builds — is lost exactly the same way and reported by nothing.
#[test]
fn r8_finding_a_retired_node_nested_in_fresh_markup_is_lost_unreported() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::default();
    let rows = Signal::new(vec![1u32]);
    let c = cache.clone();
    for_each_dom_typed(
        &mut sc,
        &b,
        move || rows.get(),
        |n: &u32| n.to_string(),
        move |n: u32, s: &mut RenderScope| {
            let hit = c.borrow().get(&n).cloned();
            let row = hit.unwrap_or_else(|| {
                let row = s.create_element("article"); // the row's own scope: #733
                c.borrow_mut().insert(n, row.clone());
                row
            });
            let w = s.create_element("section");
            w.append_child(&row);
            w
        },
    );
    let wrap = doc.borrow().get_children(doc.borrow().body())[1];
    assert_eq!(tags(&doc, wrap), ["article"], "precondition");
    rows.set(vec![]);
    rows.set(vec![1]);
    let wrap = doc.borrow().get_children(doc.borrow().body())[1];
    assert_eq!(tags(&doc, wrap), Vec::<String>::new(), "lost");
    assert_eq!(
        __retired_view_returns(),
        (0, 0),
        "finding: and D did not see it"
    );
}

/// The dedup key carries the document: the same node id lost in two documents
/// on one thread is two warnings (kills the mutant that keys by node id alone,
/// which the PR's own suite does not).
#[test]
fn r8_the_same_node_id_in_two_documents_is_reported_twice() {
    let mut ids = Vec::new();
    for _ in 0..2 {
        let doc = doc();
        let mut sc = scope(&doc);
        let b = body(&doc);
        let gone = sc.create_element("aside");
        gone.discard();
        ids.push(gone.node_id());
        show_dom(
            &mut sc,
            &b,
            || true,
            move |_s: &mut RenderScope| gone.clone(),
            None::<fn(&mut RenderScope) -> NodeHandle>,
        );
    }
    assert_eq!(ids[0], ids[1], "precondition: one id, two documents");
    assert_eq!(__retired_view_returns(), (2, 2));
}

// ── the documented eviction: the cache's owner unmounts ─────────────────────

/// R4 with the recipe's `on_cleanup`: the component that owns the cache
/// drains it when it unmounts — dispose each scope, discard each node — and
/// 50 mounts of the same 2-row list leave the document where it was. R4 above
/// is the same fixture without the drain (+200).
#[test]
fn a_cache_drained_in_on_cleanup_leaves_nothing_behind() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let open = Signal::new(false);
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    show_dom(
        &mut sc,
        &b,
        move || open.get(),
        move |s: &mut RenderScope| {
            let cache: Cache<u32> = Rc::default();
            let evict = cache.clone();
            s.on_cleanup(move || {
                for (_, (row, keep)) in evict.borrow_mut().drain() {
                    keep.dispose();
                    row.discard();
                }
            });
            let host = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            let (c, r) = (cache.clone(), r.clone());
            for_each_dom_typed(
                s,
                &host,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| cached(s, &c, n, label, &r),
            );
            host
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert_eq!(tags(&doc, doc.borrow().body()), ["div"], "control: mounted");
    open.set(false);
    let base = count(&doc);
    let tables = (__minted_by_len(), __scope_parents_len());
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        count(&doc) - base,
        0,
        "#733: an evicting cache leaks no node"
    );
    assert_eq!(
        (__minted_by_len(), __scope_parents_len()),
        tables,
        "#733: nor an ancestry record"
    );
    let before = runs.get();
    label.set(1);
    assert_eq!(runs.get(), before, "and no effect is left running");
}
