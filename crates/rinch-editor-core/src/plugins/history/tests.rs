use super::branch::{Item, Items, fold, peer_item, remapping};
use super::*;
use crate::Schema;
use crate::commands::BaseCommandsPlugin;
use crate::model::Fragment;
use crate::model::Slice;
use crate::pos::Pos;
use crate::transform::step::Step;
use crate::transform::step_map::{Mapping, StepMap};

fn editor(text: &str) -> EditorState {
    let s = Rc::new(Schema::starter_kit());
    let para = s
        .branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(para)).unwrap();
    EditorState::create(
        s,
        doc,
        vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
    )
}

/// A branch of one event: `step`, recorded with `selection`.
fn one_event(step: ReplaceStep, selection: Selection, unmapped_after: bool) -> Branch {
    Branch {
        items: Items::default().push(Item {
            map: step.get_map(),
            step: Some(Box::new(step)),
            event: Some(EventInfo {
                selection,
                typing: false,
            }),
            mirror_offset: None,
            peer: false,
            unit_run: None,
            unmapped_after,
        }),
        event_count: 1,
        hidden: 0,
        last: None,
    }
}

/// #1942, each arm alone: an undo that maps nothing lands its selection in
/// its own document when a step of it failed to apply, and when a change
/// that moved no position came after the event.
#[test]
fn an_unmapped_undo_resolves_its_selection_in_the_document() {
    let state = editor("abc");
    let far = Selection::cursor(Pos(20));
    // A step that does not apply, and nothing recorded after it.
    let skipped = one_event(ReplaceStep::new(10, 11, Slice::empty()), far.clone(), false);
    let (tr, sel, ..) = skipped.pop_event(&state).unwrap();
    assert!(sel.to().0 <= tr.doc().content_size(), "{sel:?}");
    // A step that applies, after a change that moved no position.
    let unmapped = one_event(ReplaceStep::new(1, 2, Slice::empty()), far, true);
    let (tr, sel, ..) = unmapped.pop_event(&state).unwrap();
    assert_eq!(all_text(tr.doc()), "bc");
    assert!(sel.to().0 <= tr.doc().content_size(), "{sel:?}");
}

/// #1940: a repair step that moved nothing marks the branch as a peer's
/// attribute change does, so its next undo repairs too.
#[test]
fn a_repair_that_moves_nothing_is_noted_as_unmapped() {
    let mut branch = one_event(
        ReplaceStep::new(1, 2, Slice::empty()),
        Selection::cursor(Pos(1)),
        false,
    );
    branch.add_repair_maps(&[StepMap::empty()]);
    assert!(branch.items.head().unwrap().unmapped_after);
    let mut moved = one_event(
        ReplaceStep::new(1, 2, Slice::empty()),
        Selection::cursor(Pos(1)),
        false,
    );
    moved.add_repair_maps(&[StepMap::new(vec![3, 0, 4])]);
    let head = moved.items.head().unwrap();
    assert!(head.step.is_none() && !head.unmapped_after);
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

fn type_char(state: &EditorState, text: &str, time: u64) -> EditorState {
    let mut tr = state.tr();
    tr.set_time(time);
    tr.insert_text(text).unwrap();
    state.apply(tr)
}

#[test]
fn undo_and_redo_a_single_edit() {
    let state = editor("ab");
    let typed = type_char(&state, "X", 0); // cursor at start (pos 1) → "Xab"
    assert_eq!(all_text(&typed.doc), "Xab");
    let undone = typed.run("undo").expect("undo applies");
    assert_eq!(all_text(&undone.doc), "ab");
    let redone = undone.run("redo").expect("redo applies");
    assert_eq!(all_text(&redone.doc), "Xab");
}

#[test]
fn typing_coalesces_into_one_undo_group() {
    // three chars typed within the window merge into a single undo
    let state = editor("");
    let s1 = type_char(&state, "a", 0);
    let s2 = type_char(&s1, "b", 10);
    let s3 = type_char(&s2, "c", 20);
    assert_eq!(all_text(&s3.doc), "abc");
    let undone = s3.run("undo").expect("undo applies");
    assert_eq!(
        all_text(&undone.doc),
        "",
        "one undo reverts all coalesced typing"
    );
    // and a single redo restores all of it
    let redone = undone.run("redo").unwrap();
    assert_eq!(all_text(&redone.doc), "abc");
}

#[test]
fn time_gap_breaks_typing_group() {
    let state = editor("");
    let s1 = type_char(&state, "a", 0);
    let s2 = type_char(&s1, "b", 1000); // beyond the 500ms window
    assert_eq!(all_text(&s2.doc), "ab");
    let u1 = s2.run("undo").unwrap();
    assert_eq!(all_text(&u1.doc), "a", "second char is its own group");
    let u2 = u1.run("undo").unwrap();
    assert_eq!(all_text(&u2.doc), "");
}

#[test]
fn type_then_bold_undo_reverts_bold_only() {
    // design M4 test: typing then a mark are separate groups
    let state = editor("ab");
    let typed = type_char(&state, "X", 0); // "Xab", cursor at 2
    let mut bolded_state = typed.clone();
    bolded_state.selection = Selection::text(Pos(1), Pos(4)); // select "Xab"
    let bolded = bolded_state.run("toggleBold").expect("bold applies");
    assert!(
        bolded
            .doc
            .child(0)
            .content()
            .iter()
            .all(|n| n.marks().iter().any(|m| m.type_name() == "bold"))
    );
    // first undo removes the bold (its own group), text remains
    let u1 = bolded.run("undo").expect("undo bold");
    assert_eq!(all_text(&u1.doc), "Xab");
    assert!(!range_has_bold(&u1));
    // second undo removes the typed X
    let u2 = u1.run("undo").expect("undo typing");
    assert_eq!(all_text(&u2.doc), "ab");
}

fn range_has_bold(state: &EditorState) -> bool {
    let bold = state.schema().mark_type("bold").unwrap();
    crate::commands::range_has_mark(&state.doc, 0, state.doc.content_size(), bold)
}

#[test]
fn new_edit_clears_redo() {
    let state = editor("");
    let s1 = type_char(&state, "a", 0);
    let undone = s1.run("undo").unwrap();
    assert!(undone.run("redo").is_some());
    // typing again should drop the redo stack
    let s2 = type_char(&undone, "b", 0);
    assert!(s2.run("redo").is_none(), "a fresh edit invalidates redo");
    assert_eq!(all_text(&s2.doc), "b");
}

#[test]
fn undo_restores_selection() {
    let state = editor("ab");
    let mut s0 = state.clone();
    s0.selection = Selection::cursor(Pos(3)); // end of "ab"
    let typed = {
        let mut tr = s0.tr();
        tr.insert_text("Z").unwrap();
        s0.apply(tr)
    };
    assert_eq!(typed.selection, Selection::cursor(Pos(4)));
    let undone = typed.run("undo").unwrap();
    // selection restored to where it was before the edit (pos 3)
    assert_eq!(undone.selection, Selection::cursor(Pos(3)));
}

#[test]
fn undo_unavailable_on_empty_history() {
    let state = editor("ab");
    assert!(!state.can_run("undo"));
    assert!(!state.can_run("redo"));
}

#[test]
fn redone_split_does_not_merge_with_later_typing() {
    // verification finding #1: a redone structural edit (split) must keep its
    // non-typing classification so a following keystroke is its own undo group.
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(2)); // a|b
    let split = state.run("splitBlock").expect("split");
    assert_eq!(split.doc.child_count(), 2);
    let undone = split.run("undo").expect("undo split");
    assert_eq!(undone.doc.child_count(), 1);
    let redone = undone.run("redo").expect("redo split");
    assert_eq!(redone.doc.child_count(), 2);
    // type after the redone split, within the merge window
    let typed = type_char(&redone, "Z", 0);
    let after = typed.run("undo").expect("undo typing");
    assert_eq!(
        after.doc.child_count(),
        2,
        "undo reverts only the typing, not the redone split"
    );
}

#[test]
fn stored_mark_toggle_breaks_typing_group() {
    // verification finding #2: a cursor mark toggle between keystrokes ends the
    // typing group (PM storedMarksSet group break).
    let mut state = editor("");
    state.selection = Selection::cursor(Pos(1));
    let s1 = type_char(&state, "a", 0);
    let s2 = s1.run("toggleBold").expect("cursor bold (stored marks)");
    let s3 = type_char(&s2, "b", 10); // within the window
    assert_eq!(all_text(&s3.doc), "ab");
    let undone = s3.run("undo").expect("undo");
    assert_eq!(
        all_text(&undone.doc),
        "a",
        "only 'b' reverted, not the whole word"
    );
}

#[test]
fn block_join_starts_new_undo_group() {
    // verification finding #4: a structural block join is its own undo group and
    // does not coalesce into preceding typing.
    let s = Rc::new(Schema::starter_kit());
    let p = |t: &str| {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    };
    let doc = s
        .branch("doc", Fragment::from_children(vec![p("ab"), p("cd")]))
        .unwrap();
    let mut state = EditorState::create(
        s,
        doc,
        vec![Rc::new(BaseCommandsPlugin), Rc::new(HistoryPlugin::new())],
    );
    state.selection = Selection::cursor(Pos(3)); // end of first paragraph
    let typed = type_char(&state, "Z", 0); // "abZ" / "cd"
    assert_eq!(typed.doc.child_count(), 2);
    let joined = typed.run("deleteCharForward").expect("forward join");
    assert_eq!(joined.doc.child_count(), 1);
    assert_eq!(all_text(&joined.doc), "abZcd");
    // undo reverts ONLY the join, keeping the typed Z
    let after = joined.run("undo").expect("undo join");
    assert_eq!(after.doc.child_count(), 2, "join is its own undo group");
    assert_eq!(
        all_text(&after.doc),
        "abZcd",
        "the typed Z survives the undo"
    );
}

/// A run of a peer's Backspace and Delete keystrokes folds into one item that
/// maps every position, with every flag a step reads, as the keystrokes' own
/// maps in turn (review of #1889).
#[test]
fn a_unit_deletion_run_maps_as_its_keystrokes() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    for _ in 0..2000 {
        let at0 = 2 + rnd(10);
        let mut maps = vec![StepMap::new(vec![at0, 1, 0])];
        let mut item = peer_item(maps[0].clone());
        let mut at = at0;
        for _ in 0..rnd(8) {
            // Backspace (the char before the hole) or Delete (the one after).
            let d = if at > 0 && rnd(2) == 0 { at - 1 } else { at };
            let m = StepMap::new(vec![d, 1, 0]);
            item = fold(&item, &m).expect("a keystroke beside the run folds");
            maps.push(m);
            at = d;
        }
        let (folded, _) = remapping(&[&item], 1);
        let mut seq = Mapping::new();
        for m in maps {
            seq.append_map(m);
        }
        for pos in 0..30 {
            for assoc in [-1, 1] {
                let (a, b) = (seq.map_result(pos, assoc), folded.map_result(pos, assoc));
                assert_eq!(
                    (a.pos, a.deleted(), a.deleted_after(), a.deleted_across()),
                    (b.pos, b.deleted(), b.deleted_after(), b.deleted_across()),
                    "pos {pos} assoc {assoc} run {:?}",
                    item.unit_run
                );
            }
        }
    }
}
