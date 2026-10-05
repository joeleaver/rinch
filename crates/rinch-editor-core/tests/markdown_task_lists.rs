//! GFM task lists in Markdown, both ways (#1365): `task_list` > `task_item`
//! is written as `- [ ] …` / `- [x] …` and read back as the same document,
//! so a read-edit-write round trip (Pimble's: an LLM reads a note as Markdown
//! and writes it back) keeps every checklist.
#![cfg(feature = "markdown")]

use rinch_editor_core::serialize::{
    Construct, MarkdownError, doc_from_markdown, doc_from_markdown_strict, doc_to_markdown,
    node_to_html, slice_from_html,
};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Schema};

fn s() -> Schema {
    Schema::starter_kit()
}

fn n(s: &Schema, name: &str, attrs: Attrs, kids: Vec<Node>) -> Node {
    s.create_node(name, attrs, Fragment::from_children(kids))
        .unwrap()
}

fn p(s: &Schema, text: &str) -> Node {
    if text.is_empty() {
        n(s, "paragraph", Attrs::new(), vec![])
    } else {
        n(s, "paragraph", Attrs::new(), vec![s.text(text).unwrap()])
    }
}

fn h(s: &Schema, level: i64, text: &str) -> Node {
    n(
        s,
        "heading",
        Attrs::from_iter([("level", AttrValue::Int(level))]),
        vec![s.text(text).unwrap()],
    )
}

fn task(s: &Schema, checked: bool, kids: Vec<Node>) -> Node {
    n(
        s,
        "task_item",
        Attrs::from_iter([("checked", AttrValue::Bool(checked))]),
        kids,
    )
}

fn tasks(s: &Schema, items: Vec<Node>) -> Node {
    n(s, "task_list", Attrs::new(), items)
}

fn bullets(s: &Schema, items: Vec<Vec<Node>>) -> Node {
    let items = items
        .into_iter()
        .map(|kids| n(s, "list_item", Attrs::new(), kids))
        .collect();
    n(s, "bullet_list", Attrs::new(), items)
}

fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
    n(s, "doc", Attrs::new(), blocks)
}

/// Write `d`, read it back strictly and leniently, require the same document
/// both times, and require a second write to change nothing. Returns the
/// Markdown.
fn rt(s: &Schema, d: &Node) -> String {
    let md = doc_to_markdown(d);
    let back = doc_from_markdown_strict(s, &md)
        .unwrap_or_else(|e| panic!("strict read of {md:?} failed: {e}"));
    assert_eq!(&back, d, "round trip through {md:?}");
    let lenient = doc_from_markdown(s, &md).unwrap();
    assert_eq!(&lenient, d, "lenient round trip through {md:?}");
    assert_eq!(doc_to_markdown(&back), md, "second write of {md:?}");
    md
}

fn refusal(md: &str) -> (Construct, usize) {
    match doc_from_markdown_strict(&s(), md) {
        Err(MarkdownError::Unsupported {
            construct, line, ..
        }) => (construct, line),
        other => panic!("{md:?}: expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_issue_example_is_written_and_read_back() {
    let s = s();
    let d = doc(
        &s,
        vec![
            p(&s, "before"),
            tasks(&s, vec![task(&s, true, vec![p(&s, "buy milk")])]),
        ],
    );
    assert_eq!(rt(&s, &d), "before\n\n- [x] buy milk");
}

#[test]
fn checked_and_unchecked_items_round_trip() {
    let s = s();
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![
                task(&s, false, vec![p(&s, "todo")]),
                task(&s, true, vec![p(&s, "done")]),
                task(&s, false, vec![p(&s, "later")]),
            ],
        )],
    );
    assert_eq!(rt(&s, &d), "- [ ] todo\n- [x] done\n- [ ] later");
}

#[test]
fn an_uppercase_x_is_checked_in_both_readers() {
    let s = s();
    let want = doc(
        &s,
        vec![tasks(
            &s,
            vec![
                task(&s, true, vec![p(&s, "up")]),
                task(&s, false, vec![p(&s, "no")]),
            ],
        )],
    );
    let md = "- [X] up\n- [ ] no";
    assert_eq!(doc_from_markdown_strict(&s, md).unwrap(), want);
    assert_eq!(doc_from_markdown(&s, md).unwrap(), want);
    assert_eq!(doc_to_markdown(&want), "- [x] up\n- [ ] no");
}

#[test]
fn nested_task_lists_round_trip() {
    let s = s();
    let inner = tasks(
        &s,
        vec![
            task(&s, true, vec![p(&s, "sub a")]),
            task(&s, false, vec![p(&s, "sub b")]),
        ],
    );
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![
                task(&s, false, vec![p(&s, "parent"), inner]),
                task(&s, true, vec![p(&s, "sibling")]),
            ],
        )],
    );
    rt(&s, &d);
}

#[test]
fn a_task_list_inside_a_bullet_list_and_the_reverse_round_trip() {
    let s = s();
    let in_bullets = bullets(
        &s,
        vec![
            vec![
                p(&s, "plain"),
                tasks(&s, vec![task(&s, true, vec![p(&s, "t")])]),
            ],
            vec![p(&s, "plain 2")],
        ],
    );
    rt(&s, &doc(&s, vec![in_bullets]));
    let in_tasks = tasks(
        &s,
        vec![task(
            &s,
            false,
            vec![
                p(&s, "t"),
                bullets(&s, vec![vec![p(&s, "b1")], vec![p(&s, "b2")]]),
            ],
        )],
    );
    rt(&s, &doc(&s, vec![in_tasks]));
    // And inside a blockquote and an ordered list item.
    let quoted = n(
        &s,
        "blockquote",
        Attrs::new(),
        vec![tasks(&s, vec![task(&s, true, vec![p(&s, "q")])])],
    );
    rt(&s, &doc(&s, vec![quoted]));
    let ordered = n(
        &s,
        "ordered_list",
        Attrs::from_iter([("start", AttrValue::Int(1))]),
        vec![n(
            &s,
            "list_item",
            Attrs::new(),
            vec![tasks(&s, vec![task(&s, false, vec![p(&s, "o")])])],
        )],
    );
    rt(&s, &doc(&s, vec![ordered]));
}

#[test]
fn a_task_item_of_several_blocks_round_trips() {
    let s = s();
    let code = n(
        &s,
        "code_block",
        Attrs::new(),
        vec![s.text("let x = 1;\nlet y = 2;").unwrap()],
    );
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![
                task(&s, true, vec![p(&s, "first"), code, p(&s, "third")]),
                task(&s, false, vec![p(&s, "next")]),
            ],
        )],
    );
    rt(&s, &d);
}

/// A task item whose first block is not a paragraph: GFM's marker is the
/// start of a paragraph, so the writer puts the block on the next line.
#[test]
fn a_task_item_that_starts_with_another_block_round_trips() {
    let s = s();
    let quote = n(
        &s,
        "blockquote",
        Attrs::new(),
        vec![p(&s, "q1"), p(&s, "q2")],
    );
    let code = n(
        &s,
        "code_block",
        Attrs::new(),
        vec![s.text("c\nd").unwrap()],
    );
    let rule = n(&s, "horizontal_rule", Attrs::new(), vec![]);
    let cell = |text: &str, name: &str| n(&s, name, Attrs::new(), vec![p(&s, text)]);
    let table = n(
        &s,
        "table",
        Attrs::new(),
        vec![
            n(
                &s,
                "table_row",
                Attrs::new(),
                vec![cell("h", "table_header_cell")],
            ),
            n(&s, "table_row", Attrs::new(), vec![cell("v", "table_cell")]),
        ],
    );
    let firsts = vec![
        h(&s, 2, "heading"),
        quote,
        bullets(&s, vec![vec![p(&s, "a"), p(&s, "a2")], vec![p(&s, "b")]]),
        tasks(&s, vec![task(&s, true, vec![p(&s, "inner")])]),
        code,
        rule,
        table,
    ];
    for first in firsts {
        let d = doc(
            &s,
            vec![tasks(
                &s,
                vec![
                    task(&s, false, vec![first.clone(), p(&s, "after")]),
                    task(&s, true, vec![first]),
                ],
            )],
        );
        rt(&s, &d);
    }
}

/// A first paragraph that writes nothing (whitespace) is dropped, and the
/// block after it is then the item's first: it goes on the next line.
#[test]
fn a_task_item_whose_first_paragraph_is_blank_keeps_the_next_block() {
    let s = s();
    let quote = n(
        &s,
        "blockquote",
        Attrs::new(),
        vec![p(&s, "q1"), p(&s, "q2")],
    );
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![task(&s, true, vec![p(&s, " "), quote.clone()])],
        )],
    );
    let md = doc_to_markdown(&d);
    let want = doc(&s, vec![tasks(&s, vec![task(&s, true, vec![quote])])]);
    assert_eq!(doc_from_markdown_strict(&s, &md).unwrap(), want, "{md:?}");
}

#[test]
fn an_empty_task_item_round_trips_wherever_it_is() {
    let s = s();
    let empty = || task(&s, false, vec![p(&s, "")]);
    // Last in the document.
    rt(&s, &doc(&s, vec![p(&s, "x"), tasks(&s, vec![empty()])]));
    // Between two others.
    rt(
        &s,
        &doc(
            &s,
            vec![tasks(
                &s,
                vec![
                    task(&s, true, vec![p(&s, "a")]),
                    empty(),
                    task(&s, false, vec![p(&s, "b")]),
                ],
            )],
        ),
    );
    // Last in a blockquote, and last in a nested list.
    let quoted = n(
        &s,
        "blockquote",
        Attrs::new(),
        vec![tasks(&s, vec![empty()])],
    );
    rt(&s, &doc(&s, vec![quoted, p(&s, "after")]));
    let nested = tasks(
        &s,
        vec![task(
            &s,
            true,
            vec![p(&s, "outer"), tasks(&s, vec![empty()])],
        )],
    );
    rt(&s, &doc(&s, vec![nested]));
}

/// CommonMark continues a list across a blank line when the next item has the
/// same bullet character, so a task list written right after a bullet list
/// with the same `-` would join it as one mixed list.
#[test]
fn adjacent_bullet_and_task_lists_stay_apart() {
    let s = s();
    let b = || bullets(&s, vec![vec![p(&s, "bullet")]]);
    let t = || tasks(&s, vec![task(&s, true, vec![p(&s, "task")])]);
    rt(&s, &doc(&s, vec![b(), t()]));
    rt(&s, &doc(&s, vec![t(), b()]));
    rt(&s, &doc(&s, vec![b(), t(), b(), t()]));
    rt(&s, &doc(&s, vec![t(), t()]));
}

/// A GFM pipe table cannot hold a list, so a table with one is an HTML table,
/// and the task list in it is `<ul data-type="taskList">`, which reads back.
#[test]
fn a_task_list_in_a_table_cell_round_trips_as_html() {
    let s = s();
    let list = tasks(
        &s,
        vec![
            task(&s, true, vec![p(&s, "buy milk")]),
            task(&s, false, vec![p(&s, "eggs")]),
        ],
    );
    let cell = n(&s, "table_cell", Attrs::new(), vec![p(&s, "a"), list]);
    let row = n(&s, "table_row", Attrs::new(), vec![cell]);
    let d = doc(&s, vec![n(&s, "table", Attrs::new(), vec![row])]);
    let md = rt(&s, &d);
    assert!(md.contains("data-type=\"taskList\""), "{md}");
}

#[test]
fn html_copy_out_writes_task_lists_and_paste_in_reads_them() {
    let s = s();
    let list = tasks(
        &s,
        vec![
            task(&s, true, vec![p(&s, "a")]),
            task(&s, false, vec![p(&s, "b")]),
        ],
    );
    let html = node_to_html(&list);
    assert_eq!(
        html,
        "<ul data-type=\"taskList\"><li data-type=\"taskItem\" data-checked=\"true\"><p>a</p></li>\
         <li data-type=\"taskItem\" data-checked=\"false\"><p>b</p></li></ul>"
    );
    let slice = slice_from_html(&s, &html).unwrap();
    assert_eq!(slice.content.children(), std::slice::from_ref(&list));
    // An item that says nothing is unchecked; `data-checked` on a plain
    // bullet list's item is not a task.
    let slice = slice_from_html(&s, "<ul data-type=\"taskList\"><li>x</li></ul>").unwrap();
    assert_eq!(
        slice.content.children(),
        &[tasks(&s, vec![task(&s, false, vec![p(&s, "x")])])][..]
    );
    let slice = slice_from_html(&s, "<ul><li data-checked=\"true\">x</li></ul>").unwrap();
    assert_eq!(
        slice.content.children(),
        &[bullets(&s, vec![vec![p(&s, "x")]])][..]
    );
}

#[test]
fn strict_refuses_a_table_task_attribute_the_import_would_not_keep() {
    for (html, why) in [
        (
            "<table><tr><td><ul data-type=\"taskList\"><li data-checked=\"maybe\">x</li></ul></td></tr></table>",
            "a checked value that is not true or false",
        ),
        (
            "<table><tr><td><ul><li data-checked=\"true\">x</li></ul></td></tr></table>",
            "data-checked outside a task list",
        ),
        (
            "<table><tr><td><ul><li data-type=\"taskItem\">x</li></ul></td></tr></table>",
            "a task item's data-type outside a task list",
        ),
        (
            "<table><tr><td><ul data-type=\"other\"><li>x</li></ul></td></tr></table>",
            "a data-type that is not taskList",
        ),
    ] {
        assert_eq!(refusal(html), (Construct::HtmlBlock, 1), "{why}");
    }
}

#[test]
fn strict_refuses_a_mixed_or_ordered_task_list_and_lenient_keeps_the_marker_as_text() {
    let s = s();
    // Some items have a marker, some do not: the model has no such list.
    assert_eq!(
        refusal("intro\n\n- plain\n- [ ] todo\n- [x] done"),
        (Construct::TaskList, 4)
    );
    // An ordered task list would lose its numbers.
    assert_eq!(refusal("1. [ ] a\n2. [x] b"), (Construct::TaskList, 1));
    let e = doc_from_markdown_strict(&s, "\n\n- [ ] todo\n- not").unwrap_err();
    assert_eq!(e.to_string(), "line 3: task list is not supported: [ ]");

    let d = doc_from_markdown(&s, "- plain\n- [X] todo\n- [ ] **bold** tail").unwrap();
    let list = d.child(0);
    assert_eq!(list.type_name(), "bullet_list");
    assert_eq!(list.child(1).child(0).child(0).text(), Some("[X] todo"));
    let third = list.child(2).child(0);
    assert_eq!(third.child(0).text(), Some("[ ] "));
    assert_eq!(third.child(1).text(), Some("bold"));
    let d = doc_from_markdown(&s, "1. [ ] a\n2. [x] b").unwrap();
    assert_eq!(d.child(0).type_name(), "ordered_list");
    assert_eq!(d.child(0).child(1).child(0).child(0).text(), Some("[x] b"));
}

/// pulldown-cmark consumes a marker that a heading, a quote or another block
/// follows, and emits no event for it (or emits it inside that block): the
/// reader still finds it, and gives it to the item it starts.
#[test]
fn a_marker_before_another_block_is_still_the_items() {
    let s = s();
    for (md, first) in [
        ("- [ ] # h", h(&s, 1, "h")),
        (
            "- [ ] \n  > q",
            n(&s, "blockquote", Attrs::new(), vec![p(&s, "q")]),
        ),
        (
            "- [ ] > q",
            n(&s, "blockquote", Attrs::new(), vec![p(&s, "q")]),
        ),
        ("- [ ] - a", bullets(&s, vec![vec![p(&s, "a")]])),
    ] {
        let want = doc(&s, vec![tasks(&s, vec![task(&s, false, vec![first])])]);
        assert_eq!(doc_from_markdown_strict(&s, md).unwrap(), want, "{md:?}");
        assert_eq!(doc_from_markdown(&s, md).unwrap(), want, "{md:?}");
    }
    // A marker before an HTML table on its line: the table, and nothing
    // else (pulldown reports a stray space there).
    let cell = n(&s, "table_cell", Attrs::new(), vec![p(&s, "a")]);
    let table = n(
        &s,
        "table",
        Attrs::new(),
        vec![n(&s, "table_row", Attrs::new(), vec![cell])],
    );
    let want = doc(&s, vec![tasks(&s, vec![task(&s, false, vec![table])])]);
    let md = "- [ ] <table><tr><td>a</td></tr></table>";
    assert_eq!(doc_from_markdown_strict(&s, md).unwrap(), want);
    // Not a marker: an indented code block starts before it.
    let code = n(
        &s,
        "code_block",
        Attrs::new(),
        vec![s.text("[ ] x").unwrap()],
    );
    let want = doc(&s, vec![bullets(&s, vec![vec![code]])]);
    assert_eq!(doc_from_markdown_strict(&s, "-     [ ] x").unwrap(), want);
}

/// A marker with nothing after it on its line (`- [ ]`) is text to
/// pulldown-cmark, and the writer's own empty item once anything strips its
/// trailing space. Both readers take it as an empty item's marker when it is
/// the item's whole first paragraph. A bullet whose text is `[ ]` is written
/// `\[ \]`, so it is never mistaken for one.
#[test]
fn a_bare_marker_is_an_empty_task_item() {
    let s = s();
    let quote = n(&s, "blockquote", Attrs::new(), vec![p(&s, "q")]);
    for (md, want) in [
        ("- [ ]", tasks(&s, vec![task(&s, false, vec![p(&s, "")])])),
        ("- [x]\n", tasks(&s, vec![task(&s, true, vec![p(&s, "")])])),
        (
            "* [X]\r\n* [ ] b",
            tasks(
                &s,
                vec![
                    task(&s, true, vec![p(&s, "")]),
                    task(&s, false, vec![p(&s, "b")]),
                ],
            ),
        ),
        (
            "- [ ]\n  > q",
            tasks(&s, vec![task(&s, false, vec![quote])]),
        ),
        (
            "- [ ]\n\n  later",
            tasks(&s, vec![task(&s, false, vec![p(&s, "later")])]),
        ),
    ] {
        let want = doc(&s, vec![want]);
        assert_eq!(doc_from_markdown_strict(&s, md).unwrap(), want, "{md:?}");
        assert_eq!(doc_from_markdown(&s, md).unwrap(), want, "{md:?}");
    }
    // Text continues its line's paragraph: not a marker.
    let d = doc_from_markdown_strict(&s, "- [x]\n  next").unwrap();
    assert_eq!(d, doc(&s, vec![bullets(&s, vec![vec![p(&s, "[x] next")]])]));
    // The bullet whose text is the brackets, and the same inside a task item.
    let d = doc(
        &s,
        vec![bullets(&s, vec![vec![p(&s, "[ ]")], vec![p(&s, "[x]")]])],
    );
    assert_eq!(rt(&s, &d), "- \\[ \\]\n- \\[x\\]");
    rt(
        &s,
        &doc(
            &s,
            vec![tasks(&s, vec![task(&s, true, vec![p(&s, "[ ]")])])],
        ),
    );
    // In a list with an item that has no marker it is text again, a
    // paragraph of its own.
    assert_eq!(refusal("- a\n- [ ]\n\n  later"), (Construct::TaskList, 2));
    let d = doc_from_markdown(&s, "- a\n- [ ]\n\n  later").unwrap();
    assert_eq!(
        d,
        doc(
            &s,
            vec![bullets(
                &s,
                vec![vec![p(&s, "a")], vec![p(&s, "[ ]"), p(&s, "later")]]
            )]
        )
    );
}

/// `---` under a marker's line is a setext heading to GitHub (its text the
/// marker): a rule that starts a task item is written `***`.
#[test]
fn a_task_item_that_starts_with_a_rule_writes_it_as_stars() {
    let s = s();
    let rule = || n(&s, "horizontal_rule", Attrs::new(), vec![]);
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![
                task(&s, false, vec![rule(), p(&s, "x")]),
                task(&s, true, vec![rule()]),
            ],
        )],
    );
    assert_eq!(rt(&s, &d), "- [ ] \n  ***\n\n  x\n- [x] \n  ***");
    // A rule anywhere else is still `---`.
    let d = doc(
        &s,
        vec![tasks(&s, vec![task(&s, false, vec![p(&s, "x"), rule()])])],
    );
    assert_eq!(rt(&s, &d), "- [ ] x\n  \n  ---");
}

#[test]
fn bracketed_text_in_a_bullet_item_stays_text() {
    let s = s();
    let d = doc(
        &s,
        vec![bullets(
            &s,
            vec![vec![p(&s, "[ ] not a task")], vec![p(&s, "[x] nor this")]],
        )],
    );
    rt(&s, &d);
    let d = doc(
        &s,
        vec![tasks(
            &s,
            vec![task(&s, false, vec![p(&s, "[x] text after a marker")])],
        )],
    );
    rt(&s, &d);
}
