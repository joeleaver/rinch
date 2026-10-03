//! Markdown round trips the review of #1242 found broken, each pinned here:
//! a document written with `doc_to_markdown` and read back with
//! `doc_from_markdown_strict` must be the same document, and the strict reader
//! must refuse what it would drop.
#![cfg(feature = "markdown")]

use rinch_editor_core::serialize::{
    Construct, MarkdownError, doc_from_markdown, doc_from_markdown_strict, doc_to_markdown,
};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, Schema};

fn mark(s: &Schema, name: &str, attrs: &[(&str, &str)]) -> Mark {
    let mt = s.mark_type(name).unwrap();
    let a = Attrs::from_iter(
        attrs
            .iter()
            .map(|(k, v)| (*k, AttrValue::from(v.to_string()))),
    );
    Mark::new(mt.clone(), mt.compute_attrs(&a).unwrap())
}

fn t(s: &Schema, text: &str, marks: &[&Mark]) -> Node {
    let mut set: Vec<Mark> = Vec::new();
    for m in marks {
        set = m.add_to_set(&set);
    }
    s.text_with_marks(text, set).unwrap()
}

fn br(s: &Schema) -> Node {
    s.create_node("hard_break", Attrs::new(), Fragment::empty())
        .unwrap()
}

fn p(s: &Schema, inline: Vec<Node>) -> Node {
    s.create_node("paragraph", Attrs::new(), Fragment::from_children(inline))
        .unwrap()
}

fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
    s.create_node("doc", Attrs::new(), Fragment::from_children(blocks))
        .unwrap()
}

/// Write `d`, read it back strictly, require the same document, and require a
/// second write to change nothing. Returns the Markdown.
fn rt(s: &Schema, d: &Node) -> String {
    let md = doc_to_markdown(d);
    let back = doc_from_markdown_strict(s, &md)
        .unwrap_or_else(|e| panic!("strict read of {md:?} failed: {e}"));
    assert_eq!(&back, d, "round trip through {md:?}");
    assert_eq!(doc_to_markdown(&back), md, "second write of {md:?}");
    md
}

// ── F1: a delimiter run that ends at a hard break ──

#[test]
fn a_delimiter_run_ending_at_a_hard_break_closes_before_it() {
    let s = Schema::starter_kit();
    for name in ["bold", "italic", "strike"] {
        let m = mark(&s, name, &[]);
        // Bold a line, Shift+Enter, keep typing plain text.
        let d = doc(
            &s,
            vec![p(&s, vec![t(&s, "a", &[&m]), br(&s), t(&s, "b", &[])])],
        );
        rt(&s, &d);
        // Plain, then a marked line after the break.
        let d = doc(
            &s,
            vec![p(&s, vec![t(&s, "a", &[]), br(&s), t(&s, "b", &[&m])])],
        );
        rt(&s, &d);
    }
}

#[test]
fn a_run_across_a_hard_break_still_round_trips() {
    let s = Schema::starter_kit();
    let bold = mark(&s, "bold", &[]);
    let it = mark(&s, "italic", &[]);
    let d = doc(
        &s,
        vec![p(
            &s,
            vec![t(&s, "a", &[&bold]), br(&s), t(&s, "b", &[&bold])],
        )],
    );
    rt(&s, &d);
    // The break keeps the run the next line continues and closes the rest.
    let d = doc(
        &s,
        vec![p(
            &s,
            vec![
                t(&s, "a", &[&bold, &it]),
                br(&s),
                t(&s, "b", &[&bold]),
                t(&s, " c", &[]),
            ],
        )],
    );
    rt(&s, &d);
}

// ── F2: a marked link beside a word ──

#[test]
fn a_bold_link_beside_a_word_keeps_its_bold() {
    let s = Schema::starter_kit();
    let link = mark(&s, "link", &[("href", "https://x.y")]);
    for name in ["bold", "italic", "strike"] {
        let m = mark(&s, name, &[]);
        let d = doc(
            &s,
            vec![p(&s, vec![t(&s, "a", &[&m, &link]), t(&s, "s", &[])])],
        );
        rt(&s, &d);
        let d = doc(
            &s,
            vec![p(&s, vec![t(&s, "x", &[]), t(&s, "a", &[&m, &link])])],
        );
        rt(&s, &d);
    }
}

// ── F3: leading whitespace before a block marker ──

/// The one paragraph `md` reads back as, and its text.
fn single_paragraph_text(s: &Schema, md: &str) -> Vec<String> {
    let d = doc_from_markdown_strict(s, md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    assert_eq!(d.child_count(), 1, "{md:?} read as {d:?}");
    let para = d.child(0);
    assert_eq!(para.type_name(), "paragraph", "{md:?} read as {d:?}");
    para.content()
        .children()
        .iter()
        .map(|n| n.text().unwrap_or("<br>").to_string())
        .collect()
}

#[test]
fn leading_whitespace_does_not_unescape_a_block_marker() {
    let s = Schema::starter_kit();
    let bold = mark(&s, "bold", &[]);
    // The leading space is CommonMark's to strip; the marker must stay text.
    for (text, want) in [
        (" > not a quote", "> not a quote"),
        (" - not a list", "- not a list"),
        ("  # not a heading", "# not a heading"),
        ("   1. not a list", "1. not a list"),
    ] {
        let md = doc_to_markdown(&doc(&s, vec![p(&s, vec![t(&s, text, &[])])]));
        assert_eq!(single_paragraph_text(&s, &md), vec![want], "{md:?}");
    }
    let md = doc_to_markdown(&doc(&s, vec![p(&s, vec![t(&s, " # x", &[&bold])])]));
    assert_eq!(single_paragraph_text(&s, &md), vec!["# x"], "{md:?}");
    // After a hard break, too: the break must survive as well.
    let md = doc_to_markdown(&doc(
        &s,
        vec![p(&s, vec![t(&s, "a", &[]), br(&s), t(&s, " > q", &[])])],
    ));
    assert_eq!(
        single_paragraph_text(&s, &md),
        vec!["a", "<br>", "> q"],
        "{md:?}"
    );
}

// ── F4: `!` before a link ──

#[test]
fn a_bang_before_a_link_does_not_make_an_image() {
    let s = Schema::starter_kit();
    for href in ["https://x.y", "mailto:a@b.c"] {
        let link = mark(&s, "link", &[("href", href)]);
        let d = doc(
            &s,
            vec![p(&s, vec![t(&s, "Done!", &[]), t(&s, "docs", &[&link])])],
        );
        rt(&s, &d);
    }
    // Already escaped text before it, and a bold link.
    let link = mark(&s, "link", &[("href", "https://x.y")]);
    let bold = mark(&s, "bold", &[]);
    let d = doc(
        &s,
        vec![p(&s, vec![t(&s, "a\\!", &[]), t(&s, "b", &[&link])])],
    );
    rt(&s, &d);
    let d = doc(
        &s,
        vec![p(&s, vec![t(&s, "!", &[]), t(&s, "b", &[&bold, &link])])],
    );
    rt(&s, &d);
}

// ── F5: strict refuses what an HTML table drops ──

fn refusal(s: &Schema, md: &str) -> Construct {
    match doc_from_markdown_strict(s, md) {
        Err(MarkdownError::Unsupported { construct, .. }) => construct,
        other => panic!("{md:?}: expected a refusal, got {other:?}"),
    }
}

#[test]
fn strict_refuses_unsafe_urls_inside_an_html_table() {
    let s = Schema::starter_kit();
    assert_eq!(
        refusal(
            &s,
            "<table><tr><td><a href=\"javascript:alert(1)\">x</a></td></tr></table>"
        ),
        Construct::UnsafeLink
    );
    assert_eq!(
        refusal(
            &s,
            "<table><tr><td><img src=\"javascript:alert(1)\">x</td></tr></table>"
        ),
        Construct::UnsafeImage
    );
    assert_eq!(
        refusal(
            &s,
            "<table><tr><td><img src=\"data:image/svg+xml,x\">x</td></tr></table>"
        ),
        Construct::UnsafeImage
    );
    // Lenient still drops them, as before.
    let d = doc_from_markdown(
        &s,
        "<table><tr><td><a href=\"javascript:alert(1)\">x</a></td></tr></table>",
    )
    .unwrap();
    let text = d.child(0).child(0).child(0).child(0).child(0);
    assert_eq!(text.text(), Some("x"));
    assert!(text.marks().is_empty());
}

#[test]
fn strict_refuses_attributes_an_html_table_drops() {
    let s = Schema::starter_kit();
    for md in [
        "<table><tr><td><span style=\"font-size:99px\">x</span></td></tr></table>",
        "<table><tr><td><span style=\"color:expression(1)\">x</span></td></tr></table>",
        "<table><tr><td><p style=\"margin:9px\">x</p></td></tr></table>",
        "<table><tr><td><mark style=\"background-color:url(x)\">x</mark></td></tr></table>",
        "<table><tr><td colspan=\"99999\">x</td></tr></table>",
        "<table><tr><td onclick=\"alert(1)\">x</td></tr></table>",
        "<table class=\"wide\"><tr><td>x</td></tr></table>",
    ] {
        assert_eq!(refusal(&s, md), Construct::HtmlBlock, "{md}");
    }
}

#[test]
fn strict_accepts_every_attribute_an_html_table_keeps() {
    let s = Schema::starter_kit();
    for md in [
        "<table><tr><td><span style=\"color:red\">x</span></td></tr></table>",
        "<table><tr><td><mark style=\"background-color:#ffee00\">x</mark></td></tr></table>",
        "<table><tr><td><p style=\"text-align:center\">x</p></td></tr></table>",
        "<table><tr><td colspan=\"2\" rowspan=\"2\">x</td></tr></table>",
        "<table><tr><td><a href=\"https://ok\" title=\"t\" target=\"_blank\" rel=\"noopener noreferrer\">x</a></td></tr></table>",
        "<table><tr><td><img src=\"a.png\" alt=\"a\" title=\"t\">x</td></tr></table>",
        "<table><tr><td><ol start=\"5\"><li>x</li></ol></td></tr></table>",
    ] {
        doc_from_markdown_strict(&s, md).unwrap_or_else(|e| panic!("{md}: {e}"));
    }
}

/// The text-loss check and the tag whitelist each refuse something the other
/// does not see (mutants M4 and M10 of the review).
#[test]
fn strict_refuses_stray_text_and_unknown_tags_in_an_html_table() {
    let s = Schema::starter_kit();
    for md in [
        "<table><tr>stray<td>x</td></tr></table>",
        "<table>stray<tr><td>x</td></tr></table>",
        "<table><tr><td><input type=checkbox>x</td></tr></table>",
        "<table><tr><td><input>x</td></tr></table>",
    ] {
        assert_eq!(refusal(&s, md), Construct::HtmlBlock, "{md}");
    }
}

// ── F6: the writer never puts an unvalidated colour into HTML ──

#[test]
fn an_unsafe_colour_is_never_written_into_html() {
    let s = Schema::starter_kit();
    let evil = "red\"><img src=x onerror=alert(1)><span x=\"";
    for name in ["text_color", "highlight"] {
        let m = mark(&s, name, &[("color", evil)]);
        let d = doc(&s, vec![p(&s, vec![t(&s, "a", &[&m])])]);
        let md = doc_to_markdown(&d);
        assert!(
            !md.contains("onerror") && !md.contains("<img"),
            "{name}: {md:?}"
        );
        let back = doc_from_markdown_strict(&s, &md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
        assert_eq!(back.child(0).child(0).text(), Some("a"), "{md:?}");
        // The same in a table cell, which is written as HTML.
        let cell = s
            .create_node(
                "table_cell",
                Attrs::new(),
                Fragment::from_node(p(&s, vec![t(&s, "a", &[&m])])),
            )
            .unwrap();
        let cell2 = s
            .create_node(
                "table_cell",
                Attrs::new(),
                Fragment::from_node(p(&s, vec![])),
            )
            .unwrap();
        let row = s
            .create_node(
                "table_row",
                Attrs::new(),
                Fragment::from_children(vec![cell, cell2]),
            )
            .unwrap();
        let table = s
            .create_node("table", Attrs::new(), Fragment::from_node(row))
            .unwrap();
        let md = doc_to_markdown(&doc(&s, vec![table]));
        assert!(
            !md.contains("onerror") && !md.contains("<img"),
            "{name}: {md:?}"
        );
        doc_from_markdown_strict(&s, &md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
    }
    // CSS smuggled through a colour is not written either.
    let m = mark(
        &s,
        "text_color",
        &[("color", "red;background:url(javascript:x)")],
    );
    let md = doc_to_markdown(&doc(&s, vec![p(&s, vec![t(&s, "a", &[&m])])]));
    assert!(!md.contains("url("), "{md:?}");
    // A valid colour is still written.
    let m = mark(&s, "text_color", &[("color", "rgb(1, 2, 3)")]);
    rt(&s, &doc(&s, vec![p(&s, vec![t(&s, "a", &[&m])])]));
}

// ── delimiters that would not flank: written as tags ──

#[test]
fn a_delimiter_run_that_would_not_flank_round_trips() {
    let s = Schema::starter_kit();
    let bold = mark(&s, "bold", &[]);
    let it = mark(&s, "italic", &[]);
    let st = mark(&s, "strike", &[]);
    let sup = mark(&s, "superscript", &[]);
    let link = mark(&s, "link", &[("href", "https://x.y")]);
    let cases: Vec<Vec<Node>> = vec![
        // `*~~a~~*b`: the closing `*` follows `~` and precedes a letter.
        vec![t(&s, "a", &[&it, &st]), t(&s, "b", &[])],
        // `x*~~a~~*`: the opening `*` follows a letter and precedes `~`.
        vec![t(&s, "x", &[]), t(&s, "a", &[&it, &st])],
        // Punctuation at a run's edge, a letter outside it.
        vec![t(&s, "a.", &[&bold]), t(&s, "b", &[])],
        vec![t(&s, "b", &[]), t(&s, "(a", &[&it])],
        vec![t(&s, "x", &[]), t(&s, "\"q\"", &[&st]), t(&s, "y", &[])],
        // A run that ends inside an HTML mark and one that starts after a link.
        vec![
            t(&s, "y", &[&st, &sup]),
            t(&s, "a", &[&st, &link]),
            t(&s, "c", &[]),
        ],
        // Bold and italic meeting: one closes where the other opens.
        vec![t(&s, "a", &[&it]), t(&s, "b", &[&bold])],
        vec![
            t(&s, "a", &[&bold]),
            t(&s, "b", &[&it]),
            t(&s, "c", &[&bold]),
        ],
        // Code beside a delimiter, a letter on the other side.
        vec![
            t(&s, "x", &[]),
            t(&s, "c", &[&bold, &mark(&s, "code", &[])]),
            t(&s, "s", &[]),
        ],
    ];
    for inline in cases {
        rt(&s, &doc(&s, vec![p(&s, inline)]));
    }
    // Ordinary runs are still plain Markdown.
    let md = rt(
        &s,
        &doc(
            &s,
            vec![p(
                &s,
                vec![t(&s, "a ", &[]), t(&s, "b", &[&bold]), t(&s, " c", &[])],
            )],
        ),
    );
    assert_eq!(md, "a **b** c");
}

#[test]
fn the_reader_takes_the_html_spellings_of_bold_italic_and_strike() {
    let s = Schema::starter_kit();
    for (md, name) in [
        ("x<strong>a</strong>y", "bold"),
        ("x<b>a</b>y", "bold"),
        ("x<em>a</em>y", "italic"),
        ("x<i>a</i>y", "italic"),
        ("x<s>a</s>y", "strike"),
        ("x<del>a</del>y", "strike"),
    ] {
        let d = doc_from_markdown_strict(&s, md).unwrap_or_else(|e| panic!("{md}: {e}"));
        let a = d.child(0).child(1);
        assert_eq!(a.text(), Some("a"), "{md}");
        assert_eq!(a.marks().len(), 1, "{md}");
        assert_eq!(a.marks()[0].type_name(), name, "{md}");
    }
    assert_eq!(refusal(&s, "x<strong>a"), Construct::UnmatchedTag);
    assert_eq!(refusal(&s, "x<strong>a</b>"), Construct::UnmatchedTag);
}

#[test]
fn whitespace_before_an_html_mark_stays_outside_it() {
    let s = Schema::starter_kit();
    let hl = mark(&s, "highlight", &[]);
    let d = doc(
        &s,
        vec![p(
            &s,
            vec![t(&s, "a ", &[]), t(&s, " ", &[&hl]), t(&s, "b", &[])],
        )],
    );
    rt(&s, &d);
}

// ── leading whitespace never moves a block ──

#[test]
fn leading_whitespace_keeps_the_block_structure() {
    let s = Schema::starter_kit();
    let node = |name: &str, kids: Vec<Node>| {
        s.create_node(name, Attrs::new(), Fragment::from_children(kids))
            .unwrap()
    };
    let hl = mark(&s, "highlight", &[]);
    for first in [
        vec![t(&s, " a", &[])],
        vec![t(&s, "   a", &[])],
        vec![t(&s, " ", &[&hl]), t(&s, "a", &[])],
    ] {
        // A list item whose first paragraph starts with whitespace keeps its
        // second paragraph.
        let d = doc(
            &s,
            vec![node(
                "bullet_list",
                vec![node(
                    "list_item",
                    vec![p(&s, first.clone()), p(&s, vec![t(&s, "b", &[])])],
                )],
            )],
        );
        let md = doc_to_markdown(&d);
        let back = doc_from_markdown_strict(&s, &md).unwrap_or_else(|e| panic!("{md:?}: {e}"));
        assert_eq!(back.child_count(), 1, "{md:?} read as {back:?}");
        let item = back.child(0).child(0);
        assert_eq!(item.child_count(), 2, "{md:?} read as {back:?}");
        assert_eq!(doc_to_markdown(&back), md, "{md:?}");
    }
    // Four spaces at a paragraph's start do not make it a code block.
    let md = doc_to_markdown(&doc(&s, vec![p(&s, vec![t(&s, "    code?", &[])])]));
    assert_eq!(single_paragraph_text(&s, &md), vec!["code?"], "{md:?}");
}
