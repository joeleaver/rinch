//! Copying task items and pasting them (review of PR #1374, #1365).
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{PasteContent, Pos, Selection};
use rinch_editor_view::create_editor;

/// D3 of the review — RED at d8a79deb, green on main (c1c77f36) for the marks: copying the
/// items of a task list and pasting them at a caret. The clipboard HTML is now
/// bare `<li data-type="taskItem">`s, which parse to a closed `task_list` slice
/// that `tr.replace` refuses inside a textblock, so the paste falls back to
/// `text/plain`: bold is lost (main pasted `<div><p><strong>…` as paragraphs
/// with their marks), and the task list the PR says paste-in keeps is not kept.
#[test]
fn copying_task_items_and_pasting_them_keeps_marks() {
    let src = create_editor();
    // Built from nodes, so the fixture also runs on main (whose HTML import
    // has no task list).
    src.load_html("<p>before</p><p><strong>TASKA</strong></p><p>TASKB</p><p>after</p>");
    let d = src.doc();
    let s = src.state().schema().clone();
    let item = |c: bool, para: &rinch_editor_core::Node| {
        s.create_node(
            "task_item",
            rinch_editor_core::Attrs::from_iter([(
                "checked",
                rinch_editor_core::AttrValue::Bool(c),
            )]),
            rinch_editor_core::Fragment::from_node(para.clone()),
        )
        .unwrap()
    };
    let list = s
        .create_node(
            "task_list",
            rinch_editor_core::Attrs::new(),
            rinch_editor_core::Fragment::from_children(vec![
                item(true, &d.child(1)),
                item(false, &d.child(2)),
            ]),
        )
        .unwrap();
    let doc = s
        .create_node(
            "doc",
            rinch_editor_core::Attrs::new(),
            rinch_editor_core::Fragment::from_children(vec![
                d.child(0).clone(),
                list,
                d.child(3).clone(),
            ]),
        )
        .unwrap();
    src.load_doc(doc);
    src.set_selection(Selection::text(Pos(11), Pos(27)));
    let (html, text) = src.selection_clipboard().unwrap();
    assert_eq!(text, "TASKA\nTASKB");
    let dst = create_editor();
    dst.load_html("<p></p>");
    dst.set_selection(Selection::cursor(Pos(1)));
    assert!(dst.paste(&PasteContent {
        text: Some(text),
        html: Some(html.clone())
    }));
    let out = node_to_html(&dst.doc());
    assert!(
        out.contains("<strong>TASKA</strong>"),
        "clipboard {html:?} pasted as {out}"
    );
}
