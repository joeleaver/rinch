//! Structural restyles after list churn, against a freshly built oracle (#2627,
//! review of #2651).
//!
//! The sibling sweep stops at a child stamped "this and every child after it
//! are marked" (`Node::later_siblings_marked`). These fixtures check that no
//! sequence of inserts, discards, removals, re-insertions, moves within and
//! across lists, replacements, class toggles and `take_dirty_nodes` calls
//! leaves a row with a stale style. Two documents are interleaved on one
//! thread, because the stamp epoch is thread-wide. Each frame, every row's
//! computed style is compared with the same rows built from scratch.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    li { display: block; height: 10px; font-size: 16px; line-height: 10px; color: #000000; opacity: 1; }
    li:nth-child(odd) { color: #ff0000; }
    li:nth-last-child(3n+1) { opacity: 0.5; }
    li.x ~ li { z-index: 7; }
    li.x + li { font-size: 20px; }
    li:last-child { outline-color: #0000ff; }
    li:first-child { border-top-color: #00ff00; }
    li:only-child { border-top-color: #00ffff; }
    li:nth-child(4n) > span { color: #123456; }
    li:nth-last-of-type(2) > span { opacity: 0.25; }
";

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

#[derive(Clone, Copy)]
struct Row {
    li: NodeId,
    span: NodeId,
    x: bool,
}

struct Live {
    doc: RinchDocument,
    uls: [NodeId; 2],
    lists: [Vec<Row>; 2],
    pool: Vec<Row>,
    vw: f32,
}

fn sig(doc: &RinchDocument, li: NodeId, span: NodeId) -> String {
    let s = &doc.tree.get(li.0).unwrap().computed_style;
    let sp = &doc.tree.get(span.0).unwrap().computed_style;
    format!(
        "c={:?} o={} z={:?} fs={} oc={:?} btc={:?} | sc={:?} so={}",
        s.color.map(|c| c.to_rgba8()),
        s.opacity,
        s.z_index,
        s.font_size,
        s.outline_color.map(|c| c.to_rgba8()),
        s.border_top_color.map(|c| c.to_rgba8()),
        sp.color.map(|c| c.to_rgba8()),
        sp.opacity
    )
}

fn new_row(doc: &mut RinchDocument, x: bool) -> Row {
    let li = doc.create_element("li");
    let span = doc.create_element("span");
    doc.append_child(li, span);
    if x {
        doc.set_attribute(li, "class", "x");
    }
    Row { li, span, x }
}

impl Live {
    fn new(rng: &mut Rng) -> Self {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        let uls = [doc.create_element("ul"), doc.create_element("ul")];
        let mut lists = [Vec::new(), Vec::new()];
        for k in 0..2 {
            doc.append_child(body, uls[k]);
            for _ in 0..(6 + rng.below(10)) {
                let r = new_row(&mut doc, rng.below(5) == 0);
                doc.append_child(uls[k], r.li);
                lists[k].push(r);
            }
        }
        doc.resolve_layout(800.0, 600.0);
        Live {
            doc,
            uls,
            lists,
            pool: Vec::new(),
            vw: 800.0,
        }
    }

    fn frame(&mut self) {
        self.vw += 1.0;
        self.doc.resolve_layout(self.vw, 600.0);
    }

    fn check(&self, what: &str) {
        let mut o = RinchDocument::new();
        o.load_css(CSS);
        let body = o.body();
        for k in 0..2 {
            let ul = o.create_element("ul");
            o.append_child(body, ul);
            let mut orows = Vec::new();
            for r in &self.lists[k] {
                let nr = new_row(&mut o, r.x);
                o.append_child(ul, nr.li);
                orows.push(nr);
            }
            o.resolve_layout(self.vw, 600.0);
            assert_eq!(
                self.doc.get_children(self.uls[k]).len(),
                self.lists[k].len(),
                "{what}: model drift"
            );
            for (i, (r, nr)) in self.lists[k].iter().zip(&orows).enumerate() {
                assert_eq!(
                    self.doc.get_children(self.uls[k])[i],
                    r.li,
                    "{what}: model order"
                );
                let got = sig(&self.doc, r.li, r.span);
                let want = sig(&o, nr.li, nr.span);
                assert_eq!(
                    got,
                    want,
                    "{what}: ul{k} row {i} of {}",
                    self.lists[k].len()
                );
            }
        }
    }

    fn op(&mut self, rng: &mut Rng, log: &mut Vec<String>) {
        let k = rng.below(2);
        let n = self.lists[k].len();
        match rng.below(11) {
            0 | 1 => {
                // insert a new row at a random index
                let r = new_row(&mut self.doc, rng.below(5) == 0);
                let at = rng.below(n + 1);
                self.doc.insert_child(self.uls[k], r.li, at);
                self.lists[k].insert(at, r);
                log.push(format!("insert new ul{k}@{at}"));
            }
            2 | 3 if n > 0 => {
                let at = rng.below(n);
                let r = self.lists[k].remove(at);
                self.doc.discard_node(r.li);
                log.push(format!("discard ul{k}@{at}"));
            }
            4 if n > 0 => {
                let at = rng.below(n);
                let r = self.lists[k].remove(at);
                self.doc.remove_node(r.li);
                self.pool.push(r);
                log.push(format!("remove ul{k}@{at}"));
            }
            5 if !self.pool.is_empty() => {
                let r = self.pool.remove(rng.below(self.pool.len()));
                let at = rng.below(n + 1);
                self.doc.insert_child(self.uls[k], r.li, at);
                self.lists[k].insert(at, r);
                log.push(format!("reinsert ul{k}@{at}"));
            }
            6 if n > 1 => {
                // move within the list
                let from = rng.below(n);
                let r = self.lists[k].remove(from);
                let to = rng.below(n);
                if to < n - 1 {
                    let refn = self.lists[k][to].li;
                    self.doc.insert_before(self.uls[k], r.li, refn);
                } else {
                    self.doc.append_child(self.uls[k], r.li);
                }
                self.lists[k].insert(to, r);
                log.push(format!("move ul{k} {from}->{to}"));
            }
            7 if n > 0 => {
                // move to the other list
                let from = rng.below(n);
                let r = self.lists[k].remove(from);
                let o = 1 - k;
                let to = rng.below(self.lists[o].len() + 1);
                self.doc.insert_child(self.uls[o], r.li, to);
                self.lists[o].insert(to, r);
                log.push(format!("cross ul{k}@{from} -> ul{o}@{to}"));
            }
            8 if n > 0 => {
                let at = rng.below(n);
                let r = &mut self.lists[k][at];
                r.x = !r.x;
                let li = r.li;
                if r.x {
                    self.doc.set_attribute(li, "class", "x");
                } else {
                    self.doc.remove_attribute(li, "class");
                }
                log.push(format!("toggle ul{k}@{at}"));
            }
            9 if n > 0 => {
                let at = rng.below(n);
                let nr = new_row(&mut self.doc, rng.below(5) == 0);
                let old = self.lists[k][at];
                self.doc.replace_node(old.li, nr.li);
                self.lists[k][at] = nr;
                log.push(format!("replace ul{k}@{at}"));
            }
            10 => {
                let _ = self.doc.take_dirty_nodes();
                log.push("take_dirty".into());
            }
            _ => {}
        }
    }
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed);
    let mut docs = [Live::new(&mut rng), Live::new(&mut rng)];
    let mut logs = [Vec::new(), Vec::new()];
    for step in 0..steps {
        let d = rng.below(2);
        if rng.below(6) == 0 {
            docs[d].frame();
            logs[d].push("FRAME".into());
            let tail: Vec<_> = logs[d].iter().rev().take(30).rev().cloned().collect();
            docs[d].check(&format!("seed {seed} step {step} doc{d} log {tail:?}"));
        } else {
            docs[d].op(&mut rng, &mut logs[d]);
        }
    }
}

#[test]
fn random_list_churn_restyles_like_a_fresh_build() {
    // 20 seeds by default (about 30 s in a loaded debug build; seed 2 caught
    // the cross-list stamp the review found). The review of #2651
    // ran 300; `RINCH_RESTYLE_DIFF_SEEDS=300` runs as many.
    let seeds: u64 = std::env::var("RINCH_RESTYLE_DIFF_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    for seed in 1..=seeds {
        run(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, 120);
    }
}

#[test]
fn a_stamped_row_moved_to_the_end_then_a_discard_before_it() {
    // Rows 5.. stamped by a discard; a stamped row moved within the list to
    // the end, then a removal before the stamped region; one frame.
    let mut rng = Rng(7);
    let mut l = Live::new(&mut rng);
    let r = l.lists[0].remove(4);
    l.doc.discard_node(r.li);
    let m = l.lists[0].remove(6);
    l.doc.append_child(l.uls[0], m.li);
    l.lists[0].push(m);
    let r = l.lists[0].remove(1);
    l.doc.discard_node(r.li);
    l.frame();
    l.check("moved stamped to end");
}

/// `:nth-last-child` (HAS_SLOW_SELECTOR) sweeps from index 0, so a stamped
/// row moved in from another list stops the sweep at itself.
#[test]
fn a_stamped_row_moved_from_another_list_does_not_stop_an_nth_last_child_sweep() {
    const CSS2: &str = "
        li { display: block; height: 10px; color: #000000; }
        li:nth-child(odd) { color: #ff0000; }
        li:nth-last-child(2) { opacity: 0.5; }
    ";
    let mut doc = RinchDocument::new();
    doc.load_css(CSS2);
    let body = doc.body();
    let mk = |doc: &mut RinchDocument, n: usize| {
        let ul = doc.create_element("ul");
        doc.append_child(body, ul);
        let rows: Vec<NodeId> = (0..n)
            .map(|_| {
                let li = doc.create_element("li");
                doc.append_child(ul, li);
                li
            })
            .collect();
        (ul, rows)
    };
    let (a, a_rows) = mk(&mut doc, 6);
    let (_b, b_rows) = mk(&mut doc, 6);
    doc.resolve_layout(800.0, 600.0);
    // Stamps every row of list b (the sweep runs from 0).
    doc.discard_node(b_rows[0]);
    // A stamped row of b moved into a at index 2.
    doc.insert_before(a, b_rows[3], a_rows[2]);
    doc.resolve_layout(801.0, 600.0);
    for (i, li) in doc.get_children(a).into_iter().enumerate() {
        let c = doc
            .tree
            .get(li.0)
            .unwrap()
            .computed_style
            .color
            .unwrap()
            .to_rgba8();
        let want = if i % 2 == 0 { 255 } else { 0 };
        assert_eq!(c.r, want, "row {} (1-based) of list a", i + 1);
    }
}

/// "Move the selected cards to the other column": two rows of list b moved
/// into list a in one batch, a sheet using `:last-of-type`
/// (HAS_SLOW_SELECTOR). The first move's removal sweeps b from 0 and stamps
/// the second row; in a, the second row's stamp stops a's sweep.
#[test]
fn two_rows_moved_to_another_list_in_one_frame() {
    const CSS3: &str = "
        li { display: block; height: 10px; color: #000000; }
        li:nth-child(odd) { color: #ff0000; }
        li:last-of-type { opacity: 0.5; }
    ";
    let mut doc = RinchDocument::new();
    doc.load_css(CSS3);
    let body = doc.body();
    let mut mk = |n: usize| {
        let ul = doc.create_element("ul");
        doc.append_child(body, ul);
        let rows: Vec<NodeId> = (0..n)
            .map(|_| {
                let li = doc.create_element("li");
                doc.append_child(ul, li);
                li
            })
            .collect();
        (ul, rows)
    };
    let (a, a_rows) = mk(6);
    let (_b, b_rows) = mk(6);
    doc.resolve_layout(800.0, 600.0);
    doc.insert_before(a, b_rows[1], a_rows[1]);
    doc.insert_before(a, b_rows[3], a_rows[1]);
    doc.resolve_layout(801.0, 600.0);
    for (i, li) in doc.get_children(a).into_iter().enumerate() {
        let c = doc
            .tree
            .get(li.0)
            .unwrap()
            .computed_style
            .color
            .unwrap()
            .to_rgba8();
        let want = if i % 2 == 0 { 255 } else { 0 };
        assert_eq!(c.r, want, "row {} (1-based) of list a", i + 1);
    }
}
