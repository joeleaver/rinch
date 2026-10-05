//! The HTML reader through an editor (#1397, #1392): what `load_html` and a
//! paste make of Word's markup, of table parts with no table, and of markup
//! nested without limit.
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Node, PasteContent, Pos, Selection};
use rinch_editor_view::create_editor;

/// Every node's content satisfies its type's expression.
fn assert_valid(node: &Node) {
    let names: Vec<&str> = node.content().children().iter().map(Node::type_name).collect();
    assert!(
        node.node_type().content_match().matches(&names),
        "<{}> holds {names:?}",
        node.type_name()
    );
    for child in node.content().children() {
        assert_valid(child);
    }
}

fn text_of(node: &Node, out: &mut String) {
    if let Some(t) = node.text() {
        out.push_str(t);
    }
    for child in node.content().children() {
        text_of(child, out);
    }
}

/// #1392: `load_html` loaded a document holding a bare cell or row.
#[test]
fn load_html_reads_bare_table_parts_as_a_table() {
    let e = create_editor();
    assert!(e.load_html("<td>a</td><td>b</td>"));
    assert_valid(&e.doc());
    assert_eq!(
        node_to_html(&e.doc()),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>"
    );
    assert!(e.load_html("<p>x</p><tr><td>a</td></tr>"));
    assert_valid(&e.doc());
    assert_eq!(
        node_to_html(&e.doc()),
        "<p>x</p><table><tr><td><p>a</p></td></tr></table>"
    );
    assert!(e.load_html("<blockquote><td>a</td></blockquote>"));
    assert_valid(&e.doc());
    // And a later edit of such a document goes through.
    e.set_selection(Selection::text(Pos(5), Pos(5)));
    assert!(e.insert_text("!"));
    assert_valid(&e.doc());
}

/// #1392: a paste of bare cells was refused and fell back to `text/plain`.
#[test]
fn a_paste_of_bare_cells_lands_as_a_table() {
    let e = create_editor();
    e.load_html("<p>ab</p>");
    e.set_selection(Selection::text(Pos(2), Pos(2)));
    assert!(e.paste(&PasteContent {
        html: Some("<tr><td>x</td><td>y</td></tr>".to_string()),
        text: Some("x\ty".to_string()),
    }));
    assert_eq!(
        node_to_html(&e.doc()),
        "<p>a</p><table><tr><td><p>x</p></td><td><p>y</p></td></tr></table><p>b</p>"
    );
}

/// #1397: a paste from Word kept its first paragraph and lost the rest.
#[test]
fn a_paste_from_word_keeps_every_paragraph() {
    let e = create_editor();
    e.load_html("<p></p>");
    e.set_selection(Selection::text(Pos(1), Pos(1)));
    let html = "<html xmlns:o=\"urn:schemas-microsoft-com:office:office\"><body>\
        <!--StartFragment--><p class=MsoNormal>First<o:p></o:p></p>\
        <p class=MsoNormal><o:p>&nbsp;</o:p></p>\
        <p class=MsoNormal>Third<o:p></o:p></p><!--EndFragment--></body></html>";
    assert!(e.paste(&PasteContent {
        html: Some(html.to_string()),
        text: Some("First\n\nThird".to_string()),
    }));
    assert_eq!(
        node_to_html(&e.doc()),
        "<p>First</p><p>\u{a0}</p><p>Third</p>"
    );
}

/// Markup nested 10,000 deep loads and pastes: the reader opens at most 128
/// elements at once, so neither it nor anything after it runs out of stack.
#[test]
fn deeply_nested_markup_loads_and_pastes() {
    for tag in ["div", "blockquote", "span", "ul><li", "table><tr><td", "strong><div"] {
        let html = format!("{}deep<p>after</p>", format!("<{tag}>").repeat(10_000));
        let e = create_editor();
        assert!(e.load_html(&html), "{tag}");
        assert_valid(&e.doc());
        let mut text = String::new();
        text_of(&e.doc(), &mut text);
        assert_eq!(text, "deepafter", "{tag}");

        let e = create_editor();
        e.load_html("<p>ab</p>");
        e.set_selection(Selection::text(Pos(2), Pos(2)));
        assert!(
            e.paste(&PasteContent {
                html: Some(html),
                text: Some("deep".to_string()),
            }),
            "{tag}"
        );
        assert_valid(&e.doc());
        let mut text = String::new();
        text_of(&e.doc(), &mut text);
        assert_eq!(text, "adeepafterb", "{tag}");
        assert!(e.command("undo"), "{tag}");
        assert_eq!(node_to_html(&e.doc()), "<p>ab</p>", "{tag}");
    }
}
