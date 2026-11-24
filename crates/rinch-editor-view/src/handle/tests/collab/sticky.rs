//! Sticky indices across peers.

use super::*;

/// No session, no sticky index: the methods answer `None` rather than an
/// address no one could resolve.
#[test]
fn sticky_positions_need_a_session() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "hello")])).handle;
    assert_eq!(h.collab_sticky_index(Pos(2)), None);
    h.start_collaboration_host(|_| {}).unwrap();
    let bytes = h.collab_sticky_index(Pos(2)).expect("collaborating");
    h.stop_collaboration();
    assert_eq!(h.collab_resolve_sticky(&bytes), None);
    assert_eq!(
        h.collab_locate_sticky_many(&[&bytes, &bytes]),
        vec![None, None]
    );
}

/// A sticky index made on one editor follows its character through edits on
/// both, and resolves on the peer to the same character.
#[test]
fn a_sticky_index_follows_its_character_across_peers() {
    let s = schema();
    let host = mount(doc_node(&s, vec![para(&s, "hello world")])).handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    let w = host
        .collab_sticky_index(Pos(7))
        .expect("inside the paragraph");
    assert_eq!(guest.collab_resolve_sticky(&w), Some(Pos(7)));

    // The guest types before it, the host types before it too.
    guest.set_selection(Selection::cursor(Pos(1)));
    assert!(guest.insert_text("oh, "));
    host.set_selection(Selection::cursor(Pos(1)));
    assert!(host.insert_text("so "));
    assert_eq!(doc_text(&host), "so oh, hello world");
    assert_eq!(doc_text(&guest), doc_text(&host));

    for h in [&host, &guest] {
        assert_eq!(h.collab_resolve_sticky(&w), Some(Pos(14)));
    }
    // Between blocks is not a place in any text.
    assert_eq!(host.collab_sticky_index(Pos(0)), None);
}

/// #1676: an index from `collab_sticky_index_with` outlives its block on both
/// peers, and the association decides which side of an insert it keeps.
#[test]
fn a_sticky_index_with_outlives_its_block_across_peers() {
    let s = schema();
    let host = mount(doc_node(
        &s,
        vec![para(&s, "keep"), para(&s, "gone"), para(&s, "x")],
    ))
    .handle;
    let guest = mount(doc_node(&s, vec![para(&s, "")])).handle;
    loopback(&host, &guest);

    let in_gone = guest
        .collab_sticky_index_with(Pos(9), StickyAssoc::After)
        .expect("inside a paragraph");
    let before = host
        .collab_sticky_index_with(Pos(3), StickyAssoc::Before)
        .expect("inside a paragraph");
    let after = host
        .collab_sticky_index_with(Pos(3), StickyAssoc::After)
        .expect("inside a paragraph");
    host.set_selection(Selection::cursor(Pos(3)));
    assert!(host.insert_text("--"));
    assert_eq!(
        guest.collab_locate_sticky(&before).map(|l| l.pos),
        Some(Pos(3))
    );
    assert_eq!(
        guest.collab_locate_sticky(&after).map(|l| l.pos),
        Some(Pos(5))
    );

    // The host deletes the block the guest's index is in.
    assert!(host.update(|state| {
        let mut tr = state.tr();
        tr.replace(8, 14, Slice::empty()).ok()?;
        Some(tr)
    }));
    assert_eq!(doc_text(&guest), doc_text(&host));
    for h in [&host, &guest] {
        let l = h
            .collab_locate_sticky(&in_gone)
            .expect("displaced, not lost");
        assert_eq!((l.pos, l.path, l.displaced), (Pos(7), vec![0], true));
        assert_eq!(h.collab_resolve_sticky(&in_gone), None);
    }
    // One read for many: the same answers, in order.
    let many = host.collab_locate_sticky_many(&[&in_gone, &before, &after]);
    let one: Vec<_> = [&in_gone, &before, &after]
        .iter()
        .map(|b| host.collab_locate_sticky(b))
        .collect();
    assert_eq!(many, one);
    assert_eq!(many.len(), 3);
}
