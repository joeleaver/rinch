//! `serialize::set_html_fallback`: what an app gives an `embed` in exported HTML.
//!
//! An embed's content is drawn by the app (a node view), so the document holds
//! nothing another application could show. An app can hand the HTML export markup
//! for one — a static table for a sheet, an `<img>` of a canvas — and the export
//! writes it **inside** the embed's element, which still carries the atom: rinch
//! reads the element back as the embed and ignores what is inside it.

use rinch_editor_core::serialize::{
    node_to_html, set_html_fallback, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Schema, Slice};

fn embed(schema: &Schema, sheet: &str) -> Node {
    schema
        .create_node(
            "embed",
            Attrs::from_iter([("data-sheet", AttrValue::from(sheet))]),
            Fragment::empty(),
        )
        .unwrap()
}

fn doc(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

fn para(schema: &Schema, text: &str) -> Node {
    schema
        .branch("paragraph", Fragment::from_node(schema.text(text).unwrap()))
        .unwrap()
}

/// A fallback that writes a table naming the sheet.
fn sheet_table(node: &Node) -> Option<String> {
    let sheet = node.attrs().get_str("data-sheet")?;
    Some(format!("<table><tr><td>sheet {sheet}</td></tr></table>"))
}

#[test]
fn the_export_writes_the_apps_markup_inside_the_embeds_element() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![para(&schema, "a"), embed(&schema, "s1"), para(&schema, "b")],
    );
    let bare = node_to_html(&d);
    {
        let _guard = set_html_fallback(sheet_table);
        let html = node_to_html(&d);
        assert_eq!(
            html,
            r#"<p>a</p><div data-pm-type="embed" data-sheet="s1"><table><tr><td>sheet s1</td></tr></table></div><p>b</p>"#
        );
        // Read back by rinch it is the same document: the element is the embed,
        // and what the app wrote inside it is not content.
        assert_eq!(
            slice_from_html(&schema, &html).unwrap().content,
            d.content().clone()
        );

        // The copy-out carries it too, beside the clipboard mark, and pastes
        // back as the same embed.
        let slice = Slice::new(Fragment::from_node(embed(&schema, "s2")), 0, 0);
        let copied = slice_to_html(&slice);
        assert!(copied.contains("sheet s2</td>"), "{copied}");
        assert_eq!(
            slice_from_pasted_html(&schema, &copied).unwrap().content,
            slice.content
        );
    }
    // The guard dropped, the export is bare again.
    assert_eq!(node_to_html(&d), bare);
    assert!(!bare.contains("<table>"), "{bare}");
}

#[test]
fn a_fallback_that_answers_none_leaves_the_element_empty() {
    let schema = Schema::starter_kit();
    let d = doc(&schema, vec![embed(&schema, "s1")]);
    let _guard = set_html_fallback(|_| None);
    assert_eq!(
        node_to_html(&d),
        r#"<div data-pm-type="embed" data-sheet="s1"></div>"#
    );
}

/// The fallback is asked only about embeds, not about every node.
#[test]
fn the_fallback_is_asked_only_about_embeds() {
    let schema = Schema::starter_kit();
    let d = doc(
        &schema,
        vec![
            para(&schema, "a"),
            schema.branch("horizontal_rule", Fragment::empty()).unwrap(),
            embed(&schema, "s1"),
        ],
    );
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = asked.clone();
    let _guard = set_html_fallback(move |n: &Node| {
        seen.borrow_mut().push(n.type_name().to_string());
        Some("<i>x</i>".to_string())
    });
    let html = node_to_html(&d);
    assert_eq!(*asked.borrow(), ["embed"]);
    assert!(html.starts_with("<p>a</p><hr>"), "{html}");
}

/// A later registration wins while it lives; dropping it brings the earlier
/// one back, and dropping the earlier one first never removes the later one.
#[test]
fn registrations_stack_and_release_by_their_own_guard() {
    let schema = Schema::starter_kit();
    let d = doc(&schema, vec![embed(&schema, "s1")]);
    let inner = |html: &str| {
        html.strip_prefix(r#"<div data-pm-type="embed" data-sheet="s1">"#)
            .and_then(|r| r.strip_suffix("</div>"))
            .unwrap()
            .to_string()
    };
    let first = set_html_fallback(|_| Some("first".to_string()));
    let second = set_html_fallback(|_| Some("second".to_string()));
    assert_eq!(inner(&node_to_html(&d)), "second");
    drop(second);
    assert_eq!(inner(&node_to_html(&d)), "first");
    let second = set_html_fallback(|_| Some("second".to_string()));
    drop(first);
    assert_eq!(inner(&node_to_html(&d)), "second");
    drop(second);
    assert_eq!(inner(&node_to_html(&d)), "");
}

/// A fallback may itself export HTML (it runs with nothing borrowed).
#[test]
fn a_fallback_may_export_html_itself() {
    let schema = Schema::starter_kit();
    let d = doc(&schema, vec![embed(&schema, "s1")]);
    let s2 = schema.clone();
    let _guard = set_html_fallback(move |_| Some(node_to_html(&para(&s2, "nested"))));
    assert_eq!(
        node_to_html(&d),
        r#"<div data-pm-type="embed" data-sheet="s1"><p>nested</p></div>"#
    );
}

/// Markdown is not an HTML export: it writes the bare element, so the document
/// reads back the same whatever the app's markup holds (a blank line in it would
/// end the HTML block).
#[cfg(feature = "markdown")]
#[test]
fn markdown_writes_the_bare_element() {
    use rinch_editor_core::serialize::{doc_from_markdown_strict, doc_to_markdown};
    let schema = Schema::starter_kit();
    let d = doc(&schema, vec![embed(&schema, "s1"), para(&schema, "b")]);
    let _guard = set_html_fallback(|_| Some("<p>one</p>\n\n<p>two</p>".to_string()));
    let md = doc_to_markdown(&d);
    assert!(!md.contains("one"), "{md}");
    assert_eq!(doc_from_markdown_strict(&schema, &md).unwrap(), d);
}
