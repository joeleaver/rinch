//! Round-3 review of PR #1575 (node views part B): a fuzz whose embeds are
//! EQUAL BY VALUE, with an oracle that does not read attributes.
//!
//! Round 2's fuzz gave every embed a distinct `data-eid`, so a view re-pointed at
//! another embed of equal value (round 2's N1, the stall path) could not show.
//! Here every embed starts as `<div data-pm-type="embed" data-k="x">`, copies are
//! pasted, `insert_embed` makes more of the same, and attribute writes draw from
//! a two-value space, so equal embeds are the common case.
//!
//! The oracle is the CRDT itself: at every checkpoint where a peer's model is its
//! CRDT's read (not stalled, `model ≡ project(model)`), each live view is mapped
//! to the yrs block id of the embed it stands on (decoded straight from the
//! peer's snapshot). Decision (d) at the CRDT level:
//! * across remote deliveries only, a live view keeps its block (anything else
//!   should have remounted it);
//! * across a local operation of its own peer, a live view keeps its block or
//!   stands on a block first seen at or after that operation (one it wrote: a
//!   local move, the re-base's rewrite) — never on another pre-existing block.
#![cfg(feature = "collaboration")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Every block id a checkpoint has seen, with the order it was first seen in.
type Seen = HashMap<BlockId, usize>;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_editor_collab::CollabSession;
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Pos, Schema, Selection, Slice};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

type BlockId = (u64, u32);

thread_local! {
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Run one history; on failure dump its trace and return the panic message.
fn run_traced(seed: u64, n: usize, steps: usize) -> Option<String> {
    TRACE.with(|t| t.borrow_mut().clear());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_seed(seed, n, steps)));
    let e = r.err()?;
    let dump = TRACE.with(|t| t.borrow().join("\n"));
    let _ = std::fs::write(
        format!(
            "{}/../../.scratch/fail-{seed}.log",
            env!("CARGO_MANIFEST_DIR")
        ),
        dump,
    );
    Some(
        e.downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    )
}

fn run_all(range: std::ops::Range<u64>, n: usize) {
    let mut failed = Vec::new();
    for seed in range {
        if let Some(msg) = run_traced(seed, n, 60) {
            failed.push((seed, msg));
        }
    }
    eprintln!("R3 STATS {n} peers {:?}", STATS.with(|s| s.get()));
    assert!(
        failed.is_empty(),
        "{} histories failed: {failed:#?}",
        failed.len()
    );
}

thread_local! {
    /// (checkpoints compared, views compared, views that moved to a fresh block)
    static STATS: Cell<(u64, u64, u64)> = const { Cell::new((0, 0, 0)) };
}

fn stat(f: impl FnOnce(&mut (u64, u64, u64))) {
    STATS.with(|s| {
        let mut v = s.get();
        f(&mut v);
        s.set(v);
    });
}

struct Mount {
    ctx: NodeViewCtx,
    disposed: Rc<Cell<bool>>,
    block: Cell<Option<BlockId>>,
    /// `seen.len()` at its peer's first local op since the view's block was
    /// last recorded: a block first seen at or after it may be one that op wrote.
    local_mark: Cell<Option<usize>>,
}

#[derive(Default)]
struct Views {
    mounts: RefCell<Vec<Mount>>,
}

impl Views {
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
        let disposed = Rc::new(Cell::new(false));
        let d = disposed.clone();
        rinch_core::reactive::on_cleanup(move || d.set(true));
        v.mounts.borrow_mut().push(Mount {
            ctx,
            disposed,
            block: Cell::new(None),
            local_mark: Cell::new(None),
        });
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
    peers_seeded(html, n, None)
}

thread_local! {
    /// A test's own `R_ID_SALT` ([`client_id`]), for a history pinned by a test.
    static ID_SALT: Cell<Option<u64>> = const { Cell::new(None) };
}

/// A 53-bit yrs client id for peer `i` of fuzz seed `seed` (splitmix64), so the
/// merge order, and with it the whole run, is the same every time the seed runs,
/// while the order of the peers' ids (which breaks yrs's ties) varies from seed to
/// seed as random ids would. The peers of one seed never share one.
fn client_id(seed: u64, i: usize) -> u64 {
    let mix = |x: u64| {
        let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    // `R_ID_SALT` picks another set of ids for the same seeds (a search for an
    // order that fails); unset, the ids are a function of the seed alone.
    let salt: u64 = ID_SALT.with(Cell::get).unwrap_or_else(|| {
        std::env::var("R_ID_SALT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    });
    let id = |i: u64| {
        (mix(seed.wrapping_mul(4).wrapping_add(i) ^ salt.wrapping_mul(0xA24B_AED4_963E_E407))
            & ((1 << 53) - 1))
            | 1
    };
    let mine = id(i as u64);
    assert!(
        (0..i as u64).all(|j| id(j) != mine),
        "seed {seed}: two peers share an id"
    );
    mine
}

/// [`peers`], with every session's yrs client id pinned from `seed` when given
/// (`rinch_editor_collab::testing`): a seed then replays bit for bit.
fn peers_seeded(html: &str, n: usize, seed: Option<u64>) -> Vec<Peer> {
    const NAMES: [&str; 3] = ["A", "B", "C"];
    let (d, s, h) = page(html);
    let out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let o = out.clone();
    let snapshot = match seed {
        Some(seed) => {
            let session = rinch_editor_collab::testing::session_with_client_id(
                &h.state(),
                client_id(seed, 0),
            )
            .unwrap();
            h.start_collaboration_host_with(session, move |x| o.borrow_mut().push(x))
        }
        None => h
            .start_collaboration_host(move |x| o.borrow_mut().push(x))
            .unwrap(),
    };
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
    for (i, name) in NAMES.iter().enumerate().take(n).skip(1) {
        let (d, s, h) = page("<p></p>");
        let out: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let o = out.clone();
        match seed {
            Some(seed) => {
                let session = rinch_editor_collab::testing::session_from_bytes_with_client_id(
                    &snapshot,
                    client_id(seed, i),
                )
                .unwrap();
                h.start_collaboration_guest_with(session, move |x| o.borrow_mut().push(x))
            }
            None => h.start_collaboration_guest(&snapshot, move |x| o.borrow_mut().push(x)),
        }
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

/// The yrs block id of every top-level embed of a snapshot, in order.
fn embed_blocks(snapshot: &[u8]) -> Vec<BlockId> {
    use yrs::updates::decoder::Decode;
    use yrs::{Array, ArrayRef, Map, Out, Transact, Update};
    let doc = yrs::Doc::new();
    let content: ArrayRef = doc.get_or_insert_array("content");
    {
        let mut txn = doc.transact_mut();
        txn.apply_update(Update::decode_v1(snapshot).unwrap())
            .unwrap();
    }
    let txn = doc.transact();
    let mut out = Vec::new();
    for v in content.iter(&txn) {
        if let Out::YMap(m) = v {
            let ty = match m.get(&txn, "type") {
                Some(Out::Any(yrs::Any::String(s))) => s.to_string(),
                _ => String::new(),
            };
            if ty == "embed" {
                let br: &yrs::branch::Branch = m.as_ref();
                if let yrs::branch::BranchID::Nested(id) = br.id() {
                    out.push((id.client.get(), id.clock));
                }
            }
        }
    }
    out
}

/// Positions of the embeds of `doc`, in document order.
fn embed_positions(doc: &Node) -> Vec<usize> {
    let mut v = Vec::new();
    doc.nodes_between(
        0,
        doc.content_size(),
        &mut |n: &Node, pos: usize, _: Option<&Node>| {
            if n.type_name() == "embed" {
                v.push(pos);
            }
            true
        },
    );
    v
}

/// Local view invariants of (d), plus the CRDT-level checkpoint.
fn check(p: &Peer, seen: &RefCell<Seen>, step: &str) {
    {
        let doc = p.handle.doc();
        let mut row = Vec::new();
        for (i, m) in p.views.mounts.borrow().iter().enumerate() {
            if let Some(pos) = m.ctx.pos() {
                row.push(format!(
                    "m{i}@{}{:?}{}",
                    pos.0,
                    m.block.get().map(|b| b.1),
                    if m.local_mark.get().is_some() {
                        "L"
                    } else {
                        ""
                    }
                ));
            }
        }
        let line = format!(
            "{step}: stall={} {:?} doc={}",
            p.handle.collab_outbound_stall().is_some(),
            row,
            doc.content()
                .children()
                .iter()
                .map(|c| match c.type_name() {
                    "embed" => format!(
                        "E{:?}",
                        c.attrs()
                            .iter()
                            .map(|(k, v)| format!("{k}={v:?}"))
                            .collect::<Vec<_>>()
                    ),
                    "paragraph" => format!("p{}", c.content_size()),
                    t => t.to_string(),
                })
                .collect::<Vec<_>>()
                .join(" ")
        );
        TRACE.with(|t| t.borrow_mut().push(line));
    }
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
        assert_eq!(node.type_name(), "embed", "{} {step}: mount {i}", p.name);
        assert_eq!(
            &m.ctx.attrs.get(),
            node.attrs(),
            "{} {step}: mount {i}'s memo does not follow its node",
            p.name
        );
    }
    let embeds = embed_positions(&doc);
    let mut live: Vec<usize> = p.views.live().iter().map(|(_, p)| p.0).collect();
    live.sort();
    assert_eq!(live, embeds, "{} {step}: one live view per embed", p.name);

    // CRDT checkpoint: only where the model is the CRDT's read.
    if p.handle.collab_outbound_stall().is_some() && std::env::var("R3_STALLED_TOO").is_err() {
        return;
    }
    let snap = p.handle.collab_snapshot().unwrap();
    let schema: Schema = p.handle.state().schema().clone();
    // A replica holding updates parked on missing dependencies may not project
    // yet (transient, as the live session treats it): no checkpoint then.
    let Ok(projected) = CollabSession::from_bytes(&snap).and_then(|s| s.projected_doc(&schema))
    else {
        return;
    };
    if shape(&projected) != shape(&doc) {
        return;
    }
    let blocks = embed_blocks(&snap);
    if blocks.len() != embeds.len() {
        return;
    }
    let mut seen = seen.borrow_mut();
    stat(|s| s.0 += 1);
    for (i, m) in p.views.mounts.borrow().iter().enumerate() {
        let Some(pos) = m.ctx.pos() else { continue };
        let ord = embeds.iter().position(|&e| e == pos.0).unwrap();
        let now = blocks[ord];
        if m.block.get().is_some() {
            stat(|s| s.1 += 1);
        }
        if let Some(was) = m.block.get()
            && was != now
        {
            if let Some(mark) = m.local_mark.get() {
                stat(|s| s.2 += 1);
                assert!(
                    seen.get(&now).is_none_or(|&ix| ix >= mark),
                    "{} {step}: after its own peer's local op, mount {i} moved from block \
                     {was:?} to the pre-existing block {now:?} (another embed's)",
                    p.name
                );
            } else {
                panic!(
                    "{} {step}: across remote changes only, mount {i} moved from block \
                     {was:?} to {now:?} (not remounted)",
                    p.name
                );
            }
        }
        m.block.set(Some(now));
        m.local_mark.set(None);
    }
    for b in blocks {
        let n = seen.len();
        seen.entry(b).or_insert(n);
    }
}

fn local_op(p: &Peer, seen: &RefCell<Seen>) {
    let mark = seen.borrow().len();
    for m in p.views.mounts.borrow().iter() {
        if m.local_mark.get().is_none() {
            m.local_mark.set(Some(mark));
        }
    }
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

type Placed = Vec<(usize, Node)>;

fn top(doc: &Node) -> (Placed, Placed) {
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

fn bounds(doc: &Node) -> Vec<usize> {
    let mut b = vec![0];
    let mut acc = 0;
    for i in 0..doc.child_count() {
        acc += doc.child(i).node_size();
        b.push(acc);
    }
    b
}

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

fn quiesce(net: &mut Net, ps: &[Peer], seen: &RefCell<Seen>, label: &str) {
    loop {
        net.collect(ps);
        let pend = net.pending();
        if pend.is_empty() {
            break;
        }
        for (i, j) in pend {
            while let Some(d) = net.q[i][j].pop_front() {
                ps[j].handle.collab_receive(&d);
                check(
                    &ps[j],
                    seen,
                    &format!("{label} quiesce {}→{}", ps[i].name, ps[j].name),
                );
            }
        }
    }
    for i in 0..ps.len() {
        for j in 0..ps.len() {
            if i != j {
                let sv = ps[j].handle.collab_state_vector().unwrap();
                let diff = ps[i].handle.collab_sync_diff(&sv).unwrap();
                ps[j].handle.collab_receive(&diff);
                check(
                    &ps[j],
                    seen,
                    &format!("{label} sync {}→{}", ps[i].name, ps[j].name),
                );
            }
        }
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

fn run_seed(seed: u64, n: usize, steps: usize) {
    let e = r#"<div data-pm-type="embed" data-k="x"></div>"#;
    let html = format!("<p>aa</p>{e}<p>bb</p>{e}{e}<p>cc</p>{e}<p>dd</p>");
    let ps = peers_seeded(&html, n, Some(seed));
    let seen: RefCell<Seen> = RefCell::default();
    for q in &ps {
        check(q, &seen, &format!("seed {seed} start"));
    }
    let mut net = Net::new(n);
    let mut rng = Rng(seed * 2654435761 + 7);
    for step in 0..steps {
        let who = rng.below(n);
        let p = &ps[who];
        let doc = p.handle.doc();
        let (embeds, texts) = top(&doc);
        let op = rng.below(14);
        let label = format!("seed {seed} step {step} {} op {op}", p.name);
        local_op(p, &seen);
        match op {
            0 => {
                if let Some((pos, t)) = texts.get(rng.below(texts.len().max(1))) {
                    let at = pos + 1 + rng.below(t.content_size() + 1);
                    p.handle.set_selection(Selection::cursor(Pos(at)));
                    p.handle.insert_text("z");
                }
            }
            1 | 2 => {
                // a view sets an attribute from a two-value space: equal values recur
                let live = p.views.live();
                if !live.is_empty() {
                    let (i, _) = live[rng.below(live.len())];
                    let key = if rng.below(2) == 0 {
                        "data-k"
                    } else {
                        "data-j"
                    };
                    let val = if rng.below(2) == 0 { "x" } else { "y" };
                    let ctx = p.views.mounts.borrow()[i].ctx.clone();
                    ctx.set_attrs(|a| *a = a.with(key, AttrValue::from(val)));
                }
            }
            3 => {
                // paste a copy (equal by value, a new node) at a top-level boundary
                if !embeds.is_empty() {
                    let (_, node) = embeds[rng.below(embeds.len())].clone();
                    let copy = node.copy_with_content(node.content().clone());
                    let b = bounds(&doc);
                    let to = b[rng.below(b.len())];
                    p.handle.update(|st| {
                        let mut tr = st.tr();
                        tr.replace(to, to, Slice::new(Fragment::from_node(copy.clone()), 0, 0))
                            .ok()?;
                        Some(tr)
                    });
                }
            }
            4 => {
                // move an embed (same node) to another top-level boundary
                if !embeds.is_empty() {
                    let (from, node) = embeds[rng.below(embeds.len())].clone();
                    let b = bounds(&doc);
                    let to = b[rng.below(b.len())];
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
                if !live.is_empty() && rng.below(2) == 0 {
                    let (i, _) = live[rng.below(live.len())];
                    let ctx = p.views.mounts.borrow()[i].ctx.clone();
                    ctx.delete();
                }
            }
            6 => {
                if let Some((pos, t)) = texts.get(rng.below(texts.len().max(1))) {
                    p.handle
                        .set_selection(Selection::cursor(Pos(pos + 1 + t.content_size())));
                    p.handle
                        .insert_embed(Attrs::from_iter([("data-k", AttrValue::from("x"))]));
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
                let other = rng.below(n);
                if other != who {
                    let sv = p.handle.collab_state_vector().unwrap();
                    let diff = ps[other].handle.collab_sync_diff(&sv).unwrap();
                    p.handle.collab_receive(&diff);
                }
            }
            11 | 12 => {
                // an outbound stall (#220) and its cure
                if !remove_task_lists(p)
                    && let Some((pos, _)) = texts.get(rng.below(texts.len().max(1)))
                {
                    p.handle.set_selection(Selection::cursor(Pos(pos + 1)));
                    p.handle.command("toggleTaskList");
                }
            }
            _ => {}
        }
        check(p, &seen, &label);
        net.collect(&ps);
        for _ in 0..rng.below(4) {
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
                &seen,
                &format!("{label} deliver {}→{}", ps[i].name, ps[j].name),
            );
        }
        for q in &ps {
            assert!(
                !q.handle.is_collaboration_poisoned(),
                "{label}: {} poisoned",
                q.name
            );
        }
    }
    let label = format!("seed {seed} end");
    quiesce(&mut net, &ps, &seen, &label);
    for q in &ps {
        local_op(q, &seen);
        remove_task_lists(q);
        if q.handle.collab_outbound_stall().is_some() {
            let (_, texts) = top(&q.handle.doc());
            match texts.first() {
                Some((pos, _)) => {
                    q.handle.set_selection(Selection::cursor(Pos(pos + 1)));
                    q.handle.insert_text("q");
                }
                // A document of embeds only has no line to type in (as review2_1575's
                // seed 1546 met): append a paragraph instead.
                None => {
                    assert!(q.handle.update(|st| {
                        let s = st.schema();
                        let para = s
                            .branch("paragraph", Fragment::from_node(s.text("q").ok()?))
                            .ok()?;
                        let mut tr = st.tr();
                        let end = st.doc.content_size();
                        tr.replace(end, end, Slice::new(Fragment::from_node(para), 0, 0))
                            .ok()?;
                        Some(tr)
                    }));
                }
            }
        }
        assert!(
            q.handle.collab_outbound_stall().is_none(),
            "seed {seed}: {} still stalled",
            q.name
        );
        check(q, &seen, &format!("{label} cure {}", q.name));
    }
    quiesce(&mut net, &ps, &seen, &label);
    for q in &ps[1..] {
        assert_eq!(
            shape(&q.handle.doc()),
            shape(&ps[0].handle.doc()),
            "seed {seed}: {} not converged",
            q.name
        );
    }
    // every peer's views at quiescence stand on the same block as the same
    // position on every other peer (the docs are equal), checked by the oracle
}

fn seeds(default_first: u64) -> std::ops::Range<u64> {
    let n: u64 = std::env::var("R3_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let first: u64 = std::env::var("R3_FIRST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default_first);
    first..first + n
}

#[test]
fn r3_fuzz_equal_embeds_two_peers() {
    run_all(seeds(0), 2);
}

#[test]
fn r3_fuzz_equal_embeds_three_peers() {
    run_all(seeds(5000), 3);
}

// --- targeted histories found by the fuzz -------------------------------------

fn send(from: &Peer, to: &[&Peer]) {
    let ds: Vec<_> = from.out.borrow_mut().drain(..).collect();
    for d in &ds {
        for q in to {
            q.handle.collab_receive(d);
        }
    }
}

/// The (d) oracle for one peer, by view index: `(pos, block)` of every live view.
fn views_on_blocks(p: &Peer) -> Vec<(usize, usize, BlockId)> {
    let snap = p.handle.collab_snapshot().unwrap();
    let blocks = embed_blocks(&snap);
    let embeds = embed_positions(&p.handle.doc());
    if blocks.len() != embeds.len() {
        eprintln!(
            "{}: NOT mappable: model {} embeds, CRDT {} {:?}",
            p.name,
            embeds.len(),
            blocks.len(),
            blocks
        );
        return Vec::new();
    }
    p.views
        .live()
        .into_iter()
        .map(|(i, pos)| {
            let ord = embeds.iter().position(|&e| e == pos.0).unwrap();
            (i, pos.0, blocks[ord])
        })
        .collect()
}

/// T1 — seed 5102 of the three-peer fuzz, by hand. B, stalled, sets an
/// attribute on the second of two adjacent equal embeds and recovers by deleting
/// its task list (the re-base rewrites that embed as a new block). C, which has B's
/// recovery, moves the rewritten embed to the end. A receives C's move BEFORE B's
/// recovery (it waits on B's update), then B's.
fn t1(stall: bool) {
    let e = r#"<div data-pm-type="embed" data-k="x"></div>"#;
    let ps = peers(
        &format!("<p>aa</p>{e}<p>bb</p>{e}{e}<p>cc</p>{e}<p>dd</p>"),
        3,
    );
    let (a, b, c) = (&ps[0], &ps[1], &ps[2]);
    let a_before = views_on_blocks(a);
    eprintln!("A before {a_before:?}");
    if stall {
        // p(cc) is at 11..15
        b.handle.set_selection(Selection::cursor(Pos(12)));
        assert!(b.handle.command("toggleTaskList"));
        assert!(b.handle.collab_outbound_stall().is_some());
    }
    // B's view of the embed at 10 sets data-k=y
    let i = b
        .views
        .live()
        .into_iter()
        .find(|(_, p)| p.0 == 10)
        .unwrap()
        .0;
    let ctx = b.views.mounts.borrow()[i].ctx.clone();
    assert!(ctx.set_attrs(|x| *x = x.with("data-k", AttrValue::from("y"))));
    if stall {
        assert!(remove_task_lists(b));
        assert!(b.handle.collab_outbound_stall().is_none());
    } else {
        // the same document change as the recovery: p(cc) deleted
        assert!(b.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(11, 15).ok()?;
            Some(tr)
        }));
    }
    eprintln!("B {:?}", shape(&b.handle.doc()));
    let b_deltas: Vec<_> = b.out.borrow_mut().drain(..).collect();
    for d in &b_deltas {
        c.handle.collab_receive(d);
    }
    eprintln!("C after B {:?}", views_on_blocks(c));
    // C moves the y embed (at 10) to the end
    let doc = c.handle.doc();
    let node = doc.node_at(10).unwrap();
    assert_eq!(node.attrs().get_str("data-k"), Some("y"));
    let end = doc.content_size();
    assert!(c.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(10, 11).ok()?;
        let to = tr.mapping().map(end, 1);
        tr.replace(to, to, Slice::new(Fragment::from_node(node.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    eprintln!("C after move {:?}", views_on_blocks(c));
    {
        let ds: Vec<_> = c.out.borrow_mut().drain(..).collect();
        for d in &ds {
            let ok = a.handle.collab_receive(d);
            eprintln!(
                "A receives C: {ok} err={:?} model embeds={}",
                a.handle.collab_take_error(),
                embed_positions(&a.handle.doc()).len()
            );
        }
    }
    eprintln!("A after C (pending) {:?}", views_on_blocks(a));
    for d in &b_deltas {
        a.handle.collab_receive(d);
    }
    let a_after = views_on_blocks(a);
    eprintln!("A after B {a_after:?}");
    for (i, _, block) in &a_after {
        if let Some((_, _, was)) = a_before.iter().find(|(j, _, _)| j == i) {
            assert_eq!(
                block, was,
                "A's view {i} was mounted for block {was:?}, now on {block:?}"
            );
        }
    }
    assert_eq!(shape(&a.handle.doc()), shape(&c.handle.doc()));
}

#[test]
fn t1_out_of_order_after_a_stall_recovery() {
    t1(true);
}

#[test]
fn t1_out_of_order_without_a_stall() {
    t1(false);
}

/// T2 — T1 with no stall anywhere: B MOVES the embed (a new block), C moves it
/// again, and A receives C's move (which waits on B's update) before B's.
#[test]
fn t2_out_of_order_a_move_of_a_move() {
    let e = r#"<div data-pm-type="embed" data-k="x"></div>"#;
    let ps = peers(
        &format!("<p>aa</p>{e}<p>bb</p>{e}{e}<p>cc</p>{e}<p>dd</p>"),
        3,
    );
    let (a, b, c) = (&ps[0], &ps[1], &ps[2]);
    let a_before = views_on_blocks(a);
    // B moves the embed at 10 to after p(cc) (11..15 → boundary 15)
    let node = b.handle.doc().node_at(10).unwrap();
    assert!(b.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(10, 11).ok()?;
        let to = tr.mapping().map(15, 1);
        tr.replace(to, to, Slice::new(Fragment::from_node(node.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    let b_deltas: Vec<_> = b.out.borrow_mut().drain(..).collect();
    for d in &b_deltas {
        c.handle.collab_receive(d);
    }
    eprintln!("C after B {:?}", views_on_blocks(c));
    // C moves the embed B moved (now at 14) to the end
    let doc = c.handle.doc();
    let node = doc.node_at(14).unwrap();
    assert_eq!(node.type_name(), "embed");
    let end = doc.content_size();
    assert!(c.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(14, 15).ok()?;
        let to = tr.mapping().map(end, 1);
        tr.replace(to, to, Slice::new(Fragment::from_node(node.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    eprintln!("C after move {:?}", views_on_blocks(c));
    send(c, &[a]);
    eprintln!("A after C (pending) {:?}", views_on_blocks(a));
    for d in &b_deltas {
        a.handle.collab_receive(d);
    }
    let a_after = views_on_blocks(a);
    eprintln!("A after B {a_after:?}");
    for (i, _, block) in &a_after {
        if let Some((_, _, was)) = a_before.iter().find(|(j, _, _)| j == i) {
            assert_eq!(
                block, was,
                "A's view {i} was mounted for block {was:?}, now on {block:?}"
            );
        }
    }
    assert_eq!(shape(&a.handle.doc()), shape(&c.handle.doc()));
}

/// T3 — no stall anywhere, every peer healthy. B replaces the second of two
/// adjacent equal embeds (E3) with a paragraph. C, which has B's change, moves
/// the last embed (E4) to just before B's paragraph: its new block's right
/// neighbour is B's. A receives C's move first: the delete of E4's old block
/// integrates, the insert waits on B (the read fails, transient). Then B's.
/// A's view of E3 — whose block B deleted — must not survive on C's E4.
#[test]
fn t3_out_of_order_partial_apply_hands_a_view_another_embed() {
    let e = r#"<div data-pm-type="embed" data-k="x"></div>"#;
    let ps = peers(
        &format!("<p>aa</p>{e}<p>bb</p>{e}{e}<p>cc</p>{e}<p>dd</p>"),
        3,
    );
    let (a, b, c) = (&ps[0], &ps[1], &ps[2]);
    let a_before = views_on_blocks(a);
    assert!(b.handle.update(|st| {
        let para = st
            .schema()
            .branch(
                "paragraph",
                Fragment::from_node(st.schema().text("n").ok()?),
            )
            .ok()?;
        let mut tr = st.tr();
        tr.replace(10, 11, Slice::new(Fragment::from_node(para), 0, 0))
            .ok()?;
        Some(tr)
    }));
    let b_deltas: Vec<_> = b.out.borrow_mut().drain(..).collect();
    for d in &b_deltas {
        c.handle.collab_receive(d);
    }
    let doc = c.handle.doc();
    let node = doc.node_at(17).unwrap();
    assert_eq!(node.type_name(), "embed");
    assert!(c.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(17, 18).ok()?;
        tr.replace(10, 10, Slice::new(Fragment::from_node(node.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    eprintln!("C {:?}", views_on_blocks(c));
    let ds: Vec<_> = c.out.borrow_mut().drain(..).collect();
    for d in &ds {
        let ok = a.handle.collab_receive(d);
        eprintln!(
            "A receives C: {ok} err={:?} stalled={:?} poisoned={}",
            a.handle.collab_take_error(),
            a.handle.collab_outbound_stall(),
            a.handle.is_collaboration_poisoned()
        );
    }
    for d in &b_deltas {
        a.handle.collab_receive(d);
    }
    let a_after = views_on_blocks(a);
    eprintln!("A before {a_before:?}\nA after {a_after:?}");
    for (i, _, block) in &a_after {
        if let Some((_, _, was)) = a_before.iter().find(|(j, _, _)| j == i) {
            assert_eq!(
                block, was,
                "A's view {i} was mounted for block {was:?}, now on {block:?}"
            );
        }
    }
    assert_eq!(shape(&a.handle.doc()), shape(&c.handle.doc()));
}

// --- the task-list `div` fix and the other `div` hosts --------------------------

struct Page2 {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeHandle,
}

fn page2(html: &str) -> Page2 {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    assert!(handle.load_html(html));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page2 {
        doc,
        _scope: scope,
        handle,
        container,
    }
}

/// The `data-pm-type` of every top-level host the view drew, in order (the
/// editor's own overlays carry none).
fn host_types(p: &Page2) -> Vec<String> {
    let d = p.doc.borrow();
    d.get_children(p.container.node_id())
        .into_iter()
        .filter_map(|id| d.get_attribute(id, "data-pm-type"))
        .collect()
}

fn model_types(p: &Page2) -> Vec<String> {
    let doc = p.handle.doc();
    (0..doc.child_count())
        .map(|i| doc.child(i).type_name().to_string())
        .collect()
}

const TASKS: &str =
    r#"<ul data-type="taskList"><li data-type="taskItem" data-checked="false"><p>t</p></li></ul>"#;

fn replace_child(
    p: &Page2,
    i: usize,
    html_node: impl Fn(&rinch_editor_core::EditorState) -> Option<Node>,
) {
    assert!(p.handle.update(|st| {
        let mut from = 0;
        for k in 0..i {
            from += st.doc.child(k).node_size();
        }
        let to = from + st.doc.child(i).node_size();
        let node = html_node(st)?;
        let mut tr = st.tr();
        tr.replace(from, to, Slice::new(Fragment::from_node(node), 0, 0))
            .ok()?;
        Some(tr)
    }));
}

/// D1 — no node view registered: an embed replacing a task list at its place
/// (both hosts are `div`s) must not keep the task list's host.
#[test]
fn d1_an_unclaimed_embed_replacing_a_task_list_gets_its_own_host() {
    let p = page2(&format!("<p>ab</p>{TASKS}<p>cd</p>"));
    assert_eq!(host_types(&p), model_types(&p));
    replace_child(&p, 1, |st| {
        st.schema()
            .create_node("embed", Attrs::default(), Fragment::empty())
            .ok()
    });
    assert_eq!(model_types(&p)[1], "embed");
    assert_eq!(
        host_types(&p),
        model_types(&p),
        "DOM hosts follow the model"
    );
}

/// D2 — the reverse: a task list replacing an unclaimed embed at its place.
#[test]
fn d2_a_task_list_replacing_an_unclaimed_embed_gets_its_own_host() {
    let p = page2(r#"<p>ab</p><div data-pm-type="embed"></div><p>cd</p>"#);
    let tasks = page2(&format!("<p>x</p>{TASKS}"))
        .handle
        .doc()
        .child(1)
        .clone();
    replace_child(&p, 1, move |_| Some(tasks.clone()));
    assert_eq!(model_types(&p)[1], "task_list");
    assert_eq!(
        host_types(&p),
        model_types(&p),
        "DOM hosts follow the model"
    );
}

/// D3 — with a view registered: a task list replacing a CLAIMED embed (a
/// descriptor with a slot) at its place; and an embed replacing it back.
#[test]
fn d3_claimed_embed_and_task_list_swap_places() {
    let p = page2(r#"<p>ab</p><div data-pm-type="embed" data-k="x"></div><p>cd</p>"#);
    let views = Rc::new(Views::default());
    p.handle.register_node_view(spec(&views)).keep();
    assert_eq!(views.live().len(), 1);
    let tasks = page2(&format!("<p>x</p>{TASKS}"))
        .handle
        .doc()
        .child(1)
        .clone();
    replace_child(&p, 1, move |_| Some(tasks.clone()));
    assert_eq!(views.live().len(), 0, "the view goes with its embed");
    assert_eq!(host_types(&p), model_types(&p));
    replace_child(&p, 1, |st| {
        st.schema()
            .create_node(
                "embed",
                Attrs::from_iter([("data-k", AttrValue::from("x"))]),
                Fragment::empty(),
            )
            .ok()
    });
    assert_eq!(views.live().len(), 1, "a new view for the new embed");
    assert_eq!(host_types(&p), model_types(&p));
}

// --- N5: the cap on nested re-matches -------------------------------------------

/// N5 — two specs whose renders each make the other match. At the cap the slot
/// keeps the last view, whose spec no longer matches its node: no leaked scope
/// (renders − cleanups = 1), the depth counter is back at zero (a later,
/// legitimate re-match still happens), and nothing re-matches the stranded view
/// until its node changes again.
#[test]
fn n5_at_the_cap() {
    let p = page2(r#"<p>ab</p><div data-pm-type="embed" data-k="b"></div><p>cd</p>"#);
    let renders = Rc::new(Cell::new(0));
    let cleanups = Rc::new(Cell::new(0));
    let live_spec: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    for (want, write) in [("b", "a"), ("a", "b")] {
        let (r, c, l) = (renders.clone(), cleanups.clone(), live_spec.clone());
        p.handle
            .register_node_view(
                NodeViewSpec::new("embed", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
                    r.set(r.get() + 1);
                    l.borrow_mut().push(want);
                    let c = c.clone();
                    rinch_core::reactive::on_cleanup(move || c.set(c.get() + 1));
                    ctx.set_attrs(|a| *a = a.with("data-k", AttrValue::from(write)));
                    scope.create_element("div")
                })
                .matches(move |a| a.get_str("data-k").unwrap_or("") == want),
            )
            .keep();
    }
    let k = p
        .handle
        .doc()
        .child(1)
        .attrs()
        .get_str("data-k")
        .map(str::to_string);
    eprintln!(
        "renders {} cleanups {} last spec {:?} node data-k {k:?}",
        renders.get(),
        cleanups.get(),
        live_spec.borrow().last()
    );
    assert_eq!(
        renders.get() - cleanups.get(),
        1,
        "one live view, no leaked scope"
    );
    assert!(renders.get() <= 10, "bounded: {}", renders.get());
    // typing elsewhere does not touch the stranded view
    let before = renders.get();
    p.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(p.handle.insert_text("z"));
    assert_eq!(renders.get(), before);
    // the depth counter is back at zero: a spec chain c → d settles on d
    let (r2, l2) = (Rc::new(Cell::new(0)), live_spec.clone());
    for (want, write) in [("c", Some("d")), ("d", None)] {
        let (r, l) = (r2.clone(), l2.clone());
        p.handle
            .register_node_view(
                NodeViewSpec::new("embed", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
                    r.set(r.get() + 1);
                    l.borrow_mut().push(want);
                    if let Some(w) = write {
                        ctx.set_attrs(|a| *a = a.with("data-k", AttrValue::from(w)));
                    }
                    scope.create_element("div")
                })
                .matches(move |a| a.get_str("data-k").unwrap_or("") == want),
            )
            .keep();
    }
    p.handle.set_selection(Selection::cursor(Pos(6)));
    assert!(
        p.handle
            .insert_embed(Attrs::from_iter([("data-k", AttrValue::from("c"))]))
    );
    assert_eq!(
        live_spec.borrow().last(),
        Some(&"d"),
        "the chain re-matched to d"
    );
}

/// The round-3 review of #1595, F3, pinned: seed 5730 with the client ids of salt 1
/// (`R_ID_SALT=1`), the order of the three peers' ids under which it failed every
/// time. C is stalled (a task list) and integrates B's change: the remote
/// transaction replaces the embed C's view stood on (an in-editor copy, one `Rc`
/// with another embed further down) and leaves the other copy alone. Matched by
/// `same_ref` alone, the view diff took the first copy's descriptor for the second
/// copy, so C's view moved onto another embed's CRDT block, which the cure of the
/// stall then showed. The diff now follows the remote mapping for a node that
/// stands twice (`Cx::same_place`). Before the fix 158 of salts 1..=300 failed this
/// seed; after it none.
#[test]
fn f3_seed_5730_a_stalled_peer_keeps_its_view_off_another_copys_block() {
    ID_SALT.with(|s| s.set(Some(1)));
    let failed = run_traced(5730, 3, 60);
    ID_SALT.with(|s| s.set(None));
    assert!(failed.is_none(), "{failed:?}");
}
