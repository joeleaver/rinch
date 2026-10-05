//! Fixtures from the adversarial review of PR #1374 (#1365).
#![cfg(feature = "markdown")]
use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{doc_from_markdown, doc_from_markdown_strict, doc_to_markdown};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node};
fn s() -> Schema {
    Schema::starter_kit()
}
fn n(s: &Schema, name: &str, attrs: Attrs, kids: Vec<Node>) -> Node {
    s.create_node(name, attrs, Fragment::from_children(kids))
        .unwrap()
}
fn p(s: &Schema, text: &str) -> Node {
    if text.is_empty() {
        n(s, "paragraph", Attrs::new(), vec![])
    } else {
        n(s, "paragraph", Attrs::new(), vec![s.text(text).unwrap()])
    }
}
fn h(s: &Schema, text: &str) -> Node {
    let k = if text.is_empty() {
        vec![]
    } else {
        vec![s.text(text).unwrap()]
    };
    n(
        s,
        "heading",
        Attrs::from_iter([("level", AttrValue::Int(2))]),
        k,
    )
}
fn task(s: &Schema, c: bool, kids: Vec<Node>) -> Node {
    n(
        s,
        "task_item",
        Attrs::from_iter([("checked", AttrValue::Bool(c))]),
        kids,
    )
}
fn tl(s: &Schema, items: Vec<Node>) -> Node {
    n(s, "task_list", Attrs::new(), items)
}
fn li(s: &Schema, kids: Vec<Node>) -> Node {
    n(s, "list_item", Attrs::new(), kids)
}
fn bl(s: &Schema, items: Vec<Node>) -> Node {
    n(s, "bullet_list", Attrs::new(), items)
}
fn bq(s: &Schema, kids: Vec<Node>) -> Node {
    n(s, "blockquote", Attrs::new(), kids)
}
fn doc(s: &Schema, b: Vec<Node>) -> Node {
    n(s, "doc", Attrs::new(), b)
}

// ───────────── asserting fixtures (review of #1374) ─────────────

/// D1 — RED at d8a79deb in a debug build (panics on the `debug_assert!` in
/// `claim_task_marker`); green in release, through the fallback the PR calls
/// unreachable. pulldown-cmark accepts tab/VT/FF inside the brackets and after
/// them, and reports a marker after `- <space><tab>` at the tab, not at `[`.
#[test]
fn fixture_d1_markers_the_source_scan_misses_do_not_panic() {
    let s = &s();
    // pulldown keeps a VT after the marker in the text; the others are bare.
    for (md, checked, text) in [
        ("- [\t] x", false, "x"),
        ("- \t[x] x", true, "x"),
        ("- [ ]\u{b}x", false, "\u{b}x"),
        ("- [\u{c}]\u{c}x", false, "\u{c}x"),
        ("-  \t[X] x", true, "x"),
    ] {
        let want = doc(s, vec![tl(s, vec![task(s, checked, vec![p(s, text)])])]);
        assert_eq!(doc_from_markdown(s, md).unwrap(), want, "{md:?}");
        assert_eq!(doc_from_markdown_strict(s, md).unwrap(), want, "{md:?}");
    }
    // And in a list with a plain item, the lenient reader gives the marker
    // back as the text it was.
    let d = doc_from_markdown(s, "- [\t] x\n- plain").unwrap();
    assert_eq!(d.child(0).child(0).child(0).child(0).text(), Some("[\t] x"));
}

/// D2 — RED at d8a79deb. An empty checkbox written without the trailing space
/// (what a person or an LLM types, and what any trailing-whitespace stripper
/// makes of the writer's own `- [ ] `) turns the WHOLE checklist into bullets
/// whose text starts with a literal `[ ]`; the next write escapes them for good.
#[test]
fn fixture_d2_an_empty_marker_without_its_trailing_space_keeps_the_checklist() {
    let s = &s();
    let d = doc(
        s,
        vec![tl(
            s,
            vec![
                task(s, false, vec![p(s, "a")]),
                task(s, false, vec![p(s, "")]),
                task(s, true, vec![p(s, "c")]),
            ],
        )],
    );
    let written = doc_to_markdown(&d);
    assert_eq!(written, "- [ ] a\n- [ ] \n- [x] c");
    let stripped: String = written
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(doc_from_markdown(s, &stripped).unwrap(), d, "{stripped:?}");
    // and a marker alone on its line before another block
    let d = doc(
        s,
        vec![tl(
            s,
            vec![
                task(s, false, vec![h(s, "h")]),
                task(s, true, vec![p(s, "c")]),
            ],
        )],
    );
    let stripped: String = doc_to_markdown(&d)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(doc_from_markdown(s, &stripped).unwrap(), d, "{stripped:?}");
}

/// Pin (green at d8a79deb) — kills the surviving mutant "an empty block still
/// resets `prev_bullet`": a bullet list, an empty paragraph, a task list.
#[test]
fn fixture_lists_around_a_block_that_writes_nothing_stay_apart() {
    let s = &s();
    for between in [p(s, ""), p(s, " "), bq(s, vec![p(s, "")])] {
        let d = doc(
            s,
            vec![
                bl(s, vec![li(s, vec![p(s, "a")])]),
                between,
                tl(s, vec![task(s, true, vec![p(s, "t")])]),
            ],
        );
        let want = doc(
            s,
            vec![
                bl(s, vec![li(s, vec![p(s, "a")])]),
                tl(s, vec![task(s, true, vec![p(s, "t")])]),
            ],
        );
        let md = doc_to_markdown(&d);
        assert_eq!(
            doc_from_markdown_strict(s, &md).unwrap_or_else(|e| panic!("{md:?}: {e}")),
            want,
            "{md:?}"
        );
    }
}

/// Pin (green at d8a79deb) — kills both `first_is_para` mutants, including the
/// `writes_nothing` → `is_empty()` one the PR calls equivalent: a blank first
/// paragraph before a nested list whose item holds two paragraphs.
#[test]
fn fixture_a_blank_first_paragraph_before_a_deep_list_keeps_the_lists_shape() {
    let s = &s();
    for first in ["", " ", "\t"] {
        let inner = bl(
            s,
            vec![li(s, vec![p(s, "a"), p(s, "a2")]), li(s, vec![p(s, "b")])],
        );
        let d = doc(
            s,
            vec![tl(
                s,
                vec![task(
                    s,
                    false,
                    vec![p(s, first), inner.clone(), p(s, "after")],
                )],
            )],
        );
        let want = doc(
            s,
            vec![tl(s, vec![task(s, false, vec![inner, p(s, "after")])])],
        );
        let md = doc_to_markdown(&d);
        assert_eq!(doc_from_markdown_strict(s, &md).unwrap(), want, "{md:?}");
    }
}

/// Pin (green at d8a79deb) — kills the two surviving HTML mutants: only
/// `data-checked="true"` checks, only `data-type="taskList"` is a task list.
#[test]
fn fixture_html_only_true_checks_and_only_tasklist_is_one() {
    use rinch_editor_core::serialize::slice_from_html;
    let s = &s();
    let sl = slice_from_html(s, r#"<ul data-type="taskList"><li data-checked="checked">x</li><li data-checked="">y</li></ul>"#).unwrap();
    assert_eq!(
        sl.content.children(),
        &[tl(
            s,
            vec![
                task(s, false, vec![p(s, "x")]),
                task(s, false, vec![p(s, "y")])
            ]
        )][..]
    );
    let sl = slice_from_html(s, r#"<ul data-type="other"><li>x</li></ul>"#).unwrap();
    assert_eq!(
        sl.content.children(),
        &[bl(s, vec![li(s, vec![p(s, "x")])])][..]
    );
}
