//! End-to-end serialization round-trip tests (M3) — the acceptance fixtures for
//! the structural #59 fix.
//!
//! These port the spirit of the old `rinch-editor` `from_block_data` round-trip
//! suite (`serialization.rs`, `roundtrip_tests.rs`) to the new total, schema-driven
//! pipeline, exercising the durable `DocNode`/JSON wire shape, HTML, and markdown
//! through the **public** API only. The recurring assertion is that content
//! survives the full persistence cycle with no mark, node, or attribute dropped.
//!
//! Gated on `serde` + `markdown` so `cargo test` with no features still builds an
//! empty (passing) test binary.
#![cfg(all(feature = "serde", feature = "markdown"))]

use rinch_editor_core::serialize::{
    DocMark, DocNode, JsonAttr, doc_from_markdown, doc_to_markdown, node_to_html, slice_from_html,
};
use rinch_editor_core::{Node, Schema};
use std::collections::BTreeMap;

// ─── DocNode fixture builders (the wire shape's fields are public) ────────────

fn attr_map(pairs: &[(&str, JsonAttr)]) -> BTreeMap<String, JsonAttr> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn node(ty: &str, attrs: BTreeMap<String, JsonAttr>, content: Vec<DocNode>) -> DocNode {
    DocNode {
        node_type: ty.to_string(),
        attrs,
        content,
        text: None,
        marks: Vec::new(),
    }
}

fn text(s: &str, marks: Vec<DocMark>) -> DocNode {
    DocNode {
        node_type: "text".to_string(),
        attrs: BTreeMap::new(),
        content: Vec::new(),
        text: Some(s.to_string()),
        marks,
    }
}

fn mark(ty: &str) -> DocMark {
    DocMark {
        mark_type: ty.to_string(),
        attrs: BTreeMap::new(),
    }
}

fn mark_attr(ty: &str, pairs: &[(&str, JsonAttr)]) -> DocMark {
    DocMark {
        mark_type: ty.to_string(),
        attrs: attr_map(pairs),
    }
}

fn para(content: Vec<DocNode>) -> DocNode {
    node("paragraph", BTreeMap::new(), content)
}

fn doc(blocks: Vec<DocNode>) -> DocNode {
    node("doc", BTreeMap::new(), blocks)
}

/// Round-trip a fixture through the model and through a JSON string, asserting the
/// model value and the wire value are both stable (lossless). Returns the model.
fn assert_stable(schema: &Schema, fixture: &DocNode) -> Node {
    let model = schema.node_from_doc(fixture).expect("deserialize fixture");
    let wire1 = model.to_doc().expect("serialize model");
    let json = serde_json::to_string(&wire1).expect("to json");
    let wire2: DocNode = serde_json::from_str(&json).expect("from json");
    assert_eq!(
        wire1, wire2,
        "wire shape not stable across a JSON round-trip"
    );
    let model2 = schema.node_from_doc(&wire2).expect("re-deserialize");
    assert_eq!(model, model2, "model not stable across a JSON round-trip");
    model
}

fn mark_names(text_node: &Node) -> Vec<String> {
    text_node
        .marks()
        .iter()
        .map(|m| m.type_name().to_string())
        .collect()
}

// ─── Mark preservation (the #59 class) ───────────────────────────────────────

#[test]
fn single_bold_run_survives() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![text("bold text", vec![mark("bold")])])]);
    let model = assert_stable(&s, &fixture);
    let run = model.child(0).child(0);
    assert_eq!(run.text(), Some("bold text"));
    assert_eq!(mark_names(run), vec!["bold"]);
}

#[test]
fn plain_bold_italic_plain_sequence_survives() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![
        text("start ", vec![]),
        text("bold", vec![mark("bold")]),
        text(" middle ", vec![]),
        text("italic", vec![mark("italic")]),
        text(" end", vec![]),
    ])]);
    let model = assert_stable(&s, &fixture);
    let para = model.child(0);
    assert_eq!(para.child_count(), 5);
    assert_eq!(mark_names(para.child(1)), vec!["bold"]);
    assert_eq!(mark_names(para.child(3)), vec!["italic"]);
}

#[test]
fn nested_marks_survive() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![text(
        "bold+italic",
        vec![mark("bold"), mark("italic")],
    )])]);
    let model = assert_stable(&s, &fixture);
    let names = mark_names(model.child(0).child(0));
    assert!(names.contains(&"bold".to_string()), "{names:?}");
    assert!(names.contains(&"italic".to_string()), "{names:?}");
}

#[test]
fn all_simple_mark_types_survive() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![
        text("b", vec![mark("bold")]),
        text("i", vec![mark("italic")]),
        text("u", vec![mark("underline")]),
        text("s", vec![mark("strike")]),
        text("c", vec![mark("code")]),
        text("sub", vec![mark("subscript")]),
        text("sup", vec![mark("superscript")]),
    ])]);
    let model = assert_stable(&s, &fixture);
    let para = model.child(0);
    for (i, expected) in [
        "bold",
        "italic",
        "underline",
        "strike",
        "code",
        "subscript",
        "superscript",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(mark_names(para.child(i)), vec![expected.to_string()]);
    }
}

#[test]
fn marks_with_attrs_survive() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![
        text(
            "link",
            vec![mark_attr(
                "link",
                &[("href", JsonAttr::Str("https://example.com".into()))],
            )],
        ),
        text(
            "hi",
            vec![mark_attr(
                "highlight",
                &[("color", JsonAttr::Str("yellow".into()))],
            )],
        ),
        text(
            "red",
            vec![mark_attr(
                "text_color",
                &[("color", JsonAttr::Str("#f00".into()))],
            )],
        ),
    ])]);
    let model = assert_stable(&s, &fixture);
    let para = model.child(0);
    assert_eq!(
        para.child(0).marks()[0].attrs.get_str("href"),
        Some("https://example.com")
    );
    assert_eq!(
        para.child(1).marks()[0].attrs.get_str("color"),
        Some("yellow")
    );
    assert_eq!(
        para.child(2).marks()[0].attrs.get_str("color"),
        Some("#f00")
    );
}

// ─── Block types & nesting ───────────────────────────────────────────────────

#[test]
fn mixed_and_nested_block_types_survive() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![
        node(
            "heading",
            attr_map(&[("level", JsonAttr::Int(1))]),
            vec![text("Title", vec![])],
        ),
        para(vec![text("Intro.", vec![])]),
        node(
            "blockquote",
            BTreeMap::new(),
            vec![para(vec![text("Quote.", vec![])])],
        ),
        node(
            "bullet_list",
            BTreeMap::new(),
            vec![
                node(
                    "list_item",
                    BTreeMap::new(),
                    vec![para(vec![text("Item A", vec![])])],
                ),
                node(
                    "list_item",
                    BTreeMap::new(),
                    vec![para(vec![text("Item B", vec![])])],
                ),
            ],
        ),
        node(
            "ordered_list",
            attr_map(&[("start", JsonAttr::Int(1))]),
            vec![node(
                "list_item",
                BTreeMap::new(),
                vec![para(vec![text("First", vec![])])],
            )],
        ),
        node(
            "code_block",
            attr_map(&[("language", JsonAttr::Str("rust".into()))]),
            vec![text("fn main() {}", vec![])],
        ),
        node("horizontal_rule", BTreeMap::new(), vec![]),
    ]);
    let model = assert_stable(&s, &fixture);
    let kinds: Vec<&str> = model
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    assert_eq!(
        kinds,
        [
            "heading",
            "paragraph",
            "blockquote",
            "bullet_list",
            "ordered_list",
            "code_block",
            "horizontal_rule",
        ]
    );
    // nested list_item > paragraph survives
    let list = model.child(3);
    assert_eq!(list.child(0).type_name(), "list_item");
    assert_eq!(list.child(0).child(0).type_name(), "paragraph");
}

#[test]
fn atoms_image_and_hard_break_survive() {
    let s = Schema::starter_kit();
    let mut img_attrs = attr_map(&[
        ("src", JsonAttr::Str("a.png".into())),
        ("alt", JsonAttr::Str("logo".into())),
    ]);
    img_attrs.insert("title".into(), JsonAttr::Str(String::new()));
    let fixture = doc(vec![para(vec![
        text("before", vec![]),
        node("image", img_attrs, vec![]),
        node("hard_break", BTreeMap::new(), vec![]),
        text("after", vec![]),
    ])]);
    let model = assert_stable(&s, &fixture);
    let para = model.child(0);
    assert_eq!(para.child(1).type_name(), "image");
    assert_eq!(para.child(1).attrs().get_str("src"), Some("a.png"));
    assert_eq!(para.child(2).type_name(), "hard_break");
}

// ─── Unicode ─────────────────────────────────────────────────────────────────

#[test]
fn unicode_survives_round_trip() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![
        para(vec![text("Hello 🌍 world 🎉!", vec![])]),
        para(vec![text("café résumé naïve", vec![])]),
        para(vec![text("你好世界", vec![])]),
        para(vec![
            text("café ", vec![]),
            text("résumé", vec![mark("bold")]),
            text(" 🎉", vec![]),
        ]),
    ]);
    let model = assert_stable(&s, &fixture);
    assert_eq!(model.child(0).child(0).text(), Some("Hello 🌍 world 🎉!"));
    assert_eq!(model.child(2).child(0).text(), Some("你好世界"));
}

// ─── Totality: unknown types / missing required are hard errors ───────────────

#[test]
fn unknown_node_type_rejected() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![node("blink", BTreeMap::new(), vec![])]);
    assert!(s.node_from_doc(&fixture).is_err());
}

#[test]
fn missing_required_image_src_rejected() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![para(vec![node("image", BTreeMap::new(), vec![])])]);
    assert!(s.node_from_doc(&fixture).is_err());
}

#[test]
fn special_html_characters_in_text_are_escaped_then_recovered() {
    let s = Schema::starter_kit();
    let raw = r#"Angle <brackets>, "quotes", & ampersands"#;
    let fixture = doc(vec![para(vec![text(raw, vec![])])]);
    let model = schema_doc(&s, &fixture);
    // Through HTML the special characters must survive (escaped, then decoded).
    let html = node_to_html(&model);
    assert!(html.contains("&lt;brackets&gt;"), "{html}");
    assert!(html.contains("&amp; ampersands"), "{html}");
    let slice = slice_from_html(&s, &html).unwrap();
    assert_eq!(slice.content.child(0).child(0).text(), Some(raw));
}

fn schema_doc(s: &Schema, fixture: &DocNode) -> Node {
    s.node_from_doc(fixture).unwrap()
}

// ─── Cross-format: HTML and markdown on a realistic doc ───────────────────────

#[test]
fn html_round_trip_for_representable_doc() {
    let s = Schema::starter_kit();
    let fixture = doc(vec![
        node(
            "heading",
            attr_map(&[("level", JsonAttr::Int(2))]),
            vec![text("Title", vec![])],
        ),
        para(vec![
            text("a ", vec![]),
            text("bold", vec![mark("bold")]),
            text(" and ", vec![]),
            text("italic", vec![mark("italic")]),
        ]),
        node(
            "bullet_list",
            BTreeMap::new(),
            vec![
                node(
                    "list_item",
                    BTreeMap::new(),
                    vec![para(vec![text("one", vec![])])],
                ),
                node(
                    "list_item",
                    BTreeMap::new(),
                    vec![para(vec![text("two", vec![])])],
                ),
            ],
        ),
    ]);
    let model = schema_doc(&s, &fixture);
    let html = node_to_html(&model);
    let slice = slice_from_html(&s, &html).unwrap();
    // Structural survival: same block kinds, marks preserved.
    let blocks = &slice.content;
    let kinds: Vec<&str> = blocks.children().iter().map(Node::type_name).collect();
    assert_eq!(kinds, ["heading", "paragraph", "bullet_list"]);
    let names: Vec<Vec<String>> = blocks
        .child(1)
        .content()
        .children()
        .iter()
        .map(mark_names)
        .collect();
    assert!(names.iter().any(|m| m == &vec!["bold".to_string()]));
    assert!(names.iter().any(|m| m == &vec!["italic".to_string()]));
}

#[test]
fn markdown_round_trip_for_representable_doc() {
    let s = Schema::starter_kit();
    let md = "# Heading\n\nThis is **bold** and *italic* and a [link](https://x.io).\n\n- one\n- two\n\n> a quote";
    let model = doc_from_markdown(&s, md).unwrap();
    let out = doc_to_markdown(&model);
    assert!(out.contains("# Heading"), "{out}");
    assert!(out.contains("**bold**"), "{out}");
    assert!(out.contains("*italic*"), "{out}");
    assert!(out.contains("[link](https://x.io)"), "{out}");
    assert!(out.contains("- one"), "{out}");
    assert!(out.contains("> a quote"), "{out}");
    // and the model is a valid DocNode (lossless to JSON)
    assert_stable(&s, &model.to_doc().unwrap());
}
