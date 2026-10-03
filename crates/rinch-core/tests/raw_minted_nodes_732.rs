//! Nodes the ownership records say nothing about, under branch-built markup
//! (issue #732): raw backend mints (the editor view, `Element::Html`'s
//! `parse_html`, DevTools) belong to the owned node they sit under, as they
//! did before #732. Plus retention over many cycles, and the cache rule: content
//! that must outlive a branch is built with `RenderScope::new`, never with the
//! branch's scope as parent.
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::show_dom;
use std::cell::RefCell;
use std::rc::Rc;

fn run(raw_children: usize, use_parse_html: bool) -> isize {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body_id = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut sc = RenderScope::new(dyn_doc.clone(), body_id);
    let body = NodeHandle::new(body_id, Rc::downgrade(&dyn_doc));
    let visible = Signal::new(false);
    let dw = Rc::downgrade(&dyn_doc);
    show_dom(
        &mut sc,
        &body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let d = dw.upgrade().unwrap();
            for i in 0..raw_children {
                let id = if use_parse_html {
                    d.borrow_mut().parse_html(&format!("<p>{i}</p>")).unwrap()
                } else {
                    d.borrow_mut().create_element("p")
                };
                wrap.append_child(&NodeHandle::new(id, dw.clone()));
            }
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let base = doc.borrow().__node_count() as isize;
    for _ in 0..100 {
        visible.set(true);
        visible.set(false);
    }
    doc.borrow().__node_count() as isize - base
}

#[test]
fn raw_minted_children_of_branch_markup_are_reclaimed() {
    assert_eq!(
        run(10, false),
        0,
        "raw create_element children leak per hide"
    );
}

#[test]
fn parse_html_children_of_branch_markup_are_reclaimed() {
    assert_eq!(
        run(10, true),
        0,
        "parse_html (Element::Html) children leak per hide"
    );
}

// ── retention: nothing grows over 1000 cycles ───────────────────────────────
use rinch_core::dom::{__minted_by_len, __scope_parents_len};
use rinch_core::for_each_dom_typed;
use std::collections::HashMap;

struct Fx {
    doc: Rc<RefCell<MockDomDocument>>,
    dyn_doc: Rc<RefCell<dyn DomDocument>>,
    body: NodeHandle,
    sc: RenderScope,
}
fn fx() -> Fx {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body_id = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let sc = RenderScope::new(dyn_doc.clone(), body_id);
    let body = NodeHandle::new(body_id, Rc::downgrade(&dyn_doc));
    Fx {
        doc,
        dyn_doc,
        body,
        sc,
    }
}
fn tables() -> (isize, isize) {
    (__minted_by_len() as isize, __scope_parents_len() as isize)
}
fn report(name: &str, f: &Fx, t0: (isize, isize), n0: isize) -> (isize, isize, isize) {
    let t = tables();
    let n = f.doc.borrow().__node_count() as isize;
    let g = (n - n0, t.0 - t0.0, t.1 - t0.1);
    eprintln!(
        "{name}: nodes {:+}, minted_by {:+}, scopes {:+}",
        g.0, g.1, g.2
    );
    g
}

#[test]
fn retention_captured_panel_reshown_1000_times() {
    let mut f = fx();
    let panel = f.sc.create_element("section");
    panel.append_child(&f.sc.create_text("p"));
    let visible = Signal::new(false);
    let p = panel.clone();
    show_dom(
        &mut f.sc,
        &f.body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let d = s.create_element("div");
            d.append_child(&p);
            d
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let (t0, n0) = (tables(), f.doc.borrow().__node_count() as isize);
    for _ in 0..1000 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(report("captured panel", &f, t0, n0), (0, 0, 0));
}

#[test]
fn retention_panel_from_a_dropped_throwaway_scope_reshown_1000_times() {
    let mut f = fx();
    let panel = {
        let mut t = RenderScope::new(f.dyn_doc.clone(), f.body.node_id());
        let p = t.create_element("section");
        p.append_child(&t.create_text("p"));
        p
    };
    let visible = Signal::new(false);
    let p = panel.clone();
    show_dom(
        &mut f.sc,
        &f.body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let d = s.create_element("div");
            d.append_child(&p);
            d
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(true);
    visible.set(false);
    let (t0, n0) = (tables(), f.doc.borrow().__node_count() as isize);
    for _ in 0..1000 {
        visible.set(true);
        visible.set(false);
    }
    assert_eq!(report("throwaway-scope panel", &f, t0, n0), (0, 0, 0));
    assert!(
        f.doc.borrow().tag_name(panel.node_id()).is_some(),
        "panel lost"
    );
}

/// A memoised for-row cache built outside the view (the caller's scope).
#[test]
fn retention_memoised_for_row_cache_1000_cycles() {
    let mut f = fx();
    let cache: Rc<RefCell<HashMap<u32, NodeHandle>>> = Rc::default();
    for k in 0..10u32 {
        let r = f.sc.create_element("li");
        r.append_child(&f.sc.create_text(&k.to_string()));
        cache.borrow_mut().insert(k, r);
    }
    let items = Signal::new((0..10u32).collect::<Vec<_>>());
    let c = cache.clone();
    let list = f.sc.create_element("ul");
    f.body.append_child(&list);
    for_each_dom_typed(
        &mut f.sc,
        &list,
        move || items.get(),
        |k: &u32| k.to_string(),
        move |k: u32, s: &mut RenderScope| {
            let w = s.create_element("div");
            w.append_child(&c.borrow()[&k]);
            w
        },
    );
    items.set(vec![]);
    items.set((0..10).collect());
    let (t0, n0) = (tables(), f.doc.borrow().__node_count() as isize);
    for i in 0..1000u32 {
        items.set(if i % 2 == 0 {
            (0..5).collect()
        } else {
            (0..10).rev().collect()
        });
    }
    items.set((0..10).collect());
    assert_eq!(report("memoised for cache", &f, t0, n0), (0, 0, 0));
}

/// Fresh keys every cycle, a for inside a branch inside a for: 1000 cycles.
#[test]
fn retention_fresh_keys_in_a_branch_1000_cycles() {
    let mut f = fx();
    let visible = Signal::new(true);
    let gen_ = Signal::new(0u32);
    show_dom(
        &mut f.sc,
        &f.body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let ul = s.create_element("ul");
            for_each_dom_typed(
                s,
                &ul,
                move || {
                    let g = gen_.get();
                    (g * 5..g * 5 + 5).collect::<Vec<u32>>()
                },
                |k: &u32| k.to_string(),
                |k: u32, rs: &mut RenderScope| {
                    let li = rs.create_element("li");
                    li.append_child(&rs.create_text(&k.to_string()));
                    li
                },
            );
            ul
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    let (t0, n0) = (tables(), f.doc.borrow().__node_count() as isize);
    for i in 1..=1000u32 {
        gen_.set(i);
        if i % 10 == 0 {
            visible.set(false);
            visible.set(true);
        }
    }
    gen_.set(0);
    assert_eq!(report("fresh keys in a branch", &f, t0, n0), (0, 0, 0));
}

// ── a cache built during the branch's render ───────────────────────────────
fn cache_survives(name_parent: bool) -> bool {
    let mut f = fx();
    let visible = Signal::new(true);
    let cache: Rc<RefCell<Option<NodeHandle>>> = Rc::default();
    let c = cache.clone();
    let dd = f.dyn_doc.clone();
    show_dom(
        &mut f.sc,
        &f.body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            let panel = c
                .borrow_mut()
                .get_or_insert_with(|| {
                    let parent = if name_parent { Some(s.id()) } else { None };
                    let mut cs = RenderScope::with_parent(dd.clone(), wrap.node_id(), parent);
                    let p = cs.create_element("section");
                    p.append_child(&cs.create_text("kept"));
                    p
                })
                .clone();
            wrap.append_child(&panel);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    visible.set(false);
    visible.set(true);
    let id = cache.borrow().as_ref().unwrap().node_id();
    f.doc.borrow().tag_name(id).is_some()
}

#[test]
fn a_cache_built_through_new_survives_a_hide() {
    assert!(cache_survives(false));
}

/// Naming the branch's scope as the parent of content meant to outlive the
/// branch (and so what defaulting `new` to the rendering scope would do)
/// discards it under the cache. Pinned so the guide's rule has a witness.
#[test]
fn a_cache_built_through_with_parent_of_the_branch_scope_is_lost() {
    assert!(!cache_survives(true), "expected the loss the guide invites");
}

/// A node minted by raw backend access OUTSIDE any branch, captured, and nested
/// inside branch-built markup has no ownership record, so it is read as the
/// branch's and discarded with the wrapper -- what every branch did with any
/// nested handle before #732. Only an unrecorded capture is affected; anything
/// built through a scope (every `rsx!` node) is recorded and survives.
#[test]
fn a_raw_minted_handle_captured_into_branch_markup_is_discarded_as_before_732() {
    let f = fx();
    let mut sc = f.sc;
    let raw_id = f.dyn_doc.borrow_mut().create_element("aside");
    let panel = NodeHandle::new(raw_id, Rc::downgrade(&f.dyn_doc));
    let visible = Signal::new(true);
    let p = panel.clone();
    show_dom(
        &mut sc,
        &f.body,
        move || visible.get(),
        move |s: &mut RenderScope| {
            let wrap = s.create_element("div");
            wrap.append_child(&p);
            wrap
        },
        None::<fn(&mut RenderScope) -> NodeHandle>,
    );
    assert_eq!(f.doc.borrow().tag_name(raw_id).as_deref(), Some("aside"));
    visible.set(false);
    assert_eq!(
        f.doc.borrow().tag_name(raw_id),
        None,
        "an unrecorded node under branch markup is the branch's and is retired"
    );
}
