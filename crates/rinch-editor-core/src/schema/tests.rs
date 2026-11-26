use super::*;

#[test]
fn starter_kit_has_all_nodes() {
    let schema = Schema::starter_kit();
    let expected_nodes = [
        "doc",
        "paragraph",
        "heading",
        "blockquote",
        "code_block",
        "bullet_list",
        "ordered_list",
        "list_item",
        "task_list",
        "task_item",
        "horizontal_rule",
        "embed",
        "hard_break",
        "text",
        "image",
    ];
    for name in &expected_nodes {
        assert!(schema.has_node(name), "missing node: {}", name);
    }
}

#[test]
fn starter_kit_has_all_marks() {
    let schema = Schema::starter_kit();
    let expected_marks = [
        "bold",
        "italic",
        "underline",
        "strike",
        "code",
        "link",
        "highlight",
        "text_color",
        "subscript",
        "superscript",
    ];
    for name in &expected_marks {
        assert!(schema.has_mark(name), "missing mark: {}", name);
    }
}

#[test]
fn starter_kit_top_node_is_doc() {
    let schema = Schema::starter_kit();
    assert_eq!(schema.top_node, "doc");
}

#[test]
fn node_lookup() {
    let schema = Schema::starter_kit();
    let para = schema.node("paragraph").unwrap();
    assert_eq!(para.name, "paragraph");
    assert_eq!(para.content, Some("inline*".to_string()));
    assert!(schema.node("nonexistent").is_none());
}

#[test]
fn mark_lookup() {
    let schema = Schema::starter_kit();
    let bold = schema.mark("bold").unwrap();
    assert_eq!(bold.name, "bold");
    assert!(schema.mark("nonexistent").is_none());
}

#[test]
fn nodes_in_group_block() {
    let schema = Schema::starter_kit();
    let blocks = schema.nodes_in_group("block");
    assert!(blocks.contains(&"paragraph"));
    assert!(blocks.contains(&"heading"));
    assert!(blocks.contains(&"blockquote"));
    assert!(blocks.contains(&"code_block"));
    assert!(!blocks.contains(&"text"));
    assert!(!blocks.contains(&"doc"));
}

#[test]
fn nodes_in_group_inline() {
    let schema = Schema::starter_kit();
    let inlines = schema.nodes_in_group("inline");
    assert!(inlines.contains(&"text"));
    assert!(inlines.contains(&"hard_break"));
    assert!(inlines.contains(&"image"));
    assert!(!inlines.contains(&"paragraph"));
}

#[test]
fn allows_mark_on_paragraph() {
    let schema = Schema::starter_kit();
    assert!(schema.allows_mark("paragraph", "bold"));
    assert!(schema.allows_mark("paragraph", "italic"));
    assert!(schema.allows_mark("paragraph", "link"));
}

#[test]
fn code_block_disallows_marks() {
    let schema = Schema::starter_kit();
    assert!(!schema.allows_mark("code_block", "bold"));
    assert!(!schema.allows_mark("code_block", "italic"));
    assert!(!schema.allows_mark("code_block", "link"));
}

#[test]
fn allows_mark_nonexistent() {
    let schema = Schema::starter_kit();
    assert!(!schema.allows_mark("paragraph", "nonexistent"));
    assert!(!schema.allows_mark("nonexistent", "bold"));
}

#[test]
fn allows_child_doc_paragraph() {
    let schema = Schema::starter_kit();
    assert!(schema.allows_child("doc", "paragraph"));
    assert!(schema.allows_child("doc", "heading"));
    assert!(schema.allows_child("doc", "blockquote"));
    assert!(!schema.allows_child("doc", "text"));
}

#[test]
fn allows_child_paragraph_text() {
    let schema = Schema::starter_kit();
    assert!(schema.allows_child("paragraph", "text"));
    assert!(schema.allows_child("paragraph", "hard_break"));
    assert!(schema.allows_child("paragraph", "image"));
    assert!(!schema.allows_child("paragraph", "paragraph"));
}

#[test]
fn allows_child_list() {
    let schema = Schema::starter_kit();
    assert!(schema.allows_child("bullet_list", "list_item"));
    assert!(!schema.allows_child("bullet_list", "paragraph"));
    assert!(schema.allows_child("list_item", "paragraph"));
}

#[test]
fn allows_child_leaf_node() {
    let schema = Schema::starter_kit();
    assert!(!schema.allows_child("horizontal_rule", "text"));
    assert!(!schema.allows_child("hard_break", "text"));
}

#[test]
fn builder_works() {
    let schema = Schema::builder()
        .node("doc", NodeSpec::builder("doc").content("block+").build())
        .node("paragraph", NodeSpec::block("paragraph"))
        .mark("bold", MarkSpec::simple("bold"))
        .top_node("doc")
        .build();

    assert!(schema.has_node("doc"));
    assert!(schema.has_node("paragraph"));
    assert!(schema.has_mark("bold"));
    assert_eq!(schema.top_node, "doc");
}

#[test]
fn matches_content_star_empty() {
    let schema = Schema::starter_kit();
    assert!(matches_content(&schema, "inline*", &[]));
    assert!(matches_content(&schema, "block*", &[]));
}

#[test]
fn matches_content_star_with_items() {
    let schema = Schema::starter_kit();
    assert!(matches_content(&schema, "inline*", &["text"]));
    assert!(matches_content(
        &schema,
        "inline*",
        &["text", "hard_break", "text"]
    ));
}

#[test]
fn matches_content_plus_requires_one() {
    let schema = Schema::starter_kit();
    assert!(!matches_content(&schema, "block+", &[]));
    assert!(matches_content(&schema, "block+", &["paragraph"]));
    assert!(matches_content(
        &schema,
        "block+",
        &["paragraph", "heading"]
    ));
}

#[test]
fn matches_content_specific_type() {
    let schema = Schema::starter_kit();
    assert!(matches_content(&schema, "list_item+", &["list_item"]));
    assert!(matches_content(
        &schema,
        "list_item+",
        &["list_item", "list_item"]
    ));
    assert!(!matches_content(&schema, "list_item+", &["paragraph"]));
}

#[test]
fn matches_content_text_star() {
    let schema = Schema::starter_kit();
    assert!(matches_content(&schema, "text*", &[]));
    assert!(matches_content(&schema, "text*", &["text"]));
    assert!(!matches_content(&schema, "text*", &["paragraph"]));
}

#[test]
fn matches_content_wrong_group() {
    let schema = Schema::starter_kit();
    assert!(!matches_content(&schema, "block+", &["text"]));
    assert!(!matches_content(&schema, "inline*", &["paragraph"]));
}

#[test]
fn matches_content_compound() {
    let schema = Schema::starter_kit();
    assert!(matches_content(&schema, "paragraph block*", &["paragraph"]));
    assert!(matches_content(
        &schema,
        "paragraph block*",
        &["paragraph", "heading"]
    ));
    assert!(!matches_content(&schema, "paragraph block*", &["heading"]));
}

#[test]
fn heading_has_level_attr() {
    let schema = Schema::starter_kit();
    let heading = schema.node("heading").unwrap();
    assert!(heading.attrs.contains_key("level"));
    assert_eq!(heading.attrs["level"].default, Some(AttrValue::Int(1)));
}

#[test]
fn image_has_required_src() {
    let schema = Schema::starter_kit();
    let image = schema.node("image").unwrap();
    assert!(image.attrs["src"].required);
    assert!(!image.attrs["alt"].required);
    assert_eq!(image.attrs["width"].default, Some(AttrValue::Int(0)));
    assert!(
        image.data_attrs,
        "the image keeps an app's data-* attributes"
    );
    assert!(schema.node("paragraph").unwrap().data_attrs);
    assert!(schema.node("heading").unwrap().data_attrs);
    assert!(!schema.node("code_block").unwrap().data_attrs);
}

#[test]
fn link_has_required_href() {
    let schema = Schema::starter_kit();
    let link = schema.mark("link").unwrap();
    assert!(link.attrs["href"].required);
    assert!(!link.attrs["title"].required);
    assert!(
        !link.inclusive,
        "a link does not extend over text typed after it"
    );
    for name in ["bold", "italic", "underline", "strike", "code", "highlight"] {
        assert!(
            schema.mark(name).unwrap().inclusive,
            "{name} stays inclusive"
        );
    }
}

#[test]
fn code_mark_excludes_formatting() {
    let schema = Schema::starter_kit();
    let code = schema.mark("code").unwrap();
    assert!(code.excludes_mark("bold"));
    assert!(code.excludes_mark("italic"));
    assert!(code.excludes_mark("underline"));
    assert!(code.excludes_mark("strike"));
    assert!(code.excludes_mark("link"));
    assert!(!code.excludes_mark("highlight"));
}

#[test]
fn subscript_superscript_mutual_exclusion() {
    let schema = Schema::starter_kit();
    let sub = schema.mark("subscript").unwrap();
    let sup = schema.mark("superscript").unwrap();
    assert!(sub.excludes_mark("superscript"));
    assert!(sup.excludes_mark("subscript"));
    assert!(!sub.excludes_mark("bold"));
    assert!(!sup.excludes_mark("bold"));
}
