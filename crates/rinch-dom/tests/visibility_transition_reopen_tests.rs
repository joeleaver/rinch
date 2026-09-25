//! Review probes for PR #991 (#759), round 2 — kept as fixtures.
//!
//! The checkbox-in-a-closing-drawer shape against Chrome 153's order of events
//! (root held visible, the kid's own hide starting at the root's flip, the
//! grandchild following the kid's animated value), removal mid-close, nesting,
//! and the two reopen races the round-2 review found (`p3`, `p9`): a *running*
//! visibility transition whose current value already equals the reopened
//! after-change value must be cancelled (css-transitions-1 §3 item 4.1), even
//! though `diff_animatable` sees no change in the value.
//!
//! `cost_close_pass_500_rows` is an `#[ignore]`d timing harness.
#![cfg(feature = "software-renderer")]
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::VisibilityValue;
use rinch_dom::transition::TransitionProperty as TP;

const CSS: &str = "
    div { font-size: 16px; line-height: 20px; }
    .root  { width: 200px; height: 100px; }
    .shut  { visibility: hidden; }
    .delayed.shut { transition: visibility 0s linear 300ms; }
    .kid { width: 50px; height: 20px; }
    .cb { transition: all 150ms linear; }
    .late { transition: visibility 0s linear 500ms; }
    .anim { transition: width 1000ms linear; }
    .wide { width: 90px; }
";
fn vis(doc: &RinchDocument, n: NodeId) -> VisibilityValue {
    doc.tree.get(n.0).unwrap().computed_style.visibility
}
fn tick(doc: &mut RinchDocument, at: f64) {
    rinch_dom::transition::tick_transitions(&mut doc.tree, at);
}
fn vstart(doc: &RinchDocument, n: NodeId) -> Option<f64> {
    doc.tree
        .active_transitions
        .get(&n.0)
        .and_then(|m| m.get(&TP::Visibility))
        .map(|t| t.start_time_ms)
}

fn setup(kid_class: &str) -> (RinchDocument, NodeId, NodeId, NodeId) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "root delayed");
    doc.append_child(body, root);
    let kid = doc.create_element("div");
    doc.set_attribute(kid, "class", kid_class);
    doc.append_child(root, kid);
    let gk = doc.create_element("div");
    doc.set_attribute(gk, "class", "kid");
    doc.append_child(kid, gk);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, root, kid, gk)
}

fn close(doc: &mut RinchDocument, root: NodeId, w: f32) -> f64 {
    doc.set_attribute(root, "class", "root delayed shut");
    doc.resolve_layout(w, 600.0);
    vstart(doc, root).expect("root close transition")
}

/// (1a) root ends -> kid starts exactly one transition, at the flip, never restarted by
/// later ticks or cascade passes.
#[test]
fn p1_kid_starts_one_transition_at_flip_and_not_again() {
    let (mut doc, root, kid, gk) = setup("kid cb");
    let s = close(&mut doc, root, 801.0);
    for t in [50.0, 100.0, 200.0, 299.0] {
        tick(&mut doc, s + t);
        assert_eq!(vstart(&doc, kid), None, "no kid transition at {t}");
        assert_eq!(vis(&doc, kid), VisibilityValue::Visible);
    }
    tick(&mut doc, s + 301.0);
    let k0 = vstart(&doc, kid).expect("kid transition started at flip");
    assert_eq!(k0, s + 301.0);
    for (i, t) in [320.0, 340.0, 360.0].iter().enumerate() {
        tick(&mut doc, s + t);
        // interleave cascade passes (viewport change) during kid's transition
        doc.resolve_layout(810.0 + i as f32, 600.0);
        assert_eq!(
            vstart(&doc, kid),
            Some(k0),
            "kid transition not restarted at {t}"
        );
        assert_eq!(
            vis(&doc, kid),
            VisibilityValue::Visible,
            "kid visible at {t}"
        );
        assert_eq!(
            vis(&doc, gk),
            VisibilityValue::Visible,
            "gk inherits kid's animated value at {t}"
        );
    }
    tick(&mut doc, s + 460.0);
    assert_eq!(vis(&doc, kid), VisibilityValue::Hidden);
    assert_eq!(vis(&doc, gk), VisibilityValue::Hidden);
    assert!(
        doc.tree.active_transitions.is_empty(),
        "{:?}",
        doc.tree.active_transitions.keys().collect::<Vec<_>>()
    );
    for t in [500.0, 600.0] {
        tick(&mut doc, s + t);
        doc.resolve_layout(830.0 + t as f32 / 100.0, 600.0);
    }
    assert!(doc.tree.active_transitions.is_empty());
    assert_eq!(vis(&doc, kid), VisibilityValue::Hidden);
}

/// (1b) reopen mid-close (before the root flips): kid never transitions, stays visible.
#[test]
fn p2_reopen_before_flip_kid_untouched() {
    let (mut doc, root, kid, _) = setup("kid cb");
    let s = close(&mut doc, root, 801.0);
    tick(&mut doc, s + 150.0);
    doc.set_attribute(root, "class", "root delayed");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(vstart(&doc, root), None);
    assert_eq!(
        vstart(&doc, kid),
        None,
        "no spurious kid transition on reopen"
    );
    for t in [200.0, 310.0, 500.0, 700.0] {
        tick(&mut doc, s + t);
        assert_eq!(vis(&doc, kid), VisibilityValue::Visible, "at {t}");
        assert_eq!(vis(&doc, root), VisibilityValue::Visible);
    }
}

/// (1c) reopen AFTER the root flipped but while the kid's own 150ms hide runs:
/// Chrome cancels the kid's running transition (current value == after-change value).
#[test]
fn p3_reopen_during_kid_hide_kid_stays_visible() {
    let (mut doc, root, kid, gk) = setup("kid cb");
    let s = close(&mut doc, root, 801.0);
    tick(&mut doc, s + 301.0);
    assert!(vstart(&doc, kid).is_some(), "precondition: kid hiding");
    tick(&mut doc, s + 350.0);
    doc.set_attribute(root, "class", "root delayed");
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(vis(&doc, root), VisibilityValue::Visible);
    for t in [360.0, 460.0, 600.0, 900.0] {
        tick(&mut doc, s + t);
        assert_eq!(
            vis(&doc, kid),
            VisibilityValue::Visible,
            "kid visible in a reopened drawer at {t}; kid transition {:?}",
            doc.tree
                .active_transitions
                .get(&kid.0)
                .map(|m| m.keys().collect::<Vec<_>>())
        );
        assert_eq!(vis(&doc, gk), VisibilityValue::Visible, "gk at {t}");
    }
}

/// (1d) root removed mid-close, then ticks: nothing runs, no panic.
#[test]
fn p4_root_removed_mid_close() {
    let (mut doc, root, kid, _) = setup("kid cb");
    let s = close(&mut doc, root, 801.0);
    tick(&mut doc, s + 100.0);
    doc.remove_node(root);
    doc.resolve_layout(802.0, 600.0);
    for t in [301.0, 500.0] {
        tick(&mut doc, s + t);
    }
    assert_eq!(vstart(&doc, kid), None);
    assert!(
        doc.tree.active_transitions.is_empty(),
        "{:?}",
        doc.tree.active_transitions.keys().collect::<Vec<_>>()
    );
}

/// (1e) kid removed mid-close, root ends: kid (detached) is not given a transition.
#[test]
fn p5_kid_removed_mid_close() {
    let (mut doc, root, kid, _) = setup("kid cb");
    let s = close(&mut doc, root, 801.0);
    tick(&mut doc, s + 100.0);
    doc.remove_node(kid);
    doc.resolve_layout(802.0, 600.0);
    tick(&mut doc, s + 301.0);
    tick(&mut doc, s + 500.0);
    assert_eq!(vstart(&doc, kid), None);
    assert!(doc.tree.active_transitions.is_empty());
}

/// (2) kid with longer delay: Chrome — root flips at 300, kid starts its own 0s/500ms
/// transition then, hidden at 800.
#[test]
fn p6_longer_delay_kid_hides_at_800() {
    let (mut doc, root, kid, gk) = setup("kid late");
    let s = close(&mut doc, root, 801.0);
    tick(&mut doc, s + 301.0);
    assert_eq!(vis(&doc, root), VisibilityValue::Hidden);
    for t in [400.0, 700.0, 799.0] {
        tick(&mut doc, s + t);
        assert_eq!(vis(&doc, kid), VisibilityValue::Visible, "at {t}");
        assert_eq!(vis(&doc, gk), VisibilityValue::Visible, "gk at {t}");
    }
    tick(&mut doc, s + 802.0);
    assert_eq!(vis(&doc, kid), VisibilityValue::Hidden);
    assert_eq!(vis(&doc, gk), VisibilityValue::Hidden);
}

/// (2b) nested: inner closing-pattern root (open) inside closing outer. Inner open
/// state declares no visibility transition -> held visible, hidden at 300.
/// Inner CLOSED (own hidden) inside closing outer stays hidden throughout.
#[test]
fn p7_nested_inner_open_and_inner_closed() {
    let (mut doc, root, kid, _) = setup("kid");
    let inner_open = doc.create_element("div");
    doc.set_attribute(inner_open, "class", "root delayed");
    doc.append_child(kid, inner_open);
    let io_kid = doc.create_element("div");
    doc.set_attribute(io_kid, "class", "kid cb");
    doc.append_child(inner_open, io_kid);
    let inner_shut = doc.create_element("div");
    doc.set_attribute(inner_shut, "class", "root delayed shut");
    doc.append_child(kid, inner_shut);
    let is_kid = doc.create_element("div");
    doc.set_attribute(is_kid, "class", "kid cb");
    doc.append_child(inner_shut, is_kid);
    doc.resolve_layout(801.0, 600.0);
    // let the inner shut close finish
    if let Some(s0) = vstart(&doc, inner_shut) {
        tick(&mut doc, s0 + 400.0);
    }
    doc.resolve_layout(802.0, 600.0);
    assert_eq!(vis(&doc, inner_shut), VisibilityValue::Hidden);
    assert_eq!(vis(&doc, is_kid), VisibilityValue::Hidden);
    let s = close(&mut doc, root, 803.0);
    for t in [10.0, 200.0, 299.0] {
        tick(&mut doc, s + t);
        assert_eq!(vis(&doc, inner_open), VisibilityValue::Visible, "at {t}");
        assert_eq!(vis(&doc, io_kid), VisibilityValue::Visible, "at {t}");
        assert_eq!(
            vis(&doc, inner_shut),
            VisibilityValue::Hidden,
            "closed inner stays hidden at {t}"
        );
        assert_eq!(
            vis(&doc, is_kid),
            VisibilityValue::Hidden,
            "closed inner's kid stays hidden at {t}"
        );
    }
    tick(&mut doc, s + 301.0);
    assert_eq!(vis(&doc, inner_open), VisibilityValue::Hidden);
    tick(&mut doc, s + 460.0);
    assert_eq!(vis(&doc, io_kid), VisibilityValue::Hidden);
    assert_eq!(vis(&doc, is_kid), VisibilityValue::Hidden);
}

/// Ordering hazard: an inheriting node queued in style_dirty_nodes *before* its parent
/// (note_first_child) on the pass that re-opens a fully closed root, while an unrelated
/// transition runs (so the substitution is armed).
#[test]
fn p8_reopen_with_child_queued_before_parent() {
    let (mut doc, root, kid, _) = setup("kid");
    let other = doc.create_element("div");
    doc.set_attribute(other, "class", "kid anim");
    let body = doc.body();
    doc.append_child(body, other);
    let empty = doc.create_element("div");
    doc.set_attribute(empty, "class", "kid");
    doc.append_child(root, empty);
    doc.resolve_layout(801.0, 600.0);
    let s = close(&mut doc, root, 802.0);
    tick(&mut doc, s + 400.0);
    doc.resolve_layout(803.0, 600.0);
    assert_eq!(vis(&doc, empty), VisibilityValue::Hidden);
    doc.set_attribute(other, "class", "kid anim wide");
    doc.resolve_layout(804.0, 600.0);
    assert!(
        !doc.tree.active_transitions.is_empty(),
        "precondition: unrelated transition running"
    );
    let n = doc.create_element("div");
    doc.set_attribute(n, "class", "kid");
    doc.append_child(empty, n); // queues `empty` first
    doc.set_attribute(root, "class", "root delayed");
    doc.resolve_layout(805.0, 600.0);
    assert_eq!(vis(&doc, root), VisibilityValue::Visible);
    assert_eq!(vis(&doc, empty), VisibilityValue::Visible, "empty");
    assert_eq!(vis(&doc, n), VisibilityValue::Visible, "n");
    assert_eq!(vis(&doc, kid), VisibilityValue::Visible, "kid");
}

#[test]
#[ignore]
fn cost_close_pass_500_rows() {
    let (mut doc, root, _kid, _) = setup("kid");
    let other = doc.create_element("div");
    doc.set_attribute(other, "class", "kid anim");
    let body = doc.body();
    doc.append_child(body, other);
    for _ in 0..500 {
        let r = doc.create_element("div");
        doc.set_attribute(r, "class", "kid");
        for _ in 0..3 {
            let sp = doc.create_element("span");
            let t = doc.create_text("x");
            doc.append_child(sp, t);
            doc.append_child(r, sp);
        }
        doc.append_child(root, r);
    }
    doc.resolve_layout(801.0, 600.0);
    let mut best_close = f64::MAX;
    let mut best_open = f64::MAX;
    let mut best_hover = f64::MAX;
    let mut w = 802.0;
    for i in 0..30 {
        // keep an unrelated transition running so the substitution is armed on both passes
        doc.set_attribute(
            other,
            "class",
            if i % 2 == 0 {
                "kid anim wide"
            } else {
                "kid anim"
            },
        );
        doc.set_attribute(root, "class", "root delayed shut");
        let t = std::time::Instant::now();
        doc.resolve_layout(w, 600.0);
        best_close = best_close.min(t.elapsed().as_secs_f64() * 1e3);
        w += 1.0;
        doc.set_attribute(root, "class", "root delayed shut kid");
        doc.set_attribute(root, "style", &format!("color: rgb({i},0,0)"));
        let t = std::time::Instant::now();
        doc.resolve_layout(w, 600.0);
        best_hover = best_hover.min(t.elapsed().as_secs_f64() * 1e3);
        w += 1.0;
        doc.set_attribute(root, "class", "root delayed");
        let t = std::time::Instant::now();
        doc.resolve_layout(w, 600.0);
        best_open = best_open.min(t.elapsed().as_secs_f64() * 1e3);
        w += 1.0;
    }
    eprintln!(
        "COST close {best_close:.3}ms  inherited-restyle-during-close {best_hover:.3}ms  reopen {best_open:.3}ms"
    );
}

/// Same shape on the root itself with a both-ways `transition: visibility 300ms`
/// (the `.fade` spelling in the PR's own tests): reopen mid-close.
#[test]
fn p9_fade_root_reopened_mid_close_stays_visible() {
    let mut doc = RinchDocument::new();
    doc.load_css("div{font-size:16px;line-height:20px} .r{width:100px;height:50px;transition: visibility 300ms linear} .shut{visibility:hidden}");
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", "r");
    doc.append_child(body, root);
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(root, "class", "r shut");
    doc.resolve_layout(801.0, 600.0);
    let s = vstart(&doc, root).unwrap();
    tick(&mut doc, s + 100.0);
    doc.set_attribute(root, "class", "r");
    doc.resolve_layout(802.0, 600.0);
    for t in [200.0, 310.0, 600.0] {
        tick(&mut doc, s + t);
        assert_eq!(
            vis(&doc, root),
            VisibilityValue::Visible,
            "reopened root at {t}"
        );
    }
}
