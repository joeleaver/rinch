//! Margin anchors on the mock document (#1677): what carries an anchor and what
//! loses it. The mock has no text layout, so every block here is an empty
//! paragraph — the one place a mock textblock has geometry (its box, laid out
//! by hand at `20 * i`): an anchor's line is its block's line, and a report's
//! `top` names the block it landed in.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId};
use rinch_editor_core::model::{Fragment, Slice};
use rinch_editor_core::{Node, Pos, Schema, default_plugins};

use crate::{EditorHandle, MarginAnchor, MarginAssoc, MarginLine};

struct Rig {
    mock: Rc<RefCell<MockDomDocument>>,
    container: NodeId,
    handle: EditorHandle,
    reports: Rc<RefCell<Vec<Vec<MarginLine>>>>,
}

fn empty_doc(s: &Schema, blocks: usize) -> Node {
    let p = || s.branch("paragraph", Fragment::empty()).unwrap();
    s.branch(
        "doc",
        Fragment::from_children((0..blocks).map(|_| p()).collect()),
    )
    .unwrap()
}

fn rig(blocks: usize) -> Rig {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let container = doc.borrow_mut().create_element("div");
    let handle = EditorHandle::new(
        NodeHandle::new(container, Rc::downgrade(&doc)),
        Rc::downgrade(&doc),
        Rc::new(Schema::starter_kit()),
        empty_doc(&Schema::starter_kit(), blocks),
        default_plugins(),
    );
    let reports: Rc<RefCell<Vec<Vec<MarginLine>>>> = Rc::default();
    let r = reports.clone();
    handle.on_margin_layout(move |lines| r.borrow_mut().push(lines.to_vec()));
    let rig = Rig {
        mock,
        container,
        handle,
        reports,
    };
    lay_out(&rig);
    rig
}

/// Lay every block out one line of 20px below the last.
fn lay_out(rig: &Rig) {
    let blocks: Vec<NodeId> = {
        let m = rig.mock.borrow();
        m.get_children(rig.container)
            .into_iter()
            .filter(|c| m.get_attribute(*c, "data-pm-type").is_some())
            .collect()
    };
    let mut m = rig.mock.borrow_mut();
    for (i, b) in blocks.iter().enumerate() {
        m.__set_node_layout(*b, 0.0, i as f32 * 20.0, 200.0, 20.0);
    }
}

/// Run the margin pass and answer the tops it reported, if it reported.
fn pass(rig: &Rig) -> Option<Vec<(String, f32)>> {
    lay_out(rig);
    let before = rig.reports.borrow().len();
    rig.handle.margin_pass();
    let reports = rig.reports.borrow();
    (reports.len() > before).then(|| {
        reports
            .last()
            .unwrap()
            .iter()
            .map(|l| (l.key.clone(), l.top))
            .collect()
    })
}

fn insert_paragraph_at(handle: &EditorHandle, at: usize) {
    assert!(handle.update(|state| {
        let mut tr = state.tr();
        let p = state.schema().branch("paragraph", Fragment::empty()).ok()?;
        tr.replace(at, at, Slice::from_fragment(Fragment::from_node(p)))
            .ok()?;
        Some(tr)
    }));
}

/// A paragraph inserted before an anchor carries it down a block; one
/// inserted right at it leaves it on `assoc`'s side.
#[test]
fn a_local_edit_carries_a_position_anchor_and_its_assoc_picks_the_side() {
    // Blocks at 0, 2, 4: their insides at 1, 3, 5.
    let rig = rig(3);
    rig.handle.set_margin_anchors(vec![
        MarginAnchor::at("in1", Pos(3)),
        MarginAnchor::at_with("before2", Pos(4), MarginAssoc::Before),
        MarginAnchor::at_with("after2", Pos(4), MarginAssoc::After),
    ]);
    assert_eq!(
        pass(&rig),
        Some(vec![
            ("in1".into(), 20.0),
            ("before2".into(), 40.0),
            ("after2".into(), 40.0),
        ])
    );
    // A paragraph at 0 moves every anchor down a block; one at 4 (between the
    // second block and the third) leaves `before2` where it was (the nearest
    // text after it is now the new block) and takes `after2` past it.
    insert_paragraph_at(&rig.handle, 0);
    insert_paragraph_at(&rig.handle, 6);
    assert_eq!(
        pass(&rig),
        Some(vec![
            ("in1".into(), 40.0),
            ("before2".into(), 60.0),
            ("after2".into(), 80.0),
        ])
    );
    // Nothing moved: no report.
    assert_eq!(pass(&rig), None);
}

/// A load replaces the document: a position anchor names nothing in it, and
/// is not reported again.
#[test]
fn a_load_loses_position_anchors() {
    let rig = rig(2);
    rig.handle
        .set_margin_anchors(vec![MarginAnchor::at("a", Pos(3))]);
    assert_eq!(pass(&rig), Some(vec![("a".into(), 20.0)]));
    rig.handle.load_doc(empty_doc(&Schema::starter_kit(), 2));
    assert_eq!(pass(&rig), Some(vec![]));
}

/// Anchors set before the callback, and a callback set again, report afresh;
/// an empty set reports once and then the pass is idle.
#[test]
fn a_new_callback_reports_afresh_and_an_empty_set_reports_once() {
    let rig = rig(2);
    rig.handle
        .set_margin_anchors(vec![MarginAnchor::at("a", Pos(1))]);
    assert_eq!(pass(&rig), Some(vec![("a".into(), 0.0)]));
    let r = rig.reports.clone();
    rig.handle
        .on_margin_layout(move |lines| r.borrow_mut().push(lines.to_vec()));
    assert_eq!(pass(&rig), Some(vec![("a".into(), 0.0)]), "afresh");
    rig.handle.set_margin_anchors(Vec::new());
    assert_eq!(pass(&rig), Some(vec![]));
    let counted = crate::__margin_counters();
    assert_eq!(pass(&rig), None);
    assert_eq!(
        crate::__margin_counters(),
        counted,
        "idle: nothing measured"
    );
}

/// Right before a leaf block (a horizontal rule), an anchor reports that
/// block's box.
#[test]
fn an_anchor_before_a_leaf_block_reports_its_box() {
    let s = Schema::starter_kit();
    let rig = rig(1);
    let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let p = s.branch("paragraph", Fragment::empty()).unwrap();
    rig.handle.load_doc(
        s.branch("doc", Fragment::from_children(vec![p, hr]))
            .unwrap(),
    );
    rig.handle
        .set_margin_anchors(vec![MarginAnchor::at("rule", Pos(2))]);
    // The rule is the second block: laid out at 20.
    assert_eq!(pass(&rig), Some(vec![("rule".into(), 20.0)]));
}

/// The container is watched while anchors are set, and let go once they are
/// cleared — also when the last report was empty (every anchor had lost its
/// place), so nothing is left observed (review of #1683, M10).
#[test]
fn the_container_is_let_go_when_the_anchors_are_cleared() {
    thread_local! {
        static WATCHED: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
    }
    fn listen(_: u64, _: u64, _: &NodeHandle, mounted: bool) {
        WATCHED.with(|w| w.borrow_mut().push(mounted));
    }
    crate::registry::set_node_view_host_listener(listen);
    let rig = rig(2);
    rig.handle
        .set_margin_anchors(vec![MarginAnchor::at("a", Pos(3))]);
    assert_eq!(pass(&rig), Some(vec![("a".into(), 20.0)]));
    // A load loses the anchor: an empty report, the anchor still set.
    rig.handle.load_doc(empty_doc(&Schema::starter_kit(), 2));
    assert_eq!(pass(&rig), Some(vec![]));
    rig.handle.set_margin_anchors(Vec::new());
    pass(&rig);
    assert_eq!(WATCHED.with(|w| w.borrow().clone()), vec![true, false]);
}

/// Anchors with no callback measure nothing and owe no pass when the
/// document changes (review of #1683, M11).
#[test]
fn anchors_without_a_callback_owe_nothing() {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let container = doc.borrow_mut().create_element("div");
    let handle = EditorHandle::new(
        NodeHandle::new(container, Rc::downgrade(&doc)),
        Rc::downgrade(&doc),
        Rc::new(Schema::starter_kit()),
        empty_doc(&Schema::starter_kit(), 2),
        default_plugins(),
    );
    let dk = doc.borrow().doc_key();
    handle.set_margin_anchors(vec![MarginAnchor::at("a", Pos(3))]);
    crate::registry::update_all_carets(Some(dk), None);
    assert!(!crate::registry::overlay_pass_owed(dk), "precondition");
    let selection = handle.selection();
    insert_paragraph_at(&handle, 4);
    assert_eq!(
        handle.selection(),
        selection,
        "precondition: no selection change"
    );
    assert!(
        !crate::registry::overlay_pass_owed(dk),
        "no listener, no pass"
    );
}

#[cfg(feature = "collaboration")]
mod collab {
    use super::*;
    use rinch_editor_collab::StickyAssoc;

    fn loopback(host: &EditorHandle, guest: &EditorHandle) {
        let guest_in = guest.clone();
        let snapshot = host
            .start_collaboration_host(move |delta| {
                guest_in.collab_receive(&delta);
            })
            .expect("host projects its document");
        let host_in = host.clone();
        guest
            .start_collaboration_guest(&snapshot, move |delta| {
                host_in.collab_receive(&delta);
            })
            .expect("guest joins from the snapshot");
    }

    /// A peer's paragraph above a position anchor carries it down a block, as
    /// a local one does; a sticky anchor (made by either peer) names the same
    /// place on both.
    #[test]
    fn a_peers_edit_carries_position_anchors_and_sticky_anchors_agree() {
        let host = rig(1);
        let guest = rig(1);
        // Three blocks a peer's change can tell apart (a converged rebuild
        // relates blocks by value: three equal empty paragraphs and a fourth
        // would read as one appended at the end).
        let s = Schema::starter_kit();
        let block = |ty: &str| s.branch(ty, Fragment::empty()).unwrap();
        let blocks = vec![block("code_block"), block("paragraph"), block("code_block")];
        host.handle
            .load_doc(s.branch("doc", Fragment::from_children(blocks)).unwrap());
        loopback(&host.handle, &guest.handle);
        let sticky = host
            .handle
            .collab_sticky_index_with(Pos(5), StickyAssoc::After)
            .expect("in a paragraph");
        guest.handle.set_margin_anchors(vec![
            MarginAnchor::at("pos", Pos(3)),
            MarginAnchor::sticky("sticky", sticky.clone()),
        ]);
        host.handle
            .set_margin_anchors(vec![MarginAnchor::sticky("sticky", sticky)]);
        assert_eq!(
            pass(&guest),
            Some(vec![("pos".into(), 20.0), ("sticky".into(), 40.0)])
        );
        // The host inserts a paragraph at the top.
        insert_paragraph_at(&host.handle, 0);
        assert_eq!(
            pass(&guest),
            Some(vec![("pos".into(), 40.0), ("sticky".into(), 60.0)])
        );
        assert_eq!(pass(&host), Some(vec![("sticky".into(), 60.0)]));
        // Stopped: a sticky anchor has no place to resolve to.
        guest.handle.stop_collaboration();
        assert_eq!(pass(&guest), Some(vec![("pos".into(), 40.0)]));
    }
}
