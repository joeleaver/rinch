//! What the #661/#678 repair costs a document that is not affected by it.
//!
//! `#[ignore]`d, so `cargo test` never runs them. To measure:
//!
//! ```text
//! cargo test --release -p rinch-dom --test frozen_box_remeasure_bench -- --ignored --nocapture
//! ```
//!
//! **Build both variants' binaries first and run them alternately**, and compare
//! the **minima** — these numbers move by a factor of five with machine load,
//! and this repository shares a host with other agents' gates. The neighbouring
//! `restyle_invalidation_bench` carries the recipe and the story of a number
//! that had to be withdrawn for skipping it.
//!
//! The two things worth measuring, and the mutants they are measured against:
//!
//! - **A text change in a big document** — `bench_one_text_change_in_500_rows`
//!   and its `_with_atomic_inlines` twin. The repair re-measures atomic inlines
//!   on a pass that computes Taffy without rebuilding the IFC structure, and
//!   the whole point of the `dirty_atomic_inlines` **set** is that one row's
//!   edit costs one re-measure and not five hundred. The mutant is the obvious
//!   simplification — call `compute_inline_block_layouts()` (which measures
//!   every atomic inline in the document) instead of
//!   `remeasure_dirty_atomic_inlines()`. It is *correct*; it is what the set
//!   exists to beat.
//! - **A paint-only restyle** — `bench_colour_only_restyle_500`. #678 widens
//!   what sets `layout_dirty`, and the cheap path it must not swallow is this
//!   one. `frozen_box_remeasure_tests::a_colour_only_restyle_still_skips_taffy`
//!   is the *behavioural* pin on that; this is what says the behaviour is worth
//!   having.

#![cfg(feature = "software-renderer")]

use std::time::Instant;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = "
    .sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
    .p0 { background-color: rgb(1,1,1); }
    .p1 { background-color: rgb(2,2,2); }
    .chip { display: inline-block; padding: 2px; }
";

fn report(name: &str, mut times: Vec<f64>) {
    times.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    println!(
        "{name} ms: min={:.2} med={:.2} max={:.2}",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

/// 500 rows of `div > span > text`, optionally with an `inline-block` chip
/// beside the text in every row. Returns the text node of each row.
fn build_rows(rows: usize, chips: bool) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "sans p0");
    doc.append_child(body, root);
    let mut texts = Vec::with_capacity(rows);
    for i in 0..rows {
        let row = doc.create_element("div");
        let label = doc.create_element("span");
        let text = doc.create_text(&format!("row {i}"));
        doc.append_child(label, text);
        doc.append_child(row, label);
        if chips {
            let chip = doc.create_element("span");
            doc.set_attribute(chip, "class", "chip");
            let chip_text = doc.create_text("new");
            doc.append_child(chip, chip_text);
            doc.append_child(row, chip);
        }
        doc.append_child(root, row);
        texts.push(text);
    }
    (doc, root, texts)
}

fn time_one_text_change(name: &str, chips: bool) {
    let mut times = Vec::new();
    for round in 0..12 {
        let (mut doc, _root, texts) = build_rows(500, chips);
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        doc.set_text_content(texts[250], &format!("row 250 edited {round}"));
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report(name, times);
}

/// One row's text changes in a 500-row document with no atomic inline in it.
/// The floor: whatever the repair costs here it costs every document.
#[test]
#[ignore]
fn bench_one_text_change_in_500_rows() {
    time_one_text_change("TEXTCHANGE-500", false);
}

/// The same, with an `inline-block` chip in every row — 500 atomic inlines, one
/// of which is inside the row that changed. This is the pair that separates the
/// dirty set from the whole-document re-measure.
#[test]
#[ignore]
fn bench_one_text_change_in_500_rows_with_atomic_inlines() {
    time_one_text_change("TEXTCHANGE-500-CHIPS", true);
}

/// A paint-only restyle of 500 rows: the cheap path #678 must not swallow.
#[test]
#[ignore]
fn bench_colour_only_restyle_500() {
    let mut times = Vec::new();
    for round in 0..12 {
        let (mut doc, root, _texts) = build_rows(500, true);
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        doc.set_attribute(
            root,
            "class",
            if round % 2 == 0 { "sans p1" } else { "sans p0" },
        );
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("COLOURONLY-500", times);
}

/// A typography-only restyle of 500 rows: the compute #678 **adds**.
///
/// Not a regression to optimise away — a box around re-wrapped text has to be
/// measured again, and before this it simply was not. This is what that costs,
/// against the mutant that is `main`'s behaviour: drop the
/// `if measured_size_stale { layout_dirty = true }` arm.
#[test]
#[ignore]
fn bench_typography_restyle_500() {
    let mut times = Vec::new();
    // Always `.a` → `.b`: setting the class a node already carries is a no-op,
    // so a fixture that alternates the *target* does real work on half its
    // rounds and prints a bimodal distribution whose median means nothing.
    for _ in 0..12 {
        let mut doc = RinchDocument::new();
        doc.load_css(
            ".a { font-family: sans-serif; font-size: 16px; line-height: 20px; }
             .b { font-family: monospace;  font-size: 16px; line-height: 20px; }",
        );
        let body = doc.body();
        let root = doc.create_element("div");
        doc.set_attribute(root, "class", "a");
        doc.append_child(body, root);
        for i in 0..500 {
            let row = doc.create_element("div");
            let text = doc.create_text(&format!("row {i}"));
            doc.append_child(row, text);
            doc.append_child(root, row);
        }
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        doc.set_attribute(root, "class", "b");
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("TYPOGRAPHY-500", times);
}

/// The realistic shape of the compute above: **one** row of 500 changes its
/// typography, as a `:hover { font-weight: bold }` does.
///
/// The whole-document version is the worst case (a theme font swap); this is
/// the one an app pays on a pointer move.
#[test]
#[ignore]
fn bench_typography_restyle_one_row_of_500() {
    let mut times = Vec::new();
    for _ in 0..12 {
        let mut doc = RinchDocument::new();
        doc.load_css(
            ".a { font-family: sans-serif; font-size: 16px; line-height: 20px; }
             .b { font-family: monospace;  font-size: 16px; line-height: 20px; }",
        );
        let body = doc.body();
        let root = doc.create_element("div");
        doc.set_attribute(root, "class", "a");
        doc.append_child(body, root);
        let mut rows = Vec::new();
        for i in 0..500 {
            let row = doc.create_element("div");
            let text = doc.create_text(&format!("row {i}"));
            doc.append_child(row, text);
            doc.append_child(root, row);
            rows.push(row);
        }
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        doc.set_attribute(rows[250], "class", "b");
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("TYPOGRAPHY-1-OF-500", times);
}
