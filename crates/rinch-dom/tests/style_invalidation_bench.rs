//! Wall-clock and counter harness for the style-invalidation paths
//! (`perf/style-invalidation`): hover over a deep subtree, a class toggle on
//! a list container, an attribute nobody selects on, a window resize with and
//! without viewport-unit rules, and a subtree built detached and then
//! attached.
//!
//! `#[ignore]`d and informational: nothing is asserted. The exact counters for
//! the same shapes are pinned in `perf_counter_baselines.rs`; this file is
//! where the timings come from. Run it in release:
//!
//! ```text
//! cargo test --release -p rinch-dom --test style_invalidation_bench -- --ignored --nocapture
//! ```

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::perf::Counter;

const VP: (f32, f32) = (1024.0, 768.0);
const ROWS: usize = 300;
const REPS: usize = 30;

const BASE_CSS: &str = "
    body { font-size: 16px; line-height: 20px; }
    .list { display: flex; flex-direction: column; }
    .row { padding: 2px; }
    .row:hover { background-color: rgb(200, 0, 0); }
    .card { padding: 4px; }
    .card:hover { background-color: rgb(0, 200, 0); }
    .hot:hover .title { color: rgb(0, 0, 255); }
    .list.dense .row { padding: 0; }
    .chip { display: inline-block; padding: 2px; }
";

struct Doc {
    doc: RinchDocument,
    list: NodeId,
    card: NodeId,
    rows: Vec<NodeId>,
}

/// `body > div.card > (div > div > … depth 6) > ROWS × div.row > span.title + span.chip`
/// plus a sibling `div.list` holding the same rows; the card is the deep
/// subtree a hover crosses, the list is the container a class lands on.
fn build(extra_css: &str) -> Doc {
    let mut doc = RinchDocument::new();
    doc.load_css(BASE_CSS);
    if !extra_css.is_empty() {
        doc.load_css(extra_css);
    }
    let body = doc.body();
    let card = doc.create_element("div");
    doc.set_attribute(card, "class", "card");
    let mut parent = card;
    for _ in 0..6 {
        let d = doc.create_element("div");
        doc.append_child(parent, d);
        parent = d;
    }
    let list = doc.create_element("div");
    doc.set_attribute(list, "class", "list");
    let mut rows = Vec::new();
    for i in 0..ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let title = doc.create_element("span");
        doc.set_attribute(title, "class", "title");
        let t = doc.create_text(&format!("Row {i} title text"));
        doc.append_child(title, t);
        doc.append_child(row, title);
        let chip = doc.create_element("span");
        doc.set_attribute(chip, "class", "chip");
        let ct = doc.create_text("chip");
        doc.append_child(chip, ct);
        doc.append_child(row, chip);
        doc.append_child(list, row);
        rows.push(row);
    }
    doc.append_child(parent, list);
    doc.append_child(body, card);
    doc.resolve_layout(VP.0, VP.1);
    doc.resolve_layout(VP.0, VP.1);
    doc.tree.perf.reset();
    Doc {
        doc,
        list,
        card,
        rows,
    }
}

fn run(name: &str, extra_css: &str, op: impl Fn(&mut Doc, usize)) {
    let mut d = build(extra_css);
    let mut best = f64::MAX;
    let mut counters = (0, 0);
    let mut best_style = f64::MAX;
    for r in 0..REPS {
        let t = std::time::Instant::now();
        op(&mut d, r);
        d.doc.resolve_layout(VP.0, VP.1);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let s = d.doc.tree.perf.end_frame();
        best_style = best_style.min(s.get(Counter::TimeStyleNs) as f64 / 1e6);
        if ms < best {
            best = ms;
            counters = (
                s.get(Counter::ElementsCascaded),
                s.get(Counter::StyleNodesVisited),
            );
        }
    }
    println!(
        "{name:<44} {best:>9.3} ms  (style {best_style:>7.3} ms)   cascaded {:>6}   visited {:>6}",
        counters.0, counters.1
    );
}

fn hover(d: &mut Doc, n: Option<NodeId>) {
    let mut changed = false;
    d.doc.update_hover(n.map(|n| n.0), &mut changed);
}

#[test]
#[ignore]
fn style_invalidation_timings() {
    println!("{:<44} {:>12}", "scenario", "best");
    // Hover enters/leaves the card: `.card:hover` changes only the card's own
    // background, but the card holds the whole list.
    run("hover card (self-only rule, deep subtree)", "", |d, r| {
        let c = d.card;
        hover(d, (r % 2 == 0).then_some(c));
    });
    // The same with a descendant rule keyed on the card's hover.
    run("hover card (.hot:hover .title)", "", |d, r| {
        if r == 0 {
            let c = d.card;
            d.doc.set_attribute(c, "class", "card hot");
        }
        let c = d.card;
        hover(d, (r % 2 == 0).then_some(c));
    });
    run("hover one row", "", |d, r| {
        let row = d.rows[ROWS / 2];
        hover(d, (r % 2 == 0).then_some(row));
    });
    // A class nobody's descendant selectors mention.
    run("class toggle on list (no dependent rule)", "", |d, r| {
        let l = d.list;
        d.doc
            .set_attribute(l, "class", if r % 2 == 0 { "list open" } else { "list" });
    });
    // A class a descendant rule depends on: every row must restyle.
    run("class toggle on list (.list.dense .row)", "", |d, r| {
        let l = d.list;
        d.doc
            .set_attribute(l, "class", if r % 2 == 0 { "list dense" } else { "list" });
    });
    run("unselected attribute on list (data-foo)", "", |d, r| {
        let l = d.list;
        d.doc.set_attribute(l, "data-foo", &r.to_string());
    });
    run("resize 1px (no viewport rules)", "", |d, r| {
        d.doc.resolve_layout(VP.0 + 1.0 + (r % 2) as f32, VP.1);
    });
    run(
        "resize 1px (vw rule on the chips)",
        ".chip { width: 5vw; }",
        |d, r| {
            d.doc.resolve_layout(VP.0 + 1.0 + (r % 2) as f32, VP.1);
        },
    );
    run(
        "resize 1px (@media, result unchanged)",
        "@media (min-width: 400px) { .row { margin: 1px; } }",
        |d, r| {
            d.doc.resolve_layout(VP.0 + 1.0 + (r % 2) as f32, VP.1);
        },
    );
    // Build a 3-level subtree detached, then attach it.
    run("append detached 20-node subtree", "", |d, _| {
        let root = d.doc.create_element("div");
        d.doc.set_attribute(root, "class", "row");
        for _ in 0..4 {
            let a = d.doc.create_element("div");
            for _ in 0..3 {
                let b = d.doc.create_element("span");
                let t = d.doc.create_text("x");
                d.doc.append_child(b, t);
                d.doc.append_child(a, b);
            }
            d.doc.append_child(root, a);
        }
        let l = d.list;
        d.doc.append_child(l, root);
    });
}
