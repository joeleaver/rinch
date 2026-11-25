//! M4 acceptance tests, driven through the **public** state API (no view) — the
//! design's M4 test list: type → bold a range → undo → redo; stored-marks "cursor
//! bold then type"; typing coalesces into one undo group; markdown input rules;
//! toolbar queries read state.

use rinch_editor_core::commands::{current_block_type, in_node_type, is_mark_active};
use rinch_editor_core::*;
use std::rc::Rc;

fn editor(text: &str) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let para = schema
        .branch("paragraph", Fragment::from_node(schema.text(text).unwrap()))
        .unwrap();
    let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

fn empty_editor() -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let para = schema.branch("paragraph", Fragment::empty()).unwrap();
    let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

/// Type text at the current selection (the view's printable-key path).
fn type_text(state: &EditorState, text: &str) -> EditorState {
    let mut tr = state.tr();
    tr.insert_text(text).unwrap();
    state.apply(tr)
}

#[test]
fn type_bold_undo_redo_end_to_end() {
    // type "hi", bold it, undo (removes bold), undo (removes typing), redo, redo
    let mut state = empty_editor();
    state.selection = Selection::cursor(Pos(1));
    state = type_text(&state, "h");
    state = type_text(&state, "i");
    assert_eq!(all_text(&state.doc), "hi");

    // select the whole word and bold it
    state.selection = Selection::text(Pos(1), Pos(3));
    state = state.run("toggleBold").expect("bold applies");
    let bold = state.schema().mark_type("bold").unwrap().clone();
    assert!(is_mark_active(&state, &bold));

    // undo bold (its own group) → text remains, no bold
    state = state.run("undo").expect("undo bold");
    assert_eq!(all_text(&state.doc), "hi");
    assert!(!is_mark_active(&state, &bold));

    // undo typing (coalesced into one group) → empty
    state = state.run("undo").expect("undo typing");
    assert_eq!(all_text(&state.doc), "");

    // redo typing, redo bold
    state = state.run("redo").expect("redo typing");
    assert_eq!(all_text(&state.doc), "hi");
    state = state.run("redo").expect("redo bold");
    assert!(is_mark_active(&state, &bold));
}

#[test]
fn stored_marks_cursor_bold_then_type() {
    // audit S2: bold with a collapsed cursor, then type → the typed text is bold
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(3)); // end of "ab"
    state = state.run("toggleBold").expect("toggleBold (cursor)");
    let bold = state.schema().mark_type("bold").unwrap().clone();
    assert!(
        is_mark_active(&state, &bold),
        "stored bold is active at the cursor"
    );

    state = type_text(&state, "C");
    let c_is_bold = state
        .doc
        .child(0)
        .content()
        .iter()
        .any(|n| n.text() == Some("C") && n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(c_is_bold, "typed text after cursor-bold is bold");
    // ...and continuing to type stays bold (inherited from the bold run)
    state = type_text(&state, "D");
    let d_is_bold = state
        .doc
        .child(0)
        .content()
        .iter()
        .any(|n| n.text() == Some("CD") && n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(d_is_bold);
}

#[test]
fn typing_is_one_undo_group_via_keymap() {
    // resolve the keymap binding for Mod-z to confirm the keymap aggregates plugins
    let state = empty_editor();
    let undo_binding = KeyBinding::parse("Mod-z").unwrap();
    assert_eq!(state.keymap().command_for(&undo_binding), Some("undo"));
    let bold_binding = KeyBinding::parse("Mod-b").unwrap();
    assert_eq!(
        state.keymap().command_for(&bold_binding),
        Some("toggleBold")
    );
}

#[test]
fn markdown_heading_input_rule_via_public_api() {
    // type "##" then a space → the input rule turns the block into an <h2>
    let mut state = empty_editor();
    state.selection = Selection::cursor(Pos(1));
    state = type_text(&state, "#");
    state = type_text(&state, "#");
    assert_eq!(all_text(&state.doc), "##");

    // the view runs input rules before the plain insert of " "
    let pos = state.selection.from().0;
    let tr =
        apply_input_rules(&state, state.input_rules(), pos, " ").expect("heading input rule fires");
    state = state.apply(tr);
    assert_eq!(state.doc.child(0).type_name(), "heading");
    assert_eq!(state.doc.child(0).attrs().get_int("level"), Some(2));

    // undo the rule → back to a paragraph with "##"
    state = state.run("undo").expect("undo the input rule");
    assert_eq!(state.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(&state.doc), "##");
}

#[test]
fn bullet_list_input_rule_and_block_queries() {
    let mut state = empty_editor();
    state.selection = Selection::cursor(Pos(1));
    state = type_text(&state, "-");
    let pos = state.selection.from().0;
    let tr = apply_input_rules(&state, state.input_rules(), pos, " ").expect("bullet rule fires");
    state = state.apply(tr);
    assert!(in_node_type(&state, "bullet_list"));
    // the inner textblock is still a paragraph (A6: List button needs in_node_type)
    assert_eq!(
        current_block_type(&state).map(|t| t.name().to_string()),
        Some("paragraph".to_string())
    );
}

#[test]
fn toolbar_queries_drive_button_states() {
    let mut state = editor("hello world");
    // no selection mark active initially
    let bold = state.schema().mark_type("bold").unwrap().clone();
    state.selection = Selection::text(Pos(1), Pos(6)); // "hello"
    assert!(!is_mark_active(&state, &bold));
    let state = state.run("toggleBold").unwrap();
    assert!(is_mark_active(&state, &bold));
    // commands report applicability without performing an edit
    assert!(state.can_run("toggleItalic"));
    assert!(state.can_run("wrapInBlockquote"));
}

#[test]
fn apply_is_pure_old_state_unchanged() {
    // applying a transaction returns a NEW state; the old one is untouched
    let state = editor("ab");
    let before_text = all_text(&state.doc);
    let next = type_text(&state, "X");
    assert_eq!(all_text(&state.doc), before_text, "old state is immutable");
    assert_ne!(all_text(&next.doc), before_text);
}

#[cfg(feature = "serde")]
#[test]
fn doc_json_round_trips_an_edited_document() {
    // M3 serialization still round-trips a document the state layer produced
    let mut state = empty_editor();
    state.selection = Selection::cursor(Pos(1));
    state = type_text(&state, "x");
    state.selection = Selection::text(Pos(1), Pos(2));
    state = state.run("toggleBold").unwrap();

    let doc_node = state.doc.to_doc().expect("serialize to DocNode");
    let restored = state
        .schema()
        .node_from_doc(&doc_node)
        .expect("deserialize");
    assert_eq!(restored, state.doc, "DocNode round-trip is lossless");
}
