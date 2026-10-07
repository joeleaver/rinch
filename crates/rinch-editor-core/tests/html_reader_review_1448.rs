//! Fixtures from the review of PR #1448 (#1413, #1410, #1415).
//!
//! Each pins something the PR's own tests leave free: two guards whose
//! removal no test noticed, the line rules at the depth limit, a stray
//! `</p>` in every kind of container, and that a reference decoded in a URL
//! is decoded before the URL is judged. One `#[ignore]`d test states Chrome
//! 153's reading where the reader differs (#1462).

use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{html_reader_max_depth, node_to_html, slice_from_html};

fn html(src: &str) -> String {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, src).expect("reads");
    slice.content.children().iter().map(node_to_html).collect()
}

/// The text of each paragraph, `|` after each: tags dropped.
fn lines(src: &str) -> String {
    let out = html(src);
    let mut lines = String::new();
    let mut rest = out.as_str();
    while let Some(at) = rest.find('<') {
        lines.push_str(&rest[..at]);
        let end = at + rest[at..].find('>').expect("a tag ends");
        if &rest[at..=end] == "</p>" {
            lines.push('|');
        }
        rest = &rest[end + 1..];
    }
    lines
}

/// Mutant: `list_item_contents` groups an inline element that holds a block
/// with its inline neighbours (`!holds_block` dropped). Nothing failed.
/// This is the reading #1446 is about; whatever it becomes, it is pinned.
#[test]
fn an_inline_element_holding_a_block_in_a_list_is_an_item_of_its_own() {
    assert_eq!(
        html("<ul><b>Aa<div>Bb</div></b>Cc<li>Dd</li></ul>"),
        "<ul><li><p><strong>Aa</strong></p><p><strong>Bb</strong></p></li>\
         <li><p>Cc</p></li><li><p>Dd</p></li></ul>"
    );
    assert_eq!(
        html("<ul>Aa<b>Bb<div></div></b>Cc<li>Dd</li></ul>"),
        "<ul><li><p>Aa</p></li><li><p><strong>Bb</strong></p></li>\
         <li><p>Cc</p></li><li><p>Dd</p></li></ul>"
    );
}

/// Mutant: `end_under` does not stop at a dropped element (`is_dropped`
/// removed from the loop's break). Nothing failed. With the guard, the
/// `<span>` under the `<video>` stays open, so `Ad` keeps its colour.
#[test]
fn a_formatting_end_tag_read_inside_a_dropped_element_ends_nothing_below_it() {
    assert_eq!(
        html("<b>Aa<div><span style=\"color:red\">Ab<video>Zz</b>Ac</video>Ad</span>Ae</div>Af"),
        "<p><strong>Aa</strong></p>\
         <p><span style=\"color:red\"><strong>Ab</strong></span>\
         <span style=\"color:red\">Ad</span>Ae</p><p>Af</p>"
    );
}

/// The three line rules give the same lines at every depth, under blocks and
/// under inline elements, on both sides of the depth limit.
#[test]
fn the_line_rules_hold_across_the_depth_limit() {
    let limit = html_reader_max_depth();
    for open in ["<div>", "<span>", "<blockquote>", "<i>"] {
        for depth in [
            0,
            1,
            33,
            limit - 3,
            limit - 1,
            limit,
            limit + 1,
            limit + 5,
            2 * limit,
        ] {
            let at = |tail: &str| lines(&format!("{}{tail}", open.repeat(depth)));
            assert_eq!(at("Aa<div></div>Bb"), "Aa|Bb|", "{open} {depth}");
            assert_eq!(at("Aa</p>Bb"), "Aa|Bb|", "{open} {depth}");
            assert_eq!(at("Aa<b>Bb</p>Cc</b>Dd"), "AaBb|CcDd|", "{open} {depth}");
            assert_eq!(
                at("Aa<b>Bb<div>Cc</div>Dd</b>Ee"),
                "AaBb|Cc|DdEe|",
                "{open} {depth}"
            );
            assert_eq!(
                at("Aa<b>Bb<div>Cc</b>Dd</div>Ee"),
                "AaBb|CcDd|Ee|",
                "{open} {depth}"
            );
        }
    }
}

/// A stray `</p>` where the reader builds something other than paragraphs.
#[test]
fn a_stray_p_end_tag_in_each_kind_of_container() {
    assert_eq!(html("<h1>a</p>b</h1>"), "<h1>a</h1><h1>b</h1>");
    assert_eq!(
        html("<ul><li>a</p>b</li><li>c</li></ul>"),
        "<ul><li><p>a</p><p>b</p></li><li><p>c</p></li></ul>"
    );
    // Between items, rows and cells it is nothing: no item, no cell, no line.
    assert_eq!(
        html("<ul><li>a</li></p><li>b</li></ul>"),
        "<ul><li><p>a</p></li><li><p>b</p></li></ul>"
    );
    assert_eq!(
        html("<table><tr><td>a</td></tr></p><tr><td>b</td></tr></table>"),
        "<table><tr><td><p>a</p></td></tr><tr><td><p>b</p></td></tr></table>"
    );
    assert_eq!(
        html("<table><tr><td>a</td></p><td>b</td></tr></table>"),
        "<table><tr><td><p>a</p></td><td><p>b</p></td></tr></table>"
    );
    // No blank line for the one a block's start left over.
    assert_eq!(
        html("<p>intro<ul><li>x</li></ul></p><p>next</p>"),
        "<p>intro</p><ul><li><p>x</p></li></ul><p>next</p>"
    );
    assert_eq!(html("<p>a</p></p><p>b</p>"), "<p>a</p><p>b</p>");
    assert_eq!(html("<textarea>a</p>b</textarea>"), "<p>a&lt;/p&gt;b</p>");
}

/// A mark element around a stray `</p>` or an empty block keeps its mark on
/// both lines (bold, italic, link with its attributes, highlight).
#[test]
fn a_mark_element_split_by_a_boundary_keeps_its_mark() {
    assert_eq!(
        html("<a href=\"http://x.y/\">a</p>b</a>"),
        "<p><a href=\"http://x.y/\">a</a></p><p><a href=\"http://x.y/\">b</a></p>"
    );
    assert_eq!(
        html("<mark style=\"background-color:yellow\">a</p>b</mark>"),
        "<p><mark style=\"background-color:yellow\">a</mark></p>\
         <p><mark style=\"background-color:yellow\">b</mark></p>"
    );
    assert_eq!(
        html("x<a href=\"h\">y<div>z</div>v</a>w"),
        "<p>x<a href=\"h\">y</a></p><p><a href=\"h\">z</a></p><p><a href=\"h\">v</a>w</p>"
    );
    // The copies `end_under` makes carry the element's attributes.
    assert_eq!(
        html("<a href=\"http://x/1\" title=\"T\">a<div>b</a>c</div>d"),
        "<p><a href=\"http://x/1\" title=\"T\">a</a></p>\
         <p><a href=\"http://x/1\" title=\"T\">b</a>c</p><p>d</p>"
    );
}

/// A reference is decoded before the URL it is in is judged: none of these
/// is a link or an image (#1415 made `&Tab;`, `&NewLine;` and `&colon;`
/// decode).
#[test]
fn a_reference_cannot_hide_a_script_url() {
    assert_eq!(
        html(
            "<a href=\"java&Tab;script:alert(1)\">p</a>\
             <a href=\"java&NewLine;script:alert(1)\">q</a>\
             <a href=\"javascript&colon;alert(1)\">s</a>\
             <img src=\"javascript&colon;alert(1)\">\
             <a href=\"&#106avascript:alert(1)\">t</a>"
        ),
        "<p>pqst</p>"
    );
}

/// Chrome 153 ignores a `</b>` read inside a `<select>` (`Ad` stays bold,
/// as it did on main). The reader ended the bold there until `select`
/// joined the scopes a formatting end tag does not reach out of.
#[test]
fn a_formatting_end_tag_inside_a_select_is_skipped() {
    assert_eq!(
        html("<b>Aa<select><option>Ab</b>Ac</option></select>Ad"),
        "<p><strong>AaAd</strong></p>"
    );
}

/// Chrome 153: two red lines. Main: one red line. Head: two lines, no colour
/// (the PR's stated regression; only a `<span style>` loses anything). #1462.
#[test]
#[ignore = "#1462: a styled <span> around a block carries no colour"]
fn a_styled_span_split_by_a_stray_p_end_tag_keeps_its_colour() {
    assert_eq!(
        html("<span style=\"color:red\">a</p>b</span>"),
        "<p><span style=\"color:red\">a</span></p><p><span style=\"color:red\">b</span></p>"
    );
}
