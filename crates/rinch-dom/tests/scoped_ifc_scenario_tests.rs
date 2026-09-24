//! Hand-written scenarios for the scoped IFC structural pass
//! (`rinch_dom::ifc_scope`), written by the adversarial review of #895 to reach
//! the verbs and inputs the randomized differential in
//! `scoped_ifc_oracle_tests.rs` did not: pseudo-element regeneration,
//! `set_inner_html` churn (slab id recycling), display flips driven by a
//! descendant selector under a `display: contents` chain, three-level nested
//! inline-blocks, `remove_node`, `set_text_content` over element children, a
//! block inserted between inline runs, an inline element gaining a block two
//! `contents` levels deep inside a flex item, and a mutation made while the
//! subtree was detached.
//!
//! Each scenario builds a document, lays it out, applies steps one frame at a
//! time, and after each frame compares the scoped document with (a) a
//! whole-document twin with the same history and (b) a fresh document built
//! from the final DOM.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::NodeKind;

const VP: (f32, f32) = (800.0, 600.0);

const CSS: &str = "
    body { margin: 0; font-family: sans-serif; font-size: 16px; line-height: 20px; width: 360px; }
    p, ul, li, div { margin: 0; padding: 0; }
    .chip { display: inline-block; padding: 2px; }
    .blk { display: block; }
    .inl { display: inline; }
    .ib { display: inline-block; }
    .cnt { display: contents; }
    .none { display: none; }
    .flex { display: flex; }
    .abs { position: absolute; left: 3px; top: 4px; width: 60px; }
    .pb::before { content: 'generated before text that is long enough to wrap around'; display: inline-block; width: 90px; }
    .pbb::before { content: 'generated block'; display: block; }
    .pbi::before { content: 'gen inline words here'; }
    .hide > .kid { display: none; }
    .blockkids > .kid { display: block; }
    .pct { display: inline-block; width: 50%; }
    .row { display: flex; }
    .grow { flex: 1; }
";

fn r(v: f32) -> f32 {
    (v * 64.0).round() / 64.0
}

fn snapshot(doc: &RinchDocument) -> Vec<String> {
    let mut out = Vec::new();
    walk(doc, doc.tree.body_id, 0, &mut out);
    out
}

fn walk(doc: &RinchDocument, id: usize, depth: usize, out: &mut Vec<String>) {
    let Some(node) = doc.tree.get(id) else { return };
    let pad = depth * 2;
    match &node.kind {
        NodeKind::Text(t) => out.push(format!("{:pad$}{:?}", "", t.content)),
        NodeKind::Comment(_) => out.push(format!("{:pad$}<!---->", "")),
        _ => {
            let l = &node.layout;
            out.push(format!(
                "{:pad$}<{}{}> @({}, {}) {}x{}",
                "",
                node.tag().unwrap_or("?"),
                if node.is_pseudo_element {
                    " pseudo"
                } else {
                    ""
                },
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
    if node.computed_style.display == rinch_dom::computed_style::DisplayValue::None {
        return;
    }
    for &c in &node.children {
        walk(doc, c, depth + 1, out);
    }
}

fn new_doc() -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    doc
}

fn copy_node(src: &RinchDocument, id: usize, dst: &mut RinchDocument) -> Option<NodeId> {
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

fn fresh(src: &RinchDocument) -> Vec<String> {
    let mut doc = new_doc();
    let body = doc.body();
    let kids: Vec<usize> = src.tree.get(src.tree.body_id).unwrap().children.clone();
    for c in kids {
        if let Some(n) = copy_node(src, c, &mut doc) {
            doc.append_child(body, n);
        }
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    snapshot(&doc)
}

fn diff(a: &[String], b: &[String]) -> Option<String> {
    if a == b {
        return None;
    }
    let mut out = String::new();
    let mut shown = 0;
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y && shown < 8 {
            out.push_str(&format!("  #{i}\n    got  : {x}\n    want : {y}\n"));
            shown += 1;
        }
    }
    if a.len() != b.len() {
        out.push_str(&format!("  lengths {} vs {}\n", a.len(), b.len()));
    }
    Some(out)
}

type Step = Box<dyn Fn(&mut RinchDocument, &[NodeId])>;

/// Returns a list of problems.
fn run(name: &str, build: fn(&mut RinchDocument) -> Vec<NodeId>, steps: Vec<Step>) -> Vec<String> {
    let mut s = new_doc();
    let mut w = new_doc();
    let hs = build(&mut s);
    let hw = build(&mut w);
    s.resolve_layout(VP.0, VP.1);
    w.resolve_layout(VP.0, VP.1);
    let mut problems = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        step(&mut s, &hs);
        step(&mut w, &hw);
        let full0 = s.tree.perf.get(rinch_dom::perf::Counter::IfcFullPasses);
        s.resolve_layout(VP.0, VP.1);
        let full1 = s.tree.perf.get(rinch_dom::perf::Counter::IfcFullPasses);
        w.tree.ifc_dirty = true;
        w.resolve_layout(VP.0, VP.1);
        let ss = snapshot(&s);
        let ws = snapshot(&w);
        let fs = fresh(&s);
        let tag = if full1 > full0 { " [full pass]" } else { "" };
        if let Some(d) = diff(&ss, &ws) {
            problems.push(format!("{name} step {i}{tag}: scoped vs whole-doc\n{d}"));
        }
        if let Some(d) = diff(&ss, &fs) {
            let wd = diff(&ws, &fs).is_some();
            problems.push(format!(
                "{name} step {i}{tag}: scoped vs FRESH (whole-doc also wrong: {wd})\n{d}"
            ));
        }
    }
    problems
}

fn mk(doc: &mut RinchDocument, parent: NodeId, tag: &str, class: &str, txt: &str) -> NodeId {
    let n = doc.create_element(tag);
    if !class.is_empty() {
        doc.set_attribute(n, "class", class);
    }
    if !txt.is_empty() {
        let t = doc.create_text(txt);
        doc.append_child(n, t);
    }
    doc.append_child(parent, n);
    n
}

fn report(all: Vec<String>) {
    for p in &all {
        eprintln!("{p}");
    }
    assert!(all.is_empty(), "{} problems", all.len());
}

// 1. pseudo-element inline-block ::before toggled by class on a span in a paragraph
#[test]
fn probe_pseudo_inline_block_toggle() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead text ");
        let s = mk(doc, p, "span", "", "span words ");
        mk(
            doc,
            p,
            "span",
            "",
            " and trailing text that goes on for a while",
        );
        mk(doc, body, "div", "", "after");
        vec![p, s]
    }
    report(run(
        "pseudo_ib",
        build,
        vec![
            Box::new(|d, h| d.set_attribute(h[1], "class", "pb")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "pbb")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "pbi")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "")),
            Box::new(|d, h| d.set_attribute(h[0], "class", "pb")),
            Box::new(|d, h| d.set_attribute(h[0], "class", "pbb")),
        ],
    ));
}

// 2. set_inner_html repeatedly (id recycling) with chips and blocks inside an IFC
#[test]
fn probe_set_inner_html_churn() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead ");
        let s = mk(doc, p, "span", "", "x");
        let d = mk(doc, body, "div", "", "block host");
        mk(doc, body, "div", "", "after");
        vec![p, s, d]
    }
    report(run(
        "inner_html",
        build,
        vec![
            Box::new(|d, h| {
                d.set_inner_html(
                    h[1],
                    "<span class=\"chip\">chip one two</span> tail <b>bold</b>",
                )
            }),
            Box::new(|d, h| d.set_inner_html(h[1], "<div>a block in a span</div> tail")),
            Box::new(|d, h| {
                d.set_inner_html(
                    h[2],
                    "<span>inline</span><div>block</div><span>inline2</span>",
                )
            }),
            Box::new(|d, h| d.set_inner_html(h[1], "plain")),
            Box::new(|d, h| {
                d.set_inner_html(
                    h[2],
                    "<span class=\"chip\"><span class=\"chip\">nested chip text</span></span>",
                )
            }),
            Box::new(|d, h| d.set_inner_html(h[2], "")),
        ],
    ));
}

// 3. class-driven display flips through a descendant selector (cascade reaches kid)
#[test]
fn probe_descendant_selector_display_flip() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead ");
        let w = mk(doc, p, "span", "cnt", "");
        let w2 = mk(doc, w, "span", "cnt", "");
        mk(doc, w2, "span", "kid", "kid one ");
        mk(doc, w2, "span", "kid", "kid two ");
        mk(doc, p, "span", "", "tail text");
        mk(doc, body, "div", "", "after");
        vec![p, w, w2]
    }
    report(run(
        "desc_flip",
        build,
        vec![
            Box::new(|d, h| d.set_attribute(h[2], "class", "cnt blockkids")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "cnt hide")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "cnt")),
            Box::new(|d, h| d.set_attribute(h[0], "class", "hide")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "blockkids")),
        ],
    ));
}

// 4. three-level nested inline-blocks: innermost text change relines outer
#[test]
fn probe_nested_chips_three_levels() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead text lead text ");
        let c1 = mk(doc, p, "span", "chip", "");
        let c2 = mk(doc, c1, "span", "chip", "");
        let c3 = mk(doc, c2, "span", "chip", "");
        let t = doc.create_text("x");
        doc.append_child(c3, t);
        mk(doc, p, "span", "", " trailing words after the chip");
        mk(doc, body, "div", "", "after");
        vec![p, c1, c2, c3, t]
    }
    report(run(
        "nested3",
        build,
        vec![
            Box::new(|d, h| {
                d.set_text_content(
                    h[4],
                    "a much longer innermost text that should make every chip grow",
                )
            }),
            Box::new(|d, h| {
                let s = d.create_element("div");
                let t = d.create_text("block inside innermost");
                d.append_child(s, t);
                d.append_child(h[3], s);
            }),
            Box::new(|d, h| d.set_text_content(h[4], "y")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "chip")),
        ],
    ));
}

// 5. percentage inline-block whose container width changes from a sibling change
#[test]
fn probe_percentage_chip_container_resized_by_sibling() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let row = mk(doc, body, "div", "row", "");
        let left = mk(doc, row, "div", "", "L");
        let right = mk(doc, row, "p", "grow", "text ");
        let pc = mk(
            doc,
            right,
            "span",
            "pct",
            "percentage chip with words that wrap",
        );
        mk(doc, right, "span", "", " tail");
        mk(doc, body, "div", "", "after");
        vec![row, left, right, pc]
    }
    // Pre-existing, reproduced on `main`: the left item keeps its old width
    // (230 where a fresh layout gives 218) and the percentage chip a stale
    // position and size. The whole-document twin is equally wrong, so only the
    // twin claim is asserted; the fresh divergence is printed.
    let problems = run(
        "pct",
        build,
        vec![
            Box::new(|d, h| {
                let t = d.create_text(" left grows with a lot of words");
                d.append_child(h[1], t);
            }),
            Box::new(|d, h| d.set_attribute(h[1], "style", "width: 250px")),
            Box::new(|d, h| {
                let c = d.tree.get(h[1].0).unwrap().children.clone();
                for x in c {
                    d.remove_child(h[1], NodeId(x));
                }
            }),
        ],
    );
    for p in &problems {
        eprintln!("(pre-existing) {}", p.lines().next().unwrap_or(""));
    }
    assert!(
        problems
            .iter()
            .all(|p| p.contains("whole-doc also wrong: true")),
        "the scoped pass differs from the whole-document one: {problems:#?}"
    );
}

// 6. remove_node (NodeHandle::remove) and set_text_content over element children
#[test]
fn probe_remove_node_and_text_over_children() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead ");
        let s = mk(doc, p, "span", "", "");
        let b = mk(doc, s, "div", "", "block in span splits it");
        mk(doc, s, "span", "chip", "chip");
        mk(doc, p, "span", "", " tail tail tail");
        mk(doc, body, "div", "", "after");
        vec![p, s, b]
    }
    report(run(
        "remove_text",
        build,
        vec![
            Box::new(|d, h| d.remove_node(h[2])),
            Box::new(|d, h| d.append_child(h[1], h[2])),
            Box::new(|d, h| d.set_text_content(h[1], "now just text")),
            Box::new(|d, h| d.append_child(h[1], h[2])),
            Box::new(|d, h| d.set_text_content(h[0], "paragraph text only")),
        ],
    ));
}

// 7. whitespace-only text between blocks, and a block inserted between inline runs
#[test]
fn probe_block_between_inline_runs() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let d = mk(doc, body, "div", "", "");
        let a = doc.create_text("run one words ");
        doc.append_child(d, a);
        let s = mk(doc, d, "span", "", "span in run");
        let b = doc.create_text(" run continues");
        doc.append_child(d, b);
        mk(doc, body, "div", "", "after");
        vec![d, a, s, b]
    }
    report(run(
        "block_between",
        build,
        vec![
            Box::new(|d, h| {
                let x = d.create_element("div");
                let t = d.create_text("inserted block");
                d.append_child(x, t);
                d.insert_before(h[0], x, h[2]);
            }),
            Box::new(|d, h| d.set_attribute(h[2], "class", "blk")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "ib")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "abs")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "flex")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "")),
        ],
    ));
}

// 8. an inline element gains its first block child deep in a contents chain in a flex item
#[test]
fn probe_inline_gains_block_in_flex_item() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let f = mk(doc, body, "div", "flex", "");
        let item = mk(doc, f, "div", "", "item text ");
        let c1 = mk(doc, item, "span", "cnt", "");
        let c2 = mk(doc, c1, "span", "cnt", "");
        let s = mk(doc, c2, "span", "", "inline ");
        let loose = doc.create_text("loose flex text");
        doc.append_child(f, loose);
        mk(doc, body, "div", "", "after");
        vec![f, item, c1, c2, s]
    }
    report(run(
        "inline_gains_block",
        build,
        vec![
            Box::new(|d, h| {
                let x = d.create_element("div");
                let t = d.create_text("first block");
                d.append_child(x, t);
                d.append_child(h[4], x);
            }),
            Box::new(|d, h| d.set_attribute(h[3], "class", "")),
            Box::new(|d, h| d.set_attribute(h[2], "class", "")),
            Box::new(|d, h| {
                let c = d.tree.get(h[4].0).unwrap().children.clone();
                d.remove_child(h[4], NodeId(*c.last().unwrap()));
            }),
            Box::new(|d, h| d.set_attribute(h[1], "class", "cnt")),
        ],
    ));
}

// 9. a mutation inside a detached subtree, then re-attach
#[test]
fn probe_detached_mutation_then_reattach() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let p = mk(doc, body, "p", "", "lead ");
        let s = mk(doc, p, "span", "", "span ");
        let c = mk(doc, s, "span", "chip", "chip");
        mk(doc, body, "div", "", "after");
        vec![p, s, c]
    }
    report(run(
        "detached",
        build,
        vec![
            Box::new(|d, h| d.remove_child(h[0], h[1])),
            Box::new(|d, h| {
                let x = d.create_element("div");
                let t = d.create_text("block added while detached");
                d.append_child(x, t);
                d.append_child(h[1], x);
                d.set_attribute(h[2], "class", "blk");
            }),
            Box::new(|d, h| d.append_child(h[0], h[1])),
            Box::new(|d, h| {
                let body = d.body();
                d.append_child(body, h[1]);
            }),
        ],
    ));
}

// 10. perf: a hover-like restyle of a node carrying ::after, no display change
#[test]
fn probe_pseudo_restyle_cost() {
    use rinch_dom::perf::Counter;
    let mut doc = new_doc();
    doc.load_css(".btn { display: inline-flex; padding: 2px; } .btn::after { content: 'x'; } .btn.hov { color: red; } .plain.hov { color: red; }");
    let body = doc.body();
    let p = mk(&mut doc, body, "p", "", "lead ");
    let mut btns = Vec::new();
    for _ in 0..20 {
        btns.push(mk(&mut doc, p, "span", "btn", "button"));
    }
    let plain = mk(&mut doc, p, "span", "plain", "plain");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    for (name, n, on, off) in [
        ("btn", btns[3], "btn hov", "btn"),
        ("plain", plain, "plain hov", "plain"),
    ] {
        for cls in [on, off] {
            doc.tree.perf.reset();
            doc.set_attribute(n, "class", cls);
            doc.resolve_layout(VP.0, VP.1);
            let (scoped, full) = (
                doc.tree.perf.get(Counter::IfcScopedPasses),
                doc.tree.perf.get(Counter::IfcFullPasses),
            );
            assert_eq!(full, 0, "{name} -> {cls:?}: a whole-document pass");
            // The button's `::after` is regenerated by its restyle, so its
            // IFC is set up again — scoped, one button, where `main` ran a
            // whole-document pass per such hover. A plain span sets up nothing.
            let want = if name == "btn" { 1 } else { 0 };
            assert_eq!(scoped, want, "{name} -> {cls:?}: scoped passes");
            if name == "btn" {
                assert_eq!(doc.tree.perf.get(Counter::InlineBlockComputes), 1);
            }
        }
    }
}

// 11. generated content that goes away: the regenerated pseudo-element is the
// only structural change, and its old node — a member of an anonymous box's
// run in a mixed container — is freed.
#[test]
fn probe_pseudo_content_removed_from_a_mixed_container() {
    fn build(doc: &mut RinchDocument) -> Vec<NodeId> {
        let body = doc.body();
        let d = mk(doc, body, "div", "", "run text ");
        let s = mk(doc, d, "span", "pbi", "span ");
        mk(doc, d, "div", "", "a block splits the runs");
        mk(doc, d, "span", "", " second run");
        mk(doc, body, "div", "", "after");
        vec![d, s]
    }
    report(run(
        "pseudo_removed",
        build,
        vec![
            Box::new(|d, h| d.set_attribute(h[1], "class", "")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "pbb")),
            Box::new(|d, h| d.set_attribute(h[1], "class", "")),
            Box::new(|d, h| d.set_attribute(h[0], "class", "pbi")),
            Box::new(|d, h| d.set_attribute(h[0], "class", "")),
        ],
    ));
}
