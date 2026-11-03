//! An `if let` or `match` branch that binds a value follows that value
//! (issue #2105).
//!
//! `if let Some(name) = user.get() { … }` used to rebuild its branch only when
//! the pattern started or stopped matching: the condition was
//! `matches!(expr, pat)`, a bool, and `show_dom` swaps content only when that
//! bool flips. `Some("Alice")` → `Some("Bob")` kept "Alice" forever. A `match`
//! arm with bindings had the same defect through `match_dom`'s arm index.
//!
//! The semantics now: a branch whose pattern **binds a name** is rebuilt when
//! its scrutinee's reactive reads change while the same branch still matches.
//! A branch that binds nothing (`Some(_)`, `None`, `0 =>`, a plain `if`) is
//! never rebuilt by a re-run — it has nothing that could be stale. There is no
//! value comparison: a `Memo` scrutinee (or `Signal::set_if_changed`) is the
//! equality cut-off.
//!
//! Every test that asserts a redraw samples a second value distinct from the
//! first (`Alice` → `Bob`), so the stale branch fails it; every test that
//! asserts *no* rebuild counts renders, so a rebuild that happens to draw the
//! same text still fails it.

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

/// The rendered text of a subtree, skipping comment markers.
fn text(node: &NodeHandle) -> String {
    if node.node_type() == Some(8) {
        return String::new();
    }
    let children = node.children();
    if children.is_empty() {
        return node.text_content().unwrap_or_default();
    }
    children.iter().map(text).collect()
}

thread_local! {
    /// How many times a branch body ran. Each test runs on its own thread.
    static RENDERS: Cell<u32> = const { Cell::new(0) };
}

fn renders() -> u32 {
    RENDERS.with(Cell::get)
}

/// A branch body that counts its own builds.
fn mark(__scope: &mut RenderScope, label: &str) -> NodeHandle {
    RENDERS.with(|r| r.set(r.get() + 1));
    let p = __scope.create_element("p");
    let t = __scope.create_text(label);
    p.append_child(&t);
    p
}

// ============================================================
// The issue's example
// ============================================================

#[component]
fn welcome(user: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(name) = user.get() {
                p { "Welcome, " {name} "!" }
            } else {
                p { "Please log in" }
            }
        }
    }
}

#[test]
fn if_let_follows_the_bound_value() {
    let user = Signal::new(Some("Alice".to_string()));
    let (_doc, _scope, root) = mount(|s| welcome(s, user));
    assert_eq!(text(&root), "Welcome, Alice!");

    user.set(Some("Bob".into()));
    assert_eq!(
        text(&root),
        "Welcome, Bob!",
        "Some(a) -> Some(b) must redraw"
    );

    user.set(None);
    assert_eq!(text(&root), "Please log in");
    user.set(Some("Carol".into()));
    assert_eq!(text(&root), "Welcome, Carol!");
    user.set(Some("Dave".into()));
    assert_eq!(text(&root), "Welcome, Dave!");
}

// ============================================================
// match arms with bindings
// ============================================================

#[component]
fn selection(sel: Signal<Option<u32>>) -> NodeHandle {
    rsx! {
        div {
            match sel.get() {
                Some(n) if n > 100 => { mark(__scope, &format!("big {n}")) },
                Some(n) => { mark(__scope, &format!("item {n}")) },
                None => { mark(__scope, "none") },
            }
        }
    }
}

#[test]
fn a_match_arm_follows_its_bound_value() {
    let sel = Signal::new(Some(3u32));
    let (_doc, _scope, root) = mount(|s| selection(s, sel));
    assert_eq!(text(&root), "item 3");
    assert_eq!(renders(), 1);

    sel.set(Some(7));
    assert_eq!(text(&root), "item 7", "same arm, new binding");
    assert_eq!(renders(), 2);

    sel.set(Some(200));
    assert_eq!(text(&root), "big 200");
    sel.set(Some(300));
    assert_eq!(text(&root), "big 300", "a guarded arm follows too");
}

#[test]
fn a_match_arm_that_binds_nothing_is_not_rebuilt() {
    let sel = Signal::new(None::<u32>);
    let (_doc, _scope, root) = mount(|s| selection(s, sel));
    assert_eq!(text(&root), "none");
    assert_eq!(renders(), 1);

    // The scrutinee re-ran and the `None` arm still matches; it binds nothing,
    // so there is nothing to redraw.
    sel.set(None);
    sel.set(None);
    assert_eq!(renders(), 1, "the `None` arm binds nothing");
}

#[component]
fn tabs(tab: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            match tab.get() {
                0 => { mark(__scope, "home") },
                1 => { mark(__scope, "about") },
                _ => { mark(__scope, "other") },
            }
        }
    }
}

#[test]
fn a_match_on_literals_rebuilds_only_when_the_arm_changes() {
    let tab = Signal::new(2u32);
    let (_doc, _scope, root) = mount(|s| tabs(s, tab));
    assert_eq!(text(&root), "other");
    tab.set(5);
    tab.set(9);
    assert_eq!(renders(), 1, "`_` binds nothing: same arm, no rebuild");
    tab.set(1);
    assert_eq!(text(&root), "about");
    assert_eq!(renders(), 2);
}

// ============================================================
// Patterns that bind nothing keep building once
// ============================================================

// `if let Some(_)` is what is under test; clippy would have it `is_some()`.
#[allow(clippy::redundant_pattern_matching)]
#[component]
fn wildcard(user: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(_) = user.get() {
                { mark(__scope, "someone") }
            }
            if user.get().is_some() {
                { mark(__scope, "plain") }
            }
        }
    }
}

#[test]
fn a_pattern_that_binds_nothing_is_not_rebuilt() {
    let user = Signal::new(Some("Alice".to_string()));
    let (_doc, _scope, root) = mount(|s| wildcard(s, user));
    assert_eq!(text(&root), "someoneplain");
    assert_eq!(renders(), 2);
    user.set(Some("Bob".into()));
    assert_eq!(
        renders(),
        2,
        "`Some(_)` and a plain `if` build once per show"
    );
    user.set(None);
    user.set(Some("Carol".into()));
    assert_eq!(renders(), 4, "a flip still rebuilds both");
}

// ============================================================
// Equality: a Memo is the cut-off, a Signal write is not
// ============================================================

#[component]
fn counted(user: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(name) = user.get() {
                { mark(__scope, &name) }
            } else {
                { mark(__scope, "nobody") }
            }
        }
    }
}

#[test]
fn the_else_branch_of_a_rebinding_if_let_is_not_rebuilt() {
    let user = Signal::new(None::<String>);
    let (_doc, _scope, root) = mount(|s| counted(s, user));
    assert_eq!(text(&root), "nobody");
    user.set(None);
    user.set(None);
    assert_eq!(renders(), 1, "the else branch binds nothing");
    user.set(Some("Ann".into()));
    assert_eq!(text(&root), "Ann");
    assert_eq!(renders(), 2);
}

#[test]
fn an_equal_signal_write_rebuilds_and_set_if_changed_does_not() {
    let user = Signal::new(Some("Alice".to_string()));
    let (_doc, _scope, root) = mount(|s| counted(s, user));
    assert_eq!(renders(), 1);

    // `set` notifies whatever it writes, and the branch has no way to compare:
    // an equal write rebuilds (documented).
    user.set(Some("Alice".into()));
    assert_eq!(renders(), 2);
    assert_eq!(text(&root), "Alice");

    // `set_if_changed` is the cut-off at the write.
    user.set_if_changed(Some("Alice".into()));
    assert_eq!(renders(), 2);
    user.set_if_changed(Some("Bob".into()));
    assert_eq!(renders(), 3);
    assert_eq!(text(&root), "Bob");
}

#[component]
fn memo_len(src: Signal<Option<String>>) -> NodeHandle {
    let len = Memo::new(move || src.get().map(|s| s.len()));
    rsx! {
        div {
            if let Some(n) = len.get() {
                { mark(__scope, &n.to_string()) }
            }
        }
    }
}

#[test]
fn a_memo_scrutinee_is_the_equality_cut_off() {
    let src = Signal::new(Some("abc".to_string()));
    let (_doc, _scope, root) = mount(|s| memo_len(s, src));
    assert_eq!(text(&root), "3");
    assert_eq!(renders(), 1);

    // A different string of the same length: the memo's value is equal, so the
    // branch's condition never re-runs.
    src.set(Some("xyz".into()));
    assert_eq!(renders(), 1, "the memo's equality cut-off holds");

    src.set(Some("abcdefg".into()));
    assert_eq!(text(&root), "7");
    assert_eq!(renders(), 2);
}

// ============================================================
// else-if chains and nesting
// ============================================================

#[component]
fn chain(a: Signal<Option<u32>>, b: Signal<Option<u32>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(x) = a.get() {
                p { "a=" {x.to_string()} }
            } else if let Some(y) = b.get() {
                p { "b=" {y.to_string()} }
            } else {
                p { "neither" }
            }
        }
    }
}

#[test]
fn an_else_if_let_follows_its_own_binding() {
    let a = Signal::new(None::<u32>);
    let b = Signal::new(Some(1u32));
    let (_doc, _scope, root) = mount(|s| chain(s, a, b));
    assert_eq!(text(&root), "b=1");
    b.set(Some(2));
    assert_eq!(text(&root), "b=2");
    a.set(Some(5));
    assert_eq!(text(&root), "a=5");
    a.set(Some(6));
    assert_eq!(text(&root), "a=6");
    a.set(None);
    assert_eq!(text(&root), "b=2");
    b.set(None);
    assert_eq!(text(&root), "neither");
}

thread_local! {
    static CLEANUPS: Cell<u32> = const { Cell::new(0) };
    static TICK_RUNS: Cell<u32> = const { Cell::new(0) };
}

/// A branch body that registers a cleanup on the branch's own scope and an
/// effect that counts its runs.
fn disposable(__scope: &mut RenderScope, name: &str, tick: Signal<u32>) -> NodeHandle {
    __scope.on_cleanup(|| CLEANUPS.with(|c| c.set(c.get() + 1)));
    let p = __scope.create_element("p");
    let t = __scope.create_text(name);
    p.append_child(&t);
    __scope.create_effect(move || {
        TICK_RUNS.with(|r| r.set(r.get() + 1));
        let _ = tick.get();
    });
    p
}

/// A rebuild disposes the old branch's scope: its cleanup runs, and an effect
/// it created stops running.
#[component]
fn inner_effect(user: Signal<Option<String>>, tick: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            if let Some(name) = user.get() {
                { disposable(__scope, &name, tick) }
            }
        }
    }
}

#[test]
fn a_rebuild_disposes_the_old_branch() {
    let user = Signal::new(Some("A".to_string()));
    let tick = Signal::new(0u32);
    let (_doc, _scope, root) = mount(|s| inner_effect(s, user, tick));
    assert_eq!(text(&root), "A");
    assert_eq!(TICK_RUNS.with(Cell::get), 1);
    assert_eq!(CLEANUPS.with(Cell::get), 0);

    user.set(Some("B".into()));
    assert_eq!(text(&root), "B");
    assert_eq!(CLEANUPS.with(Cell::get), 1, "the old branch's scope was disposed");
    assert_eq!(TICK_RUNS.with(Cell::get), 2, "the new branch's effect ran once");

    tick.set(4);
    assert_eq!(
        TICK_RUNS.with(Cell::get),
        3,
        "only the live branch's effect re-runs; the old one is disposed"
    );
    let ps: Vec<_> = root
        .children()
        .into_iter()
        .filter(|n| n.node_type() != Some(8))
        .collect();
    assert_eq!(ps.len(), 1, "the old branch's markup is gone: {ps:?}");
}

// ============================================================
// Nested binding branches on one signal
// ============================================================

#[component]
fn nested(user: Signal<Option<String>>) -> NodeHandle {
    rsx! {
        div {
            if let Some(a) = user.get() {
                div {
                    { mark(__scope, &format!("o{a};")) }
                    if let Some(b) = user.get() {
                        { mark(__scope, &format!("i{b};")) }
                    }
                }
            }
        }
    }
}

#[test]
fn nested_binding_branches_on_one_signal_build_once_per_write() {
    let user = Signal::new(Some("a".to_string()));
    let (_doc, _scope, root) = mount(|s| nested(s, user));
    assert_eq!(text(&root), "oa;ia;");
    assert_eq!(renders(), 2);

    // The outer branch rebuilds and takes the inner one with it. The inner
    // branch must not first rebuild on its own and then again inside the
    // outer's rebuild.
    user.set(Some("b".into()));
    assert_eq!(text(&root), "ob;ib;");
    assert_eq!(renders(), 4, "outer once, inner once");
    user.set(Some("c".into()));
    assert_eq!(renders(), 6);
}

// ============================================================
// Guards
// ============================================================

#[component]
fn guarded(sel: Signal<Option<u32>>, limit: Signal<u32>) -> NodeHandle {
    rsx! {
        div {
            match sel.get() {
                Some(n) if n > limit.get() => { mark(__scope, &format!("big {n}")) },
                Some(n) => { mark(__scope, &format!("item {n}")) },
                None => { mark(__scope, "none") },
            }
        }
    }
}

#[test]
fn an_earlier_guard_s_read_rebuilds_a_later_binding_arm() {
    let sel = Signal::new(Some(5u32));
    let limit = Signal::new(10u32);
    let (_doc, _scope, root) = mount(|s| guarded(s, sel, limit));
    assert_eq!(text(&root), "item 5");
    assert_eq!(renders(), 1);
    // The first arm's guard read `limit`; the second arm still wins.
    limit.set(20);
    assert_eq!(text(&root), "item 5");
    assert_eq!(renders(), 2, "the discriminant re-ran: the binding arm rebuilds");
    limit.set(2);
    assert_eq!(text(&root), "big 5");
}

// ============================================================
// Inside a keyed `for`
// ============================================================

#[derive(Clone, Debug)]
struct Row {
    id: u32,
    note: Signal<Option<String>>,
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[component]
fn rows(list: Signal<Vec<Row>>) -> NodeHandle {
    rsx! {
        div {
            for row in list.get() {
                div {
                    key: row.id,
                    if let Some(n) = row.note.get() {
                        { mark(__scope, &format!("{}:{n};", row.id)) }
                    }
                }
            }
        }
    }
}

#[test]
fn an_if_let_in_a_keyed_row_survives_a_reorder_and_follows_its_row() {
    let r1 = Row {
        id: 1,
        note: Signal::new(Some("x".into())),
    };
    let r2 = Row {
        id: 2,
        note: Signal::new(Some("y".into())),
    };
    let list = Signal::new(vec![r1.clone(), r2.clone()]);
    let (_doc, _scope, root) = mount(|s| rows(s, list));
    assert_eq!(text(&root), "1:x;2:y;");
    assert_eq!(renders(), 2);

    // A reorder moves the keyed rows; the branches inside them do not re-run.
    list.set(vec![r2.clone(), r1.clone()]);
    assert_eq!(text(&root), "2:y;1:x;");
    assert_eq!(renders(), 2, "a reorder rebuilds no branch");

    // One row's value changes: only that row's branch is rebuilt.
    r1.note.set(Some("z".into()));
    assert_eq!(text(&root), "2:y;1:z;");
    assert_eq!(renders(), 3);
}

// ============================================================
// A write during the initial build
// ============================================================

/// A branch body that, the first time, writes the very signal its condition
/// reads.
fn bump(__scope: &mut RenderScope, count: Signal<Option<u32>>, n: u32) -> NodeHandle {
    if n == 0 {
        count.set(Some(1));
    }
    mark(__scope, &format!("n{n};"))
}

fn flip_off(__scope: &mut RenderScope, flag: Signal<bool>) -> NodeHandle {
    flag.set(false);
    mark(__scope, "on;")
}

#[component]
fn self_writing(count: Signal<Option<u32>>, flag: Signal<bool>) -> NodeHandle {
    rsx! {
        div {
            if let Some(n) = count.get() {
                { bump(__scope, count, n) }
            }
            if flag.get() {
                { flip_off(__scope, flag) }
            } else {
                { mark(__scope, "off;") }
            }
        }
    }
}

/// The condition's effect exists before the initial build (so it runs ahead of
/// effects the branch creates); a write the build itself makes is replayed
/// after the build, not applied to half-built content.
#[test]
fn a_write_during_the_initial_build_is_replayed_after_it() {
    let count = Signal::new(Some(0u32));
    let flag = Signal::new(true);
    let (_doc, _scope, root) = mount(|s| self_writing(s, count, flag));
    assert_eq!(text(&root), "n1;off;");
    let content: Vec<_> = root
        .children()
        .into_iter()
        .filter(|n| n.node_type() != Some(8))
        .collect();
    assert_eq!(content.len(), 2, "one node per construct: {content:?}");
}
