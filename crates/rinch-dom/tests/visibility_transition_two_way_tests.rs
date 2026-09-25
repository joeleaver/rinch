//! Review probes for PR #991 (#759), round 3, kept as fixtures: a two-way
//! `transition: visibility` root reopened while a descendant runs its own hide
//! (q1), a descendant gaining a transition mid-close, a transition from a
//! descendant-combinator rule behind a wrapper, a declared-`visible` kid under
//! the shared-struct fast path, and a transform reversal beside the visibility
//! cancel (q5, real clock).

#![cfg(feature = "software-renderer")]
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::computed_style::VisibilityValue as V;
use rinch_dom::transition::TransitionProperty as TP;

const CSS: &str = "
    div { font-size: 16px; line-height: 20px; }
    .root  { width: 200px; height: 100px; }
    .shut  { visibility: hidden; }
    .delayed.shut { transition: visibility 0s linear 300ms; }
    .fade { transition: visibility 300ms linear; }
    .kid { width: 50px; height: 20px; }
    .cb { transition: all 150ms linear; }
    .root .desc { transition: all 150ms linear; }
    .vis { visibility: visible; }
    .slide { transition: transform 300ms linear, visibility 0s; }
    .slide.shut { transform: translateX(-100px); transition: transform 300ms linear, visibility 0s linear 300ms; }
";
fn vis(doc: &RinchDocument, n: NodeId) -> V {
    doc.tree.get(n.0).unwrap().computed_style.visibility
}
fn tick(doc: &mut RinchDocument, at: f64) {
    rinch_dom::transition::tick_transitions(&mut doc.tree, at);
}
fn tr(doc: &RinchDocument, n: NodeId, p: TP) -> Option<rinch_dom::transition::ActiveTransition> {
    doc.tree
        .active_transitions
        .get(&n.0)
        .and_then(|m| m.get(&p))
        .cloned()
}
fn build(root_class: &str, chain: &[&str]) -> (RinchDocument, NodeId, Vec<NodeId>) {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let body = doc.body();
    let root = doc.create_element("div");
    doc.set_attribute(root, "class", root_class);
    doc.append_child(body, root);
    let mut parent = root;
    let mut v = vec![];
    for c in chain {
        let n = doc.create_element("div");
        doc.set_attribute(n, "class", c);
        doc.append_child(parent, n);
        v.push(n);
        parent = n;
    }
    doc.tree.transitions_enabled = true;
    doc.resolve_layout(800.0, 600.0);
    (doc, root, v)
}

/// q1: two-way fade root; kid hiding after root flip; reopen at 350 (root starts hidden->visible).
#[test]
fn q1_fade_root_reopen_after_flip_kid_mid_hide() {
    let (mut doc, root, v) = build("root fade", &["kid cb", "kid"]);
    let (kid, gk) = (v[0], v[1]);
    doc.set_attribute(root, "class", "root fade shut");
    doc.resolve_layout(801.0, 600.0);
    let s = tr(&doc, root, TP::Visibility).unwrap().start_time_ms;
    tick(&mut doc, s + 301.0);
    assert_eq!(vis(&doc, root), V::Hidden);
    assert!(tr(&doc, kid, TP::Visibility).is_some(), "kid hiding");
    tick(&mut doc, s + 350.0);
    doc.set_attribute(root, "class", "root fade");
    doc.resolve_layout(802.0, 600.0);
    for t in [360.0, 460.0, 700.0, 900.0] {
        tick(&mut doc, s + t);
        eprintln!(
            "q1 t={t} root={:?} kid={:?} gk={:?} kidtr={:?}",
            vis(&doc, root),
            vis(&doc, kid),
            vis(&doc, gk),
            tr(&doc, kid, TP::Visibility).map(|t| (t.start_time_ms, t.to))
        );
    }
    assert_eq!(vis(&doc, root), V::Visible);
    assert_eq!(vis(&doc, kid), V::Visible, "kid in reopened fade root");
    assert_eq!(vis(&doc, gk), V::Visible);
}

/// q2: kid gains `transition: all` mid-close (class toggle) — must not start its hide before the flip.
#[test]
fn q2_kid_gains_transition_mid_close() {
    let (mut doc, root, v) = build("root delayed", &["kid", "kid"]);
    let (kid, gk) = (v[0], v[1]);
    doc.set_attribute(root, "class", "root delayed shut");
    doc.resolve_layout(801.0, 600.0);
    let s = tr(&doc, root, TP::Visibility).unwrap().start_time_ms;
    tick(&mut doc, s + 100.0);
    doc.set_attribute(kid, "class", "kid cb");
    doc.resolve_layout(802.0, 600.0);
    for t in [150.0, 250.0, 299.0] {
        tick(&mut doc, s + t);
        assert_eq!(vis(&doc, kid), V::Visible, "kid at {t}");
        assert_eq!(vis(&doc, gk), V::Visible, "gk at {t}");
    }
    tick(&mut doc, s + 301.0);
    assert_eq!(
        tr(&doc, kid, TP::Visibility).map(|t| t.start_time_ms),
        Some(s + 301.0)
    );
    tick(&mut doc, s + 400.0);
    assert_eq!(vis(&doc, kid), V::Visible);
    tick(&mut doc, s + 460.0);
    assert_eq!(vis(&doc, kid), V::Hidden);
    assert_eq!(vis(&doc, gk), V::Hidden);
}

/// q3: transition from a descendant-combinator rule, behind an inheriting wrapper.
#[test]
fn q3_desc_rule_behind_wrapper() {
    let (mut doc, root, v) = build("root delayed", &["kid", "kid desc", "kid"]);
    let (w, kid, gk) = (v[0], v[1], v[2]);
    doc.set_attribute(root, "class", "root delayed shut");
    doc.resolve_layout(801.0, 600.0);
    let s = tr(&doc, root, TP::Visibility).unwrap().start_time_ms;
    for t in [50.0, 299.0] {
        tick(&mut doc, s + t);
        doc.resolve_layout(803.0 + t as f32, 600.0);
        assert_eq!(vis(&doc, w), V::Visible, "w at {t}");
        assert_eq!(vis(&doc, kid), V::Visible, "kid at {t}");
        assert_eq!(vis(&doc, gk), V::Visible, "gk at {t}");
        assert!(
            tr(&doc, kid, TP::Visibility).is_none(),
            "no early kid hide at {t}"
        );
    }
    tick(&mut doc, s + 301.0);
    tick(&mut doc, s + 460.0);
    assert_eq!(vis(&doc, kid), V::Hidden);
    assert_eq!(vis(&doc, gk), V::Hidden);
}

/// q4: kid DECLARES visibility: visible (ptr-eq fast path must not call it inherited).
#[test]
fn q4_declared_visible_kid_stays_visible() {
    let (mut doc, root, v) = build("root delayed", &["kid cb vis", "kid"]);
    let (kid, gk) = (v[0], v[1]);
    doc.set_attribute(root, "class", "root delayed shut");
    doc.resolve_layout(801.0, 600.0);
    let s = tr(&doc, root, TP::Visibility).unwrap().start_time_ms;
    for t in [100.0, 301.0, 460.0, 800.0] {
        tick(&mut doc, s + t);
        doc.resolve_layout(803.0 + t as f32, 600.0);
        assert_eq!(vis(&doc, kid), V::Visible, "declared-visible kid at {t}");
        assert_eq!(vis(&doc, gk), V::Visible, "gk at {t}");
    }
    assert_eq!(vis(&doc, root), V::Hidden);
}

/// q5 (real clock): a node with BOTH a transform and a visibility transition reopened
/// mid-close: visibility cancelled, transform reverses (shortened) from its current position.
#[test]
fn q5_transform_reversal_untouched_by_visibility_cancel() {
    let (mut doc, root, v) = build("root slide", &["kid"]);
    let kid = v[0];
    doc.set_attribute(root, "class", "root slide shut");
    doc.resolve_layout(801.0, 600.0);
    let t0 = tr(&doc, root, TP::Transform).expect("slide out");
    assert!(tr(&doc, root, TP::Visibility).is_some());
    std::thread::sleep(std::time::Duration::from_millis(150));
    tick(&mut doc, now());
    doc.set_attribute(root, "class", "root slide");
    doc.resolve_layout(802.0, 600.0);
    let back = tr(&doc, root, TP::Transform).expect("reversal running");
    let el = back.start_time_ms - t0.start_time_ms;
    eprintln!(
        "q5 elapsed={el} back from={:?} dur={}",
        back.from, back.duration_ms
    );
    assert!(
        (back.duration_ms - el).abs() < 2.0,
        "shortened reversal: dur {} vs elapsed {el}",
        back.duration_ms
    );
    if let rinch_dom::transition::AnimatableValue::Transform(t) = &back.from {
        let dbg = format!("{:?}", t.functions);
        let px: f64 = dbg
            .split("px: [")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let expect = -100.0 * el / 300.0;
        assert!((px - expect).abs() < 3.0, "from {px} vs {expect}");
    } else {
        panic!()
    }
    assert!(
        tr(&doc, root, TP::Visibility).is_none(),
        "pending hide cancelled"
    );
    for _ in 0..4 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        tick(&mut doc, now());
        doc.resolve_layout(803.0, 600.0);
        assert_eq!(vis(&doc, root), V::Visible);
        assert_eq!(vis(&doc, kid), V::Visible);
    }
}
fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0
}
