//! How a pasted copy's last line ends (#3096): it depends on whether text
//! follows the caret (`PasteEnd`), and on the
//! `<br class="Apple-interchange-newline">` Chrome and WebKit end a copy
//! with.
//!
//! Chrome's answers, measured with `insertHTML` in a `contenteditable` (one
//! 20px line per line, no margins):
//! - `<p>alpha</p><br>` at the end of `x`: 2 lines; with four `<br>`s, 5.
//!   The same at the end of a paragraph with another after it, and on an
//!   empty line.
//! - The same into `x|y`: 3 lines (6 with four): the `<br>` breaks the line
//!   before `y`.
//! - `<p>alpha</p><p>beta</p>` plus an interchange newline into `x|y`:
//!   3 lines, `xalpha`, `beta`, `y`. At the end of `x` Chrome adds an empty
//!   line for it; rinch does not (an empty paragraph after every Docs paste).
use rinch_editor_core::serialize::{
    PasteEnd, node_to_html, slice_from_pasted_html, slice_from_pasted_html_ending,
};
use rinch_editor_core::{Schema, Slice};

const INTERCHANGE: &str = "<br class=\"Apple-interchange-newline\">";

fn slice(html: &str, end: PasteEnd) -> Slice {
    slice_from_pasted_html_ending(&Schema::starter_kit(), html, end).unwrap()
}

fn read(html: &str, end: PasteEnd) -> String {
    let schema = Schema::starter_kit();
    let doc = schema
        .branch("doc", slice(html, end).content.clone())
        .unwrap();
    node_to_html(&doc)
}

#[test]
fn where_nothing_follows_the_last_br_makes_no_line() {
    use PasteEnd::Ended;
    assert_eq!(read("<p>alpha</p><br>", Ended), "<p>alpha</p><p></p>");
    assert_eq!(
        read("<p>alpha</p><br><br><br><br>", Ended),
        "<p>alpha</p><p><br><br><br><br></p>"
    );
    // Inline content alone continues the caret's line and keeps its break:
    // rinch's own copy of `a` and a break inside one line (Chrome pastes
    // `a<br>` at the end of `x` as `xa` and an empty line, 2 lines).
    assert_eq!(read("a<br>", Ended), "<p>a<br><br></p>");
    // A paragraph's trailing break, as the writer writes it, stays.
    assert_eq!(read("<p>a<br><br></p>", Ended), "<p>a<br><br></p>");
}

#[test]
fn where_text_follows_the_last_br_is_kept() {
    use PasteEnd::Continued;
    assert_eq!(
        read("<p>alpha</p><br>", Continued),
        "<p>alpha</p><p><br><br></p>"
    );
    assert_eq!(
        read("<p>alpha</p><br><br><br><br>", Continued),
        "<p>alpha</p><p><br><br><br><br><br></p>"
    );
    // The read with no caret assumes text follows.
    let schema = Schema::starter_kit();
    assert_eq!(
        slice_from_pasted_html(&schema, "<p>alpha</p><br>").unwrap(),
        slice_from_pasted_html_ending(&schema, "<p>alpha</p><br>", Continued).unwrap()
    );
}

#[test]
fn an_interchange_newline_is_a_paragraph_end() {
    let html = format!("<p>alpha</p><p>beta</p>{INTERCHANGE}");
    for end in [PasteEnd::Continued, PasteEnd::Ended] {
        assert_eq!(read(&html, end), "<p>alpha</p><p>beta</p>", "{end:?}");
    }
    // Text after the caret keeps a paragraph of its own: closed at the end.
    assert_eq!(slice(&html, PasteEnd::Continued).open_end, 0);
    // Nothing after it: open, so no empty paragraph is left behind.
    assert_eq!(slice(&html, PasteEnd::Ended).open_end, 1);
    // A loose line before it is ended by it.
    assert_eq!(
        read(&format!("a<br>{INTERCHANGE}"), PasteEnd::Continued),
        "<p>a</p>"
    );
    assert_eq!(
        slice(&format!("a{INTERCHANGE}"), PasteEnd::Continued).open_end,
        0
    );
}

#[test]
fn an_interchange_newline_is_found_through_the_clipboard_wrapper() {
    let html = format!(
        "<html><body>\n<!--StartFragment--><p>alpha</p>{INTERCHANGE}<!--EndFragment-->\n</body></html>\n"
    );
    assert_eq!(read(&html, PasteEnd::Continued), "<p>alpha</p>");
    assert_eq!(slice(&html, PasteEnd::Continued).open_end, 0);
}

#[test]
fn an_interchange_newline_anywhere_else_is_a_br() {
    // Not the last node: a line like any other `<br>`.
    assert_eq!(
        read(
            &format!("<p>a</p>{INTERCHANGE}<p>b</p>"),
            PasteEnd::Continued
        ),
        "<p>a</p><p></p><p>b</p>"
    );
    // A plain `<br>` at the end is not a paragraph end.
    assert_eq!(slice("<p>a</p><br>", PasteEnd::Continued).open_end, 1);
}
