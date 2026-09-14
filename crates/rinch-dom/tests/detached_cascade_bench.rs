//! `#[ignore]`d cost harness for #651 / #668 — what the connectivity skip in
//! `resolve_styles` actually saves.
//!
//! Run it explicitly, and run the two builds **alternately** rather than one
//! after the other (a 24-core host under a parallel agent session does not hold
//! still for long enough to trust a single ordered pair):
//!
//! ```text
//! cargo test -p rinch-dom --release --test detached_cascade_bench -- --ignored --nocapture
//! ```
//!
//! Each case prints one line of minima over its rounds. The shape under test is
//! the issue's own: a reactive `if` whose inactive branch is a large prebuilt
//! panel, sitting detached while the document keeps laying out.
//!
//! # What was measured, and what it says
//!
//! **Wall clock does not move, and the honest reading is that it cannot.** Four
//! alternating rounds of both binaries, release, `resolve_layout` minima in ms:
//!
//! | case | base `ab5ffda` | with the skip |
//! |---|---|---|
//! | `DETACHED-PENDING-500` | 8.03 | 6.39 |
//! | `ATTACHED-PENDING-500` (control) | 8.34 | 11.50 |
//! | `DETACHED-IDLE-500` | 0.003 | 0.004 |
//!
//! The control moved *further* than the case, in the other direction, so the
//! spread here is the host and nothing else. What surrounds the cascade is
//! unchanged and dominates it: `set_attribute` recurses the whole 1001-node
//! subtree in `invalidate_descendant_styles`, and `build_ifc_layouts` collects
//! all 501 IFC roots from the slab whether they are in the document or not.
//! Instrumented, that collection reports `501 roots, 500 detached` on every
//! pass after the removal, at both revisions — which is #628, not this change.
//!
//! **Counted instead of timed, the work removed is exact.** A probe on the
//! Stylo cascade in `resolve_styles_recursive`, over three ticks of
//! `detached_panel_with_a_pending_entry`:
//!
//! | | cascades |
//! |---|---|
//! | base `ab5ffda` | 1503 (501 per tick: the panel and its 500 rows) |
//! | with the skip | 0 |
//!
//! So the claim this change can support is not "layout got faster". It is that
//! 501 Stylo cascades per tick stop happening, and that **every one of them
//! computed a wrong answer** — which is what the fixtures in
//! `detached_style_roots_tests.rs` are actually about.
//!
//! The `DETACHED-IDLE` row is the one to read for a real app: an inactive
//! branch nobody writes to has no pending entry, so the cascade was never
//! reached at either revision and nothing here applies to it.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const ROWS: usize = 500;
const ROUNDS: usize = 12;

/// A document with a small visible panel and a detached `ROWS`-row panel.
///
/// Returns `(doc, detached_root)`.
fn document_with_detached_panel() -> (RinchDocument, rinch_core::dom::NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(
        ".app { font-family: monospace; color: rgb(0,128,0); font-size: 16px; \
                line-height: 20px; } \
         .row { width: 400px; height: 20px; }",
    );
    let body = doc.body();
    let app = doc.create_element("div");
    doc.set_attribute(app, "class", "app");
    doc.append_child(body, app);

    // The branch that is showing.
    let visible = doc.create_element("div");
    let vt = doc.create_text("visible");
    doc.append_child(visible, vt);
    doc.append_child(app, visible);

    // The branch that is not: built under `app`, laid out once, then swapped
    // out — which is what a reactive `if` does.
    let panel = doc.create_element("div");
    doc.set_attribute(panel, "class", "panel");
    for i in 0..ROWS {
        let row = doc.create_element("div");
        doc.set_attribute(row, "class", "row");
        let t = doc.create_text(&format!("row {i} of the inactive branch"));
        doc.append_child(row, t);
        doc.append_child(panel, row);
    }
    doc.append_child(app, panel);
    doc.resolve_layout(800.0, 600.0);
    doc.remove_node(panel);
    doc.resolve_layout(800.0, 600.0);
    (doc, panel)
}

fn min_ms(mut f: impl FnMut() -> f64) -> f64 {
    (0..ROUNDS).map(|_| f()).fold(f64::INFINITY, f64::min)
}

/// The cost this change removes: a pending `style_roots` entry on the detached
/// panel, which used to recascade the whole 1001-node subtree against no
/// parent.
#[test]
#[ignore = "cost harness; run with --ignored --nocapture"]
fn detached_panel_with_a_pending_entry() {
    let (mut doc, panel) = document_with_detached_panel();
    let mut n = 0u32;
    let ms = min_ms(|| {
        n += 1;
        // A reactive effect writing to the swapped-out branch — the only way
        // the detached subtree gets a pending entry at all.
        doc.set_attribute(panel, "data-tick", &n.to_string());
        let t = web_time::Instant::now();
        doc.resolve_layout(800.0, 600.0);
        t.elapsed().as_secs_f64() * 1000.0
    });
    println!("DETACHED-PENDING-{ROWS}: {ms:.3} ms");
}

/// The control: the same document, with the pending entry on a node that *is*
/// in the document. Nothing about this case changes, and a number that moves
/// here is the host, not the patch.
#[test]
#[ignore = "cost harness; run with --ignored --nocapture"]
fn attached_panel_with_a_pending_entry() {
    let (mut doc, panel) = document_with_detached_panel();
    let body = doc.body();
    doc.append_child(body, panel);
    doc.resolve_layout(800.0, 600.0);
    let mut n = 0u32;
    let ms = min_ms(|| {
        n += 1;
        doc.set_attribute(panel, "data-tick", &n.to_string());
        let t = web_time::Instant::now();
        doc.resolve_layout(800.0, 600.0);
        t.elapsed().as_secs_f64() * 1000.0
    });
    println!("ATTACHED-PENDING-{ROWS}: {ms:.3} ms");
}

/// The steady state with **no** pending entry on the detached panel, which is
/// what an inactive branch nobody writes to actually looks like. The cascade
/// is not reached here at either revision — what remains is
/// `build_ifc_layouts` iterating the whole slab (#628), which this change does
/// not touch.
#[test]
#[ignore = "cost harness; run with --ignored --nocapture"]
fn detached_panel_with_no_pending_entry() {
    let (mut doc, _panel) = document_with_detached_panel();
    let body = doc.body();
    let ticker = doc.create_element("div");
    doc.append_child(body, ticker);
    let mut n = 0u32;
    let ms = min_ms(|| {
        n += 1;
        doc.set_attribute(ticker, "data-tick", &n.to_string());
        let t = web_time::Instant::now();
        doc.resolve_layout(800.0, 600.0);
        t.elapsed().as_secs_f64() * 1000.0
    });
    println!("DETACHED-IDLE-{ROWS}: {ms:.3} ms");
}
