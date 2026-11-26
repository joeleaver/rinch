//! Google Docs pastes with the lines and formatting a browser shows (#3096).
//!
//! Docs writes an empty paragraph as a bare `<br />` between its `<p>`s (two
//! in a row for two), inside its `<b style="font-weight:normal"
//! id="docs-internal-guid-…">` wrapper, and the full character formatting of
//! every run as CSS on a `<span>`, its default black included. The samples
//! are synthetic, built after that shape.
//!
//! The expected line counts are Chrome's: the same markup in a
//! `contenteditable` (or rendered) shows one empty line per bare `<br>`
//! between blocks, because a block's last `<br>` makes no line.
use rinch_editor_core::serialize::{node_to_html, slice_from_html, slice_from_pasted_html};
use rinch_editor_core::{Node, Schema};

/// Docs' style for a run, with `extra` declarations in place of its defaults.
fn span(extra: &str, text: &str) -> String {
    let mut style = String::from(
        "font-size:11pt;font-family:Arial,sans-serif;color:#000000;background-color:transparent;\
         font-weight:400;font-style:normal;font-variant:normal;text-decoration:none;\
         vertical-align:baseline;white-space:pre;white-space:pre-wrap;",
    );
    for decl in extra.split(';').filter(|d| !d.is_empty()) {
        let property = decl.split(':').next().unwrap();
        style = style
            .split(';')
            .filter(|d| !d.is_empty() && d.split(':').next() != Some(property))
            .map(|d| format!("{d};"))
            .collect();
        style.push_str(decl);
        style.push(';');
    }
    format!("<span style=\"{style}\">{text}</span>")
}

fn para(margin: &str, runs: &str) -> String {
    format!(
        "<p dir=\"ltr\" style=\"line-height:1.38;margin-top:{margin};margin-bottom:{margin};\">{runs}</p>"
    )
}

fn p(text: &str) -> String {
    para("0pt", &span("", text))
}

fn docs(body: &str) -> String {
    format!(
        "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" \
         id=\"docs-internal-guid-0c1d2e3f-7fff-4a5b-6c7d-8e9f0a1b2c3d\">{body}</b>"
    )
}

/// Read as the default paste reads a clipboard; the document, checked to
/// read back the same.
fn read(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_pasted_html(&schema, html).unwrap();
    let doc = schema.branch("doc", slice.content.clone()).unwrap();
    let again = slice_from_html(&schema, &node_to_html(&doc)).unwrap();
    assert_eq!(
        schema.branch("doc", again.content.clone()).unwrap(),
        doc,
        "not a fixed point"
    );
    doc
}

fn html(doc: &Node) -> String {
    node_to_html(doc)
}

/// The lines the editor draws for `doc`: one per textblock, and one more
/// per hard break (a break at the end of a block included: the view's
/// trailing-break placeholder).
fn lines(node: &Node) -> usize {
    let own = usize::from(node.is_textblock() || node.type_name() == "hard_break");
    own + node.content().children().iter().map(lines).sum::<usize>()
}

#[test]
fn a_bare_br_between_paragraphs_is_one_empty_line() {
    let doc = read(&docs(&format!(
        "{}{}<br />{}<br /><br />{}{}",
        p("alpha"),
        p("bravo"),
        p("charlie"),
        p("delta"),
        para("12pt", &span("", "echo")),
    )));
    assert_eq!(
        html(&doc),
        "<p>alpha</p><p>bravo</p><p></p><p>charlie</p><p><br></p><p>delta</p><p>echo</p>"
    );
    // Chrome: alpha, bravo, 1 empty, charlie, 2 empty, delta, echo.
    assert_eq!(lines(&doc), 8);
}

/// A run of `<br>` the copy ends with leaves its last line open: the text
/// after the caret continues it, so every `<br>` there is a break, as Chrome
/// pastes `a<br>` into `x|y` as `xa<br>y`.
#[test]
fn trailing_brs_at_the_open_end_are_kept() {
    let doc = read(&docs(&format!("{}<br /><br /><br /><br />", p("alpha"))));
    assert_eq!(html(&doc), "<p>alpha</p><p><br><br><br><br></p>");
    assert_eq!(html(&read("a<br>")), "<p>a<br></p>");
}

/// The rule is a browser's for any line outside a textblock element: a
/// `<div>`'s last `<br>` makes no line; one inside a `<p>` is the hard break
/// rinch's own writer writes there, and stays.
#[test]
fn a_blocks_last_br_makes_no_line() {
    assert_eq!(
        html(&read("<div>a<br></div><div>b</div>")),
        "<p>a</p><p>b</p>"
    );
    assert_eq!(
        html(&read("<div><br></div><div>b</div>")),
        "<p></p><p>b</p>"
    );
    assert_eq!(
        html(&read("<div>a<br><br></div><div>b</div>")),
        "<p>a<br></p><p>b</p>"
    );
    assert_eq!(html(&read("<br><p>b</p>")), "<p></p><p>b</p>");
    assert_eq!(
        html(&read("<ul><li>a<br></li></ul>")),
        "<ul><li><p>a</p></li></ul>"
    );
    assert_eq!(html(&read("<p>a<br></p><p>b</p>")), "<p>a<br></p><p>b</p>");
}

/// Docs' black is its "automatic" colour: no colour mark. A colour the writer
/// chose is kept, and black from anywhere else is a colour.
#[test]
fn docs_default_black_is_no_colour() {
    let doc = read(&docs(&para(
        "0pt",
        &format!("{}{}", span("", "plain "), span("color:#0000ff", "blue")),
    )));
    assert_eq!(
        html(&doc),
        "<p>plain <span style=\"color:#0000ff\">blue</span></p>"
    );
    assert_eq!(
        html(&read("<p><span style=\"color:#000000\">ink</span></p>")),
        "<p><span style=\"color:#000000\">ink</span></p>"
    );
}

/// Everything the starter kit has a mark or node for arrives from Docs.
#[test]
fn docs_formatting_is_kept() {
    let runs = [
        span("font-weight:700", "bold"),
        span("font-style:italic", "italic"),
        span(
            "text-decoration:underline;-webkit-text-decoration-skip:none;text-decoration-skip-ink:none",
            "under",
        ),
        span(
            "text-decoration:line-through;-webkit-text-decoration-skip:none;text-decoration-skip-ink:none",
            "struck",
        ),
        span("color:#cc0000", "red"),
        span("background-color:#ffff00", "lit"),
        span("vertical-align:super;font-size:0.6em", "sup"),
        span("vertical-align:sub;font-size:0.6em", "sub"),
        format!(
            "<a href=\"https://example.com/\" style=\"text-decoration:none;\">{}</a>",
            span(
                "color:#1155cc;text-decoration:underline;-webkit-text-decoration-skip:none;text-decoration-skip-ink:none",
                "link"
            )
        ),
    ]
    .join(&span("", " "));
    let doc = read(&docs(&format!(
        "<h2 dir=\"ltr\" style=\"line-height:1.38;margin-top:18pt;margin-bottom:6pt;\">{}</h2>{}\
         <ol style=\"margin-top:0;margin-bottom:0;padding-inline-start:48px;\"><li dir=\"ltr\" \
         style=\"list-style-type:decimal;font-size:11pt;\" aria-level=\"1\"><p dir=\"ltr\" \
         role=\"presentation\">{}</p></li></ol>\
         <p dir=\"ltr\" style=\"line-height:1.38;text-align:center;margin-top:0pt;margin-bottom:0pt;\">{}</p>",
        span("font-size:16pt", "Heading"),
        para("0pt", &runs),
        span("", "item"),
        span("", "centred"),
    )));
    assert_eq!(
        html(&doc),
        "<h2>Heading</h2>\
         <p><strong>bold</strong> <em>italic</em> <u>under</u> <s>struck</s> \
         <span style=\"color:#cc0000\">red</span> <mark style=\"background-color:#ffff00\">lit</mark> \
         <sup>sup</sup> <sub>sub</sub> \
         <u><span style=\"color:#1155cc\"><a href=\"https://example.com/\">link</a></span></u></p>\
         <ol><li><p>item</p></li></ol>\
         <p style=\"text-align:center\">centred</p>"
    );
}
