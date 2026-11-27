//! The default paste of a Google Docs copy (#3096): one empty line per
//! empty Docs paragraph, not two, and no black colour on every word.
//!
//! Docs writes an empty paragraph as a bare `<br />` between its `<p>`s and
//! `color:#000000` on every run. The clipboard here is synthetic, built after
//! that shape; `text/plain` carries the same lines, as Docs puts it there.
use rinch_editor_core::serialize::node_to_html;
use rinch_editor_core::{Node, PasteContent, Pos, Selection};
use rinch_editor_view::{EditorHandle, create_editor};

const RUN: &str = "font-size:11pt;font-family:Arial,sans-serif;color:#000000;\
                   background-color:transparent;font-weight:400;font-style:normal;\
                   font-variant:normal;text-decoration:none;vertical-align:baseline;\
                   white-space:pre;white-space:pre-wrap;";

fn p(text: &str) -> String {
    format!(
        "<p dir=\"ltr\" style=\"line-height:1.38;margin-top:0pt;margin-bottom:0pt;\">\
         <span style=\"{RUN}\">{text}</span></p>"
    )
}

/// Three paragraphs, one empty paragraph after the first and two after the
/// second, as Docs copies them.
fn clipboard() -> PasteContent {
    let html = format!(
        "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" \
         id=\"docs-internal-guid-5e6f7a8b-7fff-1c2d-3e4f-5a6b7c8d9e0f\">{}<br />{}<br /><br />{}</b>",
        p("first"),
        p("second"),
        p("third"),
    );
    PasteContent {
        text: Some("first\n\nsecond\n\n\nthird".into()),
        html: Some(html),
    }
}

fn paste_into(target: &str, at: usize) -> EditorHandle {
    let editor = create_editor();
    editor.load_html(target);
    editor.set_selection(Selection::text(Pos(at), Pos(at)));
    assert!(editor.paste(&clipboard()), "the paste was refused");
    editor
}

/// The lines the editor draws: one per textblock and one per hard break (a
/// break at the end of a block too: the view's trailing-break placeholder).
fn lines(node: &Node) -> usize {
    let own = usize::from(node.is_textblock() || node.type_name() == "hard_break");
    own + node.content().children().iter().map(lines).sum::<usize>()
}

#[test]
fn a_docs_paste_on_an_empty_line_has_the_lines_docs_shows() {
    let editor = paste_into("<p></p>", 1);
    let doc = editor.doc();
    assert_eq!(
        node_to_html(&doc),
        "<p>first</p><p></p><p>second</p><p><br><br></p><p>third</p>"
    );
    // first, 1 empty, second, 2 empty, third: what Chrome shows for the
    // markup, and what the plain text says.
    assert_eq!(lines(&doc), 6);
}

#[test]
fn a_docs_paste_inside_a_line_keeps_the_text_around_it() {
    let editor = paste_into("<p>ab</p>", 2);
    let doc = editor.doc();
    assert_eq!(
        node_to_html(&doc),
        "<p>afirst</p><p></p><p>second</p><p><br><br></p><p>thirdb</p>"
    );
    assert_eq!(lines(&doc), 6);
}

/// The paste of the same lines as plain text agrees with the HTML one.
#[test]
fn the_plain_text_of_a_docs_copy_has_the_same_lines() {
    let editor = create_editor();
    editor.load_html("<p></p>");
    editor.set_selection(Selection::text(Pos(1), Pos(1)));
    assert!(editor.paste(&PasteContent::text(clipboard().text.unwrap())));
    assert_eq!(lines(&editor.doc()), 6);
}

/// Paste `html` (with no plain text) at `at` in `target`.
fn paste_html(target: &str, at: usize, html: &str) -> Node {
    let editor = create_editor();
    editor.load_html(target);
    editor.set_selection(Selection::text(Pos(at), Pos(at)));
    let paste = PasteContent {
        text: None,
        html: Some(html.into()),
    };
    assert!(editor.paste(&paste), "the paste was refused");
    editor.doc()
}

/// Loose `<br>`s that end a copy, where nothing follows the caret: the last
/// makes no line. Chrome (`insertHTML`): `<p>alpha</p><br>` at the end of
/// `x` is 2 lines and with four `<br>`s 5; the same on an empty line, and at
/// the end of a paragraph with another after it (plus that one).
#[test]
fn trailing_brs_where_nothing_follows_the_caret_have_chromes_lines() {
    let one = "<p>alpha</p><br>";
    let four = "<p>alpha</p><br><br><br><br>";
    // The end of the document.
    assert_eq!(lines(&paste_html("<p>x</p>", 2, one)), 2);
    assert_eq!(lines(&paste_html("<p>x</p>", 2, four)), 5);
    // An empty line.
    assert_eq!(lines(&paste_html("<p></p>", 1, one)), 2);
    assert_eq!(lines(&paste_html("<p></p>", 1, four)), 5);
    // The end of a paragraph, another after it.
    let doc = paste_html("<p>x</p><p>z</p>", 2, one);
    assert_eq!(node_to_html(&doc), "<p>xalpha</p><p></p><p>z</p>");
    assert_eq!(lines(&doc), 3);
    assert_eq!(lines(&paste_html("<p>x</p><p>z</p>", 2, four)), 6);
}

/// Where text follows the caret, the `<br>` breaks the line before it, as
/// in Chrome: `<p>alpha</p><br>` into `x|y` is 3 lines, four `<br>`s 6.
#[test]
fn trailing_brs_where_text_follows_the_caret_break_before_it() {
    let doc = paste_html("<p>xy</p>", 2, "<p>alpha</p><br>");
    assert_eq!(node_to_html(&doc), "<p>xalpha</p><p><br>y</p>");
    assert_eq!(lines(&doc), 3);
    assert_eq!(
        lines(&paste_html("<p>xy</p>", 2, "<p>alpha</p><br><br><br><br>")),
        6
    );
}

/// A copy that ends in `<br class="Apple-interchange-newline">`, as Chrome's
/// and Docs' do, ends in a paragraph end: into `x|y` Chrome shows `xalpha`,
/// `beta`, `y`. Where nothing follows, it leaves no empty paragraph behind.
#[test]
fn an_interchange_newline_ends_the_paragraph() {
    let html = "<meta charset=\"utf-8\"><p>alpha</p><p>beta</p>\
                <br class=\"Apple-interchange-newline\">";
    let doc = paste_html("<p>xy</p>", 2, html);
    assert_eq!(node_to_html(&doc), "<p>xalpha</p><p>beta</p><p>y</p>");
    assert_eq!(lines(&doc), 3);
    let doc = paste_html("<p>x</p>", 2, html);
    assert_eq!(node_to_html(&doc), "<p>xalpha</p><p>beta</p>");
    let doc = paste_html("<p></p>", 1, html);
    assert_eq!(node_to_html(&doc), "<p>alpha</p><p>beta</p>");
}

/// The Docs clipboard with its interchange newline, into `x|y`: the `y`
/// follows `third` on a line of its own, with no empty line between.
#[test]
fn a_docs_copy_ending_in_an_interchange_newline_into_a_line() {
    let mut paste = clipboard();
    if let Some(html) = paste.html.as_mut() {
        html.push_str("<br class=\"Apple-interchange-newline\">");
    }
    let editor = create_editor();
    editor.load_html("<p>xy</p>");
    editor.set_selection(Selection::text(Pos(2), Pos(2)));
    assert!(editor.paste(&paste));
    assert_eq!(
        node_to_html(&editor.doc()),
        "<p>xfirst</p><p></p><p>second</p><p><br><br></p><p>third</p><p>y</p>"
    );
}

/// Pretty-printed markup pastes, and loads, as the same markup without the
/// whitespace (#3096 review): whitespace after a block's last `<br>`
/// collapses, so that `<br>` is still the placeholder. Chrome (`insertHTML`
/// at the end of `x`) gives `<p>alpha<br>\n</p>` the lines of
/// `<p>alpha<br></p>`, and `<p>alpha<br>\n</p>\n<p>beta<br>\n</p>` those of
/// `<p>alpha<br></p><p>beta<br></p>`.
#[test]
fn pretty_printed_markup_pastes_as_the_compact_markup() {
    for (pretty, compact) in [
        ("<p>alpha<br>\n</p>", "<p>alpha<br></p>"),
        (
            "<p>alpha<br>\n</p>\n<p>beta<br>\n</p>\n",
            "<p>alpha<br></p><p>beta<br></p>",
        ),
        (
            "<div>\n  <p>\n    alpha<br>\n  </p>\n  <ul>\n    <li>beta<br>\n    </li>\n  </ul>\n</div>\n",
            "<div><p>\n    alpha<br></p><ul><li>beta<br></li></ul></div>",
        ),
    ] {
        for (target, at) in [("<p>x</p>", 2), ("<p></p>", 1), ("<p>xy</p>", 2)] {
            assert_eq!(
                node_to_html(&paste_html(target, at, pretty)),
                node_to_html(&paste_html(target, at, compact)),
                "{pretty:?} into {target} at {at}"
            );
        }
        let (a, b) = (create_editor(), create_editor());
        a.load_html(pretty);
        b.load_html(compact);
        assert_eq!(
            node_to_html(&a.doc()),
            node_to_html(&b.doc()),
            "{pretty:?} loaded"
        );
    }
    let doc = paste_html("<p></p>", 1, "<p>alpha<br>\n</p>\n<p>beta<br>\n</p>\n");
    assert_eq!(node_to_html(&doc), "<p>alpha</p><p>beta</p>");
    assert_eq!(lines(&doc), 2);
}
