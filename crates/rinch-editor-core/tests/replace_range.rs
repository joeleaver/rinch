//! `Transform::replace_range` — the fitted replace (#1382): the rules that
//! the paste fixtures in `rinch-editor-view` do not reach.

use rinch_editor_core::schema::{MarkSet, NodeSpec};
use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::transform::Transform;
use rinch_editor_core::{EditorState, Fragment, Node, Pos, Schema, Selection, Slice};
use std::rc::Rc;

fn doc(schema: &Schema, html: &str) -> Node {
    schema
        .branch("doc", slice_from_html(schema, html).unwrap().content)
        .unwrap()
}

fn fit(target: &str, from: usize, to: usize, html: &str) -> Result<(String, usize), String> {
    let schema = Schema::starter_kit();
    let mut tf = Transform::new(&schema, doc(&schema, target));
    let slice = slice_from_html(&schema, html).unwrap();
    let end = tf
        .replace_range(from, to, slice)
        .map_err(|e| e.to_string())?;
    assert!(tf.steps().len() == 1, "one step");
    Ok((node_to_html(&tf.doc), end))
}

/// What the HTML reader leaves open: every node down to the textblock at each
/// edge, and nothing where the edge reaches none.
#[test]
fn pasted_html_is_open_down_to_the_textblock_at_each_edge() {
    let schema = Schema::starter_kit();
    let open = |html: &str| {
        let s = slice_from_html(&schema, html).unwrap();
        (s.open_start, s.open_end)
    };
    assert_eq!(open("<p>a</p><p>b</p>"), (1, 1));
    assert_eq!(open("<ul><li>a</li><li>b</li></ul>"), (3, 3));
    assert_eq!(open("<p>a</p><ul><li>b</li></ul>"), (1, 3));
    assert_eq!(
        open("<blockquote><ul><li>a</li></ul></blockquote><hr>"),
        (4, 0)
    );
    assert_eq!(open("<hr><h1>a</h1>"), (0, 1));
    // A table cell is isolating: a table is never open.
    assert_eq!(
        open("<table><tr><td>a</td></tr></table>"),
        (0, 0),
        "a table is closed"
    );
    assert_eq!(open("<blockquote><hr></blockquote>"), (0, 0));
    assert_eq!(open("<ul><li><hr><p>a</p></li></ul>"), (0, 3));
}

/// The position returned is right after what was inserted.
#[test]
fn the_end_is_after_the_inserted_content() {
    // Inline into text.
    assert_eq!(
        fit("<p>abc</p>", 2, 2, "<p>XY</p>").unwrap(),
        ("<p>aXYbc</p>".into(), 4)
    );
    // A list in text: after `B`, before the text that moved into its item.
    assert_eq!(
        fit("<p>abc</p>", 2, 2, "<ul><li>A</li><li>B</li></ul>").unwrap(),
        ("<p>aA</p><ul><li><p>Bbc</p></li></ul>".into(), 8)
    );
    // A closed block that split the paragraph: the start of the second half.
    assert_eq!(
        fit("<p>abc</p>", 2, 2, "<hr>").unwrap(),
        ("<p>a</p><hr><p>bc</p>".into(), 5)
    );
    // A list that replaced an empty line: after the list.
    assert_eq!(
        fit("<p></p><p>z</p>", 1, 1, "<ul><li>A</li></ul>").unwrap(),
        ("<ul><li><p>A</p></li></ul><p>z</p>".into(), 7)
    );
}

/// The caret a paste leaves: in the pasted content when that ends in text,
/// after it when it ends in a block.
#[test]
fn replace_selection_puts_the_caret_after_the_paste() {
    let schema = Rc::new(Schema::starter_kit());
    let caret = |target: &str, at: usize, html: &str| {
        let state = EditorState::create(schema.clone(), doc(&schema, target), Vec::new());
        let mut tr = state.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        tr.replace_selection(slice_from_html(&schema, html).unwrap())
            .unwrap();
        let next = state.apply(tr);
        assert!(next.selection.is_empty());
        (node_to_html(&next.doc), next.selection.from().0)
    };
    // Ends in text: the caret stays behind `A`, not before `z`.
    assert_eq!(
        caret("<p></p><p>z</p>", 1, "<ul><li>A</li></ul>"),
        ("<ul><li><p>A</p></li></ul><p>z</p>".into(), 4)
    );
    // Ends in a rule: the caret goes on to the next textblock, not back.
    assert_eq!(
        caret("<p>x</p><p></p><p>z</p>", 4, "<hr>"),
        ("<p>x</p><hr><p>z</p>".into(), 5)
    );
}

/// Marks the target does not allow are dropped, not a reason to refuse.
#[test]
fn marks_the_target_refuses_are_dropped() {
    assert_eq!(
        fit("<pre>ab</pre>", 2, 2, "<p><strong>X</strong></p>")
            .unwrap()
            .0,
        "<pre>aXb</pre>"
    );
}

/// A range whose ends are in two table cells is not fitted.
#[test]
fn a_range_across_table_cells_is_not_fitted() {
    let table = "<table><tr><td><p>ab</p></td><td><p>cd</p></td></tr></table>";
    assert!(fit(table, 5, 11, "<hr>").is_err());
    // Inside one cell it is.
    assert_eq!(
        fit(table, 5, 5, "<hr>").unwrap().0,
        "<table><tr><td><p>a</p><hr><p>b</p></td><td><p>cd</p></td></tr></table>"
    );
}

/// A slice that holds an invalid node is refused whole.
#[test]
fn an_unsound_slice_is_refused() {
    let schema = Schema::starter_kit();
    let source = doc(&schema, "<ul><li><p>a</p></li></ul><p>b</p>");
    // Cut from the end of the item's content: the item is open and empty.
    let cut = source.slice(5, 9).unwrap();
    assert_eq!((cut.open_start, cut.open_end), (2, 1));
    let closed = Slice::new(cut.content.clone(), 0, 1);
    let mut tf = Transform::new(&schema, doc(&schema, "<p>xy</p>"));
    assert!(tf.replace_range(2, 2, closed).is_err());
    assert!(tf.steps().is_empty());
    // With its own open depths the empty item is dropped and the rest fits.
    let end = tf.replace_range(2, 2, cut).unwrap();
    assert_eq!(node_to_html(&tf.doc), "<p>x</p><p>by</p>");
    assert_eq!(end, 5);
}

/// A schema with an isolating box that takes paragraphs only, and a figure
/// whose content is a heading then a paragraph.
fn boxed() -> Schema {
    let mut rule = NodeSpec::atom("rule");
    rule.group = Some("block".into());
    Schema::builder()
        .node("doc", NodeSpec::builder("doc").content("block+").build())
        .node("paragraph", NodeSpec::block("paragraph"))
        .node(
            "title",
            NodeSpec::builder("title")
                .content("inline*")
                .marks(MarkSet::None)
                .build(),
        )
        .node("rule", rule)
        .node(
            "box",
            NodeSpec::builder("box")
                .content("paragraph+")
                .group("block")
                .isolating(true)
                .build(),
        )
        .node(
            "figure",
            NodeSpec::builder("figure")
                .content("title paragraph")
                .group("block")
                .build(),
        )
        .node(
            "text",
            NodeSpec::builder("text").group("inline").inline().build(),
        )
        .top_node("doc")
        .build()
}

/// The fit never leaves the isolating node the range starts in: a block the
/// box does not take is not put beside the box by splitting it.
#[test]
fn a_fit_does_not_leave_an_isolating_node() {
    let s = boxed();
    let p = |t: &str| {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    };
    let boxed = s
        .branch("box", Fragment::from_children(vec![p("ab"), p("cd")]))
        .unwrap();
    let before = s.branch("doc", Fragment::from_node(boxed)).unwrap();
    let rule = s.branch("rule", Fragment::empty()).unwrap();

    let mut tf = Transform::new(&s, before.clone());
    assert!(
        tf.replace_range(
            3,
            3,
            Slice::from_fragment(Fragment::from_node(rule.clone()))
        )
        .is_err(),
        "the box takes no rule and is not split for one: {:?}",
        tf.doc
    );
    assert_eq!(tf.doc, before);

    // A rule and then a paragraph: the rule has no place and is dropped, the
    // paragraph's text goes in.
    let slice = Slice::new(Fragment::from_children(vec![rule, p("X")]), 0, 1);
    let mut tf = Transform::new(&s, before);
    tf.replace_range(3, 3, slice).unwrap();
    let text = |n: &Node| -> String {
        n.content()
            .children()
            .iter()
            .filter_map(Node::text)
            .collect()
    };
    let inner = tf.doc.child(0);
    assert_eq!(inner.type_name(), "box");
    let texts: Vec<String> = inner.content().children().iter().map(text).collect();
    assert_eq!(texts, ["a", "Xb", "cd"]);
}

/// Nodes are never created to make content valid: a fit that would close a
/// node short of its required content fails, and nothing invalid is written.
#[test]
fn a_node_is_not_closed_short_of_its_content() {
    let s = boxed();
    let text = |t: &str| Fragment::from_node(s.text(t).unwrap());
    let figure = s
        .branch(
            "figure",
            Fragment::from_children(vec![
                s.branch("title", text("ti")).unwrap(),
                s.branch("paragraph", text("pa")).unwrap(),
            ]),
        )
        .unwrap();
    let before = s.branch("doc", Fragment::from_node(figure)).unwrap();
    // A rule at a caret in the title would close the figure after its title.
    let rule = Slice::from_fragment(Fragment::from_node(
        s.branch("rule", Fragment::empty()).unwrap(),
    ));
    let mut tf = Transform::new(&s, before.clone());
    assert!(tf.replace_range(3, 3, rule).is_err());
    assert_eq!(tf.doc, before);
    // Text goes in.
    let mut tf = Transform::new(&s, before);
    tf.replace_range(3, 3, Slice::from_fragment(text("X")))
        .unwrap();
    assert_eq!(tf.doc.child(0).child(0).content(), &text("tXi"));
}

/// Content is not pulled out of a slice node past a place that takes the node
/// itself: an item of a list that is closed around it stays in that list,
/// which goes in the item the caret is in, rather than becoming a sibling of
/// that item.
#[test]
fn a_closed_node_is_not_opened_past_where_it_fits() {
    let schema = Schema::starter_kit();
    let list = slice_from_html(&schema, "<ul><li>A</li></ul>").unwrap();
    // The list open, its item closed.
    let slice = Slice::new(list.content, 1, 0);
    let mut tf = Transform::new(&schema, doc(&schema, "<ol><li><p>xy</p></li></ol>"));
    tf.replace_range(4, 4, slice).unwrap();
    assert_eq!(
        node_to_html(&tf.doc),
        "<ol><li><p>x</p><ul><li><p>A</p></li></ul><p>y</p></li></ol>"
    );
}

/// A node that has no place where the range is, is opened and its content
/// placed: a figure pasted in a box that takes paragraphs only gives the box
/// its title's text and its paragraph.
#[test]
fn a_node_with_no_place_gives_its_content() {
    let s = boxed();
    let text = |t: &str| Fragment::from_node(s.text(t).unwrap());
    let p = |t: &str| s.branch("paragraph", text(t)).unwrap();
    let figure = s
        .branch(
            "figure",
            Fragment::from_children(vec![s.branch("title", text("ti")).unwrap(), p("pa")]),
        )
        .unwrap();
    let boxed = s.branch("box", Fragment::from_node(p("ab"))).unwrap();
    let mut tf = Transform::new(&s, s.branch("doc", Fragment::from_node(boxed)).unwrap());
    tf.replace_range(3, 3, Slice::from_fragment(Fragment::from_node(figure)))
        .unwrap();
    let inner = tf.doc.child(0);
    assert_eq!(inner.type_name(), "box");
    let texts: Vec<String> = inner
        .content()
        .children()
        .iter()
        .map(|n| {
            assert_eq!(n.type_name(), "paragraph");
            n.content()
                .children()
                .iter()
                .filter_map(Node::text)
                .collect()
        })
        .collect();
    assert_eq!(texts, ["ati", "pab"]);
}

/// A node that was open at its start is not placed whole when it is not valid
/// closed there: a figure cut after its title is no figure.
#[test]
fn a_node_cut_at_its_start_is_not_placed_whole_when_that_is_invalid() {
    let s = Schema::builder()
        .node("doc", NodeSpec::builder("doc").content("figure+").build())
        .node(
            "paragraph",
            NodeSpec::builder("paragraph").content("text*").build(),
        )
        .node("title", NodeSpec::builder("title").content("text*").build())
        .node(
            "figure",
            NodeSpec::builder("figure")
                .content("title paragraph")
                .build(),
        )
        .node(
            "text",
            NodeSpec::builder("text").group("inline").inline().build(),
        )
        .top_node("doc")
        .build();
    let text = |t: &str| Fragment::from_node(s.text(t).unwrap());
    let figure = |t: &str, p: &str| {
        s.branch(
            "figure",
            Fragment::from_children(vec![
                s.branch("title", text(t)).unwrap(),
                s.branch("paragraph", text(p)).unwrap(),
            ]),
        )
        .unwrap()
    };
    let source = s
        .branch("doc", Fragment::from_children(vec![figure("t1", "p1")]))
        .unwrap();
    // From between the title and the paragraph to the end: the figure is
    // open at its start and holds a paragraph only.
    let cut = source.slice(5, 10).unwrap();
    assert_eq!((cut.open_start, cut.open_end), (1, 0));
    let before = s
        .branch("doc", Fragment::from_children(vec![figure("t2", "p2")]))
        .unwrap();
    let mut tf = Transform::new(&s, before.clone());
    // Between blocks at the top: the doc takes figures only.
    let result = tf.replace_range(0, 0, cut);
    assert!(result.is_err(), "{:?}", tf.doc);
    assert_eq!(tf.doc, before);
}
