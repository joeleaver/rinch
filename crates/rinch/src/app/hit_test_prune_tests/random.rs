//! The seeded document generator and the fuzzed agreement test over it.

use super::super::hit_test;
use super::assert_agree;
use rinch_core::dom::{DomContent, DomScroll, DomTree, NodeId};
use rinch_dom::RinchDocument;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

fn random_style(r: &mut Rng, depth: usize) -> String {
    let mut s = String::new();
    let display = r.pick(&[
        "block",
        "block",
        "block",
        "flex",
        "inline-block",
        "inline",
        "contents",
        "grid",
        "none",
    ]);
    // `none` rarely, or whole generated subtrees vanish.
    let display = if display == "none" && !r.chance(30) {
        "block"
    } else {
        display
    };
    s += &format!("display: {display}; ");
    if r.chance(60) {
        s += &format!("width: {}px; ", 10 + r.below(300));
    }
    if r.chance(60) {
        s += &format!("height: {}px; ", 5 + r.below(200));
    }
    if r.chance(30) {
        s += &format!("margin: {}px; ", r.below(30) as i64 - 10);
    }
    if r.chance(25) {
        s += &format!("padding: {}px; ", r.below(20));
    }
    if r.chance(15) {
        s += &format!("border: {}px solid black; ", r.below(6));
    }
    match r.below(12) {
        0 | 1 => s += "position: relative; ",
        2 | 3 => s += "position: absolute; ",
        4 if depth > 0 => s += "position: fixed; ",
        5 => s += "position: sticky; ",
        _ => {}
    }
    if r.chance(40) {
        s += &format!(
            "left: {}px; top: {}px; ",
            r.below(400) as i64 - 120,
            r.below(300) as i64 - 90
        );
    }
    if r.chance(25) {
        s += &format!("z-index: {}; ", r.below(7) as i64 - 3);
    }
    match r.below(12) {
        0 => s += "overflow: hidden; ",
        1 => s += "overflow: auto; ",
        2 => s += "overflow: clip; ",
        3 => s += "overflow-y: scroll; ",
        // The one pair css-overflow-3 lets stay asymmetric (#535) — every
        // other combination Stylo normalizes to a symmetric pair (see
        // `clip_predicate_tests::stylo_pairs_a_non_visible_axis_with_auto`),
        // so these two arms are the only way this generator can ever produce
        // a node whose `clips_overflow_x()` and `clips_overflow_y()`
        // disagree, which is exactly the shape `reference_hit_test_node` and
        // the fast path have to keep agreeing on.
        4 => s += "overflow-x: clip; overflow-y: visible; ",
        5 => s += "overflow-y: clip; overflow-x: visible; ",
        _ => {}
    }
    // Lines placed off `0..width` from the content origin (#2956 review): a
    // negative `text-indent` starts one left of it, an rtl `nowrap` line
    // wider than its box overflows to the left, and the prune's flow extent
    // must still reach them. Drawn from a copy of the stream, which `r`
    // does not advance past, so every other choice — every document this
    // generator made before — stays as it was, and these properties are
    // added to them. (Perturbing the stream itself turns up an unrelated
    // pruned/unpruned divergence, #3012.)
    let mut l = Rng(r.0.rotate_left(29) | 1);
    match l.below(10) {
        0 => s += &format!("text-indent: {}px; ", l.below(120) as i64 - 80),
        1 => s += "direction: rtl; ",
        2 => s += "white-space: nowrap; ",
        3 => s += "direction: rtl; white-space: nowrap; ",
        _ => {}
    }
    match r.below(14) {
        0 => {
            s += &format!(
                "transform: translate({}px, {}px); ",
                r.below(80) as i64 - 40,
                r.below(80) as i64 - 40
            )
        }
        1 => s += "transform: scale(1.5); ",
        2 => s += "transform: rotate(20deg); ",
        3 => s += "transform: scale(0); ",
        _ => {}
    }
    if r.chance(8) {
        s += "opacity: 0.5; ";
    }
    if r.chance(10) {
        s += "pointer-events: none; ";
    }
    if r.chance(6) {
        s += "visibility: hidden; ";
    }
    if r.chance(6) {
        s += "pointer-events: auto; visibility: visible; ";
    }
    s
}

fn build(
    doc: &mut RinchDocument,
    r: &mut Rng,
    parent: NodeId,
    depth: usize,
    all: &mut Vec<NodeId>,
) {
    let kids = if depth >= 5 { 0 } else { 1 + r.below(4) };
    for _ in 0..kids {
        match r.below(10) {
            0 | 1 => {
                let t = doc.create_text(r.pick(&[
                    "hello world ",
                    "x",
                    "some longer text that wraps around ",
                ]));
                doc.append_child(parent, t);
            }
            2 => {
                let b = doc.create_element("button");
                let style = format!(
                    "width: {}px; height: {}px; ",
                    10 + r.below(60),
                    8 + r.below(30)
                );
                doc.set_attribute(b, "style", &style);
                doc.append_child(parent, b);
                all.push(b);
            }
            _ => {
                let tag = r.pick(&["div", "div", "div", "span", "section", "p"]);
                let el = doc.create_element(tag);
                let style = random_style(r, depth);
                doc.set_attribute(el, "style", &style);
                doc.append_child(parent, el);
                all.push(el);
                build(doc, r, el, depth + 1, all);
            }
        }
    }
}

fn scroll_everything(doc: &mut RinchDocument, r: &mut Rng, all: &[NodeId]) {
    for &n in all {
        if r.chance(60) {
            let v = r.below(120) as f64;
            doc.set_scroll_top(n, v);
            if r.chance(40) {
                doc.set_scroll_left(n, r.below(60) as f64);
            }
        }
    }
}

#[test]
fn random_documents_hit_identically() {
    let mut hits_seen = 0usize;
    for seed in 1..=60u64 {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let mut all = Vec::new();
        build(&mut doc, &mut r, body, 0, &mut all);
        doc.resolve_layout(800.0, 600.0);
        assert_agree(&doc, &format!("seed {seed}, as laid out"));

        // Scroll every scroll container somewhere off zero, through the
        // public API, and probe again — a stale stacking sequence or extent
        // would answer from before the scroll.
        scroll_everything(&mut doc, &mut r, &all);
        assert_agree(&doc, &format!("seed {seed}, scrolled"));

        // One node at a time, warm between: a scroll keeps every extent but
        // the scrolled node's chain (#911), so a chain that stops one link
        // short answers from before the scroll. A paint-only layout pass in
        // between keeps the memo too, and must still agree.
        for round in 0..4 {
            if all.is_empty() {
                break;
            }
            let n = all[r.below(all.len() as u64) as usize];
            doc.set_scroll_top(n, r.below(150) as f64);
            if r.chance(50) {
                doc.set_scroll_left(n, r.below(80) as f64);
            }
            if round % 2 == 1 {
                doc.resolve_layout(800.0, 600.0);
            }
            assert_agree(&doc, &format!("seed {seed}, scrolled one, round {round}"));
        }

        // Restyle some boxes and move one subtree, then lay out again.
        for &n in &all {
            if r.chance(20) {
                let style = random_style(&mut r, 1);
                doc.set_attribute(n, "style", &style);
            }
        }
        if all.len() > 2 {
            let a = all[r.below(all.len() as u64) as usize];
            doc.remove_node(a);
        }
        doc.resolve_layout(800.0, 600.0);
        assert_agree(&doc, &format!("seed {seed}, restyled"));

        // A mutation with no layout after it: the old code answered from the
        // stale layout it found, and so must the new one.
        if let Some(&n) = all.first() {
            doc.set_scroll_top(n, 7.0);
        }
        let probed = assert_agree(&doc, &format!("seed {seed}, before relayout"));
        assert!(probed > 10_000);
        hits_seen += (0..600)
            .step_by(37)
            .filter(|&v| hit_test(&doc.tree, v as f32, v as f32).is_some())
            .count();
    }
    // Positive control: the grids did find boxes, so agreement was not two
    // walks that both answered `None` everywhere.
    assert!(hits_seen > 100, "hits seen: {hits_seen}");
}
