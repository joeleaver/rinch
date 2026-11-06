//! What `set_inner_html` makes of character references (#2239), shared by
//! the desktop test (`crates/rinch-dom/tests/inner_html_char_refs_2239_tests.rs`)
//! and the browser one (`crates/rinch-web/tests/inner_html_char_refs_2239.rs`),
//! where the browser's own HTML parser answers: every expectation here is
//! what Chrome and Firefox make of the markup, so the two backends agree.

use rinch_core::dom::{DomDocument, NodeId};

/// `(markup set as an element's inner HTML, its text content)`.
pub const TEXT: &[(&str, &str)] = &[
    // The issue: a decimal reference to U+200B ZERO WIDTH SPACE.
    ("a&#8203;b", "a\u{200b}b"),
    // Hexadecimal, either `x`, and no `;` (still a reference).
    ("&#x200B;&#X41;&#x42&#67;&#68", "\u{200b}ABCD"),
    ("&#x1F600;&#233;", "\u{1f600}\u{e9}"),
    // 128..159 are Windows-1252; 0, a surrogate and past U+10FFFF are U+FFFD.
    ("&#128;&#146;&#150;", "\u{20ac}\u{2019}\u{2013}"),
    ("&#0;&#xD800;&#x110000;", "\u{fffd}\u{fffd}\u{fffd}"),
    // Named references beyond the six the old parser knew.
    (
        "&hellip;&rsquo;&eacute;&mdash;&apos;&NotEqualTilde;",
        "\u{2026}\u{2019}\u{e9}\u{2014}'\u{2242}\u{338}",
    ),
    // One pass: what a reference decodes to is not decoded again.
    ("&amp;lt; &amp;#8203; &amp;amp;", "&lt; &#8203; &amp;"),
    // A legacy name needs no `;`, and is the longest one the run starts with.
    (
        "&copy 2 &notit; &notin; &ampx",
        "\u{a9} 2 \u{ac}it; \u{2209} &x",
    ),
    // Not references: left as written.
    ("R&D &nosuch; &# &#x; & &;", "R&D &nosuch; &# &#x; & &;"),
    // In an element: the references of each text are decoded.
    ("<b>&lt;</b>&#x2D;<i>&gt;</i>", "<->"),
];

/// `(markup of a `<span>` start tag, its `title` attribute)`.
pub const ATTRIBUTES: &[(&str, &str)] = &[
    (r#"<span title="a&#8203;b&#x41;"></span>"#, "a\u{200b}bA"),
    (r#"<span title='&lt;x&gt;&quot;'></span>"#, "<x>\""),
    // Unquoted.
    ("<span title=a&#65;&amp;b></span>", "aA&b"),
];

/// A legacy name with no `;` before `=`, a letter or a digit in an
/// attribute value is left alone: `(the value as written, as read)`, set as
/// a `<span title>`. Firefox 155 reads these so; Chrome 156 does with its
/// full parser, which an `<svg>` in the markup takes it to
/// ([`legacy_markup`]), but its fast `innerHTML` path decodes some of them
/// as if they were text (`?a=1&copy=2&amp;` reads `?a=1©=2&`, though
/// `?a=1&copy=2` alone reads as written). Desktop follows the standard.
pub const LEGACY_IN_ATTRIBUTES: &[(&str, &str)] = &[
    ("?a=1&copy=2&amp;&lt&copy;", "?a=1&copy=2&<\u{a9}"),
    ("&reg2&notit;", "&reg2&notit;"),
    ("x&notin;y&copy;&copy2", "x\u{2209}y\u{a9}&copy2"),
];

/// A `<span>` titled `value`, then an `<svg>` (which keeps Chrome off its
/// fast path) unless `fast`.
pub fn legacy_markup(value: &str, fast: bool) -> String {
    let svg = if fast { "" } else { "<svg></svg>" };
    format!("<span title=\"{value}\"></span>{svg}")
}

/// `(markup, the text of its first element)`: raw text is not decoded, in
/// HTML only (`<style>` and `<script>` in SVG or MathML are decoded, and an
/// SVG `<foreignObject>` or `<desc>` holds HTML again; review of #2258).
/// Chrome 156 and Firefox 155.
pub const RAW_TEXT: &[(&str, &str)] = &[
    (
        "<svg><style>.a{content:\"&amp;\"}</style></svg>",
        ".a{content:\"&\"}",
    ),
    (
        "<svg><script>var a=\"&lt;\";</script></svg>",
        "var a=\"<\";",
    ),
    ("<math><style>&amp;</style></math>", "&"),
    (
        "<svg><foreignObject><style>&amp;</style></foreignObject></svg>",
        "&amp;",
    ),
    ("<svg><desc><style>&amp;</style></desc></svg>", "&amp;"),
    ("<xmp>&amp;</xmp>", "&amp;"),
    ("<iframe>&amp;</iframe>", "&amp;"),
    ("<noembed>&amp;</noembed>", "&amp;"),
    ("<noframes>&amp;</noframes>", "&amp;"),
    ("<noscript>&amp;</noscript>", "&amp;"),
    (
        "<style>.never-matches-2239::after{content:\"&amp;\"}</style>",
        ".never-matches-2239::after{content:\"&amp;\"}",
    ),
    (
        "<script>var a = \"&lt;&#65;\";</script>",
        "var a = \"&lt;&#65;\";",
    ),
];

/// Set each case as `host`'s inner HTML and check what it became.
pub fn check(doc: &mut dyn DomDocument, host: NodeId) {
    for &(html, want) in TEXT {
        doc.set_inner_html(host, html);
        assert_eq!(
            doc.text_content(host).as_deref(),
            Some(want),
            "text of {html:?}"
        );
    }
    for &(html, want) in ATTRIBUTES {
        doc.set_inner_html(host, html);
        let span = doc.get_children(host)[0];
        assert_eq!(
            doc.get_attribute(span, "title").as_deref(),
            Some(want),
            "title of {html:?}"
        );
    }
    for &(value, want) in LEGACY_IN_ATTRIBUTES {
        doc.set_inner_html(host, &legacy_markup(value, false));
        let span = doc.get_children(host)[0];
        assert_eq!(
            doc.get_attribute(span, "title").as_deref(),
            Some(want),
            "title {value:?}"
        );
    }
    for &(html, want) in RAW_TEXT {
        doc.set_inner_html(host, html);
        let el = doc.get_children(host)[0];
        assert_eq!(
            doc.text_content(el).as_deref(),
            Some(want),
            "raw text of {html:?}"
        );
    }
    doc.set_inner_html(host, "");
}
