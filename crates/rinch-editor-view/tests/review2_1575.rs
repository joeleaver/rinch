//! Round-2 adversarial review of PR #1575 (node views part B): concurrency
//! histories with embeds, through real `EditorHandle`s with mounted views.
//!
//! The invariant of decision (d), checked after every step on every peer: a live
//! view's node is the node it was mounted for. Every embed carries a `data-eid`
//! no edit ever changes, so a view whose `ctx.attrs` shows another `data-eid` (or
//! whose position holds a node with another one) has been handed another node.
#![cfg(feature = "collaboration")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_editor_collab::CollabSession;
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Pos, Schema, Selection, Slice};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

struct Mount {
    eid: String,
    ctx: NodeViewCtx,
    disposed: Rc<Cell<bool>>,
}

#[derive(Default)]
struct Views {
    mounts: RefCell<Vec<Mount>>,
}

impl Views {
    fn renders(&self) -> usize {
        self.mounts.borrow().len()
    }
    fn live(&self) -> Vec<(usize, Pos)> {
        self.mounts
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(i, m)| m.ctx.pos().map(|p| (i, p)))
            .collect()
    }
}

fn spec(views: &Rc<Views>) -> NodeViewSpec {
    let v = views.clone();
    NodeViewSpec::new("embed", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
        let eid = ctx
            .attrs
            .get()
            .get_str("data-eid")
            .unwrap_or("")
            .to_string();
        let disposed = Rc::new(Cell::new(false));
        let d = disposed.clone();
        rinch_core::reactive::on_cleanup(move || d.set(true));
        v.mounts.borrow_mut().push(Mount { eid, ctx, disposed });
        scope.create_element("div")
    })
}

struct Peer {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    views: Rc<Views>,
    out: Rc<RefCell<Vec<Vec<u8>>>>,
    name: &'static str,
}

fn page(html: &str) -> (Rc<RefCell<MockDomDocument>>, RenderScope, EditorHandle) {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    assert!(handle.load_html(html));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    (doc, scope, handle)
}

fn peers(html: &str, n: usize) -> Vec<Peer> {
    const NAMES: [&str; 3] = ["A", "B", "C"];
    let (d, s, h) = page(html);
    let out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let o = out.clone();
    let snapshot = h
        .start_collaboration_host(move |x| o.borrow_mut().push(x))
        .unwrap();
    let views = Rc::new(Views::default());
    h.register_node_view(spec(&views)).keep();
    let mut v = vec![Peer {
        _doc: d,
        _scope: s,
        handle: h,
        views,
        out,
        name: NAMES[0],
    }];
    for name in NAMES.iter().take(n).skip(1) {
        let (d, s, h) = page("<p></p>");
        let out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let o = out.clone();
        h.start_collaboration_guest(&snapshot, move |x| o.borrow_mut().push(x))
            .unwrap();
        let views = Rc::new(Views::default());
        h.register_node_view(spec(&views)).keep();
        v.push(Peer {
            _doc: d,
            _scope: s,
            handle: h,
            views,
            out,
            name,
        });
    }
    v
}

/// Decision (d), on one peer: every live view stands on the node it was mounted
/// for, and its memo shows that node's attributes.
fn check(p: &Peer, step: &str) {
    let doc = p.handle.doc();
    for (i, m) in p.views.mounts.borrow().iter().enumerate() {
        let Some(pos) = m.ctx.pos() else { continue };
        assert!(
            !m.disposed.get(),
            "{} {step}: mount {i} disposed but live",
            p.name
        );
        let node = doc
            .node_at(pos.0)
            .unwrap_or_else(|| panic!("{} {step}: mount {i} at {pos:?}: no node", p.name));
        assert_eq!(
            node.attrs().get_str("data-eid"),
            Some(m.eid.as_str()),
            "{} {step}: mount {i} (for {}) stands on another node: {:?}",
            p.name,
            m.eid,
            node.attrs()
        );
        assert_eq!(
            m.ctx.attrs.get().get_str("data-eid"),
            Some(m.eid.as_str()),
            "{} {step}: mount {i} (for {}) was handed {:?}",
            p.name,
            m.eid,
            m.ctx.attrs.get()
        );
        assert_eq!(
            &m.ctx.attrs.get(),
            node.attrs(),
            "{} {step}: mount {i}'s memo does not follow its node",
            p.name
        );
    }
    // Every embed in the document has exactly one live view.
    let mut embeds = Vec::new();
    doc.nodes_between(
        0,
        doc.content_size(),
        &mut |n: &Node, pos: usize, _: Option<&Node>| {
            if n.type_name() == "embed" {
                embeds.push(pos);
            }
            true
        },
    );
    let mut live: Vec<usize> = p.views.live().iter().map(|(_, p)| p.0).collect();
    live.sort();
    assert_eq!(live, embeds, "{} {step}: one live view per embed", p.name);
}

/// The CRDT's read equals the model (`model ≡ project(model)`), once quiet.
fn check_projection(p: &Peer) {
    if p.handle.collab_outbound_stall().is_some() {
        return;
    }
    let schema: Schema = p.handle.state().schema().clone();
    let snap = p.handle.collab_snapshot().unwrap();
    let s = CollabSession::from_bytes(&snap).unwrap();

    assert_eq!(
        shape(&s.projected_doc(&schema).unwrap()),
        shape(&p.handle.doc()),
        "{}: model ≢ project(model)",
        p.name
    );
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Top-level positions of the embeds and the textblocks of `doc`.
fn top(doc: &Node) -> (Vec<(usize, Node)>, Vec<(usize, Node)>) {
    let (mut embeds, mut texts) = (Vec::new(), Vec::new());
    let mut pos = 0;
    for i in 0..doc.child_count() {
        let c = doc.child(i).clone();
        let size = c.node_size();
        if c.type_name() == "embed" {
            embeds.push((pos, c));
        } else if c.is_textblock() {
            texts.push((pos, c));
        }
        pos += size;
    }
    (embeds, texts)
}

/// A channel per ordered pair: (from, to) → FIFO of deltas.
struct Net {
    q: Vec<Vec<std::collections::VecDeque<Vec<u8>>>>,
}

impl Net {
    fn new(n: usize) -> Net {
        Net {
            q: (0..n)
                .map(|_| (0..n).map(|_| Default::default()).collect())
                .collect(),
        }
    }
    fn collect(&mut self, ps: &[Peer]) {
        for (i, p) in ps.iter().enumerate() {
            for d in p.out.borrow_mut().drain(..) {
                for j in 0..ps.len() {
                    if j != i {
                        self.q[i][j].push_back(d.clone());
                    }
                }
            }
        }
    }
    fn pending(&self) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for i in 0..self.q.len() {
            for j in 0..self.q.len() {
                if !self.q[i][j].is_empty() {
                    v.push((i, j));
                }
            }
        }
        v
    }
}

/// Delete every top-level task list of `p`'s document; whether there was one.
fn remove_task_lists(p: &Peer) -> bool {
    let mut any = false;
    loop {
        let doc = p.handle.doc();
        let mut pos = 0;
        let mut found = None;
        for i in 0..doc.child_count() {
            let c = doc.child(i);
            if c.type_name() == "task_list" {
                found = Some((pos, pos + c.node_size()));
                break;
            }
            pos += c.node_size();
        }
        let Some((from, to)) = found else { return any };
        any = true;
        let last = doc.child_count() == 1;
        assert!(p.handle.update(|st| {
            let mut tr = st.tr();
            if last {
                let para = st.schema().branch("paragraph", Fragment::empty()).ok()?;
                tr.replace(from, to, Slice::new(Fragment::from_node(para), 0, 0))
                    .ok()?;
            } else {
                tr.delete(from, to).ok()?;
            }
            Some(tr)
        }));
    }
}

/// Deliver everything in flight, then reconcile every pair.
fn quiesce(net: &mut Net, ps: &[Peer]) {
    loop {
        net.collect(ps);
        let pend = net.pending();
        if pend.is_empty() {
            break;
        }
        for (i, j) in pend {
            while let Some(d) = net.q[i][j].pop_front() {
                ps[j].handle.collab_receive(&d);
            }
        }
    }
    for i in 0..ps.len() {
        for j in 0..ps.len() {
            if i != j {
                let sv = ps[j].handle.collab_state_vector().unwrap();
                let diff = ps[i].handle.collab_sync_diff(&sv).unwrap();
                ps[j].handle.collab_receive(&diff);
            }
        }
    }
}

fn run_seed(seed: u64, n: usize, steps: usize) {
    let html = r#"<p>aa</p><div data-pm-type="embed" data-eid="e0" data-k="x"></div><p>bb</p><div data-pm-type="embed" data-eid="e1"></div><p>cc</p><div data-pm-type="embed" data-eid="e2" data-k="x"></div><p>dd</p>"#;
    let ps = peers(html, n);
    let mut net = Net::new(n);
    let mut rng = Rng(seed * 2654435761 + 1);
    let mut fresh = 0;
    for step in 0..steps {
        let who = rng.below(n);
        let p = &ps[who];
        let doc = p.handle.doc();
        let (embeds, texts) = top(&doc);
        let op = rng.below(13);
        let label = format!("seed {seed} step {step} {} op {op}", p.name);
        match op {
            0 | 1 => {
                if let Some((pos, t)) = texts.get(rng.below(texts.len().max(1))) {
                    let at = pos + 1 + rng.below(t.content_size() + 1);
                    p.handle.set_selection(Selection::cursor(Pos(at)));
                    p.handle.insert_text("z");
                }
            }
            2 | 3 => {
                // a view sets an attribute of its node
                let live = p.views.live();
                if !live.is_empty() {
                    let (i, _) = live[rng.below(live.len())];
                    let key = format!("data-k{}", rng.below(3));
                    let val = format!("{}{step}", p.name);
                    let ctx = p.views.mounts.borrow()[i].ctx.clone();
                    ctx.set_attrs(|a| *a = a.with(key.as_str(), AttrValue::from(val.as_str())));
                }
            }
            4 => {
                // move an embed (same node) to another top-level boundary
                if !embeds.is_empty() {
                    let (from, node) = embeds[rng.below(embeds.len())].clone();
                    let mut bounds = vec![0];
                    let mut acc = 0;
                    for i in 0..doc.child_count() {
                        acc += doc.child(i).node_size();
                        bounds.push(acc);
                    }
                    let to = bounds[rng.below(bounds.len())];
                    p.handle.update(|st| {
                        let mut tr = st.tr();
                        tr.delete(from, from + 1).ok()?;
                        let to = tr.mapping().map(to, 1);
                        tr.replace(to, to, Slice::new(Fragment::from_node(node.clone()), 0, 0))
                            .ok()?;
                        Some(tr)
                    });
                }
            }
            5 => {
                let live = p.views.live();
                if !live.is_empty() && rng.below(3) == 0 {
                    let (i, _) = live[rng.below(live.len())];
                    let ctx = p.views.mounts.borrow()[i].ctx.clone();
                    ctx.delete();
                }
            }
            6 => {
                if let Some((pos, t)) = texts.get(rng.below(texts.len().max(1))) {
                    p.handle
                        .set_selection(Selection::cursor(Pos(pos + 1 + t.content_size())));
                    fresh += 1;
                    let eid = format!("{}n{fresh}", p.name);
                    p.handle.insert_embed(Attrs::from_iter([(
                        "data-eid",
                        AttrValue::from(eid.as_str()),
                    )]));
                }
            }
            7 => {
                p.handle.command("undo");
            }
            8 => {
                p.handle.command("redo");
            }
            9 => {
                if let Some((pos, t)) = texts.get(rng.below(texts.len().max(1))) {
                    let at = pos + 1 + rng.below(t.content_size() + 1);
                    p.handle.set_selection(Selection::cursor(Pos(at)));
                    p.handle.command("splitBlock");
                }
            }
            10 => {
                // reconcile: a peer asks another for what it lacks
                let other = rng.below(n);
                if other != who {
                    let sv = p.handle.collab_state_vector().unwrap();
                    let diff = ps[other].handle.collab_sync_diff(&sv).unwrap();
                    p.handle.collab_receive(&diff);
                }
            }
            11 => {
                // an outbound stall (#220) and its cure: a task list is outside
                // collaboration's scope (review 2 of #1575, N1)
                if !remove_task_lists(p) {
                    if let Some((pos, _)) = texts.get(rng.below(texts.len().max(1))) {
                        p.handle.set_selection(Selection::cursor(Pos(pos + 1)));
                        p.handle.command("toggleTaskList");
                    }
                }
            }
            _ => {}
        }
        net.collect(&ps);
        // deliver a few messages
        for _ in 0..rng.below(3) {
            let pend = net.pending();
            if pend.is_empty() {
                break;
            }
            let (i, j) = pend[rng.below(pend.len())];
            let d = net.q[i][j].pop_front().unwrap();
            ps[j].handle.collab_receive(&d);
            net.collect(&ps);
            check(
                &ps[j],
                &format!("{label} deliver {}→{}", ps[i].name, ps[j].name),
            );
        }
        for q in &ps {
            check(q, &label);
            assert!(
                !q.handle.is_collaboration_poisoned(),
                "{label}: {} poisoned",
                q.name
            );
        }
    }
    // everything delivered first: a replica whose CRDT waits on a missing
    // update cannot re-base, so it cannot recover until that update arrives
    quiesce(&mut net, &ps);
    // cure every stall: no task list, and one projectable edit
    for q in &ps {
        remove_task_lists(q);
        if q.handle.collab_outbound_stall().is_some() {
            let (_, texts) = top(&q.handle.doc());
            let at = texts.first().map_or(1, |(pos, _)| pos + 1);
            q.handle.set_selection(Selection::cursor(Pos(at)));
            q.handle.insert_text("q");
        }
        assert!(
            q.handle.collab_outbound_stall().is_none(),
            "seed {seed}: {} still stalled: {:?} {:?}",
            q.name,
            q.handle.collab_outbound_stall(),
            shape(&q.handle.doc())
        );
    }
    quiesce(&mut net, &ps);
    for q in &ps {
        check(q, &format!("seed {seed} end"));
        check_projection(q);
    }
    for q in &ps[1..] {
        if shape(&q.handle.doc()) != shape(&ps[0].handle.doc()) {
            eprintln!(
                "A stall {:?} {} stall {:?}",
                ps[0].handle.collab_outbound_stall(),
                q.name,
                q.handle.collab_outbound_stall()
            );
            for (x, y) in shape(&ps[0].handle.doc())
                .iter()
                .zip(shape(&q.handle.doc()).iter())
            {
                eprintln!("{} {x}\n   {y}", if x == y { " " } else { "!" });
            }
            panic!("seed {seed}: not converged");
        }
    }
}

#[test]
fn fuzz_two_peers() {
    let seeds: u64 = std::env::var("R2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let first: u64 = std::env::var("R2_FIRST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    for seed in first..first + seeds {
        run_seed(seed, 2, 60);
    }
}

#[test]
fn fuzz_three_peers() {
    let seeds: u64 = std::env::var("R2_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(40);
    let first: u64 = std::env::var("R2_FIRST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);
    for seed in first..first + seeds {
        run_seed(seed, 3, 60);
    }
}

fn shape(doc: &Node) -> Vec<String> {
    let mut v = Vec::new();
    doc.nodes_between(
        0,
        doc.content_size(),
        &mut |n: &Node, _pos: usize, _: Option<&Node>| {
            if n.is_text() {
                v.push(format!("{:?}", n));
            } else {
                v.push(format!("{} {:?} {:?}", n.type_name(), n.attrs(), n.marks()));
            }
            true
        },
    );
    v
}

// --- targeted histories -------------------------------------------------------

fn deliver_all(ps: &[Peer]) {
    loop {
        let mut any = false;
        for (i, p) in ps.iter().enumerate() {
            let ds: Vec<_> = p.out.borrow_mut().drain(..).collect();
            for d in ds {
                any = true;
                for (j, q) in ps.iter().enumerate() {
                    if i != j {
                        assert!(q.handle.collab_receive(&d) || true);
                    }
                }
            }
        }
        if !any {
            break;
        }
    }
}

fn move_first_embed_after_second(p: &Peer, at: usize) {
    let first = p.handle.doc().node_at(at).unwrap();
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(at, at + 1).ok()?;
        tr.replace(
            at + 1,
            at + 1,
            Slice::new(Fragment::from_node(first.clone()), 0, 0),
        )
        .ok()?;
        Some(tr)
    }));
}

/// S1 — R10b while the receiver's outbound is stalled (#220). B holds a task
/// list (outside collaboration's scope), so its session integrates with no atom
/// ids; A moves the first of two identical embeds after the second. Under the
/// fix round's reading of R10b, B's view of the moved embed must be gone (its
/// CRDT block was deleted) — it is not, and once B's stall clears, its two views
/// stand on each other's nodes for good.
#[test]
fn s1_an_identical_swap_received_while_stalled_keeps_views_on_deleted_blocks() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    // ab 0..4, embeds 4 and 5, cd 6..10, todo 10..16.
    b.handle.set_selection(Selection::cursor(Pos(12)));
    assert!(b.handle.command("toggleTaskList"));
    assert!(b.handle.collab_outbound_stall().is_some(), "B is stalled");
    move_first_embed_after_second(a, 4);
    let ds: Vec<_> = a.out.borrow_mut().drain(..).collect();
    for d in &ds {
        b.handle.collab_receive(d);
    }
    eprintln!("B renders {} live {:?}", b.views.renders(), b.views.live());
    assert_eq!(
        b.views.mounts.borrow()[0].ctx.pos(),
        None,
        "B's view of E1 stands on a deleted CRDT block (as in R10b without a stall)"
    );
}

/// S1b — the consequence: after B's stall clears, B's views edit the other node
/// on A.
#[test]
fn s1b_after_the_stall_clears_bs_views_edit_the_same_nodes_on_a() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    b.handle.set_selection(Selection::cursor(Pos(12)));
    assert!(b.handle.command("toggleTaskList"));
    move_first_embed_after_second(a, 4);
    deliver_all(&ps);
    assert!(
        b.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(10, st.doc.content_size()).ok()?;
            Some(tr)
        }),
        "B removes the task list"
    );
    assert!(
        b.handle.collab_outbound_stall().is_none(),
        "B recovered: {:?} {:?}",
        b.handle.collab_outbound_stall(),
        shape(&b.handle.doc())
    );
    deliver_all(&ps);
    let live = b.views.live();
    for (i, _) in &live {
        let tag = format!("B{i}");
        let ctx = b.views.mounts.borrow()[*i].ctx.clone();
        assert!(ctx.set_attrs(|x| *x = x.with("data-tag", AttrValue::from(tag.as_str()))));
    }
    deliver_all(&ps);
    // A's mount 0 is E1 (moved second), mount 1 is E2. B's mount i was mounted
    // for B's E_{i+1}. A tag from B's view of E1 must reach A's view of E1.
    for (i, m) in a.views.mounts.borrow().iter().enumerate().take(2) {
        let got = m.ctx.attrs.get().get_str("data-tag").map(str::to_string);
        if let Some(g) = &got {
            let from: usize = g[1..].parse().unwrap();
            assert!(
                from >= 2 || from == i,
                "B's view of E{} edited A's view of E{}",
                from + 1,
                i + 1
            );
        }
    }
}

/// S2 — requirement 2 while stalled: a peer's attribute change remounts.
#[test]
fn s2_a_peers_attr_change_while_stalled_keeps_the_mount() {
    let e = r#"<div data-pm-type="embed" data-eid="e0" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    b.handle.set_selection(Selection::cursor(Pos(11)));
    assert!(b.handle.command("toggleTaskList"));
    assert!(b.handle.collab_outbound_stall().is_some());
    let ctx = a.views.mounts.borrow()[0].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2"))));
    deliver_all(&ps);
    assert_eq!(
        b.handle.doc().child(1).attrs().get_str("data-sheet"),
        Some("s2")
    );
    assert_eq!(b.views.renders(), 1, "no remount");
}

/// S3 — an oversized-table freeze with an embed: the embed's views stay mounted
/// through the freeze, the cure, and a later attribute change.
#[test]
fn s3_a_freeze_and_its_cure_keep_an_embeds_views() {
    use yrs::updates::decoder::Decode;
    use yrs::{Any, Array, ArrayRef, Map, MapPrelim, Out, ReadTxn, Transact, Update};
    fn grown(snapshot: &[u8]) -> Vec<u8> {
        let doc = yrs::Doc::with_client_id(999);
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(snapshot).unwrap())
                .unwrap();
        }
        let sv = doc.transact().state_vector();
        {
            let content: ArrayRef = doc.get_or_insert_array("content");
            let mut txn = doc.transact_mut();
            let Some(Out::YMap(table)) = content.get(&txn, 0) else {
                panic!("no table")
            };
            let Some(Out::YArray(cols)) = table.get(&txn, "cols") else {
                panic!()
            };
            let Some(Out::YArray(rows)) = table.get(&txn, "rows") else {
                panic!()
            };
            for i in 0..2000 {
                let m = cols.insert(&mut txn, 1, MapPrelim::default());
                m.insert(&mut txn, "id", Any::String(format!("c{i}").into()));
                let r = rows.insert(&mut txn, 1, MapPrelim::default());
                r.insert(&mut txn, "id", Any::String(format!("r{i}").into()));
            }
        }
        doc.transact().encode_state_as_update_v1(&sv)
    }
    let e = r#"<div data-pm-type="embed" data-eid="e0" data-sheet="s1"></div>"#;
    let ps = peers(
        &format!("<table><tr><td>c</td></tr></table><p>ab</p>{e}<p>cd</p>"),
        2,
    );
    let (a, b) = (&ps[0], &ps[1]);
    let update = grown(&b.handle.collab_snapshot().unwrap());
    assert!(b.handle.collab_receive(&update));
    assert!(a.handle.collab_receive(&update));
    assert_eq!(a.handle.collab_oversized_tables().len(), 1);
    check(a, "frozen");
    check(b, "frozen");
    let ctx = a.views.mounts.borrow()[0].ctx.clone();
    assert!(
        !ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("no"))),
        "frozen"
    );
    let id = a.handle.collab_oversized_tables()[0].id.clone();
    assert_eq!(a.handle.collab_delete_oversized_table(&id), Ok(true));
    deliver_all(&ps);
    check(a, "cured");
    check(b, "cured");
    assert!(ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2"))));
    deliver_all(&ps);
    check(a, "after");
    check(b, "after");
    assert_eq!(
        (a.views.renders(), b.views.renders()),
        (1, 1),
        "no remount on either side"
    );
}

/// S4 — `load_html` while collaborating: the loading peer's view remounts (a
/// load is a new document), a peer whose embed the load left equal keeps its
/// view, and later attribute changes from either side keep both mounted.
#[test]
fn s4_load_html_while_collaborating() {
    let e = r#"<div data-pm-type="embed" data-eid="e0" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}<p>cd</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    assert!(a.handle.load_html(&format!("<p>new</p>{e}<p>cd</p>")));
    deliver_all(&ps);
    check(a, "loaded");
    check(b, "loaded");
    eprintln!("renders a {} b {}", a.views.renders(), b.views.renders());
    let (ra, rb) = (a.views.renders(), b.views.renders());
    let live = a.views.live();
    let ctx = a.views.mounts.borrow()[live[0].0].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2"))));
    deliver_all(&ps);
    let live = b.views.live();
    let ctx = b.views.mounts.borrow()[live[0].0].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-k", AttrValue::from("b"))));
    deliver_all(&ps);
    check(a, "after");
    check(b, "after");
    assert_eq!(
        (a.views.renders(), b.views.renders()),
        (ra, rb),
        "attr changes never remount"
    );
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
}

/// S5 — stop and rejoin: the guest leaves, the host keeps editing, the guest
/// rejoins from a fresh snapshot; the cache of the old session must not leak into
/// the new one (views keyed right; an attribute change keeps the mount).
#[test]
fn s5_stop_and_rejoin() {
    let e0 = r#"<div data-pm-type="embed" data-eid="e0"></div>"#;
    let e1 = r#"<div data-pm-type="embed" data-eid="e1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e0}{e1}<p>ef</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    b.handle.stop_collaboration();
    move_first_embed_after_second(a, 4); // e0 4, cd 5..9 → moves e0 to 5 (after cd's start?)
    a.out.borrow_mut().clear();
    let snap = a.handle.collab_snapshot().unwrap();
    let o = b.out.clone();
    b.handle
        .start_collaboration_guest(&snap, move |x| o.borrow_mut().push(x))
        .unwrap();
    check(b, "rejoined");
    let r = b.views.renders();
    let live = a.views.live();
    for (i, _) in live {
        let ctx = a.views.mounts.borrow()[i].ctx.clone();
        assert!(ctx.set_attrs(|x| *x = x.with("data-k", AttrValue::from("a"))));
    }
    deliver_all(&ps);
    check(a, "after");
    check(b, "after");
    assert_eq!(
        b.views.renders(),
        r,
        "a peer's attr change keeps B's mounts"
    );
}

/// S6 — an embed deleted by A while B sets its attributes: it stays deleted on
/// both, no view lives on either, and undo on A brings it back with a new view
/// on both (B's lost change stays lost, both converge).
#[test]
fn s6_delete_while_a_peer_sets_attrs_then_undo() {
    let e = r#"<div data-pm-type="embed" data-eid="e0" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}<p>cd</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    let ca = a.views.mounts.borrow()[0].ctx.clone();
    let cb = b.views.mounts.borrow()[0].ctx.clone();
    assert!(ca.delete());
    assert!(cb.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("B"))));
    deliver_all(&ps);
    check(a, "deleted");
    check(b, "deleted");
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
    assert!(
        a.views.live().is_empty() && b.views.live().is_empty(),
        "{:?} {:?}",
        shape(&b.handle.doc()),
        b.views.live()
    );
    assert!(a.handle.command("undo"));
    deliver_all(&ps);
    check(a, "undone");
    check(b, "undone");
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
    assert_eq!(a.views.live().len(), 1);
    assert_eq!(b.views.live().len(), 1);
    // B's undo of its (lost) attr change must not touch A's restored embed's view.
    let r = (a.views.renders(), b.views.renders());
    b.handle.command("undo");
    deliver_all(&ps);
    check(a, "b undo");
    check(b, "b undo");
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
    eprintln!(
        "renders before {r:?} after {:?}",
        (a.views.renders(), b.views.renders())
    );
}

/// S7 — an embed inside a table cell, beside one at the top level: a peer's
/// attribute changes to both keep every mount, and a typing peer in another cell
/// in the same update too.
#[test]
fn s7_embeds_in_a_table_cell() {
    let e0 = r#"<div data-pm-type="embed" data-eid="e0"></div>"#;
    let e1 = r#"<div data-pm-type="embed" data-eid="e1"></div>"#;
    let ps = peers(
        &format!(
            "<p>ab</p><table><tr><td><p>x</p>{e0}</td><td><p>y</p></td></tr></table>{e1}<p>cd</p>"
        ),
        2,
    );
    let (a, b) = (&ps[0], &ps[1]);
    check(a, "start");
    check(b, "start");
    assert_eq!(b.views.live().len(), 2, "{:?}", shape(&b.handle.doc()));
    let sv = b.handle.collab_state_vector().unwrap();
    for (i, _) in a.views.live() {
        let ctx = a.views.mounts.borrow()[i].ctx.clone();
        assert!(ctx.set_attrs(|x| *x = x.with("data-k", AttrValue::from("a"))));
    }
    // type in the second cell's paragraph
    let doc = a.handle.doc();
    let mut ypos = None;
    doc.nodes_between(
        0,
        doc.content_size(),
        &mut |n: &Node, pos: usize, _: Option<&Node>| {
            if n.text() == Some("y") {
                ypos = Some(pos);
            }
            true
        },
    );
    a.handle
        .set_selection(Selection::cursor(Pos(ypos.unwrap() + 1)));
    assert!(a.handle.insert_text("z"));
    a.out.borrow_mut().clear();
    let diff = a.handle.collab_sync_diff(&sv).unwrap();
    assert!(b.handle.collab_receive(&diff));
    check(b, "batched");
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
    assert_eq!(b.views.renders(), 2, "no remount in or beside a table");
}

/// S8 — the reaching case for `ViewDesc::update`'s "kept node was this desc's
/// node" check (it survived every mutant run without it): one transaction puts the
/// same `Rc` of embed E earlier (E's view follows it), sets an attribute on the
/// original E (a kept node, whose old descriptor is already taken), and replaces
/// the embed F after it with another. The kept node's position then meets F's
/// descriptor positionally; without the check F's view is handed E'.
#[test]
fn s8_a_kept_node_never_takes_another_nodes_descriptor() {
    let e = r#"<div data-pm-type="embed" data-eid="E"></div>"#;
    let f = r#"<div data-pm-type="embed" data-eid="F"></div>"#;
    let (d, s, h) = page(&format!("<p>ab</p>{e}{f}<p>cd</p>"));
    let views = Rc::new(Views::default());
    h.register_node_view(spec(&views)).keep();
    let p = Peer {
        _doc: d,
        _scope: s,
        handle: h,
        views,
        out: Rc::default(),
        name: "A",
    };
    // ab 0..4, E 4, F 5, cd 6..10
    let doc = p.handle.doc();
    let (en, schema) = (doc.child(1).clone(), p.handle.state().schema().clone());
    let f2 = schema
        .create_node(
            "embed",
            Attrs::from_iter([("data-eid", AttrValue::from("F2"))]),
            Fragment::empty(),
        )
        .unwrap();
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(0, 0, Slice::new(Fragment::from_node(en.clone()), 0, 0))
            .ok()?;
        // the original E is at 5 now, F at 6
        tr.step(Box::new(rinch_editor_core::SetNodeAttrStep::new(
            5,
            "data-k",
            AttrValue::from("x"),
        )))
        .ok()?;
        tr.replace(6, 7, Slice::new(Fragment::from_node(f2.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    check(&p, "after");
}

// --- the re-base that ends a stall (round 2's fix of N1) -----------------------

/// Tag every live view of `p` with `"{p.name}{mount index}"`.
fn tag_views(p: &Peer) {
    for (i, _) in p.views.live() {
        let tag = format!("{}{i}", p.name);
        let ctx = p.views.mounts.borrow()[i].ctx.clone();
        assert!(ctx.set_attrs(|x| *x = x.with("data-tag", AttrValue::from(tag.as_str()))));
    }
}

/// B stalls on a task list in `<p>todo</p>` (the last block) and later removes
/// it, which ends the stall with a re-base on the CRDT's read.
fn stall(b: &Peer) {
    let last = b.handle.doc().content_size() - 3;
    b.handle.set_selection(Selection::cursor(Pos(last)));
    assert!(b.handle.command("toggleTaskList"));
    assert!(b.handle.collab_outbound_stall().is_some(), "B is stalled");
}

fn cure(b: &Peer) {
    assert!(b.handle.update(|st| {
        let mut tr = st.tr();
        let n = st.doc.child_count();
        let from = st.doc.content_size() - st.doc.child(n - 1).node_size();
        tr.delete(from, st.doc.content_size()).ok()?;
        Some(tr)
    }));
    assert!(
        b.handle.collab_outbound_stall().is_none(),
        "B recovered: {:?}",
        b.handle.collab_outbound_stall()
    );
}

/// A's views live since before B's recovery carry only their own counterparts'
/// tags: B's view `i` and A's view `i` were mounted for one CRDT block.
fn assert_counterparts(a: &Peer, before: usize) {
    for (i, m) in a.views.mounts.borrow().iter().enumerate().take(before) {
        if m.ctx.pos().is_none() {
            continue;
        }
        if let Some(tag) = m.ctx.attrs.get().get_str("data-tag") {
            assert_eq!(
                tag,
                format!("B{i}"),
                "A's view {i} was edited by B's view {}",
                &tag[1..]
            );
        }
    }
}

/// S9 — B, stalled, pastes a copy of an embed (equal by value, a new node)
/// before the original, then recovers. The re-base pairs blocks by value, so the
/// copy would take the original's block: B's view of the copy would then write to
/// the block A's view of the original is keyed to.
#[test]
fn s9_a_copy_pasted_before_its_original_while_stalled() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    stall(b);
    assert!(b.handle.update(|st| {
        let original = st.doc.child(1).clone();
        let copy = original.copy_with_content(original.content().clone());
        let mut tr = st.tr();
        tr.replace(4, 4, Slice::new(Fragment::from_node(copy), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert_eq!(b.views.live().len(), 2);
    let before = a.views.mounts.borrow().len();
    cure(b);
    deliver_all(&ps);
    tag_views(b);
    deliver_all(&ps);
    assert_counterparts(a, before);
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
}

/// S10 — B, stalled, moves the first of two equal embeds after the second, then
/// recovers: each of B's views must still edit the node A's view of the same
/// embed is mounted for.
#[test]
fn s10_equal_embeds_reordered_while_stalled() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    stall(b);
    move_first_embed_after_second(b, 4);
    cure(b);
    deliver_all(&ps);
    tag_views(b);
    deliver_all(&ps);
    assert_counterparts(a, 2);
    assert_eq!(shape(&a.handle.doc()), shape(&b.handle.doc()));
}

/// S11 — what a stall proves is kept: B, stalled, types beside an embed and its
/// view sets an attribute; a peer's keystroke arrives; B recovers. A's view of the
/// embed is never remounted and shows B's attribute.
#[test]
fn s11_a_stall_that_moves_no_embed_remounts_none() {
    let e = r#"<div data-pm-type="embed" data-sheet="s1"></div>"#;
    let ps = peers(&format!("<p>ab</p>{e}<p>cd</p><p>todo</p>"), 2);
    let (a, b) = (&ps[0], &ps[1]);
    stall(b);
    b.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(b.handle.insert_text("x"));
    let ctx = b.views.mounts.borrow()[0].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2"))));
    a.handle.set_selection(Selection::cursor(Pos(7)));
    assert!(a.handle.insert_text("y"));
    deliver_all(&ps);
    assert_eq!(
        b.views.renders(),
        1,
        "B: no remount on the peer's keystroke"
    );
    // A peer's change brings a stalled replica's model back to the shared
    // document, local edits included (#220); the view's own change again:
    let ctx = b.views.mounts.borrow()[0].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-sheet", AttrValue::from("s2"))));
    cure(b);
    deliver_all(&ps);
    assert_eq!(a.views.renders(), 1, "A: no remount");
    assert_eq!(b.views.renders(), 1, "B: no remount");
    assert_eq!(
        a.views.mounts.borrow()[0]
            .ctx
            .attrs
            .get()
            .get_str("data-sheet"),
        Some("s2")
    );
}
