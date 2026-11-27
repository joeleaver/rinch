//! Whitespace after a block's last `<br>` (#3096 review): pretty-printed
//! HTML reads as the same markup written without it.
//!
//! Chrome's line counts, measured by rendering each shape with no margins:
//! `<p>a<br>\n</p>`, `<p>a<br> </p>`, `<p>\n  a<br>\n</p>`,
//! `<p>a<br><!-- c -->\n</p>`, `<p><span>a<br>\n  </span>\n</p>`,
//! `<div><span>a<br>\n</span></div>`, `<p>a<br><span></span>\n</p>`,
//! `<ul>\n  <li>a<br>\n  </li>\n</ul>` and
//! `<p style="white-space:pre-line">a<br>  </p>` are each 1 line, as
//! `<p>a<br></p>` is. `<div><span>a<br>\n</span><p>b</p></div>` is 2.
//! Whitespace that does not collapse is a line: `<p>a<br>&nbsp;</p>`,
//! `<p style="white-space:pre">a<br>\n</p>`,
//! `<p style="white-space:pre-wrap">a<br> </p>`,
//! `<p style="white-space:pre-line">a<br>\n</p>` and
//! `<p><span style="white-space:pre-wrap">a<br>\n</span></p>` are 2 lines.
//! Pasted with `insertHTML` at the end of `x`, `<p>alpha<br>\n</p>` gives
//! what `<p>alpha<br></p>` gives, and `<p>alpha<br>\n</p>\n<p>beta<br>\n</p>`
//! what `<p>alpha<br></p><p>beta<br></p>` gives.
use rinch_editor_core::model::Fragment;
use rinch_editor_core::serialize::{
    PasteEnd, node_to_html, slice_from_html, slice_from_pasted_html_ending,
};
use rinch_editor_core::{Node, Schema, Slice};

fn doc_of(slice: Slice) -> Node {
    Schema::starter_kit()
        .branch("doc", slice.content.clone())
        .unwrap()
}

fn load(html: &str) -> Node {
    doc_of(slice_from_html(&Schema::starter_kit(), html).unwrap())
}

fn paste(html: &str, end: PasteEnd) -> Node {
    doc_of(slice_from_pasted_html_ending(&Schema::starter_kit(), html, end).unwrap())
}

/// The lines the editor draws: one per textblock and one per hard break (a
/// break at the end of a block too: the view's trailing-break placeholder).
fn lines(node: &Node) -> usize {
    let own = usize::from(node.is_textblock() || node.type_name() == "hard_break");
    own + node.content().children().iter().map(lines).sum::<usize>()
}

/// Read `html` every way the reader is entered and check it gives
/// `expected` and Chrome's `n` lines each time.
fn check(html: &str, expected: &str, n: usize) {
    for (how, doc) in [
        ("loaded", load(html)),
        ("pasted, nothing after", paste(html, PasteEnd::Ended)),
        ("pasted, text after", paste(html, PasteEnd::Continued)),
    ] {
        assert_eq!(node_to_html(&doc), expected, "{html:?} {how}");
        assert_eq!(lines(&doc), n, "{html:?} {how}: Chrome's lines");
    }
}

#[test]
fn collapsible_whitespace_after_the_last_br_is_not_a_line() {
    for html in [
        "<p>a<br>\n</p>",
        "<p>a<br> </p>",
        "<p>a<br>\t \n</p>",
        "<p>a<br><!-- c -->\n</p>",
        "<p>a<br>\n<!-- c -->\n</p>",
        "<p>a<br><span></span>\n</p>",
        "<p>a<br><style>p {}</style>\n</p>",
        "<p>a<br>\n<span>\n</span>\n</p>",
        "<p style=\"white-space:pre-line\">a<br>  </p>",
        "<p style=\"white-space:nowrap\">a<br>\n</p>",
    ] {
        check(html, "<p>a</p>", 1);
    }
}

#[test]
fn pretty_printed_blocks_read_as_written_without_the_whitespace() {
    check("<p>\n  a<br>\n</p>", "<p>\n  a</p>", 1);
    check("<p><span>a<br>\n  </span>\n</p>", "<p>a</p>", 1);
    check(
        "<p><strong>a<br>\n</strong>\n</p>",
        "<p><strong>a</strong></p>",
        1,
    );
    check("<div>a<br>\n</div>", "<p>a</p>", 1);
    check("<div><span>a<br>\n</span></div>", "<p>a</p>", 1);
    check(
        "<div><span>a<br>\n</span><p>b</p></div>",
        "<p>a</p><p>b</p>",
        2,
    );
    check(
        "<ul>\n  <li>a<br>\n  </li>\n</ul>",
        "<ul><li><p>a</p></li></ul>",
        1,
    );
    check(
        "<table><tr><td>a<br>\n</td></tr></table>",
        "<table><tr><td><p>a</p></td></tr></table>",
        1,
    );
}

#[test]
fn a_hard_break_before_the_placeholder_survives_the_whitespace() {
    check("<p>a<br><br>\n</p>", "<p>a<br><br></p>", 2);
    // The `\n` after the first break is kept (the reader does not collapse
    // whitespace inside a line, #1406), so the writer writes the placeholder
    // after it.
    check("<p>a<br>\n<br>\n</p>", "<p>a<br>\n<br></p>", 2);
}

/// A paragraph whose last hard break is followed by whitespace text is
/// written with the placeholder after that text, so it reads back whole.
#[test]
fn the_writer_writes_whitespace_after_a_break_so_it_reads_back() {
    let schema = Schema::starter_kit();
    for tail in [" ", "\n", " \t "] {
        let para = schema
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    schema.text("a").unwrap(),
                    schema.branch("hard_break", Fragment::empty()).unwrap(),
                    schema.text(tail).unwrap(),
                ]),
            )
            .unwrap();
        let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
        let html = node_to_html(&doc);
        assert_eq!(html, format!("<p>a<br>{tail}<br></p>"), "{tail:?}");
        assert_eq!(node_to_html(&load(&html)), html, "{tail:?} reads back");
        assert_eq!(
            node_to_html(&paste(&html, PasteEnd::Ended)),
            html,
            "{tail:?} pastes back"
        );
        assert_eq!(lines(&load(&html)), 2, "{tail:?}");
    }
}

#[test]
fn whitespace_that_does_not_collapse_is_a_line() {
    for (html, expected) in [
        ("<p>a<br>&nbsp;</p>", "<p>a<br>\u{a0}</p>"),
        ("<p>a<br>\n&nbsp;\n</p>", "<p>a<br>\n\u{a0}\n</p>"),
        (
            "<p style=\"white-space:pre\">a<br>\n</p>",
            "<p>a<br>\n<br></p>",
        ),
        (
            "<p style=\"white-space:pre-wrap\">a<br> </p>",
            "<p>a<br> <br></p>",
        ),
        (
            "<p style=\"white-space:break-spaces\">a<br> </p>",
            "<p>a<br> <br></p>",
        ),
        (
            "<p style=\"white-space:pre-line\">a<br>\n</p>",
            "<p>a<br>\n<br></p>",
        ),
        (
            "<p><span style=\"white-space:pre-wrap\">a<br>\n</span></p>",
            "<p>a<br>\n<br></p>",
        ),
        (
            "<p>a<br><span style=\"white-space:pre\"> </span></p>",
            "<p>a<br> <br></p>",
        ),
    ] {
        // The whitespace is kept, so the writer writes the placeholder
        // after it.
        let doc = load(html);
        assert_eq!(node_to_html(&doc), expected, "{html:?}");
        assert_eq!(lines(&doc), 2, "{html:?}: Chrome draws two lines");
    }
}

/// A pretty-printed paste gives what the same markup without the
/// whitespace gives, wherever the caret is.
#[test]
fn a_pretty_printed_paste_pastes_as_the_compact_one() {
    for (pretty, compact) in [
        ("<p>alpha<br>\n</p>", "<p>alpha<br></p>"),
        (
            "<p>alpha<br>\n</p>\n<p>beta<br>\n</p>\n",
            "<p>alpha<br></p><p>beta<br></p>",
        ),
        (
            "<div>\n  <p>alpha<br>\n  </p>\n  <p>beta<br>\n  </p>\n</div>\n",
            "<div><p>alpha<br></p><p>beta<br></p></div>",
        ),
        ("<p>alpha</p>\n<br>\n", "<p>alpha</p><br>"),
    ] {
        for end in [PasteEnd::Ended, PasteEnd::Continued] {
            assert_eq!(
                node_to_html(&paste(pretty, end)),
                node_to_html(&paste(compact, end)),
                "{pretty:?} {end:?}"
            );
        }
        assert_eq!(
            node_to_html(&load(pretty)),
            node_to_html(&load(compact)),
            "{pretty:?} loaded"
        );
    }
}
