//! The pruned, cached hit test answers exactly what the unpruned one did.
//!
//! `hit_test` skips subtrees whose flow extent misses the point and reuses
//! stacking sequences across probes (`rinch_dom::hit_cache`). Both are pure
//! optimisations, so the oracle here is the walk they replaced, kept verbatim
//! as [`reference_hit_test_node`] — no extent, no cache, a fresh
//! `stacking_paint_order` per stacking root per probe. Every fixture compares
//! the two over a grid of points that reaches past the window on every side.
//!
//! The documents are generated from a fixed seed and mix everything the walk
//! special-cases: `relative`/`absolute`/`fixed`/`sticky` boxes with and
//! without `z-index`, negative offsets that overflow a parent, transforms,
//! `opacity`, `overflow: hidden`/`auto`/`clip` scrolled to a non-zero offset,
//! `pointer-events: none`, `visibility: hidden`, `display: contents`/`none`,
//! inline-blocks in text flow, and flex containers. The cache half is pinned
//! by probing, mutating (a style write, a scroll, a structural change, each
//! followed by a layout), and probing again: a stale memo would answer from
//! the old geometry.

use super::{Frame, descend, hit_test};
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::stacking::{paints_at_stacking_root, stacking_paint_order};

/// The oracle: `hit_test` as it was before the prune and the cache.
fn reference_hit_test(tree: &rinch_dom::NodeTree, x: f32, y: f32) -> Option<usize> {
    reference_hit_test_node(tree, tree.body_id, 0.0, 0.0, x, y, x, y)
}

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
    match r.below(10) {
        0 => s += "overflow: hidden; ",
        1 => s += "overflow: auto; ",
        2 => s += "overflow: clip; ",
        3 => s += "overflow-y: scroll; ",
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

/// Compare the two over a grid; returns how many points were probed.
#[track_caller]
fn assert_agree(doc: &RinchDocument, label: &str) -> usize {
    let mut probed = 0;
    let mut y = -23.0f32;
    while y < 640.0 {
        let mut x = -23.0f32;
        while x < 840.0 {
            let want = reference_hit_test(&doc.tree, x, y);
            let got = hit_test(&doc.tree, x, y);
            assert_eq!(
                got, want,
                "{label}: the pruned hit test disagrees at ({x}, {y})"
            );
            probed += 1;
            x += 7.0;
        }
        y += 7.0;
    }
    probed
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

/// A long list in a scroller — the shape the prune exists for — scrolled to
/// the middle, with positioned rows and a nested clipper so the clip chain and
/// the `PositionedAuto` entries are exercised too.
#[test]
fn a_scrolled_list_hits_identically() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let scroller = doc.create_element("div");
    doc.set_attribute(
        scroller,
        "style",
        "height: 400px; overflow-y: auto; margin: 10px",
    );
    doc.append_child(body, scroller);
    for i in 0..300 {
        let row = doc.create_element("div");
        let pos = if i % 7 == 0 {
            "position: relative; "
        } else {
            ""
        };
        doc.set_attribute(
            row,
            "style",
            &format!("{pos}height: 20px; margin-left: {}px", i % 5),
        );
        let t = doc.create_text(&format!("row {i}"));
        doc.append_child(row, t);
        doc.append_child(scroller, row);
        if i % 11 == 0 {
            let over = doc.create_element("div");
            doc.set_attribute(
                over,
                "style",
                "position: relative; left: -30px; top: 35px; width: 50px; height: 50px",
            );
            doc.append_child(row, over);
        }
    }
    doc.resolve_layout(800.0, 600.0);
    for top in [0.0, 1234.5, 5000.0] {
        doc.set_scroll_top(scroller, top);
        assert_agree(&doc, &format!("scrolled to {top}"));
    }
}

// ── The oracle, verbatim from before the prune ──────────────────────────────

#[allow(clippy::too_many_arguments)]
fn reference_hit_test_node(
    tree: &rinch_dom::NodeTree,
    node_id: usize,
    offset_x: f32,
    offset_y: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
) -> Option<usize> {
    tree.perf
        .bump(rinch_dom::perf::Counter::HitTestNodesVisited);
    let node = tree.get(node_id)?;

    // The `position: fixed` hoist, the IFC content-box offset, the transform
    // inverse and the body's viewport re-seed all live in `descend`, which every
    // probe-point walk in this file shares.
    let Frame {
        nx,
        ny,
        x,
        y,
        vx,
        vy,
    } = descend(tree, node, node_id, offset_x, offset_y, x, y, vx, vy)?;

    let nw = node.layout.width;
    let nh = node.layout.height;

    // Skip entire subtree when visibility: hidden — per CSS spec, hidden elements
    // and their descendants don't receive pointer events. This also guards against
    // Stylo not cascading visibility to children when a parent's class changes.
    if matches!(
        node.computed_style.visibility,
        rinch_dom::computed_style::VisibilityValue::Hidden
            | rinch_dom::computed_style::VisibilityValue::Collapse
    ) {
        return None;
    }

    let point_in_bounds = x >= nx && x <= nx + nw && y >= ny && y <= ny + nh;

    // Nodes with overflow clipping must restrict child hit testing to within
    // bounds — the same predicate paint clips pixels with (#324), so a box
    // cannot be drawn somewhere it cannot be tapped.
    let check_children = !node.clips_overflow() || point_in_bounds;

    let sx = node.scroll_offset.0 as f32;
    let sy = node.scroll_offset.1 as f32;

    // `check_children` is deliberately not a gate around the whole block: a
    // `position: fixed` entry of this root's sequence is not clipped by this
    // root (#545), so it must be probed even when the point is outside these
    // bounds. It is applied per entry below instead.
    //
    // The cost was weighed rather than overlooked: a stacking-context root now
    // builds its `stacking_paint_order` and walks it even when the probe misses
    // its box, skipping every non-fixed entry. That is real work per probe on a
    // tree with many out-of-bounds clipping stacking contexts, and probes run
    // per pointer move. Nothing cheaper is correct — knowing whether a subtree
    // holds a fixed box is what building the sequence answers — and if it ever
    // shows up in a profile the fix is a per-node "has a fixed descendant" bit
    // maintained at style time, not moving this gate back.
    {
        // An IFC text node's layout is stretched to the whole container
        // (write_inline_positions, for scroll-height) — those artificial bounds
        // would shadow inline-block siblings laid out in the same text flow,
        // swallowing clicks meant for e.g. a button. Skip it: the IFC root
        // itself is returned for plain-text hits and text selection resolves by
        // walking up to it.
        let is_stretched_ifc_text =
            |child: &rinch_dom::Node| child.is_text() && child.ifc_root.is_some();

        if node_id == tree.body_id || node.creates_stacking_context() {
            // A stacking-context root probes exactly the sequence paint draws,
            // read backwards — the last box painted is the first one tapped.
            // Same function, same offsets, opposite direction: the two cannot
            // drift apart the way two hand-written phase walks did.
            let order =
                stacking_paint_order(tree, node_id, 1.0, (nx - sx) as f64, (ny - sy) as f64);
            for entry in order.iter().rev() {
                let Some(child) = tree.get(entry.node_id) else {
                    continue;
                };
                // This root's own bounds gate. A `position: fixed` entry is
                // exempt: this root is not its containing block, so its clip
                // does not apply — paint lifts the same bracket around it
                // (`paint_children_with_stacking`, #545). Without the exemption
                // a fixed modal inside an `overflow: hidden; z-index: 1` panel
                // would be painted and never tappable.
                let escapes_root_clip = child.computed_style.position
                    == rinch_dom::computed_style::PositionValue::Fixed;
                if !check_children && !escapes_root_clip {
                    continue;
                }
                if is_stretched_ifc_text(child) {
                    continue;
                }
                // The clipping ancestors this entry was hoisted past. Since
                // #324 an `overflow` box is not a stacking context, so the
                // walk went straight through them and the `check_children`
                // gate below never saw them — the chain is what keeps a
                // hoisted box unreachable exactly where it is unpainted.
                //
                // The clips are in this root's own space, which is the space
                // `x`/`y` are in here, and at scale 1.0 because that is what
                // this walk asked `stacking_paint_order` for.
                if !order
                    .clips_for(entry)
                    .iter()
                    .all(|c| c.contains(x as f64, y as f64))
                {
                    continue;
                }
                if let Some(hit) = reference_hit_test_node(
                    tree,
                    entry.node_id,
                    entry.offset_x as f32,
                    entry.offset_y as f32,
                    x,
                    y,
                    vx,
                    vy,
                ) {
                    return Some(hit);
                }
            }
        } else if check_children {
            // Not a stacking-context root — test only the children that were
            // not hoisted to an ancestor's sequence, in reverse tree order.
            // Nothing here can be a fixed box: one creates a stacking context
            // unconditionally, so `paints_at_stacking_root` skips it and it is
            // an entry of some ancestor's sequence instead. So this branch keeps
            // the plain bounds gate.
            for &child_id in rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id)
                .iter()
                .rev()
            {
                let Some(child) = tree.get(child_id) else {
                    continue;
                };
                if paints_at_stacking_root(child) || is_stretched_ifc_text(child) {
                    continue;
                }
                if let Some(hit) =
                    reference_hit_test_node(tree, child_id, nx - sx, ny - sy, x, y, vx, vy)
                {
                    return Some(hit);
                }
            }
        }
    }

    if !point_in_bounds {
        return None;
    }

    // Skip this element if pointer-events: none (children still checked above)
    if matches!(
        node.computed_style.pointer_events,
        rinch_dom::computed_style::PointerEventsValue::None
    ) {
        return None;
    }

    Some(node_id)
}
