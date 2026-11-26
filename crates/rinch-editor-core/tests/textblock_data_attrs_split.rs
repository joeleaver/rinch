//! What a split (Enter, a rule inserted in a block) and pasted lines of plain
//! text do with a textblock's `data-*` attributes, by default and by an app's
//! choice (`DataAttrSplit`). The rules are in `schema::data_attrs`.

mod data_attr_common;

use data_attr_common::*;
use rinch_editor_core::commands::{
    delete_char_backward, insert_horizontal_rule, split_block, split_block_or_list_item,
};
use rinch_editor_core::schema::DataAttrSplit;
use rinch_editor_core::{DataAttrSplitPlugin, Pos};

#[test]
fn enter_copies_them_to_both_blocks() {
    for cmd in [split_block, split_block_or_list_item] {
        let enter = |at| html(&run(&state(SUB, at), cmd()));
        assert_eq!(
            enter(5),
            r#"<p data-role="sub">abcd</p><p data-role="sub"></p>"#
        );
        assert_eq!(
            enter(1),
            r#"<p data-role="sub"></p><p data-role="sub">abcd</p>"#
        );
        assert_eq!(
            enter(3),
            r#"<p data-role="sub">ab</p><p data-role="sub">cd</p>"#
        );
    }
    let s = run(&state(r#"<p data-role="sub"></p>"#, 1), split_block());
    assert_eq!(
        html(&s),
        r#"<p data-role="sub"></p><p data-role="sub"></p>"#
    );
    // Over a selection, where the split falls.
    let s = run(&select(state(SUB, 1), 3, 5), split_block());
    assert_eq!(
        html(&s),
        r#"<p data-role="sub">ab</p><p data-role="sub"></p>"#
    );
}

#[test]
fn enter_in_a_list_item_copies_them_too() {
    let list = r#"<ul><li><p data-role="sub">abcd</p></li></ul>"#;
    let enter = |at| html(&run(&state(list, at), split_block_or_list_item()));
    assert_eq!(
        enter(7),
        r#"<ul><li><p data-role="sub">abcd</p></li><li><p data-role="sub"></p></li></ul>"#
    );
    assert_eq!(
        enter(3),
        r#"<ul><li><p data-role="sub"></p></li><li><p data-role="sub">abcd</p></li></ul>"#
    );
    assert_eq!(
        enter(5),
        r#"<ul><li><p data-role="sub">ab</p></li><li><p data-role="sub">cd</p></li></ul>"#
    );
}

/// The paragraph after a heading is another element: it gets none. The
/// heading's other splits copy them.
#[test]
fn the_paragraph_after_a_heading_is_plain() {
    let enter = |doc, at| html(&run(&state(doc, at), split_block()));
    let full = r#"<h2 data-role="h">ab</h2>"#;
    assert_eq!(enter(full, 3), r#"<h2 data-role="h">ab</h2><p></p>"#);
    assert_eq!(
        enter(full, 1),
        r#"<h2 data-role="h"></h2><h2 data-role="h">ab</h2>"#
    );
    assert_eq!(
        enter(full, 2),
        r#"<h2 data-role="h">a</h2><h2 data-role="h">b</h2>"#
    );
    assert_eq!(
        enter(r#"<h2 data-role="h"></h2>"#, 1),
        r#"<h2 data-role="h"></h2><p></p>"#
    );
}

/// The paragraph's other attributes go on as they did.
#[test]
fn enter_keeps_the_paragraphs_own_attributes() {
    let s = run(
        &state_with(
            r#"<p style="text-align:center" data-role="sub">ab</p>"#,
            3,
            DataAttrSplitPlugin::new().with("data-role", DataAttrSplit::CopyToText),
        ),
        split_block(),
    );
    assert_eq!(
        html(&s),
        r#"<p style="text-align:center" data-role="sub">ab</p><p style="text-align:center"></p>"#
    );
}

fn choosing(split: DataAttrSplit) -> DataAttrSplitPlugin {
    DataAttrSplitPlugin::new().with("data-role", split)
}

/// Enter at the end, the start, the middle and in an empty block, in a
/// paragraph and in a list item.
fn four_enters(split: DataAttrSplit, expected: [&str; 4]) {
    let empty = r#"<p data-role="sub"></p>"#;
    for ((doc, at), want) in [(SUB, 5), (SUB, 1), (SUB, 3), (empty, 1)]
        .into_iter()
        .zip(expected)
    {
        let s = run(&state_with(doc, at, choosing(split)), split_block());
        assert_eq!(html(&s), want, "{split:?} at {at} of {doc}");
        // The caret goes with the text, whatever the attributes do.
        assert_eq!(s.selection.from(), Pos(at + 2), "{split:?}");
        let list = format!("<ul><li>{doc}</li></ul>");
        // An empty item's Enter leaves the list instead.
        if doc != empty {
            let s = run(
                &state_with(&list, at + 2, choosing(split)),
                split_block_or_list_item(),
            );
            let items = want.replace("<p", "<li><p").replace("</p>", "</p></li>");
            assert_eq!(html(&s), format!("<ul>{items}</ul>"), "{split:?} in a list");
        }
    }
}

#[test]
fn copy_to_text_starts_a_plain_block_where_there_is_no_text() {
    four_enters(
        DataAttrSplit::CopyToText,
        [
            r#"<p data-role="sub">abcd</p><p></p>"#,
            r#"<p></p><p data-role="sub">abcd</p>"#,
            r#"<p data-role="sub">ab</p><p data-role="sub">cd</p>"#,
            r#"<p data-role="sub"></p><p></p>"#,
        ],
    );
}

#[test]
fn keep_leaves_them_on_one_block() {
    four_enters(
        DataAttrSplit::Keep,
        [
            r#"<p data-role="sub">abcd</p><p></p>"#,
            r#"<p></p><p data-role="sub">abcd</p>"#,
            r#"<p data-role="sub">ab</p><p>cd</p>"#,
            r#"<p data-role="sub"></p><p></p>"#,
        ],
    );
}

#[test]
fn copy_is_the_default_by_name_too() {
    four_enters(
        DataAttrSplit::Copy,
        [
            r#"<p data-role="sub">abcd</p><p data-role="sub"></p>"#,
            r#"<p data-role="sub"></p><p data-role="sub">abcd</p>"#,
            r#"<p data-role="sub">ab</p><p data-role="sub">cd</p>"#,
            r#"<p data-role="sub"></p><p data-role="sub"></p>"#,
        ],
    );
}

/// A choice is by name: an attribute nobody names is copied. A later plugin
/// replaces an earlier one's choice.
#[test]
fn a_choice_is_one_attributes() {
    let doc = r#"<p data-id="7" data-role="sub">ab</p>"#;
    let s = state_with(
        doc,
        3,
        DataAttrSplitPlugin::new().with("data-id", DataAttrSplit::Keep),
    );
    assert_eq!(
        html(&run(&s, split_block())),
        r#"<p data-id="7" data-role="sub">ab</p><p data-role="sub"></p>"#
    );
    let twice = DataAttrSplitPlugin::new()
        .with("data-role", DataAttrSplit::Keep)
        .with("data-role", DataAttrSplit::Copy);
    let s = run(&state_with(SUB, 5, twice), split_block());
    assert_eq!(html(&s).matches("data-role").count(), 2);
}

/// A raw split copies the markup and asks nobody.
#[test]
fn a_raw_split_copies_the_markup() {
    let s = state_with(SUB, 5, choosing(DataAttrSplit::Keep));
    let mut tr = s.tr();
    tr.split(5, 1, None).unwrap();
    assert_eq!(html(&s.apply(tr)).matches("data-role").count(), 2);
}

/// A rule inserted inside a block splits it as Enter does.
#[test]
fn a_rule_inserted_in_a_block_splits_it_as_enter_does() {
    let rule = |split, at| {
        html(&run(
            &state_with(SUB, at, choosing(split)),
            insert_horizontal_rule(),
        ))
    };
    assert_eq!(rule(DataAttrSplit::Copy, 3).matches("data-role").count(), 2);
    assert_eq!(rule(DataAttrSplit::Keep, 3).matches("data-role").count(), 1);
    assert_eq!(rule(DataAttrSplit::Copy, 5).matches("data-role").count(), 1);
}

#[test]
fn undo_takes_a_split_back_in_one_step() {
    let start = state_with(SUB, 5, choosing(DataAttrSplit::CopyToText));
    let s = run(&start, split_block());
    let undo = s.command("undo").expect("history");
    assert_eq!(run(&s, undo).doc, start.doc);
    // A join's attributes come back with it too.
    let start = state(TWO, 1);
    let s = run(&select(start, 1, 6), delete_char_backward());
    let undo = s.command("undo").expect("history");
    assert_eq!(html(&run(&s, undo)), TWO);
}

// ── Lines of plain text ──────────────────────────────────────────────────

fn lines(split: DataAttrSplit, doc: &str, from: usize, to: usize, text: &str) -> String {
    let s = select(state_with(doc, 1, choosing(split)), from, to);
    html(&paste_text(&s, text))
}

#[test]
fn lines_of_plain_text_place_them_as_enter_does() {
    let all = r#"<p data-role="sub">1</p><p data-role="sub">2</p><p data-role="sub">3abcd</p>"#;
    assert_eq!(lines(DataAttrSplit::Copy, SUB, 1, 1, "1\n2\n3"), all);
    for split in [DataAttrSplit::CopyToText, DataAttrSplit::Keep] {
        // At the start the new lines hold only pasted text.
        assert_eq!(
            lines(split, SUB, 1, 1, "1\n2\n3"),
            r#"<p>1</p><p>2</p><p data-role="sub">3abcd</p>"#,
            "{split:?}"
        );
        // At the end the block keeps them.
        let out = lines(split, SUB, 5, 5, "1\n2\n3");
        assert_eq!(
            out, r#"<p data-role="sub">abcd1</p><p>2</p><p>3</p>"#,
            "{split:?}"
        );
    }
    assert_eq!(
        lines(DataAttrSplit::CopyToText, SUB, 3, 3, "1\n2\n3"),
        r#"<p data-role="sub">ab1</p><p data-role="sub">2</p><p data-role="sub">3cd</p>"#
    );
    assert_eq!(
        lines(DataAttrSplit::Keep, SUB, 3, 3, "1\n2\n3"),
        r#"<p data-role="sub">ab1</p><p>2</p><p>3cd</p>"#
    );
    // Over all of the text from its start.
    assert_eq!(
        lines(DataAttrSplit::Keep, SUB, 1, 3, "1\n2"),
        r#"<p>1</p><p data-role="sub">2cd</p>"#
    );
}

/// With nothing after the range the block keeps them, an empty first line too.
#[test]
fn an_empty_first_line_leaves_them_on_the_block() {
    for (doc, to) in [(r#"<p data-role="sub"></p>"#, 1), (SUB, 5)] {
        let out = lines(DataAttrSplit::CopyToText, doc, 1, to, "\n2");
        assert_eq!(out, r#"<p data-role="sub"></p><p>2</p>"#);
        let out = lines(DataAttrSplit::Copy, doc, 1, to, "\n2");
        assert_eq!(out, r#"<p data-role="sub"></p><p data-role="sub">2</p>"#);
    }
}

/// Over two blocks, the text left of the second keeps the second's.
#[test]
fn lines_over_two_blocks_leave_each_texts_own() {
    for split in [
        DataAttrSplit::Copy,
        DataAttrSplit::CopyToText,
        DataAttrSplit::Keep,
    ] {
        let out = lines(split, TWO, 2, 6, "1\n2");
        assert_eq!(
            out, r#"<p data-role="a">a1</p><p data-role="b">2d</p>"#,
            "{split:?}"
        );
    }
    assert_eq!(
        lines(DataAttrSplit::Copy, TWO, 2, 6, "1\n2\n3"),
        r#"<p data-role="a">a1</p><p data-role="a">2</p><p data-role="b">3d</p>"#
    );
    assert_eq!(
        lines(DataAttrSplit::Keep, TWO, 2, 6, "1\n2\n3"),
        r#"<p data-role="a">a1</p><p>2</p><p data-role="b">3d</p>"#
    );
}

/// The paragraph a paste makes of a heading's tail gets none, as at Enter.
#[test]
fn a_headings_tail_made_a_paragraph_is_plain() {
    let h = r#"<h2 data-role="h">abcd</h2>"#;
    assert_eq!(
        lines(DataAttrSplit::Copy, h, 3, 3, "1\n2"),
        r#"<h2 data-role="h">ab1</h2><p>2cd</p>"#
    );
    assert_eq!(
        lines(DataAttrSplit::Copy, h, 5, 5, "1\n2"),
        r#"<h2 data-role="h">abcd1</h2><p>2</p>"#
    );
}
