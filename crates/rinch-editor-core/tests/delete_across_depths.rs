//! A delete whose ends are at different depths (reported from Pimble: two bullet
//! items selected, the second nested under the first, and Delete did nothing).
#![cfg(feature = "markdown")]

use rinch_editor_core::serialize::doc_from_markdown;
use rinch_editor_core::{EditorState, Pos, Schema, Selection, default_plugins};
use std::rc::Rc;

fn state(md: &str) -> EditorState {
    let schema = Rc::new(Schema::starter_kit());
    let doc = doc_from_markdown(&schema, md).unwrap();
    EditorState::create(schema, doc, default_plugins())
}

/// The position of `needle`'s first character in the document's text.
fn pos_of(state: &EditorState, needle: &str) -> usize {
    let size = state.doc.content().size();
    (0..=size)
        .find(|&p| {
            state.doc.resolve(Pos(p)).is_ok_and(|r| {
                let text: String = r
                    .parent()
                    .content()
                    .iter()
                    .filter_map(|c| c.text())
                    .collect();
                r.parent().is_textblock()
                    && text
                        .get(r.parent_offset()..)
                        .is_some_and(|t| t.starts_with(needle))
            })
        })
        .unwrap_or_else(|| panic!("no {needle:?}"))
}

/// The document after deleting from `from`'s first character through `to`'s last.
fn after(md: &str, from: &str, to: &str, command: &str) -> String {
    let mut s = state(md);
    let (a, b) = (pos_of(&s, from), pos_of(&s, to) + to.len());
    s.selection = Selection::text(Pos(a), Pos(b));
    let after = s
        .run(command)
        .unwrap_or_else(|| panic!("{command} did nothing on {md:?}"));
    after.doc.rebind(after.schema()).unwrap();
    assert!(
        after.selection.is_empty() && after.selection.from() == Pos(a),
        "the caret is where the range began"
    );
    format!("{:?}", after.doc)
}

#[test]
fn an_item_and_the_item_nested_under_it() {
    for command in ["deleteCharForward", "deleteCharBackward", "deleteSelection"] {
        // Part of each: what is left of the nested item's text joins the outer item's.
        assert_eq!(
            after("- one\n  - two", "ne", "tw", command),
            r#"doc([bullet_list([list_item([paragraph([text("oo")])])])])"#,
            "{command}"
        );
        // Both whole: one empty item is left.
        assert_eq!(
            after("- zero\n- one\n  - two\n- three", "one", "two", command),
            r#"doc([bullet_list([list_item([paragraph([text("zero")])]), list_item([paragraph]), list_item([paragraph([text("three")])])])])"#,
            "{command}"
        );
        // The nested list's later items stay nested under the item the range began in.
        assert_eq!(
            after(
                "- zero\n- one\n  - two\n  - more\n- three",
                "ro",
                "tw",
                command
            ),
            r#"doc([bullet_list([list_item([paragraph([text("zeo")]), bullet_list([list_item([paragraph([text("more")])])])]), list_item([paragraph([text("three")])])])])"#,
            "{command}"
        );
    }
}

#[test]
fn a_paragraph_into_a_list_and_a_list_out_to_a_paragraph() {
    let md = "before\n\n- one\n  - two\n\nafter";
    assert_eq!(
        after(md, "ne", "af", "deleteCharForward"),
        r#"doc([paragraph([text("before")]), bullet_list([list_item([paragraph([text("oter")])])])])"#
    );
    assert_eq!(
        after(md, "wo", "af", "deleteCharForward"),
        r#"doc([paragraph([text("before")]), bullet_list([list_item([paragraph([text("one")]), bullet_list([list_item([paragraph([text("tter")])])])])])])"#
    );
    // As ProseMirror leaves it: the nested list stays in its item.
    assert_eq!(
        after(md, "fore", "on", "deleteCharForward"),
        r#"doc([paragraph([text("bee")]), bullet_list([list_item([bullet_list([list_item([paragraph([text("two")])])])])]), paragraph([text("after")])])"#
    );
}

#[test]
fn every_text_range_of_a_nested_document_deletes() {
    let s = state(
        "before\n\n- one\n  - two\n    1. deep\n- three\n\n> quoted\n>\n> - in a quote\n\nafter",
    );
    let size = s.doc.content().size();
    let in_text = |p: usize| {
        s.doc
            .resolve(Pos(p))
            .is_ok_and(|r| r.parent().is_textblock())
    };
    for from in (0..=size).filter(|&p| in_text(p)) {
        for to in (from + 1..=size).filter(|&p| in_text(p)) {
            let mut state = s.clone();
            state.selection = Selection::text(Pos(from), Pos(to));
            let after = state
                .run("deleteCharForward")
                .unwrap_or_else(|| panic!("{from}..{to} did nothing"));
            after
                .doc
                .rebind(after.schema())
                .unwrap_or_else(|e| panic!("{from}..{to}: {e}"));
            assert!(
                after.doc.content().size() < size,
                "{from}..{to} deleted nothing"
            );
        }
    }
}
