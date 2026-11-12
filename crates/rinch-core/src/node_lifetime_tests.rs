//! Node and cache-scope lifetime (issues #1454, #1456, #1508).
//!
//! - #1508: a node a scope minted and something `remove()`d, with nobody
//!   putting it back, is discarded when that scope goes.
//! - #1456: inserting a retired node is counted at the insertion verb, so a
//!   retired node nested in fresh markup is seen, and the warning's memory is
//!   per document.
//! - #1454: [`RenderScope::build_cached`] gives a cached subtree one owner,
//!   [`Cached`], whose drop is the eviction.

use crate::dom::mock::MockDomDocument;
use crate::dom::{
    __filed_orphans, __minted_by_len, __retired_insertions, __scope_parents_len, Cached,
    DomDocument, NodeHandle, NodeId, RenderScope,
};
use crate::reactive::Signal;
use crate::{for_each_dom_typed, show_dom};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

type Doc = Rc<RefCell<MockDomDocument>>;

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
fn count(doc: &Doc) -> isize {
    doc.borrow().__node_count() as isize
}
fn retired(doc: &Doc, node: &NodeHandle) -> bool {
    doc.borrow().is_retired(node.node_id())
}

// ── #1508: `remove()` inside a branch, handle dropped ───────────────────────

/// The issue's measurement: a branch builds `div > (span, span)` and removes
/// both spans with `remove()`, dropping the handles. Before, every hide left
/// both spans in the node table and the minting table: +200 / +200 per 100
/// cycles.
#[test]
fn removed_children_of_a_branch_go_with_the_branch() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let open = Signal::new(false);
    show_dom(
        &mut sc,
        &b,
        move || open.get(),
        |s: &mut RenderScope| {
            let div = s.create_element("div");
            let (a, c) = (s.create_element("span"), s.create_element("span"));
            div.append_child(&a);
            div.append_child(&c);
            a.remove();
            c.remove();
            div
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    open.set(false);
    let (n, m) = (count(&doc), __minted_by_len());
    for i in 0..200 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        (count(&doc) - n, __minted_by_len() as isize - m as isize),
        (0, 0),
        "#1508: the removed spans stayed in the node / minting tables"
    );
    assert_eq!(__filed_orphans(), 0, "nothing left filed");
}

/// A removed node is still re-insertable for as long as its scope lives
/// (#719's promise), and putting it back takes it off the scope's list: it
/// survives the scope's end if it is in a tree then.
#[test]
fn a_removed_node_reinserts_while_its_scope_lives_and_is_kept_once_back() {
    let doc = doc();
    let b = body(&doc);
    let mut owner = scope(&doc);
    let p = owner.create_element("p");
    let t = owner.create_text("hi");
    p.append_child(&t);
    b.append_child(&p);
    p.remove();
    assert_eq!(__filed_orphans(), 1, "filed under its minting scope");
    b.append_child(&p);
    assert_eq!(__filed_orphans(), 0, "an insertion unfiles it");
    assert_eq!(tags(&doc, b.node_id()), ["p"]);
    drop(owner);
    assert!(!retired(&doc, &p), "attached at the scope's end: kept");
    assert_eq!(tags(&doc, b.node_id()), ["p"]);
}

/// Detached at its scope's end: discarded, subtree and all — and a handle
/// from another scope nested inside is taken out first and kept.
#[test]
fn a_node_still_removed_when_its_scope_drops_is_discarded_but_a_captured_child_is_kept() {
    let doc = doc();
    let b = body(&doc);
    let mut outer = scope(&doc);
    let kept = outer.create_element("em");
    let base = count(&doc);
    let mut inner = scope(&doc);
    let p = inner.create_element("p");
    p.append_child(&inner.create_text("x"));
    p.append_child(&kept);
    b.append_child(&p);
    p.remove();
    drop(inner);
    assert!(retired(&doc, &p), "#1508: discarded with its scope");
    assert!(
        !retired(&doc, &kept),
        "another scope's node is only detached"
    );
    assert_eq!(count(&doc), base, "p and its text are gone");
    b.append_child(&kept);
    assert_eq!(tags(&doc, b.node_id()), ["em"]);
}

/// A node removed and then discarded by the app is not discarded twice, and
/// leaves nothing filed.
#[test]
fn a_removed_then_discarded_node_is_unfiled() {
    let doc = doc();
    let b = body(&doc);
    let mut sc = scope(&doc);
    let p = sc.create_element("p");
    b.append_child(&p);
    p.remove();
    assert_eq!(__filed_orphans(), 1);
    p.discard();
    assert_eq!(__filed_orphans(), 0);
}

/// A node with no minting record (raw backend access: the editor view,
/// `parse_html`) is never filed: no scope can say it is done with it.
#[test]
fn a_raw_minted_node_is_not_filed() {
    let doc = doc();
    let b = body(&doc);
    let _sc = scope(&doc);
    let id = doc.borrow_mut().create_element("p");
    let p = NodeHandle::new(id, Rc::downgrade(&doc) as _);
    b.append_child(&p);
    p.remove();
    assert_eq!(__filed_orphans(), 0);
}

// ── #1454's fourth row through the low-level pair ───────────────────────────

/// The recipe with `(NodeHandle, RenderScope)` pairs, inside a component that
/// comes and goes, and **no** drain: dropping the cache drops the scopes, and
/// each cached row — detached by the hide — now goes with its scope. Was
/// +200 over 50 mounts of a 2-row list (`cache_scope_733_tests::r4_…`).
#[test]
fn a_dropped_cache_of_pairs_no_longer_leaks_its_rows() {
    type Pairs = Rc<RefCell<HashMap<u32, (NodeHandle, RenderScope)>>>;
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let open = Signal::new(false);
    show_dom(
        &mut sc,
        &b,
        move || open.get(),
        move |s: &mut RenderScope| {
            let cache: Pairs = Rc::default();
            let host = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            for_each_dom_typed(
                s,
                &host,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| {
                    if let Some((row, _)) = cache.borrow().get(&n) {
                        return row.clone();
                    }
                    let mut keep = s.cache_scope();
                    let row = keep.build(|k| {
                        let row = k.create_element("article");
                        row.append_child(&k.create_text("ROW"));
                        row
                    });
                    cache.borrow_mut().insert(n, (row.clone(), keep));
                    row
                },
            );
            host
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    open.set(false);
    let (n, m, p) = (count(&doc), __minted_by_len(), __scope_parents_len());
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    assert_eq!(
        (count(&doc) - n, __minted_by_len(), __scope_parents_len()),
        (0, m, p),
        "#1454/#1508: a dropped cache's rows stayed in the document"
    );
}

// ── #1456: a retired node is seen where it is inserted ──────────────────────

/// The issue's shape: a cached row built through the row's own scope, inside
/// a wrapper the row builds. The wrapper comes back empty — still lost, that
/// is #733 — and now the insertion is counted.
#[test]
fn a_retired_node_nested_in_fresh_markup_is_reported() {
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
                let row = s.create_element("article");
                c.borrow_mut().insert(n, row.clone());
                row
            });
            let w = s.create_element("section");
            w.append_child(&row);
            w
        },
    );
    assert_eq!(__retired_insertions(), (0, 0), "control: a live row");
    rows.set(vec![]);
    rows.set(vec![1]);
    let wrap = doc.borrow().get_children(doc.borrow().body())[1];
    assert_eq!(tags(&doc, wrap), Vec::<String>::new(), "still lost (#733)");
    assert_eq!(
        __retired_insertions(),
        (1, 1),
        "#1456: the nested retired node was inserted unreported"
    );
}

/// Every insertion verb asks.
#[test]
fn every_insertion_verb_counts_a_retired_node() {
    let doc = doc();
    let b = body(&doc);
    let mut sc = scope(&doc);
    let anchor = sc.create_element("p");
    b.append_child(&anchor);
    let dead = sc.create_element("aside");
    dead.discard();
    b.append_child(&dead);
    b.insert_before(&dead, &anchor);
    anchor.insert_after(&dead);
    let spare = sc.create_element("i");
    b.append_child(&spare);
    spare.replace_with(&dead);
    assert_eq!(__retired_insertions(), (4, 1), "four insertions, one node");
}

/// The warning's memory is per document: a second document's lost nodes are
/// reported even after the first used up its 32.
#[test]
fn the_warning_memory_is_per_document() {
    let mut keep = Vec::new();
    for _ in 0..2 {
        let doc = doc();
        let b = body(&doc);
        let mut sc = scope(&doc);
        for _ in 0..40 {
            let n = sc.create_element("aside");
            n.discard();
            b.append_child(&n);
        }
        keep.push((doc, sc));
    }
    assert_eq!(
        __retired_insertions(),
        (80, 64),
        "#1456: the second document's lost nodes went unreported"
    );
}

/// A live node is never counted, whatever verb inserts it.
#[test]
fn a_live_node_is_not_counted() {
    let doc = doc();
    let b = body(&doc);
    let mut sc = scope(&doc);
    let p = sc.create_element("p");
    b.append_child(&p);
    p.remove();
    b.append_child(&p);
    assert_eq!(__retired_insertions(), (0, 0));
}

// ── #1454: one owner for a cached subtree ───────────────────────────────────

/// Build one cached `article` with a reactive attribute.
fn cached(
    s: &mut RenderScope,
    cache: &Rc<RefCell<HashMap<u32, Cached>>>,
    key: u32,
    label: Signal<u32>,
    runs: &Rc<Cell<u32>>,
) -> NodeHandle {
    if let Some(c) = cache.borrow().get(&key) {
        return c.node();
    }
    let runs = runs.clone();
    let entry = s.build_cached(|k| {
        let row = k.create_element("article");
        row.append_child(&k.create_text("ROW"));
        let local = Signal::new(0u32);
        let target = row.clone();
        k.create_effect(move || {
            runs.set(runs.get() + 1);
            target.set_attribute("data-l", &(label.get() + local.get()).to_string());
        });
        row
    });
    let node = entry.node();
    cache.borrow_mut().insert(key, entry);
    node
}

/// The cached row survives its row leaving and coming back, live — the signal
/// made inside the build belongs to the entry, not to the row (`build` is
/// part of the call and cannot be forgotten).
#[test]
fn a_cached_row_comes_back_live() {
    let doc = doc();
    let mut sc = scope(&doc);
    let b = body(&doc);
    let cache: Rc<RefCell<HashMap<u32, Cached>>> = Rc::default();
    let rows = Signal::new(vec![7u32]);
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
    let row = cache.borrow()[&7].node();
    let base = count(&doc);
    for _ in 0..50 {
        rows.set(vec![]);
        rows.set(vec![7]);
    }
    assert_eq!(count(&doc), base, "no growth");
    assert_eq!(tags(&doc, b.node_id()), ["article"]);
    label.set(5);
    assert_eq!(
        doc.borrow()
            .get_attribute(row.node_id(), "data-l")
            .as_deref(),
        Some("5")
    );
    assert_eq!(__retired_insertions(), (0, 0));
}

/// Dropping an entry is the eviction: its effects stop and its nodes go,
/// whether it is shown or not.
#[test]
fn dropping_a_cached_entry_evicts_it() {
    for shown in [false, true] {
        let doc = doc();
        let mut sc = scope(&doc);
        let b = body(&doc);
        let cache: Rc<RefCell<HashMap<u32, Cached>>> = Rc::default();
        let label = Signal::new(0u32);
        let runs = Rc::new(Cell::new(0));
        let empty = count(&doc);
        let row = cached(&mut sc, &cache, 1, label, &runs);
        if shown {
            b.append_child(&row);
        }
        assert_eq!(count(&doc), empty + 2, "precondition: article + text");
        let cleaned = Rc::new(Cell::new(false));
        let saw_live = cleaned.clone();
        let (d, r) = (doc.clone(), row.clone());
        // A cleanup on the entry runs while its node is still live.
        cache
            .borrow_mut()
            .get_mut(&1)
            .unwrap()
            .scope()
            .on_cleanup(move || {
                saw_live.set(!d.borrow().is_retired(r.node_id()));
            });
        drop(cache.borrow_mut().remove(&1));
        assert!(
            cleaned.get(),
            "shown={shown}: cleanup ran before the discard"
        );
        assert_eq!(count(&doc), empty, "shown={shown}: #1454 nodes released");
        assert!(retired(&doc, &row));
        assert_eq!(tags(&doc, b.node_id()), Vec::<String>::new());
        let before = runs.get();
        label.set(1);
        assert_eq!(runs.get(), before, "shown={shown}: #1454 effects released");
    }
}

/// #1454's fourth row with the one-owner value: the component that owns the
/// cache unmounts, the map drops, and nothing is left — no drain needed.
#[test]
fn a_dropped_cache_of_entries_leaves_nothing_behind() {
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
            let cache: Rc<RefCell<HashMap<u32, Cached>>> = Rc::default();
            let host = s.create_element("div");
            let rows = Signal::new(vec![1u32, 2]);
            let r = r.clone();
            for_each_dom_typed(
                s,
                &host,
                move || rows.get(),
                |n: &u32| n.to_string(),
                move |n: u32, s: &mut RenderScope| cached(s, &cache, n, label, &r),
            );
            host
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    open.set(true);
    assert_eq!(tags(&doc, b.node_id()), ["div"], "control: mounted");
    open.set(false);
    let (n, tables) = (count(&doc), (__minted_by_len(), __scope_parents_len()));
    for i in 0..100 {
        open.set(i % 2 == 0);
    }
    assert_eq!(count(&doc) - n, 0, "#1454: a dropped cache leaks no node");
    assert_eq!((__minted_by_len(), __scope_parents_len()), tables);
    let before = runs.get();
    label.set(1);
    assert_eq!(runs.get(), before, "and no effect is left running");
}

/// Dropped while the document is borrowed (a cache dropped from inside a
/// backend call): no panic; the scope is still disposed.
#[test]
fn an_entry_dropped_while_the_document_is_borrowed_does_not_panic() {
    let doc = doc();
    let mut sc = scope(&doc);
    let cache: Rc<RefCell<HashMap<u32, Cached>>> = Rc::default();
    let label = Signal::new(0u32);
    let runs = Rc::new(Cell::new(0));
    cached(&mut sc, &cache, 1, label, &runs);
    let entry = cache.borrow_mut().remove(&1).unwrap();
    {
        let _held = doc.borrow_mut();
        drop(entry);
    }
    let before = runs.get();
    label.set(1);
    assert_eq!(runs.get(), before, "the scope was disposed");
}
