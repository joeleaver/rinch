//! Fixtures from the review of #836: the `---` / `***` horizontal-rule input rule.
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

#[test]
fn triple_dash_before_existing_text_keeps_the_text() {
    // Caret at the START of "world", type "---": the rule sees only the text
    // before the caret ("---"), so `^...$` matches although the paragraph is
    // not empty.
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "world")))
        .unwrap();
    let mut st = state_of(s, doc);
    st = type_at(&st, 1, "-");
    st = type_at(&st, 2, "-");
    st = type_at(&st, 3, "-");
    eprintln!("after: {}", types(&st.doc));
    assert!(
        all_text(&st.doc).contains("world"),
        "the paragraph's text was deleted by the hr rule: {}",
        types(&st.doc)
    );
}

#[test]
fn triple_dash_in_a_list_item() {
    let s = Rc::new(Schema::starter_kit());
    let li = s
        .branch("list_item", Fragment::from_node(para(&s, "--")))
        .unwrap();
    let ul = s.branch("bullet_list", Fragment::from_node(li)).unwrap();
    let doc = s.branch("doc", Fragment::from_node(ul)).unwrap();
    let st = state_of(s, doc);
    // doc > ul(0) > li(1) > p(2) content starts at 3; "--" → caret at 5
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| type_at(&st, 5, "-")));
    match r {
        Ok(next) => eprintln!("list: {} sel={:?}", types(&next.doc), next.selection),
        Err(_) => eprintln!("list: PANIC"),
    }
}

#[test]
fn triple_dash_in_a_blockquote() {
    let s = Rc::new(Schema::starter_kit());
    let bq = s
        .branch("blockquote", Fragment::from_node(para(&s, "--")))
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(bq)).unwrap();
    let st = state_of(s, doc);
    let next = type_at(&st, 4, "-");
    eprintln!("blockquote: {} sel={:?}", types(&next.doc), next.selection);
}

#[test]
fn triple_dash_in_a_heading() {
    let s = Rc::new(Schema::starter_kit());
    let h = s
        .create_node(
            "heading",
            Attrs::new().with("level", AttrValue::from(2i64)),
            Fragment::from_node(s.text("--").unwrap()),
        )
        .unwrap();
    let doc = s.branch("doc", Fragment::from_node(h)).unwrap();
    let st = state_of(s, doc);
    let next = type_at(&st, 3, "-");
    eprintln!("heading: {} sel={:?}", types(&next.doc), next.selection);
}

#[test]
fn triple_star_after_bold_text_is_not_eaten() {
    // "**" then "*" at the block start is claimed by the hr rule; check that
    // "**bold*" + "*" still bolds (the rule is anchored, so it must).
    let s = Rc::new(Schema::starter_kit());
    let doc = s
        .branch("doc", Fragment::from_node(para(&s, "**bold*")))
        .unwrap();
    let st = state_of(s, doc);
    let next = type_at(&st, 8, "*");
    eprintln!("bold: {}", types(&next.doc));
}

#[test]
fn undo_after_the_rule() {
    let s = Rc::new(Schema::starter_kit());
    let doc = s.branch("doc", Fragment::from_node(para(&s, ""))).unwrap();
    let mut st = state_of(s, doc);
    st = type_at(&st, 1, "-");
    st = type_at(&st, 2, "-");
    st = type_at(&st, 3, "-");
    eprintln!("rule: {}", types(&st.doc));
    match st.run("undo") {
        Some(u) => eprintln!("undone: {}", types(&u.doc)),
        None => eprintln!("undo refused"),
    }
}
