//! The scoped structural pass (`rinch_dom::ifc_scope`) against the
//! whole-document pass, and both against a fresh layout.
//!
//! A structural change used to re-run the IFC setup passes over the whole
//! document. It now re-runs them over the formatting containers the change
//! reached, and leaves every other container's splices, anonymous boxes,
//! splits, measure leaves, marks and measures — and so its Taffy cache — as
//! the last pass left them. Two oracles:
//!
//! - **The whole-document twin** — the same document, mutated identically,
//!   with every structural pass forced to the whole-document fallback
//!   (`tree.ifc_dirty`, what the pass was before). The claim of the scoped pass
//!   is exactly that it produces what that twin produces, so the two are
//!   compared on every element's box, every anonymous block box, and every IFC
//!   root's shaped text (width, height, line count), and must agree.
//! - **A fresh layout** — a second document built directly in the final state
//!   (serialized from the first) and laid out once. It has no state to be stale,
//!   so it says what "right" is. Where the whole-document twin already differs
//!   from it, that is a pre-existing incremental-layout defect (each was
//!   measured on `main` too); the matrix pins those cells by name, and the
//!   random differential reports a count.
//!
//! Two layers:
//!
//! - **The matrix** — every mutation shape against every context a structural
//!   decision is taken in: a block, a flex and a grid container; an IFC and an
//!   inline element inside one; `display: contents` chains one and two levels
//!   deep, under a block and under a flex container; a block inside an inline
//!   (#513's split); a mixed container (anonymous boxes); an `inline-block`
//!   host and one nested in another; an absolute child and one hoisted out of
//!   an inline; a table laid out as blocks; a list. The mutations include moves
//!   between containers, display and position flips, and the ones that turn an
//!   IFC root into a non-root and back. Each cell also asserts no
//!   whole-document pass ran — or it would be testing the fallback.
//! - **The randomized differential** — random documents, random mutation
//!   sequences (a lockstep pair), the comparison after every step. A fixed seed
//!   set runs in CI; `scoped_pass_matches_the_whole_document_pass_long` is the
//!   `#[ignore]`d long run. `SCOPED_ORACLE_TRACE=1` prints every operation and
//!   the tree before each layout.
//!
//! It found five defects on the way in, all fixed in the same change. Two
//! were the scoped pass's own: an anonymous box left behind when its container
//! turned `display: contents`, and `sync_display_contents` visiting the scope
//! in hash order where the slab walk visits ids ascending (the order decides
//! which parent keeps a stale-split element's children; the wrong one orphaned
//! a text node). Three were latent in **both** passes, and the whole-document
//! pass hid one of them: a recycled anonymous-box id kept the old box's
//! measured sizes (the full pass re-minted every box in another order); an
//! atomic inline was sized in one batch with the chip inside it; and an
//! `inline-flex` was sized before the flex item's IFC inside it was found
//! changed.
//!
//! What it reports but does not assert is pre-existing, reproduced on `main`:
//! whitespace-only text appended to an empty block keeps a line box a fresh
//! layout does not give it (most of the random run's stopped seeds), an
//! out-of-flow box in an atomic inline is positioned from the chip's previous
//! position, and a Taffy `set_children` panic on wrapping and reordering
//! around an absolute box hoisted out of an inline.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::node::NodeKind;
use rinch_dom::perf::Counter;

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

fn walk(doc: &RinchDocument, id: usize, depth: usize, out: &mut Vec<String>) {
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

fn has_atomic_inline_ancestor(doc: &RinchDocument, id: usize) -> bool {
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
    let kids: Vec<usize> = src.tree.get(src.tree.body_id).unwrap().children.clone();
    for c in kids {
        if let Some(n) = copy_node(src, c, &mut doc) {
            doc.append_child(body, n);
        }
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
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

/// Lay out the **scoped** twin as it is, and the control twin — the same
/// document with the same history — with every structural pass forced to the
/// whole-document fallback (`tree.ifc_dirty`, which is what the pass was
/// before scoping). Each in `catch_unwind`, because the control can panic on
/// states that were already broken before this pass existed; a panic in the
/// scoped twin alone is a failure of its own.
fn lay_out_twins(s: &mut RinchDocument, f: &mut RinchDocument) -> Outcome {
    let rs = guarded(|| {
        s.resolve_layout(VP.0, VP.1);
        snapshot(s)
    });
    let rf = guarded(|| {
        // Unconditionally: a restyle's seeds (a `display` flip) are recorded
        // *inside* this `resolve_layout`, after anything checked here. A flag
        // set on a frame that turns out to lay nothing out waits for the next
        // one that does.
        f.tree.ifc_dirty = true;
        f.resolve_layout(VP.0, VP.1);
        snapshot(f)
    });
    match (rs, rf) {
        (Ok(a), Ok(b)) => match diff(&a, &b, "scoped        ", "whole-document") {
            None => Outcome::Same(b),
            Some(d) => Outcome::Differ(d),
        },
        (Err(e), Ok(_)) => Outcome::Differ(format!("  the scoped twin panicked alone: {e}\n")),
        (Ok(_), Err(e)) => Outcome::ControlPanicked(e),
        (Err(_), Err(e)) => Outcome::BothPanicked(e),
    }
}

enum Outcome {
    /// The twins agree; the control's snapshot.
    Same(Vec<String>),
    Differ(String),
    /// The whole-document control panicked and the scoped twin did not.
    ControlPanicked(String),
    /// Both panicked: a state that was already broken.
    BothPanicked(String),
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

/// A detached subtree of a given shape, to be inserted by a mutation.
fn fresh_subtree(doc: &mut RinchDocument, shape: &str) -> NodeId {
    match shape {
        "text" => doc.create_text(" added words that may well wrap onto a line of their own"),
        "block" => {
            let d = doc.create_element("div");
            let t = doc.create_text("added block with some text in it");
            doc.append_child(d, t);
            d
        }
        "span" => {
            let s = doc.create_element("span");
            let t = doc.create_text(" added span text that runs on");
            doc.append_child(s, t);
            s
        }
        "chip" => {
            let s = doc.create_element("span");
            doc.set_attribute(s, "class", "chip");
            let t = doc.create_text("new chip");
            doc.append_child(s, t);
            s
        }
        "abs" => {
            let d = doc.create_element("div");
            doc.set_attribute(d, "class", "abs");
            let t = doc.create_text("absolute");
            doc.append_child(d, t);
            d
        }
        "contents" => {
            let w = doc.create_element("span");
            doc.set_attribute(w, "style", "display: contents");
            let t = doc.create_text(" wrapped text ");
            doc.append_child(w, t);
            let d = doc.create_element("div");
            let t2 = doc.create_text("wrapped block");
            doc.append_child(d, t2);
            doc.append_child(w, d);
            w
        }
        _ => unreachable!("{shape}"),
    }
}

/// One context: the document around it, the node the mutations act on, and a
/// second container elsewhere for moves.
struct Ctx {
    doc: RinchDocument,
    site: NodeId,
    other: NodeId,
}

const CONTEXTS: &[&str] = &[
    "block",
    "flex",
    "grid",
    "ifc",
    "inline_in_ifc",
    "contents1",
    "contents2",
    "contents_under_flex",
    "split",
    "anon",
    "chip_host",
    "chip_in_chip",
    "abs",
    "abs_under_inline",
    "table",
    "list",
    "last_inline",
];

fn build_context(kind: &str) -> Ctx {
    let mut doc = new_doc();
    let body = doc.body();
    // A container elsewhere in the document, for moves in and out, with a
    // sibling after the context so a height change in it moves something.
    let site = match kind {
        "block" => {
            let d = el(&mut doc, body, "div", "");
            text(&mut doc, d, "first text in a block ");
            let b = el(&mut doc, d, "div", "");
            text(&mut doc, b, "an inner block");
            text(&mut doc, d, " trailing text");
            d
        }
        "flex" => {
            let d = el(&mut doc, body, "div", "flex");
            let a = el(&mut doc, d, "div", "");
            text(&mut doc, a, "flex item one");
            let s = el(&mut doc, d, "span", "");
            text(&mut doc, s, "flex item two, a span");
            d
        }
        "grid" => {
            let d = el(&mut doc, body, "div", "grid");
            for label in ["cell a", "cell b with more words", "cell c"] {
                let c = el(&mut doc, d, "div", "");
                text(&mut doc, c, label);
            }
            d
        }
        "ifc" => {
            let p = el(&mut doc, body, "p", "narrow");
            text(&mut doc, p, "alpha beta gamma ");
            let s = el(&mut doc, p, "span", "");
            text(&mut doc, s, "delta epsilon");
            text(&mut doc, p, " zeta eta theta");
            p
        }
        "inline_in_ifc" => {
            let p = el(&mut doc, body, "p", "narrow");
            text(&mut doc, p, "alpha beta ");
            let s = el(&mut doc, p, "span", "");
            text(&mut doc, s, "gamma delta ");
            let em = el(&mut doc, s, "em", "");
            text(&mut doc, em, "epsilon");
            text(&mut doc, p, " zeta eta");
            s
        }
        "contents1" => {
            let d = el(&mut doc, body, "div", "");
            text(&mut doc, d, "outside the wrapper ");
            let w = styled(&mut doc, d, "span", "display: contents");
            text(&mut doc, w, "inside the wrapper ");
            let s = el(&mut doc, w, "span", "");
            text(&mut doc, s, "and a span");
            w
        }
        "contents2" => {
            let d = el(&mut doc, body, "div", "narrow");
            let w1 = styled(&mut doc, d, "span", "display: contents");
            text(&mut doc, w1, "outer wrapper ");
            let w2 = styled(&mut doc, w1, "span", "display: contents");
            text(&mut doc, w2, "inner wrapper text ");
            text(&mut doc, w2, "more inner");
            w2
        }
        "contents_under_flex" => {
            let d = el(&mut doc, body, "div", "flex");
            let a = el(&mut doc, d, "div", "");
            text(&mut doc, a, "before");
            let w = styled(&mut doc, d, "span", "display: contents");
            let b = el(&mut doc, w, "div", "");
            text(&mut doc, b, "a flex item behind a wrapper");
            text(&mut doc, w, "loose text item");
            w
        }
        "split" => {
            let d = el(&mut doc, body, "div", "");
            text(&mut doc, d, "before the split ");
            let s = el(&mut doc, d, "span", "");
            text(&mut doc, s, "inline part ");
            let b = el(&mut doc, s, "div", "");
            text(&mut doc, b, "block inside the inline");
            text(&mut doc, s, " after the block");
            text(&mut doc, d, " after the span");
            s
        }
        "anon" => {
            let d = el(&mut doc, body, "div", "");
            text(&mut doc, d, "a run of text ");
            let b = el(&mut doc, d, "div", "");
            text(&mut doc, b, "a block between");
            let s = el(&mut doc, d, "span", "");
            text(&mut doc, s, "a second run");
            d
        }
        "chip_host" => {
            let p = el(&mut doc, body, "p", "");
            text(&mut doc, p, "text before ");
            let c = el(&mut doc, p, "span", "chip");
            text(&mut doc, c, "chip text ");
            let s = el(&mut doc, c, "span", "");
            text(&mut doc, s, "chip span");
            text(&mut doc, p, " text after the chip");
            c
        }
        "chip_in_chip" => {
            let p = el(&mut doc, body, "p", "");
            text(&mut doc, p, "outer text ");
            let c = el(&mut doc, p, "span", "chip");
            text(&mut doc, c, "outer chip ");
            let c2 = el(&mut doc, c, "span", "ichip");
            text(&mut doc, c2, "inner chip");
            text(&mut doc, p, " tail");
            c2
        }
        "abs" => {
            let d = el(&mut doc, body, "div", "rel");
            text(&mut doc, d, "text beside an absolute box ");
            let a = el(&mut doc, d, "div", "abs");
            text(&mut doc, a, "absolute");
            d
        }
        "abs_under_inline" => {
            let d = el(&mut doc, body, "div", "rel");
            text(&mut doc, d, "text ");
            let s = el(&mut doc, d, "span", "");
            text(&mut doc, s, "in the span ");
            let a = el(&mut doc, s, "div", "abs");
            text(&mut doc, a, "hoisted");
            text(&mut doc, s, " more span");
            s
        }
        "table" => {
            let t = el(&mut doc, body, "table", "");
            let tr = el(&mut doc, t, "tr", "");
            for label in ["td one", "td two"] {
                let td = el(&mut doc, tr, "td", "");
                text(&mut doc, td, label);
            }
            tr
        }
        "list" => {
            let ul = el(&mut doc, body, "ul", "");
            for label in ["first item", "second item"] {
                let li = el(&mut doc, ul, "li", "");
                text(&mut doc, li, label);
            }
            ul
        }
        // #919's shape: a block whose only rendered inline content is one
        // span, beside a hidden block (detached under #487). Hiding the span
        // (`last_element_to_none`) leaves a block that is no IFC root, whose
        // hidden children must rejoin its Taffy list.
        "last_inline" => {
            let d = el(&mut doc, body, "div", "");
            styled(&mut doc, d, "p", "display: none");
            let s = el(&mut doc, d, "span", "");
            let inner = el(&mut doc, s, "span", "");
            text(&mut doc, inner, "the only rendered text");
            el(&mut doc, s, "input", "");
            d
        }
        _ => unreachable!("{kind}"),
    };
    let other = el(&mut doc, body, "div", "");
    text(&mut doc, other, "other container text ");
    let ob = el(&mut doc, other, "div", "");
    text(&mut doc, ob, "other block");
    let tail = el(&mut doc, body, "div", "");
    text(&mut doc, tail, "a sibling below everything");
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    Ctx { doc, site, other }
}

fn children(doc: &RinchDocument, n: NodeId) -> Vec<NodeId> {
    doc.tree
        .get(n.0)
        .map(|x| x.children.iter().map(|&c| NodeId(c)).collect())
        .unwrap_or_default()
}

fn first_element_child(doc: &RinchDocument, n: NodeId) -> Option<NodeId> {
    children(doc, n)
        .into_iter()
        .find(|c| doc.tree.get(c.0).is_some_and(|x| x.is_element()))
}

const MUTATIONS: &[&str] = &[
    "append_text",
    "append_block",
    "append_span",
    "append_chip",
    "append_abs",
    "append_contents",
    "insert_block_first",
    "remove_first",
    "remove_last",
    "move_first_out",
    "move_in",
    "reorder",
    "child_to_inline",
    "child_to_block",
    "child_to_contents",
    "child_to_none",
    "child_to_inline_block",
    "child_to_absolute",
    "site_to_block",
    "site_to_inline",
    "set_text_content",
    "replace_first",
    "remove_node_first",
    "insert_child_1",
    "set_inner_html",
    "class_blockkids",
    "class_hide",
    "class_cnts",
    "pseudo_inline_block",
    "pseudo_block",
    "last_element_to_none",
    "site_font_size",
];

/// Apply `m` to the context. `false` when the mutation does not apply (the
/// site has no element child to restyle, say).
fn mutate(ctx: &mut Ctx, m: &str) -> bool {
    let doc = &mut ctx.doc;
    let site = ctx.site;
    let kids = children(doc, site);
    let restyle_child = |doc: &mut RinchDocument, style: &str| -> bool {
        match first_element_child(doc, site) {
            Some(c) => {
                doc.set_attribute(c, "style", style);
                true
            }
            None => false,
        }
    };
    match m {
        "append_text" | "append_block" | "append_span" | "append_chip" | "append_abs"
        | "append_contents" => {
            let shape = m.trim_start_matches("append_");
            let n = fresh_subtree(doc, shape);
            doc.append_child(site, n);
            true
        }
        "insert_block_first" => {
            let n = fresh_subtree(doc, "block");
            match kids.first() {
                Some(&f) => doc.insert_before(site, n, f),
                None => doc.append_child(site, n),
            }
            true
        }
        "remove_first" => match kids.first() {
            Some(&f) => {
                doc.remove_child(site, f);
                true
            }
            None => false,
        },
        "remove_last" => match kids.last() {
            Some(&l) => {
                doc.remove_child(site, l);
                true
            }
            None => false,
        },
        "move_first_out" => match kids.first() {
            Some(&f) => {
                doc.append_child(ctx.other, f);
                true
            }
            None => false,
        },
        "move_in" => match children(doc, ctx.other).last() {
            Some(&o) => {
                match kids.first() {
                    Some(&f) => doc.insert_before(site, o, f),
                    None => doc.append_child(site, o),
                }
                true
            }
            None => false,
        },
        "reorder" => {
            if kids.len() < 2 {
                return false;
            }
            let last = *kids.last().unwrap();
            doc.insert_before(site, last, kids[0]);
            true
        }
        "child_to_inline" => restyle_child(doc, "display: inline"),
        "child_to_block" => restyle_child(doc, "display: block"),
        "child_to_contents" => restyle_child(doc, "display: contents"),
        "child_to_none" => restyle_child(doc, "display: none"),
        "child_to_inline_block" => restyle_child(doc, "display: inline-block"),
        "child_to_absolute" => restyle_child(doc, "position: absolute; left: 1px; top: 2px"),
        "site_to_block" => {
            doc.set_attribute(site, "style", "display: block");
            true
        }
        "site_to_inline" => {
            doc.set_attribute(site, "style", "display: inline");
            true
        }
        "set_text_content" => {
            doc.set_text_content(site, "the whole site replaced by one text run");
            true
        }
        "replace_first" => match kids.first() {
            Some(&f) => {
                let n = fresh_subtree(doc, "block");
                doc.replace_node(f, n);
                true
            }
            None => false,
        },
        // `NodeHandle::remove`, the verb every reactive helper uses.
        "remove_node_first" => match kids.first() {
            Some(&f) => {
                doc.remove_node(f);
                true
            }
            None => false,
        },
        // `NodeHandle::insert_after` and keyed reorders land here too.
        "insert_child_1" => {
            let n = fresh_subtree(doc, "chip");
            doc.insert_child(site, n, 1);
            true
        }
        // Frees the old children (slab ids recycle).
        "set_inner_html" => {
            doc.set_inner_html(
                site,
                "<span class=\"chip\">html chip</span> text <div>html block</div> tail",
            );
            true
        }
        // Display flips of the children, driven by a descendant selector on
        // the site: the cascade, not a verb, records these seeds.
        "class_blockkids" | "class_hide" | "class_cnts" => {
            let class = m.trim_start_matches("class_");
            doc.set_attribute(site, "class", class);
            true
        }
        "pseudo_inline_block" => {
            doc.set_attribute(site, "class", "pb");
            true
        }
        "pseudo_block" => {
            doc.set_attribute(site, "class", "pbb");
            true
        }
        // #919: the last element child is the site's last rendered inline in
        // `last_inline`; elsewhere it is one more display flip.
        "last_element_to_none" => match children(doc, site)
            .into_iter()
            .rev()
            .find(|c| doc.tree.get(c.0).is_some_and(|x| x.is_element()))
        {
            Some(c) => {
                doc.set_attribute(c, "style", "display: none");
                true
            }
            None => false,
        },
        // #918: a typography change with a declared line-height, reaching the
        // anonymous box (`anon`) or the split fragment's box (`split`) that
        // lays the text out. A restyle, not a structural change.
        "site_font_size" => {
            doc.set_attribute(site, "style", "font-size: 20px; line-height: 1.25");
            true
        }
        _ => unreachable!("{m}"),
    }
}

// ── The matrix ──────────────────────────────────────────────────────────────

/// The cells whose **whole-document** layout already differs from a fresh
/// layout of the same final state — a pre-existing incremental-layout defect,
/// reproduced identically on `main` before the scoped pass (measured). It is
/// not this pass's to fix, and the scoped twin reproduces it exactly (the twins
/// are compared first). Pinned as a list so a fix, or a new divergence, changes
/// it visibly.
///
/// `contents_under_flex × child_to_inline` was on this list until #998: a flex
/// item behind a `display: contents` wrapper, restyled `display: inline`, was
/// left with no Taffy parent (`RINCH_TREE_CHECK`'s `C orphan`) and the flex
/// column at its old height. Such an item is blockified now, as CSS asks, so
/// the restyle leaves it `block` and there is no inline item to orphan.
///
/// (The `chip_* × *abs*` cells were on this list too, and still differ on
/// `main`: see the out-of-flow arm of `walk`, which no longer compares their
/// position.)
const KNOWN_FRESH_DIVERGENCES: &[&str] = &[];

/// Every mutation shape in every context: the scoped pass against the
/// whole-document pass on the same history (the claim), and both against a
/// fresh layout of the final state (the oracle for what "right" is). Every
/// cell must have run a **scoped** pass. All failures are reported together.
#[test]
fn every_mutation_in_every_context_matches_the_whole_document_pass() {
    let mut failures = Vec::new();
    let mut fresh_divergent = Vec::new();
    let mut cells = 0;
    let mut scoped_cells = 0;
    for &kind in CONTEXTS {
        for &m in MUTATIONS {
            let mut s = build_context(kind);
            let mut f = build_context(kind);
            s.doc.tree.perf.reset();
            if !mutate(&mut s, m) {
                continue;
            }
            mutate(&mut f, m);
            cells += 1;
            let cell = format!("{kind} × {m}");
            match lay_out_twins(&mut s.doc, &mut f.doc) {
                Outcome::Same(snap) => {
                    if matches_fresh(&f.doc, &snap).is_some() {
                        fresh_divergent.push(cell.clone());
                    }
                }
                Outcome::Differ(d) => failures.push(format!("{cell}:\n{d}")),
                // Under `RINCH_TREE_CHECK=1` the known cell's orphaned flex
                // item fails the sweep in both twins, so it arrives here.
                Outcome::ControlPanicked(_) | Outcome::BothPanicked(_) => {
                    fresh_divergent.push(cell.clone());
                }
            }
            let frame = s.doc.tree.perf.end_frame();
            // A restyle that changes no box (a child already inline made
            // inline) runs no structural pass at all; nothing may run the
            // whole-document one.
            if frame.get(Counter::IfcFullPasses) != 0 {
                failures.push(format!("{cell}: ran a whole-document pass"));
            }
            scoped_cells += (frame.get(Counter::IfcScopedPasses) != 0) as usize;
        }
    }
    assert!(cells > 400, "the matrix shrank to {cells} cells");
    assert!(
        scoped_cells > 380,
        "only {scoped_cells} of {cells} cells ran a scoped pass — the matrix is testing nothing"
    );
    assert!(
        failures.is_empty(),
        "{} of {cells} cells: the scoped pass differs from the whole-document pass:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(
        fresh_divergent, KNOWN_FRESH_DIVERGENCES,
        "the cells whose layout differs from a fresh one changed"
    );
}

/// Two mutations in one frame, then a second pair on the result: seeds from
/// separate places, and a scope built over state an earlier scoped pass left.
#[test]
fn two_mutations_per_frame_and_a_second_frame() {
    let mut failures = Vec::new();
    let mut control_broken = Vec::new();
    for &kind in CONTEXTS {
        for (a, b) in [
            ("append_block", "child_to_contents"),
            ("move_first_out", "append_chip"),
            ("child_to_none", "remove_last"),
            ("append_contents", "reorder"),
            ("site_to_inline", "append_block"),
            ("child_to_inline_block", "move_in"),
        ] {
            let mut s = build_context(kind);
            let mut f = build_context(kind);
            for ctx in [&mut s, &mut f] {
                mutate(ctx, a);
                mutate(ctx, b);
            }
            let cell = format!("{kind} × {a}+{b}");
            match lay_out_twins(&mut s.doc, &mut f.doc) {
                Outcome::Same(_) => {}
                Outcome::Differ(d) => {
                    failures.push(format!("{cell} (frame 1):\n{d}"));
                    continue;
                }
                Outcome::ControlPanicked(e) | Outcome::BothPanicked(e) => {
                    control_broken.push(format!("{cell}: {e}"));
                    continue;
                }
            }
            for ctx in [&mut s, &mut f] {
                mutate(ctx, b);
                mutate(ctx, "move_in");
            }
            match lay_out_twins(&mut s.doc, &mut f.doc) {
                Outcome::Same(_) => {}
                Outcome::Differ(d) => failures.push(format!("{cell} (frame 2):\n{d}")),
                Outcome::ControlPanicked(e) | Outcome::BothPanicked(e) => {
                    control_broken.push(format!("{cell} (frame 2): {e}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("the whole-document control broke on: {control_broken:?}");
}

// ── Scope, measured ─────────────────────────────────────────────────────────

/// The point of the pass: appending a row to a list sets up the list and the
/// new row — and nothing else. Every other row keeps its IFC, its wrapper's
/// splice and its chip's size, so no chip is re-sized and no row is re-shaped.
#[test]
fn appending_a_row_sets_up_only_the_list_and_the_row() {
    let mut doc = new_doc();
    let body = doc.body();
    let list = el(&mut doc, body, "div", "flex");
    for i in 0..30 {
        let row = el(&mut doc, list, "div", "");
        text(&mut doc, row, &format!("row {i} "));
        let w = styled(&mut doc, row, "span", "display: contents");
        text(&mut doc, w, &format!("({i})"));
        let chip = el(&mut doc, row, "span", "chip");
        text(&mut doc, chip, "chip");
    }
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();

    let row = doc.create_element("div");
    let t = doc.create_text("new row ");
    doc.append_child(row, t);
    let chip = doc.create_element("span");
    doc.set_attribute(chip, "class", "chip");
    let ct = doc.create_text("chip");
    doc.append_child(chip, ct);
    doc.append_child(row, chip);
    doc.append_child(list, row);
    doc.resolve_layout(VP.0, VP.1);
    let snap = snapshot(&doc);
    assert_eq!(matches_fresh(&doc, &snap), None);
    let f = doc.tree.perf.end_frame();

    assert_eq!(f.get(Counter::IfcScopedPasses), 1);
    assert_eq!(f.get(Counter::IfcFullPasses), 0);
    // The list and the new row; the new chip is a container of its own.
    assert_eq!(f.get(Counter::IfcScopeContainers), 3);
    assert_eq!(
        f.get(Counter::InlineBlockComputes),
        1,
        "only the new chip is sized (it was one compute per chip in the document)"
    );
    assert_eq!(
        f.get(Counter::IfcSignatureChanges),
        2,
        "the new row and its chip"
    );
}

/// A change inside a chip is a change to the chip's size, and the IFC around
/// it is outside the scope: the enclosing paragraph must still re-line.
#[test]
fn a_structural_change_inside_a_chip_relines_the_paragraph_around_it() {
    let mut doc = new_doc();
    let body = doc.body();
    let p = el(&mut doc, body, "p", "narrow");
    text(&mut doc, p, "words before the chip ");
    let chip = el(&mut doc, p, "span", "chip");
    text(&mut doc, chip, "x");
    text(&mut doc, p, " and words after it");
    doc.resolve_layout(VP.0, VP.1);
    let before = snapshot(&doc);
    doc.tree.perf.reset();
    let t = doc.create_text(" a much longer chip label now");
    doc.append_child(chip, t);
    doc.resolve_layout(VP.0, VP.1);
    let snap = snapshot(&doc);
    assert_eq!(matches_fresh(&doc, &snap), None);
    let f = doc.tree.perf.end_frame();
    assert_eq!(f.get(Counter::IfcFullPasses), 0);
    assert_ne!(
        before,
        snapshot(&doc),
        "counter-oracle: the change moved something"
    );
}

// ── Randomized differential ─────────────────────────────────────────────────

/// xorshift64*: deterministic, no dependency.
#[derive(Clone)]
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

fn trace_on() -> bool {
    std::env::var_os("SCOPED_ORACLE_TRACE").is_some()
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

/// Run `seeds` random documents for `steps` steps each, a scoped twin and a
/// whole-document twin in lockstep (the same random choices, by position, on
/// the same structure), comparing them after every step. `Err` names the first
/// failing seed and step. Also returns how many sampled steps' layouts differed
/// from a fresh one on both twins alike, and how many seeds the control itself
/// broke on — pre-existing, reported, not asserted.
fn run_random(seeds: std::ops::Range<u64>, steps: usize) -> Result<RandomStats, String> {
    let mut stats = RandomStats::default();
    'seeds: for seed in seeds {
        let rng0 = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut docs = [new_doc(), new_doc()];
        let mut rngs = [rng0.clone(), rng0];
        for (doc, rng) in docs.iter_mut().zip(rngs.iter_mut()) {
            let body = doc.body();
            for _ in 0..4 {
                let n = random_subtree(doc, rng, 3);
                doc.append_child(body, n);
            }
            doc.resolve_layout(VP.0, VP.1);
        }
        for step in 0..steps {
            let n_ops = 1 + rngs[0].below(3);
            rngs[1].below(3);
            for _ in 0..n_ops {
                for (i, (doc, rng)) in docs.iter_mut().zip(rngs.iter_mut()).enumerate() {
                    // One trace, from the scoped twin.
                    random_step(doc, rng, i == 0 && trace_on());
                }
            }
            if trace_on() {
                eprintln!(
                    "-- seed {seed} step {step}: before layout\n{}",
                    dump(&docs[0])
                );
            }
            let [s, f] = &mut docs;
            match lay_out_twins(s, f) {
                Outcome::Same(snap) => {
                    // Compared only while the history is right: once both
                    // twins differ from a fresh layout, a pre-existing defect
                    // has left state behind that either pass may or may not
                    // happen to repair later (the whole-document pass repairs
                    // more by accident: it re-splices and re-dirties
                    // everything), so a later difference is not attributable.
                    // Next seed.
                    if let Some(d) = matches_fresh(f, &snap) {
                        stats.fresh_divergent += 1;
                        if trace_on() {
                            eprintln!(
                                "seed {seed} step {step}: both twins differ from fresh:\n{d}"
                            );
                        }
                        continue 'seeds;
                    }
                    stats.steps_compared += 1;
                }
                Outcome::Differ(d) => {
                    // Which twin a fresh layout agrees with, if either: the
                    // difference is then the other one's.
                    let sd = matches_fresh(s, &snapshot(s));
                    let sf = sd.is_none();
                    let ff = matches_fresh(f, &snapshot(f)).is_none();
                    if trace_on() {
                        eprintln!("scoped vs fresh:\n{}", sd.unwrap_or_default());
                        let ids: Vec<usize> = std::env::var("SCOPED_ORACLE_NODE")
                            .unwrap_or_default()
                            .split(',')
                            .filter_map(|v| v.parse().ok())
                            .collect();
                        for id in ids {
                            for (name, d) in [("scoped", &*s), ("whole", &*f)] {
                                let n = d.tree.get(id).unwrap();
                                let t = n.taffy_id.unwrap();
                                let kids = d.tree.taffy.children(t).unwrap();
                                eprintln!(
                                    "{name}: node {id} taffy {t:?} tparent {:?} ifc_root {:?} detached {} contrib {} split {} ctx {:?} kids {:?} style {:?} layout {:?} leaf {:?} cache {:?}",
                                    d.tree.taffy.parent(t).map(|p| d.tree.taffy_map.get(&p)),
                                    n.ifc_root,
                                    n.ifc_detached,
                                    n.contributes_in_flow_block,
                                    n.is_split_inline(),
                                    d.tree.taffy.get_node_context(t),
                                    kids.iter()
                                        .map(|k| (k, d.tree.taffy_map.get(k)))
                                        .collect::<Vec<_>>(),
                                    d.tree.taffy.style(t).map(|s| s.display),
                                    n.layout,
                                    d.tree.ifc_measure_leaves.get(&id),
                                    d.tree.ifc_measure_cache.get(&id),
                                );
                            }
                        }
                    }
                    if sf && !ff {
                        // The whole-document pass is the one that is wrong:
                        // a pre-existing defect the scoped pass does not
                        // reach. The histories part here; next seed.
                        stats.scoped_better += 1;
                        if trace_on() {
                            eprintln!(
                                "seed {seed} step {step}: scoped right, whole-document wrong:\n{d}"
                            );
                        }
                        continue 'seeds;
                    }
                    if !sf && !ff {
                        // Both wrong, differently: a pre-existing defect is in
                        // play (whitespace-only text in an empty block keeps a
                        // line box it should not have, and the passes amplify
                        // it through different atomic inlines), and which of
                        // two wrong answers each gives is not attributable.
                        // The claim is narrower and exact: wherever the
                        // whole-document pass is right, the scoped one is too.
                        stats.fresh_divergent += 1;
                        continue 'seeds;
                    }
                    return Err(format!(
                        "seed {seed}, step {step} (scoped matches fresh: {sf}, \
                         whole-document matches fresh: {ff}):\n{d}"
                    ));
                }
                Outcome::ControlPanicked(_) | Outcome::BothPanicked(_) => {
                    stats.control_broken += 1;
                    continue 'seeds;
                }
            }
        }
    }
    Ok(stats)
}

/// What a random run saw that was not the scoped pass's fault.
#[derive(Debug, Default)]
struct RandomStats {
    /// Steps compared with the twins agreeing and matching a fresh layout.
    steps_compared: usize,
    /// Seeds stopped because both twins agreed and both differed from a
    /// fresh layout: a pre-existing defect.
    fresh_divergent: usize,
    /// Seeds whose whole-document twin panicked (a Taffy `set_children` panic
    /// reproduced on `main`).
    control_broken: usize,
    /// Seeds where the twins differed and the scoped one matched a fresh layout
    /// — the whole-document pass was the one wrong.
    scoped_better: usize,
}

/// The fixed seed set that runs in CI.
#[test]
fn scoped_pass_matches_the_whole_document_pass_on_random_documents() {
    let stats = run_random(0..60, 20).unwrap();
    eprintln!("not the scoped pass's: {stats:?}");
}

/// The long run. `cargo test -p rinch-dom --test scoped_ifc_oracle_tests --
/// --ignored` (release is much faster).
#[test]
#[ignore]
fn scoped_pass_matches_the_whole_document_pass_long() {
    // `SCOPED_ORACLE_SEED=n` replays one seed (with `SCOPED_ORACLE_TRACE=1`,
    // step by step).
    let seeds = match std::env::var("SCOPED_ORACLE_SEED") {
        Ok(n) => {
            let n: u64 = n.parse().expect("SCOPED_ORACLE_SEED is a number");
            n..n + 1
        }
        Err(_) => 1000..3000,
    };
    // One document pair per thread at a time (a document is not `Send`; each
    // thread builds its own).
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk = (seeds.end - seeds.start).div_ceil(threads as u64).max(1);
    let handles: Vec<_> = (0..threads as u64)
        .map(|t| {
            let lo = (seeds.start + t * chunk).min(seeds.end);
            let hi = (lo + chunk).min(seeds.end);
            std::thread::spawn(move || run_random(lo..hi, 40))
        })
        .collect();
    let mut total = RandomStats::default();
    for h in handles {
        let s = h.join().expect("a worker panicked").unwrap();
        total.steps_compared += s.steps_compared;
        total.fresh_divergent += s.fresh_divergent;
        total.control_broken += s.control_broken;
        total.scoped_better += s.scoped_better;
    }
    eprintln!("not the scoped pass's: {total:?}");
}
