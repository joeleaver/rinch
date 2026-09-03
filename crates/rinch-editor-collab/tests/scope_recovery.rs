//! Issue #220: what happens after a local edit the staged A22 scope cannot express.
//!
//! `record_local` refuses such an edit with the CRDT untouched — but the *editor model*
//! has already applied it, so the caller's `before` stops describing the CRDT. These
//! tests pin the two things that used to go wrong from there, and the behaviour that
//! replaces them:
//!
//! 1. **The wedge.** Every later projection failed the block-count gate, *including the
//!    undo that was supposed to be the cure*, until the counts happened to realign.
//! 2. **The false recovery.** When they did realign, the diff skipped the blocks it
//!    believed unchanged — so an edit made during the wedge stayed in the model and
//!    never reached the CRDT, and `record_local` returned `Ok` on a document that no
//!    longer matched its own projection. Silent divergence, with nothing reporting it.
//!
//! Neither is poison (issue #196): the shared CRDT is healthy the whole time, inbound
//! keeps working, and the session heals itself.

use std::rc::Rc;

use rinch_editor_collab::{CollabError, CollabSession};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{
    EditorState, Node, Pos, Schema, Selection, Slice, Transaction, default_plugins,
};

// --- harness -------------------------------------------------------------------

fn para(s: &Schema, text: &str) -> Node {
    let content = if text.is_empty() {
        Fragment::empty()
    } else {
        Fragment::from_node(s.text(text).unwrap())
    };
    s.branch("paragraph", content).unwrap()
}

fn doc_of(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// A `blockquote` — deliberately outside the A22 scope, and the shape a real paste or
/// "wrap in quote" command produces.
fn blockquote(s: &Schema, text: &str) -> Node {
    s.branch("blockquote", Fragment::from_node(para(s, text)))
        .unwrap()
}

/// One editor plus its session, driven the way `EditorHandle::commit` drives them: the
/// model applies the transaction whatever the projection then says.
struct Peer {
    schema: Rc<Schema>,
    state: EditorState,
    session: CollabSession,
}

impl Peer {
    fn new(schema: &Rc<Schema>, blocks: Vec<Node>) -> Peer {
        let state = EditorState::create(schema.clone(), doc_of(schema, blocks), default_plugins());
        let session = CollabSession::new(&state).expect("initial projection");
        Peer {
            schema: schema.clone(),
            state,
            session,
        }
    }

    /// Apply `f` and project. Returns the projection error, if any.
    fn edit(&mut self, f: impl FnOnce(&mut Transaction)) -> Option<CollabError> {
        let mut tr = self.state.tr();
        f(&mut tr);
        let before = self.state.doc.clone();
        let next = self.state.apply(tr);
        let out = self
            .session
            .record_local(&self.schema, &before, &next.doc)
            .err();
        self.state = next;
        out
    }

    fn type_at(&mut self, pos: usize, text: &str) -> Option<CollabError> {
        self.edit(|tr| {
            tr.set_selection(Selection::cursor(Pos(pos)));
            tr.insert_text(text).unwrap();
        })
    }

    /// Append an out-of-scope `blockquote` — the "paste a table" gesture.
    fn paste_blockquote(&mut self) -> Option<CollabError> {
        let bq = blockquote(&self.schema, "q");
        let end = self.state.doc.content().size();
        self.edit(|tr| {
            tr.replace(end, end, Slice::from_fragment(Fragment::from_node(bq)))
                .unwrap();
        })
    }

    /// Delete the last block — the undo of the paste above.
    fn drop_last_block(&mut self) -> Option<CollabError> {
        let n = self.state.doc.child_count();
        let start: usize = (0..n - 1)
            .map(|i| self.state.doc.child(i).node_size())
            .sum();
        let end = start + self.state.doc.child(n - 1).node_size();
        self.edit(|tr| {
            tr.delete(start, end).unwrap();
        })
    }

    /// The load-bearing invariant, asserted directly.
    fn assert_model_is_projection(&self, what: &str) {
        let projected = self
            .session
            .projected_doc(&self.schema)
            .expect("the CRDT stays projectable throughout");
        assert_eq!(
            self.state.doc, projected,
            "{what}: model ≡ project(model) must hold"
        );
    }
}

// --- tests ---------------------------------------------------------------------

#[test]
fn an_out_of_scope_edit_is_refused_loud_and_names_the_content() {
    let schema = Rc::new(Schema::starter_kit());
    let mut p = Peer::new(&schema, vec![para(&schema, "hello")]);

    let err = p
        .paste_blockquote()
        .expect("a blockquote is out of A22 scope");
    assert!(
        matches!(&err, CollabError::Unsupported(m) if m.contains("blockquote")),
        "the refusal must name the offending content, got {err:?}"
    );
    // Outbound is stalled, and says why — the app can render this verbatim.
    let stall = p.session.outbound_stall().expect("outbound is stalled");
    assert!(stall.to_string().contains("blockquote"));
    // Not poison: this is local-outbound-only and the shared document is untouched.
    assert!(
        !p.session.is_poisoned(),
        "an out-of-scope local edit is not poison"
    );
}

#[test]
fn a_later_edit_while_out_of_scope_still_names_the_real_cause() {
    // The wedge symptom (#220): before the re-base, this reported
    // `Schema("model/CRDT out of step: the model document holds 2 block(s) but the CRDT
    // holds 1")` — a block count, naming neither the cause nor the cure. Every keystroke
    // after the paste said that, so an app had nothing to show the user.
    let schema = Rc::new(Schema::starter_kit());
    let mut p = Peer::new(&schema, vec![para(&schema, "hello")]);
    p.paste_blockquote().expect("refused");

    for _ in 0..3 {
        let err = p.type_at(3, "x").expect("still out of scope");
        assert!(
            matches!(&err, CollabError::Unsupported(m) if m.contains("blockquote")),
            "every later refusal must still name the blockquote, got {err:?}"
        );
    }
    assert!(!p.session.is_poisoned());
}

#[test]
fn removing_the_out_of_scope_content_resumes_outbound_immediately() {
    // The cure used to be refused itself: `drop_last_block` failed the block-count gate
    // exactly like the keystrokes did, so "undo to resume" did not even work on the
    // first try — outbound resumed only when a *later* edit happened to find the counts
    // realigned.
    let schema = Rc::new(Schema::starter_kit());
    let mut p = Peer::new(&schema, vec![para(&schema, "hello")]);
    p.paste_blockquote().expect("refused");
    p.type_at(3, "x").expect("refused");

    assert!(
        p.drop_last_block().is_none(),
        "removing the offending block must project, not be refused as well"
    );
    assert!(
        p.session.outbound_stall().is_none(),
        "and outbound must be reported healthy again"
    );
    p.assert_model_is_projection("after removing the out-of-scope block");
    assert!(
        !p.session.save_incremental().unwrap().is_empty(),
        "the catch-up must actually be broadcast"
    );
}

#[test]
fn an_edit_made_during_the_stall_is_not_silently_lost() {
    // The correctness hole, not just the UX gap. Two blocks; the edit made while stalled
    // lands in block 1, and the edit that clears the stall touches only block 0. The
    // block-list diff skips block 1 (unchanged by *this* transaction), so before the
    // re-base its "Q" stayed in the model forever while `record_local` answered `Ok`:
    //
    //     model     = <paragraph>Zone<paragraph>twoQ
    //     projected = <paragraph>Zone<paragraph>two
    //
    // Diverged, converged-looking, and silent — the exact class this crate exists to
    // kill. Re-basing on the CRDT's own read-back reconciles every block, not just the
    // ones this transaction touched.
    let schema = Rc::new(Schema::starter_kit());
    let mut p = Peer::new(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
    p.paste_blockquote().expect("refused");

    // Block 1's content is 6..10; type at its end. Refused — nothing broadcast.
    p.type_at(9, "Q").expect("refused while out of scope");

    // Removing the blockquote clears the stall. The transaction itself touches only the
    // last block, so a `before`-based diff would reconcile nothing else; the re-base is
    // what carries block 1's "Q" across.
    assert!(p.drop_last_block().is_none(), "the cure projects");
    p.assert_model_is_projection("immediately after the stall cleared");

    // And a subsequent edit that touches block 0 ONLY leaves block 1 correct.
    assert!(p.type_at(1, "Z").is_none(), "block 0 edit projects");
    p.assert_model_is_projection("after a later single-block edit");
    let projected = p.session.projected_doc(&schema).unwrap();
    assert_eq!(
        projected.child(1).child(0).text(),
        Some("twoQ"),
        "the edit made during the stall must have reached the CRDT too"
    );
}

/// The same hole, in the shape the other tests miss: an out-of-scope edit that
/// **preserves the block count**. `wrapInBlockquote` on a paragraph is one — and so is
/// any replace-in-place — so this is a command away, not a corner.
///
/// The count-changing shape is caught because the block-count gate fails and forces the
/// re-base. This one is not: counts still match, and the block-list diff puts the
/// offending block in its `Rc`-identity suffix, where it is never compared against the
/// CRDT. Trying the fast diff first therefore let it *succeed* on a document that had
/// already diverged:
///
/// ```text
/// model     = <paragraph>Zone<blockquote>two
/// projected = <paragraph>Zone<paragraph>two
/// model == projected ? false
/// stall     = None            <-- reporting healthy
/// ```
///
/// Silent divergence with the new indicator actively saying "syncing", which is worse
/// than the block-count error it replaced. A session that is already stalled therefore
/// skips the fast path entirely: `before` is *known* not to describe the CRDT.
#[test]
fn a_count_preserving_out_of_scope_edit_stalls_too_and_still_ships_the_backlog() {
    let schema = Rc::new(Schema::starter_kit());
    let mut p = Peer::new(&schema, vec![para(&schema, "one"), para(&schema, "two")]);

    // Replace block 1 with a blockquote — two blocks before, two after.
    let start: usize = p.state.doc.child(0).node_size();
    let end = start + p.state.doc.child(1).node_size();
    let bq = blockquote(&schema, "two");
    let err = p
        .edit(|tr| {
            tr.replace(start, end, Slice::from_fragment(Fragment::from_node(bq)))
                .unwrap();
        })
        .expect("a blockquote is out of A22 scope however it got there");
    assert!(matches!(&err, CollabError::Unsupported(m) if m.contains("blockquote")));
    assert_eq!(
        p.state.doc.child_count(),
        2,
        "the block count did not change"
    );

    // An edit that touches only block 0 leaves the blockquote in the diff's suffix.
    let err = p
        .type_at(1, "Z")
        .expect("still out of scope — the model holds content the CRDT cannot");
    assert!(
        matches!(&err, CollabError::Unsupported(m) if m.contains("blockquote")),
        "and it must still name the blockquote, not answer Ok, got {err:?}"
    );
    assert!(
        p.session.outbound_stall().is_some(),
        "outbound is still stalled; reporting healthy here is the silent-divergence bug"
    );

    // Unwrap the quote — also count-preserving. The "Z" typed while stalled ships too.
    let start: usize = p.state.doc.child(0).node_size();
    let end = start + p.state.doc.child(1).node_size();
    let plain = para(&schema, "two");
    assert!(
        p.edit(|tr| {
            tr.replace(start, end, Slice::from_fragment(Fragment::from_node(plain)))
                .unwrap();
        })
        .is_none(),
        "removing the offending block must project"
    );
    assert!(p.session.outbound_stall().is_none(), "and clear the stall");
    p.assert_model_is_projection("after a count-preserving cure");
    assert_eq!(
        p.session
            .projected_doc(&schema)
            .unwrap()
            .child(0)
            .child(0)
            .text(),
        Some("Zone"),
        "the edit made during the stall reached the CRDT"
    );
}

#[test]
fn a_peer_receives_everything_that_accumulated_during_the_stall() {
    // End-to-end: the catch-up must reach the wire, not just the local CRDT.
    let schema = Rc::new(Schema::starter_kit());
    let mut a = Peer::new(&schema, vec![para(&schema, "one"), para(&schema, "two")]);
    let mut b_session = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    let b_doc = b_session.projected_doc(&schema).unwrap();
    let mut b_state = EditorState::create(schema.clone(), b_doc, default_plugins());
    let _ = a.session.save_incremental().unwrap(); // B joined from the snapshot

    a.paste_blockquote().expect("refused");
    a.type_at(9, "Q").expect("refused");
    assert!(a.drop_last_block().is_none(), "outbound resumes");
    assert!(a.type_at(1, "Z").is_none(), "and stays healthy");

    let delta = a.session.save_incremental().unwrap();
    assert!(!delta.is_empty(), "there is something to send");
    if let Some(next) = b_session.integrate_incremental(&b_state, &delta).unwrap() {
        b_state = next;
    }
    assert_eq!(
        b_state.doc, a.state.doc,
        "B must converge on everything A accumulated during the stall"
    );
}

#[test]
fn inbound_keeps_working_while_outbound_is_stalled() {
    // The stall is local-outbound-only. A peer's edits must keep arriving — which is
    // also what distinguishes this from #196's poison, where both directions refuse.
    let schema = Rc::new(Schema::starter_kit());
    let mut a = Peer::new(&schema, vec![para(&schema, "one")]);
    let mut b_session = CollabSession::from_bytes(&a.session.snapshot()).unwrap();
    let b_doc = b_session.projected_doc(&schema).unwrap();
    let b_state = EditorState::create(schema.clone(), b_doc, default_plugins());
    let _ = a.session.save_incremental().unwrap();

    a.paste_blockquote().expect("refused");
    assert!(a.session.outbound_stall().is_some());

    // B types; A integrates it fine.
    let mut tr = b_state.tr();
    tr.set_selection(Selection::cursor(Pos(4)));
    tr.insert_text("!").unwrap();
    let b_next = b_state.apply(tr);
    b_session
        .record_local(&schema, &b_state.doc, &b_next.doc)
        .unwrap();
    let delta = b_session.save_incremental().unwrap();

    let integrated = a
        .session
        .integrate_incremental(&a.state, &delta)
        .expect("inbound is unaffected by an outbound stall");
    assert!(
        integrated.is_some(),
        "B's edit must arrive even though A cannot send"
    );
}
