//! Review probe for PR #1494: incremental layout == fresh layout over random
//! histories of documents holding out-of-flow boxes with static axes.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    doc
}

const WORDS: &[&str] = &[
    "alpha", "be", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "io",
];
fn words(rng: &mut Rng) -> String {
    let n = 1 + rng.below(5);
    let mut s = String::new();
    if rng.below(4) == 0 {
        s.push(' ');
    }
    for i in 0..n {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(WORDS[rng.below(WORDS.len())]);
    }
    if rng.below(3) == 0 {
        s.push(' ');
    }
    s
}

struct Built {
    doc: RinchDocument,
    c: NodeId,
    elements: Vec<(NodeId, String)>,
    texts: Vec<NodeId>,
    oofs: Vec<NodeId>,
    scrollers: Vec<NodeId>,
    containers: Vec<NodeId>,
}

fn oof_style(pos: usize, inset: usize, margin: usize, disp: usize) -> String {
    let pos = ["absolute", "fixed", "static"][pos];
    let inset = ["", "top: 5px;", "left: 5px;", "right: 3px;", "bottom: 4px;"][inset];
    let margin = ["", "margin: 3px 0 0 4px;", "margin-left: 10%;"][margin];
    let disp = [
        "",
        "display: block;",
        "display: inline-block;",
        "display: inline;",
        "display: none;",
        "display: flex;",
    ][disp];
    format!("position: {pos}; width: 40px; height: 30px; {inset}{margin}{disp}")
}

impl Built {
    fn el(&mut self, tag: &str, style: &str, kind: &str) -> NodeId {
        let id = self.doc.create_element(tag);
        self.doc.set_attribute(id, "style", style);
        self.elements
            .push((id, format!("{kind}#{}", self.elements.len())));
        id
    }
    fn text(&mut self, rng: &mut Rng) -> NodeId {
        let t = self.doc.create_text(&words(rng));
        self.texts.push(t);
        t
    }
    fn oof(&mut self, rng: &mut Rng) -> NodeId {
        let tag = if rng.below(2) == 0 { "div" } else { "span" };
        let style = oof_style(rng.below(2), rng.below(3), rng.below(3), 0);
        let id = self.el(tag, &style, "oof");
        self.oofs.push(id);
        id
    }
    fn fill(&mut self, parent: NodeId, rng: &mut Rng, depth: usize) {
        let n = 2 + rng.below(6);
        for _ in 0..n {
            let child = match rng.below(if depth == 0 { 10 } else { 6 }) {
                0 | 1 => self.text(rng),
                2 | 3 => self.oof(rng),
                4 => {
                    let s = self.el(
                        "span",
                        ["", "position: relative;", "font-size: 24px;"][rng.below(3)],
                        "span",
                    );
                    let t = self.text(rng);
                    self.doc.append_child(s, t);
                    if rng.below(2) == 0 {
                        let o = self.oof(rng);
                        self.doc.append_child(s, o);
                    }
                    let t = self.text(rng);
                    self.doc.append_child(s, t);
                    s
                }
                5 => self.el(
                    "span",
                    "display: inline-block; width: 30px; height: 26px;",
                    "ib",
                ),
                6 => self.el("div", "height: 10px;", "blk"),
                7 => {
                    let s = self.el("div", "overflow: auto; height: 50px;", "scroller");
                    self.scrollers.push(s);
                    self.containers.push(s);
                    self.fill(s, rng, depth + 1);
                    let tall = self.el("div", "height: 200px;", "tall");
                    self.doc.append_child(s, tall);
                    s
                }
                8 => {
                    let p = self.el(
                        "div",
                        [
                            "",
                            "position: relative;",
                            "text-align: right;",
                            "display: flex;",
                            "padding: 3px 0 0 6px; border: 2px solid black;",
                        ][rng.below(5)],
                        "p",
                    );
                    self.containers.push(p);
                    self.fill(p, rng, depth + 1);
                    p
                }
                _ => {
                    let p = self.el("span", "display: inline-block; width: 120px;", "ibhost");
                    self.containers.push(p);
                    self.fill(p, rng, depth + 1);
                    p
                }
            };
            self.doc.append_child(parent, child);
        }
    }
}

fn c_style(pos: bool, width: usize, align: usize) -> String {
    format!(
        "{}width: {}px; font: 16px/20px ProbeFace; margin: 13px 0 0 17px; padding: 7px 0 0 11px; text-align: {};",
        if pos { "position: relative; " } else { "" },
        [300, 131, 220, 400][width],
        ["left", "center", "right"][align]
    )
}

fn build(seed: u64) -> Built {
    let mut rng = Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1);
    let mut doc = document();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0;");
    let mut b = Built {
        doc,
        c: body,
        elements: vec![],
        texts: vec![],
        oofs: vec![],
        scrollers: vec![],
        containers: vec![],
    };
    let c = b.el(
        "div",
        &c_style(seed.is_multiple_of(2), 0, (seed % 3) as usize),
        "c",
    );
    b.c = c;
    b.containers.push(c);
    b.fill(c, &mut rng, 0);
    let tall = b.el("div", "height: 900px;", "pagetall");
    b.doc.append_child(body, c);
    b.doc.append_child(body, tall);
    b
}

#[derive(Clone, Debug)]
enum Op {
    Text(usize, String),
    CStyle(bool, usize, usize),
    OofStyle(usize, String),
    Move(usize, usize, usize),
    Remove(usize),
    Scroll(usize, f64),
    PageScroll(f64),
    Viewport(f32),
    Hide(usize, bool),
}

fn gen_op(b: &Built, rng: &mut Rng, seed: u64) -> Op {
    loop {
        match rng.below(12) {
            0..=2 if !b.texts.is_empty() => {
                return Op::Text(rng.below(b.texts.len()), words(rng));
            }
            3 => {
                return Op::CStyle(
                    seed.is_multiple_of(2) || rng.below(3) == 0,
                    rng.below(4),
                    (seed % 3) as usize,
                );
            }
            4 | 5 if !b.oofs.is_empty() => {
                return Op::OofStyle(
                    rng.below(b.oofs.len()),
                    oof_style(rng.below(3), rng.below(5), rng.below(3), rng.below(6)),
                );
            }
            6 if !b.oofs.is_empty() => {
                return Op::Move(
                    rng.below(b.oofs.len()),
                    rng.below(b.containers.len()),
                    rng.below(6),
                );
            }
            7 if !b.oofs.is_empty() && rng.below(3) == 0 => {
                return Op::Remove(rng.below(b.oofs.len()));
            }
            8 if !b.scrollers.is_empty() => {
                return Op::Scroll(
                    rng.below(b.scrollers.len()),
                    [0.0, 15.0, 40.0][rng.below(3)],
                );
            }
            9 => return Op::PageScroll([0.0, 50.0, 120.0][rng.below(3)]),
            10 => return Op::Viewport([800.0, 500.0, 640.0][rng.below(3)]),
            11 => {
                return Op::Hide(
                    rng.below(b.elements.len().max(2) - 1) + 1,
                    rng.below(2) == 0,
                );
            }
            _ => {}
        }
    }
}

fn is_under(doc: &RinchDocument, mut node: NodeId, anc: NodeId) -> bool {
    loop {
        if node == anc {
            return true;
        }
        match doc.parent_node(node) {
            Some(p) => node = p,
            None => return false,
        }
    }
}

fn apply(b: &mut Built, op: &Op, vw: &mut f32, scrolls: bool) {
    match op {
        Op::Text(i, s) => b.doc.set_text_content(b.texts[*i], s),
        Op::CStyle(p, w, a) => {
            let c = b.c;
            b.doc.set_attribute(c, "style", &c_style(*p, *w, *a))
        }
        Op::OofStyle(i, s) => b.doc.set_attribute(b.oofs[*i], "style", s),
        Op::Move(i, to, at) => {
            let (o, to) = (b.oofs[*i], b.containers[*to]);
            if is_under(&b.doc, to, o) {
                return;
            }
            let kids = b.doc.get_children(to);
            let kids: Vec<NodeId> = kids.into_iter().filter(|k| *k != o).collect();
            if kids.is_empty() || *at >= kids.len() {
                b.doc.append_child(to, o)
            } else {
                b.doc.insert_before(to, o, kids[*at])
            }
        }
        Op::Remove(i) => b.doc.remove_node(b.oofs[*i]),
        Op::Scroll(i, v) => {
            if scrolls {
                b.doc.set_scroll_top(b.scrollers[*i], *v)
            }
        }
        Op::PageScroll(v) => {
            if scrolls {
                let body = b.doc.body();
                b.doc.set_scroll_top(body, *v)
            }
        }
        Op::Viewport(w) => *vw = *w,
        Op::Hide(i, on) => {
            let id = b.elements[*i].0;
            if *on {
                b.doc.set_style(id, "display", "none")
            } else {
                // put back whatever the style attribute said: re-set it
                let s = b.doc.get_attribute(id, "style").unwrap_or_default();
                let s: String = s
                    .split(';')
                    .filter(|d| {
                        !d.trim_start().starts_with("display: none") && !d.trim().is_empty()
                    })
                    .map(|d| format!("{};", d.trim()))
                    .collect();
                b.doc.set_attribute(id, "style", &s);
            }
        }
    }
}

fn snapshot(b: &Built) -> Vec<(String, [f32; 4])> {
    let mut out = Vec::new();
    for (id, name) in &b.elements {
        if !b.doc.is_connected(*id) {
            continue;
        }
        let n = b.doc.tree.get(id.0).unwrap();
        let (x, y) = rinch_dom::paint::compute_absolute_position(&b.doc.tree, id.0, 1.0);
        out.push((
            name.clone(),
            [x as f32, y as f32, n.layout.width, n.layout.height],
        ));
    }
    out
}

fn scroll_state(b: &Built) -> Vec<(f64, f64)> {
    let body = b.doc.body();
    std::iter::once(body)
        .chain(b.scrollers.iter().copied())
        .map(|s| {
            b.doc
                .tree
                .get(s.0)
                .map(|n| n.scroll_offset)
                .unwrap_or((0.0, 0.0))
        })
        .collect()
}

#[test]
#[ignore = "review of #1494: a differential probe; main fails some seeds (see the report)"]
fn histories() {
    let seeds: u64 = std::env::var("R1494_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(150);
    let first: u64 = std::env::var("R1494_FIRST")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let steps = 14;
    let mut report = String::new();
    let mut bad = 0;
    'seed: for seed in first..first + seeds {
        let mut inc = build(seed);
        let mut vw = 800.0f32;
        inc.doc.resolve_layout(vw, 600.0);
        let mut rng = Rng(seed ^ 0xABCDEF1234567);
        let mut ops: Vec<Op> = Vec::new();
        for step in 0..steps {
            let op = gen_op(&inc, &mut rng, seed);
            apply(&mut inc, &op, &mut vw, true);
            ops.push(op.clone());
            inc.doc.resolve_layout(vw, 600.0);
            // fresh: every op with no layout in between, then the scroll state copied.
            let mut fresh = build(seed);
            let mut fvw = 800.0f32;
            for o in &ops {
                apply(&mut fresh, o, &mut fvw, false);
            }
            fresh.doc.resolve_layout(fvw, 600.0);
            let st = scroll_state(&inc);
            let body = fresh.doc.body();
            let targets: Vec<NodeId> = std::iter::once(body)
                .chain(fresh.scrollers.iter().copied())
                .collect();
            for (t, s) in targets.iter().zip(&st) {
                if s.1 != 0.0 {
                    fresh.doc.set_scroll_top(*t, s.1);
                }
            }
            fresh.doc.resolve_layout(fvw, 600.0);
            if scroll_state(&fresh) != st {
                continue;
            } // clamp history differs: not comparable
            let (a, f) = (snapshot(&inc), snapshot(&fresh));
            if a != f {
                bad += 1;
                let d: Vec<String> = a
                    .iter()
                    .zip(&f)
                    .filter(|(x, y)| x != y)
                    .take(4)
                    .map(|(x, y)| format!("{} inc {:?} fresh {:?}", x.0, x.1, y.1))
                    .collect();
                // is it stable? one more layout of the incremental document at a nudged height
                inc.doc.resolve_layout(vw, 601.0);
                let healed = snapshot(&inc) == {
                    fresh.doc.resolve_layout(fvw, 601.0);
                    snapshot(&fresh)
                };
                report.push_str(&format!(
                    "seed {seed} step {step} op {:?} healed_by_relayout={healed}\n   {}\n",
                    ops.last().unwrap(),
                    d.join("\n   ")
                ));
                continue 'seed;
            }
        }
    }
    println!("{report}BAD={bad} of {seeds}");
}
