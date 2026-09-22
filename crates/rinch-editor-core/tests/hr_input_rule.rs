//! The `---` / `***` horizontal-rule input rule, end to end through
//! `apply_input_rules` (fixtures from the review of #836).
use rinch_editor_core::*;
use std::rc::Rc;

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

fn types(node: &Node) -> String {
    if node.is_text() {
        return format!("\"{}\"", node.text().unwrap());
    }
    let kids: Vec<String> = node.content().iter().map(types).collect();
    if kids.is_empty() {
        node.type_name().to_string()
    } else {
        format!("{}({})", node.type_name(), kids.join(","))
    }
}

fn state_of(s: Rc<Schema>, doc: Node) -> EditorState {
    EditorState::create(s, doc, default_plugins())
}

fn para(s: &Schema, t: &str) -> Node {
    if t.is_empty() {
        s.branch("paragraph", Fragment::empty()).unwrap()
    } else {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }
}

/// Type `ch` at `pos` through the input-rule path, falling back to plain insert.
fn type_at(state: &EditorState, pos: usize, ch: &str) -> EditorState {
    let mut st = state.clone();
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(pos)));
    st = st.apply(tr);
    if let Some(tr) = apply_input_rules(&st, st.input_rules(), pos, ch) {
        return st.apply(tr);
    }
    let mut tr = st.tr();
    tr.insert_text(ch).unwrap();
    st.apply(tr)
}

fn is_text_cursor_in_textblock(st: &EditorState) -> bool {
    matches!(st.selection, Selection::Text(_))
        && st
            .doc
            .resolve(st.selection.head())
            .map(|r| r.parent().is_textblock())
            .unwrap_or(false)
}

fn typed(st: EditorState, from: usize, chars: &str) -> EditorState {
    let mut st = st;
    for (i, ch) in chars.chars().enumerate() {
        st = type_at(&st, from + i, &ch.to_string());
    }
    st
}

#[test]
fn triple_dash_before_existing_text_keeps_the_text() {
    // Caret at the START of "world", type "---": the rule sees only the text
    // before the caret ("---"), so `^...$` matches although the paragraph is
    // not empty. It used to replace the whole paragraph, deleting "world".
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "world")))
        .unwrap();
    let st = typed(state_of(s, doc), 1, "---");
    assert_eq!(types(&st.doc), r#"doc(paragraph("---world"))"#);
}

#[test]
fn triple_star_before_existing_text_keeps_the_text() {
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "world")))
        .unwrap();
    let st = typed(state_of(s, doc), 1, "***");
    assert_eq!(all_text(&st.doc), "***world");
    assert_eq!(st.doc.child(0).type_name(), "paragraph");
}

#[test]
fn triple_dash_on_an_empty_paragraph_between_two_others_lands_in_the_next() {
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch(
            "doc",
            Fragment::from_children(vec![para(&s, "a"), para(&s, ""), para(&s, "b")]),
        )
        .unwrap();
    // p("a") = 0..3, the empty paragraph opens at 3, content at 4.
    let st = typed(state_of(s, doc), 4, "---");
    assert_eq!(
        types(&st.doc),
        r#"doc(paragraph("a"),horizontal_rule,paragraph("b"))"#
    );
    assert_eq!(st.selection, Selection::cursor(Pos(5)));
}

#[test]
fn triple_dash_as_a_list_items_only_paragraph_keeps_a_textblock_for_the_caret() {
    // The starter kit's `list_item` is `block+`, so a rule may lead it; what
    // must not happen is a list item holding only the atom, with the caret on it
    // (review of #836, finding 9).
    let s = Rc::new(Schema::starter_kit());
    let li = s
        .branch("list_item", Fragment::from_node(para(&s, "--")))
        .unwrap();
    let ul = s.branch("bullet_list", Fragment::from_node(li)).unwrap();
    let doc = s.branch("doc", Fragment::from_node(ul)).unwrap();
    // doc > ul(0) > li(1) > p(2) content starts at 3; "--" → caret at 5
    let next = type_at(&state_of(s, doc), 5, "-");
    assert_eq!(
        types(&next.doc),
        "doc(bullet_list(list_item(horizontal_rule,paragraph)))"
    );
    assert!(
        is_text_cursor_in_textblock(&next),
        "caret on an atom: {:?}",
        next.selection
    );
}

#[test]
fn triple_dash_as_a_list_items_last_paragraph_keeps_a_textblock_for_the_caret() {
    let s = Rc::new(Schema::starter_kit());
    let li = s
        .branch(
            "list_item",
            Fragment::from_children(vec![para(&s, "a"), para(&s, "--")]),
        )
        .unwrap();
    let ul = s.branch("bullet_list", Fragment::from_node(li)).unwrap();
    let doc = s.branch("doc", Fragment::from_node(ul)).unwrap();
    // ul 0, li 1, p("a") 2..5, p("--") opens at 5, content 6..8.
    let next = type_at(&state_of(s, doc), 8, "-");
    assert_eq!(
        types(&next.doc),
        r#"doc(bullet_list(list_item(paragraph("a"),horizontal_rule,paragraph)))"#
    );
    assert!(
        is_text_cursor_in_textblock(&next),
        "caret on an atom: {:?}",
        next.selection
    );
}

#[test]
fn triple_dash_in_a_blockquote_keeps_a_textblock_for_the_caret() {
    // `has_following_block` used to ask about the whole doc, so a blockquote's
    // last paragraph became `blockquote(horizontal_rule)` with the caret on the
    // atom (review of #836, finding 9).
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch("blockquote", Fragment::from_node(para(&s, "--")))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
    let next = type_at(&state_of(s, doc), 4, "-");
    assert_eq!(
        types(&next.doc),
        "doc(blockquote(horizontal_rule,paragraph))"
    );
    assert!(
        is_text_cursor_in_textblock(&next),
        "caret on an atom: {:?}",
        next.selection
    );
}

#[test]
fn triple_dash_in_a_heading_stays_text() {
    let s = Rc::new(Schema::starter_kit());
    let h = s
        .create_node(
            "heading",
            Attrs::new().with("level", AttrValue::from(2i64)),
            Fragment::from_node(s.text("--").unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(h)).unwrap();
    let next = type_at(&state_of(s, doc), 3, "-");
    assert_eq!(types(&next.doc), r#"doc(heading("---"))"#);
}

#[test]
fn triple_star_in_a_code_block_stays_text() {
    let s = Rc::new(Schema::starter_kit());
    let cb = s
        .branch("code_block", Fragment::from_node(s.text("**").unwrap()))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(cb)).unwrap();
    let next = type_at(&state_of(s, doc), 3, "*");
    assert_eq!(types(&next.doc), r#"doc(code_block("***"))"#);
}

#[test]
fn bold_and_italic_rules_still_fire_beside_the_star_rule() {
    // "**bold*" + "*" → bold; "*it" + "*" → italic. The `***` rule is anchored
    // to the whole paragraph, so neither is claimed by it.
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "**bold*")))
        .unwrap();
    let next = type_at(&state_of(s.clone(), doc), 8, "*");
    let run = next.doc.child(0).child(0);
    assert_eq!(run.text(), Some("bold"));
    assert!(run.marks().iter().any(|m| m.type_name() == "bold"));

    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "*it")))
        .unwrap();
    let next = type_at(&state_of(s, doc), 4, "*");
    let run = next.doc.child(0).child(0);
    assert_eq!(run.text(), Some("it"));
    assert!(run.marks().iter().any(|m| m.type_name() == "italic"));
}

#[test]
fn undo_after_the_rule_restores_the_marker() {
    let s = Rc::new(Schema::starter_kit());
    let doc = s.branch("doc", Fragment::from_node(para(&s, ""))).unwrap();
    let st = typed(state_of(s, doc), 1, "---");
    assert_eq!(types(&st.doc), "doc(horizontal_rule,paragraph)");
    let undone = st.run("undo").expect("undo");
    assert_eq!(types(&undone.doc), r#"doc(paragraph("--"))"#);
}
