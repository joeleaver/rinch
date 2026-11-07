//! What a copy writes around text that has no text of its own: empty and
//! picture cells, `<br>` lines, pictures, empty paragraphs and `pre-line`
//! line breaks, against Chrome 156's `getSelection().toString()` (review of
//! #2240; the rules are `text_selection_copy.rs`'s).

use super::text_selection_chrome_tests::copy_a_to_b;

/// Copies of `#a` to `#b` in each of `cases` (name, markup, Chrome 156's
/// `toString()`), checked together so a failure names every case off.
fn copies_as_chrome(cases: &[(&str, &'static str, &str)]) {
    let off: Vec<String> = cases
        .iter()
        .filter_map(|&(name, html, chrome)| {
            let (_, t) = copy_a_to_b(html);
            (t != chrome).then(|| format!("{name}: {t:?}, Chrome {chrome:?}"))
        })
        .collect();
    assert!(off.is_empty(), "{off:#?}");
}

#[test]
fn a_cell_with_no_text_still_has_its_tab() {
    // Blink writes a tab before every cell but its row's first, whatever it
    // holds: white space, a picture or nothing (review of #2240).
    copies_as_chrome(&[
        (
            "white space",
            r#"<table><tr><td id="a">first</td><td> </td><td id="b">second</td></tr></table>"#,
            "first\t\tsecond",
        ),
        (
            "picture",
            r#"<table><tr><td id="a">first</td><td><img width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></td><td id="b">second</td></tr></table>"#,
            "first\t\tsecond",
        ),
        (
            "empty",
            r#"<table><tr><td id="a">first</td><td></td><td id="b">second</td></tr></table>"#,
            "first\t\tsecond",
        ),
        (
            "row of white space",
            r#"<table><tr><td id="a">first</td></tr><tr><td> </td></tr><tr><td id="b">second</td></tr></table>"#,
            "first\nsecond",
        ),
    ]);
}

#[test]
fn a_line_holding_only_a_br_is_a_blank_line() {
    // Gmail writes every blank line as `<div><br></div>` (review of #2240).
    copies_as_chrome(&[
        (
            "one",
            r#"<div id="a">a</div><div><br></div><div id="b">b</div>"#,
            "a\n\nb",
        ),
        (
            "two",
            r#"<div id="a">a</div><div><br></div><div><br></div><div id="b">b</div>"#,
            "a\n\n\nb",
        ),
        (
            "in a paragraph",
            r#"<div id="a">a</div><p><br></p><div id="b">b</div>"#,
            "a\n\n\nb",
        ),
    ]);
}

#[test]
fn pictures_copy_as_blink_writes_them() {
    // Blink writes an `<img>` or `<iframe>` as nothing that ends its block
    // with a line break; an `<svg>`, `<canvas>` or `<video>` it does not
    // write at all, nor a hidden picture (review of #2240).
    copies_as_chrome(&[
        (
            "two block pictures",
            r#"<div id="a">a</div><div><img style="display:block" width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="><img style="display:block" width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></div><div id="b">b</div>"#,
            "a\n\nb",
        ),
        (
            "a br between pictures",
            r#"<div id="a">a</div><div><img width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="><br><img width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></div><div id="b">b</div>"#,
            "a\n\n\nb",
        ),
        (
            "iframe",
            r#"<div id="a">a</div><div><iframe width="10" height="10"></iframe></div><div id="b">b</div>"#,
            "a\n\nb",
        ),
        (
            "svg",
            r#"<div id="a">a</div><div><svg width="10" height="10"></svg></div><div id="b">b</div>"#,
            "a\nb",
        ),
        (
            "canvas",
            r#"<div id="a">a</div><div><canvas width="10" height="10"></canvas></div><div id="b">b</div>"#,
            "a\nb",
        ),
        (
            "video",
            r#"<div id="a">a</div><div><video width="10" height="10"></video></div><div id="b">b</div>"#,
            "a\nb",
        ),
        (
            "hidden",
            r#"<div id="a">a</div><div><img style="visibility:hidden" width="10" height="10" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></div><div id="b">b</div>"#,
            "a\nb",
        ),
    ]);
}

#[test]
fn a_placeholders_alt_text_is_not_copied() {
    // #2309: an image with no picture draws its alt text, inline or in its
    // box, and Chrome 156 copies none of it (`a<img alt="Hello">b` is "ab").
    copies_as_chrome(&[
        (
            "inline alt",
            r#"<div id="a">a<img alt="Hello">b</div><div id="b">c</div>"#,
            "ab\nc",
        ),
        (
            "sized alt",
            r#"<div id="a">a<img width="100" height="50" alt="Hello">b</div><div id="b">c</div>"#,
            "ab\nc",
        ),
    ]);
}

#[test]
fn an_empty_paragraph_is_no_paragraph_left() {
    // Blink's extra line break is for a `<p>` with height (review of #2240).
    copies_as_chrome(&[
        (
            "passed",
            r#"<div id="a">first</div><p></p><div id="b">second</div>"#,
            "first\nsecond",
        ),
        (
            "in a block passed",
            r#"<div id="a">first</div><div><p></p></div><div id="b">second</div>"#,
            "first\nsecond",
        ),
    ]);
}

#[test]
fn pre_line_keeps_its_line_breaks() {
    // `pre-line` drops spaces but keeps line breaks (review of #2240).
    copies_as_chrome(&[(
        "pre-line",
        "<div id=\"a\">a</div><div style=\"white-space:pre-line\">\n\n</div><div id=\"b\">b</div>",
        "a\n\n\nb",
    )]);
}

#[test]
fn white_space_is_read_from_the_texts_own_element() {
    // A `white-space: pre` span in a root that collapses: its spaces are
    // drawn, so they are copied (round 2 of #2240's review; main had it).
    copies_as_chrome(&[(
        "pre span",
        r#"<div id="a">first</div><div> <span style="white-space:pre">   </span> </div><div id="b">second</div>"#,
        "first\n   \nsecond",
    )]);
}

#[test]
fn an_unselectable_cell_has_no_tab() {
    // Blink writes a cell's tab only when the cell can be selected (round 2
    // of #2240's review).
    copies_as_chrome(&[
        (
            "with text",
            r#"<table><tr><td id="a">first</td><td style="user-select:none">mid</td><td id="b">second</td></tr></table>"#,
            "first\tsecond",
        ),
        (
            "empty",
            r#"<table><tr><td id="a">first</td><td style="user-select:none"></td><td id="b">second</td></tr></table>"#,
            "first\tsecond",
        ),
        (
            "two",
            r#"<table><tr><td id="a">first</td><td style="user-select:none">m1</td><td style="user-select:none">m2</td><td id="b">second</td></tr></table>"#,
            "first\tsecond",
        ),
    ]);
}
