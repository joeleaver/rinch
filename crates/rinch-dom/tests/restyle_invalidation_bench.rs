//! The workloads behind #654's claim that the staleness gate in
//! `apply_stylo_styles_to_taffy` is worth its comparison.
//!
//! `#[ignore]`d, so `cargo test` never runs them. To measure:
//!
//! ```text
//! cargo test --release -p rinch-dom --test restyle_invalidation_bench -- --ignored --nocapture
//! ```
//!
//! **Build both variants' binaries first and run them alternately.** These
//! numbers move by a factor of five with machine load, and this repository
//! shares a host with other agents' gates; a measurement of variant A taken
//! before variant B was compiled is a measurement of the host, and the first
//! round of #654 reported one. `cargo test --no-run --message-format=json`
//! prints the binary path, so:
//!
//! ```text
//! # build head's binary, stash it, apply the mutant, build again, then:
//! for i in 1 2 3 4 5; do ./head-bin --ignored --nocapture; ./mutant-bin --ignored --nocapture; done
//! ```
//!
//! and compare the **minima**, which are the least contaminated statistic here.
//!
//! The mutant the gate exists to beat is `same_text_layout_inputs` returning
//! `false` unconditionally: correct, and it re-invalidates every restyled
//! node's Parley layout and Taffy measurement whether or not any typography
//! changed.
//!
//! # What this harness has actually shown so far: nothing
//!
//! An earlier revision of #654 claimed the gate saved 2-9x on the reversal.
//! That number was taken without interleaving the variants, on this host at a
//! load average of 98 across 24 cores, and it is **withdrawn**. Both the PR's
//! reviewer and a re-run through this harness — binaries built once, run
//! alternately, four rounds short-text and three rounds with `ROWTEXT` set —
//! found head and the unconditional mutant indistinguishable, with the mutant
//! ahead as often as behind. Minima over four interleaved rounds, ms:
//!
//! | workload | head | unconditional |
//! |---|---|---|
//! | REVERSAL-500 | 18.7 | 17.6 |
//! | RESTYLE-3000-no-typography | 23.3 | 20.6 |
//! | FIRSTBUILD-500 | 59.2 | 65.7 |
//!
//! So the gate is kept on a mechanical argument — a scalar comparison in place
//! of a guaranteed invalidation cannot be the slower of the two — and not on
//! evidence. Anyone who wants the evidence should run this on a quiet host;
//! that is what it is committed for.

#![cfg(feature = "software-renderer")]

use std::time::Instant;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const CSS: &str = ".sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }";

/// Longer row text makes each row's shaping dominate, which is what an
/// unconditional invalidation pays for. Set `ROWTEXT` to a sentence to measure
/// the wrapping case.
fn row_text(i: usize) -> String {
    format!("row {i} {}", std::env::var("ROWTEXT").unwrap_or_default())
}

fn report(name: &str, mut times: Vec<f64>) {
    times.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    println!(
        "{name} ms: min={:.1} med={:.1} max={:.1}",
        times[0],
        times[times.len() / 2],
        times[times.len() - 1]
    );
}

fn build_rows(rows: usize) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "sans");
    doc.append_child(body, root);
    let mut ids = Vec::with_capacity(rows);
    for i in 0..rows {
        let row = doc.create_element("div");
        let label = doc.create_element("span");
        let text = doc.create_text(&row_text(i));
        doc.append_child(label, text);
        doc.append_child(row, label);
        doc.append_child(root, row);
        ids.push(row);
    }
    (doc, root, ids)
}

/// A keyed list reversal: every row is re-inserted under the **same** parent,
/// so its recascade changes no typography at all. This is the workload the gate
/// protects — an unconditional invalidation re-shapes 500 rows for nothing.
#[test]
#[ignore]
fn bench_reversal_500() {
    let mut times = Vec::new();
    for _ in 0..12 {
        let (mut doc, root, ids) = build_rows(500);
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        for &id in ids.iter().rev() {
            doc.append_child(root, id);
        }
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("REVERSAL-500", times);
}

/// The first build, where every node is styled for the first time. Included
/// because the gate's removed `!has_been_styled` clause only ever ran here, so
/// a reversal alone could not have shown what it cost.
#[test]
#[ignore]
fn bench_first_build_500() {
    let mut times = Vec::new();
    for _ in 0..12 {
        let t = Instant::now();
        let (mut doc, _root, _ids) = build_rows(500);
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("FIRSTBUILD-500", times);
}

/// A restyle of ~3000 elements in which no typography changes: a class swap
/// whose two classes differ only in `background-color`, on a block. The gate
/// should make this cost nothing beyond the comparison itself.
#[test]
#[ignore]
fn bench_restyle_3000_no_typography() {
    let mut times = Vec::new();
    for _ in 0..12 {
        let mut doc = RinchDocument::new();
        doc.load_css(
            ".sans { font-family: sans-serif; font-size: 16px; line-height: 20px; }
             .p0 { background-color: rgb(1,1,1); } .p1 { background-color: rgb(2,2,2); }",
        );
        let body = doc.body();
        let root = doc.create_element("div");
        doc.set_attribute(root, "class", "sans p0");
        doc.append_child(body, root);
        for i in 0..1000 {
            let row = doc.create_element("div");
            let label = doc.create_element("span");
            let text = doc.create_text(&row_text(i));
            doc.append_child(label, text);
            doc.append_child(row, label);
            doc.append_child(root, row);
        }
        doc.resolve_layout(800.0, 600.0);
        let t = Instant::now();
        doc.set_attribute(root, "class", "sans p1");
        doc.resolve_layout(800.0, 600.0);
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    report("RESTYLE-3000-no-typography", times);
}
