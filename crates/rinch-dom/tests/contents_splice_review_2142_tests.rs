//! Review of #2142 (#2136): `sync_display_contents` leaves a splice that comes
//! out the same alone. A container whose only content sits inside a `display:
//! contents` wrapper is an IFC root, so its Taffy child list is empty before
//! and after that content goes — and `set_children` had been the only thing
//! marking it dirty. The skip left the box at its old size (a block `div` >
//! wrapper > `span`, the span removed: 20px tall where a fresh layout gives
//! 0). (A plain rsx `div { if c { span {..} } }` toggled off through the shell
//! does not reach it: it passes with or without the fix.) The skip now marks
//! the parent dirty.
//!
//! Every scenario is a history compared with a fresh twin built from the DOM
//! it ends in (the reviewer's harness). `review_2142_random_per_seed` is the
//! seeded random differential (`#[ignore]`d; `R2142_SEEDS=0..600`).

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::NodeKind;

const VP: (f32, f32) = (800.0, 600.0);

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; width: 360px; }
    p, ul, li, table, tr, td { margin: 0; padding: 0; }
    .flex { display: flex; flex-direction: column; }
    .row { display: flex; }
    .grid { display: grid; grid-template-columns: 1fr 1fr; }
    .chip { display: inline-block; padding: 2px; }
    .ichip { display: inline-flex; padding: 1px; }
    .rel { position: relative; }
    .abs { position: absolute; left: 3px; top: 4px; width: 60px; }
    .narrow { width: 120px; }
    .hide > * { display: none; }
    .blockkids > * { display: block; }
    .cnts > * { display: contents; }
    .pb::before { content: 'generated inline-block before'; display: inline-block; width: 90px; }
    .pbb::before { content: 'generated block'; display: block; }
    .pbi::before { content: 'generated inline'; }
";

// ── Snapshot ────────────────────────────────────────────────────────────────

fn r(v: f32) -> f32 {
    (v * 64.0).round() / 64.0
}

/// Everything layout produced below `<body>`, in DOM pre-order, free of node
/// ids: each element's box, each anonymous block box its container holds, and
/// each IFC root's shaped text.
fn snapshot(doc: &RinchDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk(doc, doc.tree.body_id, 0, &mut out);
    out
}

fn walk(doc: &RinchDocument, id: u64, depth: usize, out: &mut Vec<String>) {
    let Some(node) = doc.tree.get(id) else {
        return;
    };
    let pad = depth * 2;
    let pseudo = if node.is_pseudo_element {
        " pseudo"
    } else {
        ""
    };
    match &node.kind {
        NodeKind::Text(t) => {
            // A text node owns no box of its own in an IFC; its glyphs are the
            // root's, compared below.
            out.push(format!("{:pad$}{:?}", "", t.content));
        }
        NodeKind::Comment(_) => out.push(format!("{:pad$}<!---->", "")),
        _ if node.is_out_of_flow() && has_atomic_inline_ancestor(doc, id) => {
            // Pre-existing, and not this pass's: an out-of-flow box inside an
            // atomic inline has its position patched (`read_layout_results`)
            // *before* `build_ifc_layouts` places the atomic inline on its
            // line, so the patch reads the chip's position from the previous
            // pass — history, which the two passes do not share, and which a
            // fresh layout does not have at all (it differs from both, on
            // `main` too). The size is still compared.
            let l = &node.layout;
            out.push(format!(
                "{:pad$}<{}> (out-of-flow in an atomic inline) {}x{}",
                "",
                node.tag().unwrap_or("?"),
                r(l.width),
                r(l.height)
            ));
        }
        _ => {
            let l = &node.layout;
            out.push(format!(
                "{:pad$}<{}{pseudo}> @({}, {}) {}x{}",
                "",
                node.tag().unwrap_or("?"),
                r(l.x),
                r(l.y),
                r(l.width),
                r(l.height)
            ));
        }
    }
    for &b in &node.run_boxes {
        if let Some(bn) = doc.tree.get(b) {
            let l = &bn.layout;
            out.push(format!(
                "{:pad$}  [anon] @({}, {}) {}x{} members={}",
                "",
                r(l.x),
                r(l.y),
                r(l.width),
                r(l.height),
                bn.run_members.len()
            ));
            if let Some(tl) = &bn.text_layout {
                out.push(format!(
                    "{:pad$}    ifc {:?} {}x{} lines={}",
                    "",
                    tl.text_content,
                    r(tl.layout.width()),
                    r(tl.layout.height()),
                    tl.layout.len()
                ));
            }
        }
    }
    if let Some(tl) = &node.text_layout {
        out.push(format!(
            "{:pad$}  ifc {:?} {}x{} lines={}",
            "",
            tl.text_content,
            r(tl.layout.width()),
            r(tl.layout.height()),
            tl.layout.len()
        ));
    }
    // A `display: none` subtree generates no boxes: nothing paints or hit
    // tests it, and the positions left in it are whatever the last pass that
    // happened to rebuild an IFC inside it wrote — the whole-document pass
    // rebuilt more of them, the scoped pass fewer, and neither means anything.
    if node.computed_style.display == rinch_dom::computed_style::DisplayValue::None {
        return;
    }
    for &c in &node.children {
        walk(doc, c, depth + 1, out);
    }
}

fn has_atomic_inline_ancestor(doc: &RinchDocument, id: u64) -> bool {
    let mut cur = doc.tree.get(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        let Some(n) = doc.tree.get(p) else {
            return false;
        };
        if n.display_mode.is_atomic_inline() {
            return true;
        }
        cur = n.parent;
    }
    false
}

// ── The fresh twin ──────────────────────────────────────────────────────────

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    doc
}

/// Rebuild `src`'s `<body>` content in a new document, the way `rsx!` builds a
/// tree (children into detached parents, the root appended last), and lay it
/// out.
fn fresh_twin(src: &RinchDocument) -> RinchDocument {
    let mut doc = new_doc();
    let body = doc.body();
    let kids: Vec<u64> = src.tree.get(src.tree.body_id).unwrap().children.clone();
    for c in kids {
        if let Some(n) = copy_node(src, c, &mut doc) {
            doc.append_child(body, n);
        }
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc
}

fn copy_node(src: &RinchDocument, id: u64, dst: &mut RinchDocument) -> Option<NodeId> {
    let node = src.tree.get(id)?;
    if node.is_pseudo_element || node.is_anonymous_block_box {
        return None;
    }
    let out = match &node.kind {
        NodeKind::Text(t) => dst.create_text(&t.content),
        NodeKind::Comment(c) => dst.create_comment(c),
        NodeKind::Element(e) => {
            let el = dst.create_element(&e.tag);
            let mut attrs: Vec<(&String, &String)> = node.attributes.iter().collect();
            attrs.sort();
            for (k, v) in attrs {
                dst.set_attribute(el, k, v);
            }
            for &c in &node.children {
                if let Some(cn) = copy_node(src, c, dst) {
                    dst.append_child(el, cn);
                }
            }
            el
        }
        NodeKind::Document => return None,
    };
    Some(out)
}

/// The first differences between two snapshots, or `None` when they agree.
fn diff(a: &[String], b: &[String], an: &str, bn: &str) -> Option<String> {
    if a == b {
        return None;
    }
    let mut out = String::new();
    let mut shown = 0;
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y && shown < 6 {
            out.push_str(&format!("  #{i}\n    {an}: {x}\n    {bn}: {y}\n"));
            shown += 1;
        }
    }
    if a.len() != b.len() {
        out.push_str(&format!(
            "  lengths differ: {an} {} vs {bn} {}\n",
            a.len(),
            b.len()
        ));
    }
    Some(out)
}

fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .to_string()
    })
}

/// Whether a laid-out document matches a fresh twin built from its DOM.
fn matches_fresh(doc: &RinchDocument, snap: &[String]) -> Option<String> {
    let fresh = guarded(|| snapshot(&fresh_twin(doc))).ok()?;
    diff(snap, &fresh, "incremental", "fresh      ")
}

// ── Building blocks ─────────────────────────────────────────────────────────
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

fn el(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str) -> NodeId {
    let n = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(n, "class", class);
    }
    doc.append_child(parent, n);
    n
}

fn styled(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
    let n = doc.create_element(tag);
    doc.set_attribute(n, "style", style);
    doc.append_child(parent, n);
    n
}

fn text(doc: &mut RinchDocument, parent: NodeId, s: &str) -> NodeId {
    let t = doc.create_text(s);
    doc.append_child(parent, t);
    t
}

// ── Review of #2142: sync_display_contents skip / dirty-only-new ──────────

fn wrap(doc: &mut RinchDocument, parent: NodeId) -> NodeId {
    styled(doc, parent, "div", "display: contents")
}

fn card_h(doc: &mut RinchDocument, parent: NodeId, h: u32, label: &str) -> NodeId {
    let c = styled(doc, parent, "div", &format!("height: {h}px"));
    text(doc, c, label);
    c
}

type Build = fn(&mut RinchDocument) -> Vec<NodeId>;
type Mutate = fn(&mut RinchDocument, &[NodeId]);

/// Build, lay out twice, mutate, lay out ONCE, compare with a fresh layout;
/// then a second layout and compare again. Returns failure text.
fn run(name: &str, build: Build, mutate: Mutate) -> Option<String> {
    let mut doc = new_doc();
    let h = build(&mut doc);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    // positive control: snapshot before the mutation
    let before = snapshot(&doc);
    mutate(&mut doc, &h);
    let r = guarded(|| {
        doc.resolve_layout(VP.0, VP.1);
        snapshot(&doc)
    });
    let snap = match r {
        Ok(s) => s,
        Err(e) => return Some(format!("{name}: PANIC {e}")),
    };
    if snap == before {
        eprintln!("{name}: (note) layout unchanged by the mutation");
    }
    if let Some(d) = matches_fresh(&doc, &snap) {
        return Some(format!("{name}: first layout vs fresh\n{d}"));
    }
    doc.resolve_layout(VP.0 - 1.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    let snap2 = snapshot(&doc);
    if let Some(d) = matches_fresh(&doc, &snap2) {
        return Some(format!("{name}: after a later layout vs fresh\n{d}"));
    }
    eprintln!("{name}: ok");
    None
}

fn grid3(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let g = styled(
        doc,
        body,
        "div",
        "display: grid; grid-template-columns: 50px 50px 50px",
    );
    let mut v = vec![g];
    for (i, h) in [10u32, 40, 20, 70, 15, 30, 25].iter().enumerate() {
        let w = wrap(doc, g);
        card_h(doc, w, *h, &format!("c{i}"));
        v.push(w);
    }
    v
}

fn flexrow(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let g = styled(doc, body, "div", "display: flex; width: 300px");
    let mut v = vec![g];
    for (i, grow) in [1u32, 3, 0, 2].iter().enumerate() {
        let w = wrap(doc, g);
        let c = styled(
            doc,
            w,
            "div",
            &format!("flex: {grow} 1 20px; height: {}px", 10 + i * 7),
        );
        text(doc, c, &format!("f{i}"));
        v.push(w);
    }
    v
}

fn blockcol(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let g = styled(doc, body, "div", "display: block");
    let mut v = vec![g];
    for i in 0..4u32 {
        let w = wrap(doc, g);
        let c = styled(
            doc,
            w,
            "p",
            &format!("margin: {}px 0; height: {}px", 3 + i * 5, 10 + i),
        );
        text(doc, c, &format!("b{i}"));
        v.push(w);
    }
    v
}

fn nested(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let g = styled(
        doc,
        body,
        "div",
        "display: grid; grid-template-columns: 50px 50px",
    );
    let w1 = wrap(doc, g);
    let w2 = wrap(doc, w1);
    card_h(doc, w2, 10, "n0");
    card_h(doc, w2, 30, "n1");
    let w3 = wrap(doc, g);
    let w4 = wrap(doc, w3);
    card_h(doc, w4, 50, "n2");
    card_h(doc, w3, 20, "n3");
    vec![g, w1, w2, w3, w4]
}

fn mixed(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let g = styled(doc, body, "div", "width: 120px");
    text(doc, g, "leading text ");
    let w = wrap(doc, g);
    let s = el(doc, w, "span", "");
    text(doc, s, "inline in wrapper ");
    text(doc, g, "middle ");
    let w2 = wrap(doc, g);
    let b = styled(doc, w2, "div", "height: 12px");
    text(doc, b, "block in wrapper");
    text(doc, g, " trailing words here");
    vec![g, w, w2, s, b]
}

fn table(doc: &mut RinchDocument) -> Vec<NodeId> {
    let body = doc.body();
    let t = el(doc, body, "table", "");
    let tb = el(doc, t, "tbody", "");
    let mut v = vec![tb];
    for i in 0..3u32 {
        let w = wrap(doc, tb);
        let tr = el(doc, w, "tr", "");
        let td = el(doc, tr, "td", "");
        text(doc, td, &"cell ".repeat(1 + i as usize * 3));
        let td2 = el(doc, tr, "td", "");
        doc.set_attribute(td2, "style", &format!("height: {}px", 10 + i * 9));
        text(doc, td2, "x");
        v.push(w);
    }
    v
}

#[test]
fn review_2142_contents_matrix() {
    let mut fails = Vec::new();
    let cases: Vec<(&str, Build, Mutate)> = vec![
        // grid
        ("grid: swap two wrappers", grid3, |d, h| {
            d.insert_before(h[0], h[3], h[2])
        }),
        ("grid: move last wrapper first", grid3, |d, h| {
            d.insert_before(h[0], h[7], h[1])
        }),
        ("grid: move first wrapper last", grid3, |d, h| {
            d.append_child(h[0], h[1])
        }),
        ("grid: remove a wrapper", grid3, |d, h| {
            d.remove_child(h[0], h[2])
        }),
        ("grid: remove_node a wrapper", grid3, |d, h| {
            d.remove_node(h[2])
        }),
        ("grid: a wrapper gains a second card", grid3, |d, h| {
            let c = styled(d, h[2], "div", "height: 33px");
            text(d, c, "extra");
        }),
        (
            "grid: a wrapper's card removed (wrapper stays, empty)",
            grid3,
            |d, h| {
                let c = d.tree.nodes[h[2].0].children[0];
                d.remove_child(h[2], NodeId(c));
            },
        ),
        ("grid: card moved between wrappers", grid3, |d, h| {
            let c = d.tree.nodes[h[2].0].children[0];
            d.append_child(h[5], NodeId(c));
        }),
        ("grid: card moved out of the grid", grid3, |d, h| {
            let c = d.tree.nodes[h[2].0].children[0];
            let body = d.body();
            d.append_child(body, NodeId(c));
        }),
        ("grid: card moved in from outside", grid3, |d, h| {
            let body = d.body();
            let c = card_h(d, body, 44, "outside");
            d.resolve_layout(VP.0, VP.1);
            d.insert_before(h[3], c, NodeId(d.tree.nodes[h[3].0].children[0]));
        }),
        ("grid: wrapper contents -> block", grid3, |d, h| {
            d.set_attribute(h[3], "style", "display: block")
        }),
        ("grid: wrapper contents -> none", grid3, |d, h| {
            d.set_attribute(h[3], "style", "display: none")
        }),
        (
            "grid: wrapper block -> contents",
            |d| {
                let v = grid3(d);
                d.set_attribute(v[3], "style", "display: block");
                v
            },
            |d, h| d.set_attribute(h[3], "style", "display: contents"),
        ),
        ("grid: card style changes span", grid3, |d, h| {
            let c = d.tree.nodes[h[2].0].children[0];
            d.set_attribute(NodeId(c), "style", "height: 40px; grid-column: span 2");
        }),
        (
            "grid: swap with a span-2 card present",
            |d| {
                let v = grid3(d);
                let c = d.tree.nodes[v[2].0].children[0];
                d.set_attribute(NodeId(c), "style", "height: 40px; grid-column: span 2");
                v
            },
            |d, h| d.insert_before(h[0], h[3], h[2]),
        ),
        ("grid: two ops (swap + remove)", grid3, |d, h| {
            d.insert_before(h[0], h[5], h[1]);
            d.remove_child(h[0], h[6]);
        }),
        ("grid: remove then reinsert same wrapper", grid3, |d, h| {
            d.remove_child(h[0], h[4]);
            d.insert_before(h[0], h[4], h[1]);
        }),
        ("grid: remove, layout, reinsert elsewhere", grid3, |d, h| {
            d.remove_child(h[0], h[4]);
            d.resolve_layout(VP.0, VP.1);
            d.insert_before(h[0], h[4], h[2]);
        }),
        // nested
        ("nested: append into inner wrapper", nested, |d, h| {
            card_h(d, h[2], 17, "n-new");
        }),
        ("nested: remove from inner wrapper", nested, |d, h| {
            let c = d.tree.nodes[h[2].0].children[0];
            d.remove_child(h[2], NodeId(c));
        }),
        (
            "nested: move inner wrapper to the other outer",
            nested,
            |d, h| d.append_child(h[3], h[2]),
        ),
        ("nested: swap outer wrappers", nested, |d, h| {
            d.insert_before(h[0], h[3], h[1])
        }),
        ("nested: inner wrapper -> block", nested, |d, h| {
            d.set_attribute(h[2], "style", "display: block")
        }),
        ("nested: outer wrapper -> block", nested, |d, h| {
            d.set_attribute(h[1], "style", "display: block")
        }),
        ("nested: outer wrapper -> grid", nested, |d, h| {
            d.set_attribute(h[1], "style", "display: grid")
        }),
        ("nested: reorder inside inner", nested, |d, h| {
            let k = d.tree.nodes[h[2].0].children.clone();
            d.insert_before(h[2], NodeId(k[1]), NodeId(k[0]));
        }),
        // flex
        ("flex: swap", flexrow, |d, h| {
            d.insert_before(h[0], h[3], h[1])
        }),
        ("flex: remove", flexrow, |d, h| d.remove_child(h[0], h[2])),
        ("flex: wrapper -> block", flexrow, |d, h| {
            d.set_attribute(h[2], "style", "display: block")
        }),
        ("flex: wrapper gains child", flexrow, |d, h| {
            let c = styled(d, h[1], "div", "flex: 5 1 10px; height: 9px");
            text(d, c, "g");
        }),
        // block (margin collapsing)
        ("block: swap", blockcol, |d, h| {
            d.insert_before(h[0], h[3], h[1])
        }),
        ("block: first to last", blockcol, |d, h| {
            d.append_child(h[0], h[1])
        }),
        ("block: remove first", blockcol, |d, h| {
            d.remove_child(h[0], h[1])
        }),
        ("block: wrapper -> block", blockcol, |d, h| {
            d.set_attribute(h[1], "style", "display: block")
        }),
        // mixed / anonymous boxes
        ("mixed: append inline to wrapper", mixed, |d, h| {
            let s = el(d, h[1], "em", "");
            text(d, s, "more inline ");
        }),
        ("mixed: append block to inline wrapper", mixed, |d, h| {
            let b = styled(d, h[1], "div", "height: 7px");
            text(d, b, "blk");
        }),
        ("mixed: swap wrappers", mixed, |d, h| {
            d.insert_before(h[0], h[2], h[1])
        }),
        ("mixed: remove block wrapper", mixed, |d, h| {
            d.remove_child(h[0], h[2])
        }),
        ("mixed: block wrapper -> block", mixed, |d, h| {
            d.set_attribute(h[2], "style", "display: block")
        }),
        ("mixed: inline wrapper -> block", mixed, |d, h| {
            d.set_attribute(h[1], "style", "display: block")
        }),
        ("mixed: block wrapper's block -> inline", mixed, |d, h| {
            d.set_attribute(h[4], "style", "display: inline")
        }),
        // tables
        ("table: swap rows", table, |d, h| {
            d.insert_before(h[0], h[3], h[1])
        }),
        ("table: remove row", table, |d, h| {
            d.remove_child(h[0], h[2])
        }),
        ("table: append row", table, |d, h| {
            let w = wrap(d, h[0]);
            let tr = el(d, w, "tr", "");
            let td = el(d, tr, "td", "");
            text(d, td, "new row with a longer cell text here");
        }),
        ("table: wrapper gains a row", table, |d, h| {
            let tr = el(d, h[1], "tr", "");
            let td = el(d, tr, "td", "");
            text(d, td, "second row in wrapper, long long text");
        }),
        // ("table: wrapper -> block") differs from a fresh layout on main as
        // well, before any of #2142: filed separately, not this splice.
    ];
    for (name, b, m) in cases {
        if let Some(f) = run(name, b, m) {
            eprintln!("FAIL {f}");
            fails.push(f);
        }
    }
    assert!(
        fails.is_empty(),
        "{} failing:\n{}",
        fails.len(),
        fails.join("\n")
    );
}
const TAGS: &[&str] = &["div", "span", "p", "a", "section", "ul", "li", "em"];
const STYLES: &[&str] = &[
    "",
    "",
    "display: block",
    "display: inline",
    "display: contents",
    "display: inline-block; padding: 1px",
    "display: flex; flex-direction: column",
    "display: flex",
    "display: grid; grid-template-columns: 1fr 1fr",
    "position: absolute; left: 3px; top: 4px; width: 50px",
    "display: none",
    "position: relative",
    "display: inline-flex",
    "width: 110px",
];
const WORDS: &[&str] = &[
    "a",
    "word",
    "several words here",
    "a rather longer run of text that wraps somewhere",
    " ",
    "x y z",
];

fn random_subtree(doc: &mut RinchDocument, rng: &mut Rng, depth: usize) -> NodeId {
    if depth == 0 || rng.below(3) == 0 {
        return doc.create_text(rng.pick(WORDS));
    }
    let e = doc.create_element(rng.pick(TAGS));
    let style = rng.pick(STYLES);
    if !style.is_empty() {
        doc.set_attribute(e, "style", style);
    }
    for _ in 0..rng.below(4) {
        let c = random_subtree(doc, rng, depth - 1);
        doc.append_child(e, c);
    }
    e
}

/// Every attached node under `<body>` (elements and text), pre-order.
fn attached(doc: &RinchDocument) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![doc.tree.body_id];
    while let Some(id) = stack.pop() {
        let Some(n) = doc.tree.get(id) else { continue };
        if n.is_pseudo_element {
            continue;
        }
        if id != doc.tree.body_id {
            out.push(NodeId(id));
        }
        stack.extend(n.children.iter().rev().copied());
    }
    out
}

fn is_ancestor(doc: &RinchDocument, anc: NodeId, mut n: NodeId) -> bool {
    loop {
        if n == anc {
            return true;
        }
        match doc.tree.get(n.0).and_then(|x| x.parent) {
            Some(p) => n = NodeId(p),
            None => return false,
        }
    }
}

/// `node` as `id<tag style>` for a trace.
fn describe(doc: &RinchDocument, n: NodeId) -> String {
    match doc.tree.get(n.0) {
        Some(x) => match &x.kind {
            NodeKind::Text(t) => format!("{}{:?}", n.0, t.content),
            _ => format!(
                "{}<{} {:?}>",
                n.0,
                x.tag().unwrap_or("?"),
                x.attributes.get("style").map(String::as_str).unwrap_or("")
            ),
        },
        None => format!("{}<freed>", n.0),
    }
}

/// The whole tree under `<body>`, one node per line, for a trace.
fn dump(doc: &RinchDocument) -> String {
    let mut out = String::new();
    let mut stack = vec![(doc.tree.body_id, 0usize)];
    while let Some((id, d)) = stack.pop() {
        let Some(n) = doc.tree.get(id) else { continue };
        out.push_str(&format!(
            "{:w$}{} ifc_root={:?} boxes={:?} spliced={}\n",
            "",
            describe(doc, NodeId(id)),
            n.ifc_root,
            n.run_boxes,
            n.contents_spliced,
            w = d * 2
        ));
        for &c in n.children.iter().rev() {
            stack.push((c, d + 1));
        }
    }
    out
}

fn random_step(doc: &mut RinchDocument, rng: &mut Rng, trace: bool) {
    let nodes = attached(doc);
    let body = doc.body();
    let elements: Vec<NodeId> = std::iter::once(body)
        .chain(
            nodes
                .iter()
                .copied()
                .filter(|n| doc.tree.get(n.0).is_some_and(|x| x.is_element())),
        )
        .collect();
    let texts: Vec<NodeId> = nodes
        .iter()
        .copied()
        .filter(|n| doc.tree.get(n.0).is_some_and(|x| x.is_text()))
        .collect();
    match rng.below(13) {
        0 | 1 => {
            let p = *rng.pick(&elements);
            let n = random_subtree(doc, rng, 3);
            if trace {
                eprintln!("append {} into {}", describe(doc, n), describe(doc, p));
            }
            doc.append_child(p, n);
        }
        2 => {
            let p = *rng.pick(&elements);
            let kids = children(doc, p);
            let n = random_subtree(doc, rng, 2);
            if kids.is_empty() {
                if trace {
                    eprintln!("append {} into {}", describe(doc, n), describe(doc, p));
                }
                doc.append_child(p, n);
            } else {
                let r = *rng.pick(&kids);
                if trace {
                    eprintln!(
                        "insert {} into {} before {}",
                        describe(doc, n),
                        describe(doc, p),
                        describe(doc, r)
                    );
                }
                doc.insert_before(p, n, r);
            }
        }
        3 if !nodes.is_empty() => {
            let n = *rng.pick(&nodes);
            if let Some(p) = doc.tree.get(n.0).and_then(|x| x.parent) {
                if trace {
                    eprintln!("remove {}", describe(doc, n));
                }
                doc.remove_child(NodeId(p), n);
            }
        }
        4 if !nodes.is_empty() => {
            // A move, possibly between containers of different kinds.
            let n = *rng.pick(&nodes);
            let p = *rng.pick(&elements);
            if !is_ancestor(doc, n, p) {
                let kids = children(doc, p);
                if kids.is_empty() || rng.below(2) == 0 {
                    if trace {
                        eprintln!("move {} into {}", describe(doc, n), describe(doc, p));
                    }
                    doc.append_child(p, n);
                } else {
                    let r = *rng.pick(&kids);
                    if r != n {
                        if trace {
                            eprintln!(
                                "move {} into {} before {}",
                                describe(doc, n),
                                describe(doc, p),
                                describe(doc, r)
                            );
                        }
                        doc.insert_before(p, n, r);
                    }
                }
            }
        }
        5 if elements.len() > 1 => {
            let e = elements[1 + rng.below(elements.len() - 1)];
            let st = rng.pick(STYLES);
            if trace {
                eprintln!("restyle {} to {st:?}", describe(doc, e));
            }
            doc.set_attribute(e, "style", st);
        }
        6 if !texts.is_empty() => {
            let t = *rng.pick(&texts);
            let w = rng.pick(WORDS);
            if trace {
                eprintln!("text {} = {w:?}", describe(doc, t));
            }
            doc.set_text_content(t, w);
        }
        7 if elements.len() > 1 => {
            let e = elements[1 + rng.below(elements.len() - 1)];
            let w = rng.pick(WORDS);
            if trace {
                eprintln!("text_content {} = {w:?}", describe(doc, e));
            }
            doc.set_text_content(e, w);
        }
        8 if !nodes.is_empty() => {
            let old = *rng.pick(&nodes);
            if doc.tree.get(old.0).is_some_and(|x| x.parent.is_some()) {
                let n = random_subtree(doc, rng, 2);
                if trace {
                    eprintln!("replace {} with {}", describe(doc, old), describe(doc, n));
                }
                doc.replace_node(old, n);
            }
        }
        9 if !nodes.is_empty() => {
            let n = *rng.pick(&nodes);
            if trace {
                eprintln!("remove_node {}", describe(doc, n));
            }
            doc.remove_node(n);
        }
        10 => {
            let p = *rng.pick(&elements);
            let idx = rng.below(children(doc, p).len() + 1);
            let n = random_subtree(doc, rng, 2);
            if trace {
                eprintln!(
                    "insert_child {} into {} at {idx}",
                    describe(doc, n),
                    describe(doc, p)
                );
            }
            doc.insert_child(p, n, idx);
        }
        11 if elements.len() > 1 => {
            let e = elements[1 + rng.below(elements.len() - 1)];
            let html = rng.pick(HTMLS);
            if trace {
                eprintln!("inner_html {} = {html:?}", describe(doc, e));
            }
            doc.set_inner_html(e, html);
        }
        12 if elements.len() > 1 => {
            let e = elements[1 + rng.below(elements.len() - 1)];
            let class = rng.pick(CLASSES);
            if trace {
                eprintln!("class {} = {class:?}", describe(doc, e));
            }
            doc.set_attribute(e, "class", class);
        }
        _ => {}
    }
}

/// `set_inner_html` bodies for the random run.
const HTMLS: &[&str] = &[
    "",
    "plain text",
    "<span>inline</span> and <b>bold</b>",
    "<div>a block</div> text after",
    "<span style=\"display: inline-block\">chip <span style=\"display: inline-block\">in chip</span></span>",
];

/// Classes for the random run: display flips of the children through a
/// descendant selector, and generated content.
const CLASSES: &[&str] = &["", "hide", "blockkids", "cnts", "pb", "pbb", "pbi"];

/// Per seed: the first step whose scoped layout differs from fresh (or panics).
#[test]
#[ignore]
fn review_2142_random_per_seed() {
    let (lo, hi): (u64, u64) = std::env::var("R2142_SEEDS")
        .ok()
        .and_then(|r| {
            r.split_once("..")
                .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        })
        .unwrap_or((0, 400));
    let steps = 30;
    let threads = 6u64;
    let chunk = (hi - lo).div_ceil(threads);
    let hs: Vec<_> = (0..threads)
        .map(|t| {
            let a = (lo + t * chunk).min(hi);
            let b = (a + chunk).min(hi);
            std::thread::spawn(move || {
                let mut out = Vec::new();
                for seed in a..b {
                    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
                    let mut doc = new_doc();
                    let body = doc.body();
                    for _ in 0..4 {
                        let n = random_subtree(&mut doc, &mut rng, 3);
                        doc.append_child(body, n);
                    }
                    doc.resolve_layout(VP.0, VP.1);
                    let mut res = format!("seed {seed}: clean");
                    for step in 0..steps {
                        let n_ops = 1 + rng.below(3);
                        for _ in 0..n_ops {
                            random_step(&mut doc, &mut rng, false);
                        }
                        let r = guarded(|| {
                            doc.resolve_layout(VP.0, VP.1);
                            snapshot(&doc)
                        });
                        match r {
                            Err(e) => {
                                res = format!("seed {seed}: panic at {step}: {e}");
                                break;
                            }
                            Ok(s) => {
                                if matches_fresh(&doc, &s).is_some() {
                                    res = format!("seed {seed}: diverge at {step}");
                                    break;
                                }
                            }
                        }
                    }
                    out.push(res);
                }
                out
            })
        })
        .collect();
    for h in hs {
        for l in h.join().unwrap() {
            println!("R2142 {l}");
        }
    }
}
fn children(doc: &RinchDocument, n: NodeId) -> Vec<NodeId> {
    doc.tree
        .get(n.0)
        .map(|x| x.children.iter().map(|&c| NodeId(c)).collect())
        .unwrap_or_default()
}

#[test]
#[ignore]
fn review_2142_replay() {
    let seed: u64 = std::env::var("R2142_SEED").unwrap().parse().unwrap();
    let upto: usize = std::env::var("R2142_STEP").unwrap().parse().unwrap();
    let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    let mut doc = new_doc();
    let body = doc.body();
    for _ in 0..4 {
        let n = random_subtree(&mut doc, &mut rng, 3);
        doc.append_child(body, n);
    }
    doc.resolve_layout(VP.0, VP.1);
    eprintln!("initial:\n{}", dump(&doc));
    for step in 0..=upto {
        let n_ops = 1 + rng.below(3);
        eprintln!("-- step {step}");
        for _ in 0..n_ops {
            random_step(&mut doc, &mut rng, true);
        }
        if step == upto {
            eprintln!("before layout:\n{}", dump(&doc));
        }
        doc.resolve_layout(VP.0, VP.1);
        let s = snapshot(&doc);
        if let Some(d) = matches_fresh(&doc, &s) {
            eprintln!("DIVERGE at {step}:\n{d}");
            eprintln!("incremental snapshot:\n{}", s.join("\n"));
            break;
        }
    }
}

fn ifc_in_wrapper(doc: &mut RinchDocument, outer: &str) -> Vec<NodeId> {
    let body = doc.body();
    let a = styled(doc, body, "a", outer);
    text(doc, a, "several words here");
    let em = el(doc, a, "em", "");
    let w = wrap(doc, em);
    let li = styled(doc, w, "li", "display: inline");
    text(doc, li, "a rather longer run of text that wraps somewhere");
    vec![a, em, w, li]
}

#[test]
fn review_2142_seed133_minimal() {
    let mut fails = Vec::new();
    let cases: Vec<(&str, Build, Mutate)> = vec![
        (
            "133: inline-flex, remove_node the wrapper's only inline child",
            |d| ifc_in_wrapper(d, "display: inline-flex"),
            |d, h| d.remove_node(h[3]),
        ),
        (
            "133: inline-flex, remove_child the wrapper's only inline child",
            |d| ifc_in_wrapper(d, "display: inline-flex"),
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: flex, remove_child",
            |d| ifc_in_wrapper(d, "display: flex"),
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: block, em as block, remove_child",
            |d| {
                let v = ifc_in_wrapper(d, "display: block");
                d.set_attribute(v[1], "style", "display: block");
                v
            },
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: grid, remove_child",
            |d| ifc_in_wrapper(d, "display: grid"),
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: flex, change text of wrapper's inline child",
            |d| ifc_in_wrapper(d, "display: flex"),
            |d, h| {
                let t = d.tree.nodes[h[3].0].children[0];
                d.set_text_content(NodeId(t), "x");
            },
        ),
        (
            "133: flex, append inline into wrapper",
            |d| ifc_in_wrapper(d, "display: flex"),
            |d, h| {
                let s = el(d, h[2], "span", "");
                text(d, s, "another quite long run of words to wrap more");
            },
        ),
        (
            "133: flex, remove inline from wrapper keeping a sibling",
            |d| {
                let v = ifc_in_wrapper(d, "display: flex");
                let s = el(d, v[2], "span", "");
                text(d, s, "x");
                v
            },
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: flex, move inline child out of wrapper to body",
            |d| ifc_in_wrapper(d, "display: flex"),
            |d, h| {
                let b = d.body();
                d.append_child(b, h[3]);
            },
        ),
        (
            "133: flex, wrapper text child directly removed",
            |d| {
                let body = d.body();
                let a = styled(d, body, "a", "display: flex");
                let em = el(d, a, "em", "");
                let w = wrap(d, em);
                let t = text(d, w, "a rather longer run of text that wraps somewhere");
                vec![a, em, w, t]
            },
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "133: block em inline, remove_child (IFC in block)",
            |d| ifc_in_wrapper(d, "display: block; width: 100px"),
            |d, h| d.remove_child(h[2], h[3]),
        ),
    ];
    for (name, b, m) in cases {
        if let Some(f) = run(name, b, m) {
            eprintln!("FAIL {f}");
            fails.push(f);
        }
    }
    assert!(
        fails.is_empty(),
        "{} failing:\n{}",
        fails.len(),
        fails.join("\n")
    );
}

#[test]
fn review_2142_empty_ifc_variants() {
    let mut fails = Vec::new();
    let cases: Vec<(&str, Build, Mutate)> = vec![
        (
            "block div > for-wrapper > spans: remove last span",
            |d| {
                let body = d.body();
                let g = styled(d, body, "div", "background: red");
                let w = wrap(d, g);
                let s = el(d, w, "span", "");
                text(d, s, "only item");
                vec![g, w, s]
            },
            |d, h| d.remove_child(h[1], h[2]),
        ),
        (
            "block div > for-wrapper (empty): append a span",
            |d| {
                let body = d.body();
                let g = styled(d, body, "div", "");
                let w = wrap(d, g);
                vec![g, w]
            },
            |d, h| {
                let s = el(d, h[1], "span", "");
                text(d, s, "first item");
            },
        ),
        (
            "block div > if-wrapper > text: remove wrapper",
            |d| {
                let body = d.body();
                let g = styled(d, body, "div", "");
                let w = wrap(d, g);
                text(d, w, "shown");
                vec![g, w]
            },
            |d, h| d.remove_child(h[0], h[1]),
        ),
        (
            "block div > wrapper > wrapper > text: remove text",
            |d| {
                let body = d.body();
                let g = styled(d, body, "div", "");
                let w = wrap(d, g);
                let w2 = wrap(d, w);
                let t = text(d, w2, "deep");
                vec![g, w, w2, t]
            },
            |d, h| d.remove_child(h[2], h[3]),
        ),
        (
            "button-like inline-block > wrapper > text: remove text",
            |d| {
                let body = d.body();
                let g = styled(d, body, "span", "display: inline-block");
                let w = wrap(d, g);
                let t = text(d, w, "label text");
                vec![g, w, t]
            },
            |d, h| d.remove_child(h[1], h[2]),
        ),
    ];
    for (name, b, m) in cases {
        if let Some(f) = run(name, b, m) {
            eprintln!("FAIL {f}");
            fails.push(f);
        }
    }
    assert!(
        fails.is_empty(),
        "{} failing:\n{}",
        fails.len(),
        fails.join("\n")
    );
}
