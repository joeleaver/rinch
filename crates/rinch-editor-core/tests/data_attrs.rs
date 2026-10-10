//! An image keeps an app's `data-*` attributes (HTML's custom data attributes):
//! through the model's attribute validation, copy and paste (HTML), the durable
//! `DocNode` shape and Markdown's HTML tables. A node keeps them only when its
//! schema opts in (`NodeSpec::data_attrs`); the starter kit's `image` does.
//!
//! What is kept is a *valid* custom data attribute name (`data-` + at least one
//! character, no ASCII uppercase, XML-compatible) that rinch does not reserve
//! for itself (`data-pm-*`, `data-rid`, `data-on*`, …), with a string value.

use rinch_editor_core::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS};
use rinch_editor_core::serialize::{
    CLIPBOARD_MARK, node_to_html, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Schema};

fn image(schema: &Schema, attrs: &[(&str, AttrValue)]) -> Node {
    let mut all = vec![("src", AttrValue::from("a.png"))];
    all.extend(attrs.iter().cloned());
    schema
        .create_node("image", Attrs::from_iter(all), Fragment::empty())
        .unwrap()
}

fn pasted_image(html: &str) -> Node {
    first_image(
        &slice_from_html(&Schema::starter_kit(), html)
            .unwrap()
            .content,
    )
}

fn first_image(content: &Fragment) -> Node {
    fn find(n: &Node) -> Option<Node> {
        if n.type_name() == "image" {
            return Some(n.clone());
        }
        n.content().children().iter().find_map(find)
    }
    content.children().iter().find_map(find).expect("an image")
}

/// The attribute validation keeps an opted-in node's app data attributes, and
/// only those: an undeclared attribute that is no data attribute, a name that
/// is not a valid custom data attribute, a name rinch reserves and a value that
/// is not a string are all dropped.
#[test]
fn an_image_keeps_valid_app_data_attrs_and_nothing_else() {
    let schema = Schema::starter_kit();
    let nt = schema.node_type("image").unwrap();
    let provided = Attrs::from_iter([
        ("src", AttrValue::from("a.png")),
        ("data-ref", AttrValue::from("r1")),
        ("data-annotation-id", AttrValue::from("")),
        ("data-é", AttrValue::from("accented")),
        // Not kept:
        ("caption", AttrValue::from("undeclared")),
        ("onerror", AttrValue::from("alert(1)")),
        ("data-", AttrValue::from("no name")),
        ("data-Ref", AttrValue::from("uppercase")),
        ("data-a:b", AttrValue::from("colon")),
        ("data-a b", AttrValue::from("space")),
        ("data-pm-type", AttrValue::from("paragraph")),
        ("data-rid", AttrValue::from("7")),
        ("data-onmousedown", AttrValue::from("x")),
        ("data-count", AttrValue::Int(3)),
    ]);
    let kept = nt.compute_attrs(&provided).unwrap();
    let names: Vec<&str> = kept.iter().map(|(k, _)| k).collect();
    assert_eq!(
        names,
        ["data-annotation-id", "data-ref", "data-é", "src"],
        "{kept:?}"
    );
    assert_eq!(kept.get_str("data-ref"), Some("r1"));
    assert_eq!(kept.get_str("data-annotation-id"), Some(""));
}

/// A node whose schema does not opt in keeps no data attribute.
#[test]
fn a_paragraph_keeps_no_data_attr() {
    let schema = Schema::starter_kit();
    let nt = schema.node_type("paragraph").unwrap();
    let kept = nt
        .compute_attrs(&Attrs::from_iter([("data-ref", AttrValue::from("r1"))]))
        .unwrap();
    assert!(kept.is_empty(), "{kept:?}");
}

/// The starter kit's image declares `src`, `alt`, `title` and `width`, and
/// nothing an app would otherwise have to ask for: its own data goes in `data-*`.
#[test]
fn the_starter_kits_image_declares_src_alt_title_width() {
    let schema = Schema::starter_kit();
    let spec = schema.node("image").unwrap();
    let names: Vec<&str> = spec.attrs.keys().map(String::as_str).collect();
    assert_eq!(names, ["alt", "src", "title", "width"]);
}

/// Copy-out writes every app data attribute of an image as itself, empty ones
/// included, and nothing undeclared that is not one.
#[test]
fn the_html_writer_writes_an_images_data_attrs() {
    let schema = Schema::starter_kit();
    let img = image(
        &schema,
        &[
            ("data-ref", AttrValue::from("r\"1<")),
            ("data-annotation-id", AttrValue::from("")),
            ("data-rid", AttrValue::from("9")),
            ("onerror", AttrValue::from("alert(1)")),
        ],
    );
    let html = node_to_html(&img);
    assert!(html.contains(r#" data-ref="r&quot;1&lt;""#), "{html}");
    assert!(html.contains(r#" data-annotation-id="""#), "{html}");
    assert!(!html.contains("data-rid"), "{html}");
    assert!(!html.contains("onerror"), "{html}");
}

/// Paste reads every app data attribute on an `<img>` back, and never one
/// rinch reserves: a pasted `data-pm-*`, `data-rid` or `data-on*` is markup
/// rinch's own view writes, and must not come back into a document.
#[test]
fn the_html_reader_reads_an_images_data_attrs_and_no_reserved_one() {
    let img = pasted_image(
        r#"<p><img src="a.png" data-ref="r1" data-annotation-id="" DATA-UPPER="u" data-pm-type="paragraph" data-rid="12" data-onmousedown="x" data-nofocus="" data-tcm-item="1" data-block-index="0" data-cursor-pos="1" data-text-sel-start="0" data-user-rid="3" onerror="alert(1)"></p>"#,
    );
    let names: Vec<&str> = img.attrs().iter().map(|(k, _)| k).collect();
    // The tokenizer lowercases attribute names, as HTML does.
    assert_eq!(
        names,
        ["data-annotation-id", "data-ref", "data-upper", "src"],
        "{:?}",
        img.attrs()
    );
}

/// What the writer writes the reader reads back: a round trip through HTML.
#[test]
fn an_images_data_attrs_round_trip_through_html() {
    let schema = Schema::starter_kit();
    let img = image(
        &schema,
        &[
            ("data-ref", AttrValue::from("r1")),
            ("data-annotation-id", AttrValue::from("a&b")),
            ("width", AttrValue::Int(320)),
        ],
    );
    let para = schema
        .branch("paragraph", Fragment::from_node(img.clone()))
        .unwrap();
    let back = pasted_image(&node_to_html(&para));
    assert_eq!(back.attrs(), img.attrs());
}

#[cfg(feature = "serde")]
mod doc_json {
    use super::*;

    /// The durable shape carries an image's data attributes.
    #[test]
    fn an_images_data_attrs_survive_doc_node() {
        let schema = Schema::starter_kit();
        let img = image(&schema, &[("data-ref", AttrValue::from("r1"))]);
        let para = schema
            .branch("paragraph", Fragment::from_node(img))
            .unwrap();
        let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
        let wire = doc.to_doc().unwrap();
        let back = schema.node_from_doc(&wire).unwrap();
        assert_eq!(back, doc);
        assert_eq!(
            back.child(0).child(0).attrs().get_str("data-ref"),
            Some("r1")
        );
    }

    /// A stored image attribute the schema does not declare and that is no data
    /// attribute is dropped on load, as any unknown attribute is.
    #[test]
    fn an_unknown_image_attr_is_dropped_on_load() {
        let schema = Schema::starter_kit();
        let json = r#"{"type":"doc","content":[{"type":"paragraph","content":[
            {"type":"image","attrs":{"src":"a.png","caption":"undeclared","data-ref":"r1"}}]}]}"#;
        let wire: rinch_editor_core::serialize::DocNode = serde_json::from_str(json).unwrap();
        let doc = schema.node_from_doc(&wire).unwrap();
        let img = doc.child(0).child(0);
        assert_eq!(img.attrs().get("caption"), None);
        assert_eq!(img.attrs().get_str("data-ref"), Some("r1"));
    }
}

#[cfg(feature = "markdown")]
mod markdown {
    use super::*;
    use rinch_editor_core::serialize::{MarkdownError, doc_from_markdown_strict, doc_to_markdown};

    fn table(img: &str) -> String {
        format!(
            "<table>\n<tbody><tr><td colspan=\"2\">{img}</td></tr><tr><td>a</td><td>b</td></tr></tbody>\n</table>"
        )
    }

    /// An image in a table written as HTML keeps its data attributes, and the
    /// strict reader takes them back; a reserved one is a refusal, not a drop.
    #[test]
    fn an_image_in_an_html_table_keeps_its_data_attrs() {
        let schema = Schema::starter_kit();
        let md = table(r#"<img src="a.png" data-ref="r1" data-annotation-id="">"#);
        let d = doc_from_markdown_strict(&schema, &md).unwrap();
        let img = d.child(0).child(0).child(0).child(0).child(0);
        assert_eq!(img.attrs().get_str("data-ref"), Some("r1"));
        assert_eq!(img.attrs().get_str("data-annotation-id"), Some(""));
        let out = doc_to_markdown(&d);
        assert!(out.contains(r#"data-ref="r1""#), "{out}");
        for attr in [r#"data-pm-type="x""#, r#"data-rid="1""#] {
            let md = table(&format!(r#"<img src="a.png" {attr}>"#));
            match doc_from_markdown_strict(&schema, &md) {
                Err(MarkdownError::Unsupported { .. }) => {}
                other => panic!("{attr}: expected a refusal, got {other:?}"),
            }
        }
    }
}

fn image_para(schema: &Schema, attrs: &[(&str, AttrValue)]) -> rinch_editor_core::Slice {
    let img = image(schema, attrs);
    let para = schema
        .branch("paragraph", Fragment::from_node(img))
        .unwrap();
    rinch_editor_core::Slice::new(Fragment::from_node(para), 0, 0)
}

/// The default paste keeps an image's data attributes only from rinch's own
/// copy-out, which marks each element that carries them; a load
/// ([`slice_from_html`]) keeps them from any HTML, and an export
/// ([`node_to_html`]) writes no mark. The mark never reaches a document.
#[test]
fn the_default_paste_keeps_data_attrs_from_rinchs_own_copy_only() {
    let schema = Schema::starter_kit();
    let slice = image_para(&schema, &[("data-ref", AttrValue::from("r1"))]);
    let copied = slice_to_html(&slice);
    assert!(
        copied.contains(&format!(" {CLIPBOARD_MARK}=\"\"")),
        "{copied}"
    );
    let pasted = first_image(&slice_from_pasted_html(&schema, &copied).unwrap().content);
    assert_eq!(pasted.attrs().get_str("data-ref"), Some("r1"));
    assert_eq!(pasted.attrs().get(CLIPBOARD_MARK), None);
    // No data attribute, no mark.
    let plain = slice_to_html(&image_para(&schema, &[]));
    assert!(!plain.contains(CLIPBOARD_MARK), "{plain}");
    // An export is not a copy.
    assert!(!node_to_html(slice.content.child(0)).contains(CLIPBOARD_MARK));

    // Another application's markup: Vue's scoping attribute, React's id, a lazy
    // loader's `data-src`, Slack's `data-stringify-type`.
    let foreign = r#"<p><img src="a.png" data-v-7ba5bd90="" data-reactid=".0" data-src="lazy.png" data-stringify-type="emoji"></p>"#;
    let pasted = first_image(&slice_from_pasted_html(&schema, foreign).unwrap().content);
    assert_eq!(
        pasted.attrs().iter().map(|(k, _)| k).collect::<Vec<_>>(),
        ["src"]
    );
    // A load of the same markup keeps them.
    let loaded = first_image(&slice_from_html(&schema, foreign).unwrap().content);
    assert_eq!(loaded.attrs().len(), 5);
}

/// What the HTML reader keeps is bounded: the first `MAX_DATA_ATTRS` in
/// document order, a value past `MAX_DATA_ATTR_VALUE` bytes dropped whole, and
/// of two attributes with one name the first (as HTML keeps it).
#[test]
fn the_html_reader_bounds_what_an_image_keeps() {
    let mut html = String::from(r#"<p><img src="a.png" data-dup="first" data-dup="second""#);
    html.push_str(&format!(
        r#" data-big="{}""#,
        "x".repeat(MAX_DATA_ATTR_VALUE + 1)
    ));
    html.push_str(&format!(
        r#" data-fits="{}""#,
        "x".repeat(MAX_DATA_ATTR_VALUE)
    ));
    for i in 0..40 {
        html.push_str(&format!(r#" data-k{i:02}="v""#));
    }
    html.push_str("></p>");
    let img = pasted_image(&html);
    let data: Vec<&str> = img
        .attrs()
        .iter()
        .map(|(k, _)| k)
        .filter(|k| k.starts_with("data-"))
        .collect();
    assert_eq!(data.len(), MAX_DATA_ATTRS, "{data:?}");
    assert_eq!(img.attrs().get_str("data-dup"), Some("first"));
    assert_eq!(img.attrs().get("data-big"), None);
    assert!(img.attrs().get("data-fits").is_some());
    // Document order: `data-dup`, `data-fits`, then the first 30 of the rest.
    assert!(img.attrs().get("data-k29").is_some());
    assert_eq!(img.attrs().get("data-k30"), None);
}

/// Validation (`DocNode` load, `to_doc`) applies the same bound, in name order.
#[test]
fn validation_bounds_what_an_image_keeps() {
    let schema = Schema::starter_kit();
    let nt = schema.node_type("image").unwrap();
    let mut provided: Vec<(String, AttrValue)> = vec![("src".into(), AttrValue::from("a.png"))];
    provided.extend((0..40).map(|i| (format!("data-k{i:02}"), AttrValue::from("v"))));
    provided.push((
        "data-a-big".into(),
        AttrValue::from("x".repeat(MAX_DATA_ATTR_VALUE + 1)),
    ));
    let kept = nt.compute_attrs(&Attrs::from_iter(provided)).unwrap();
    assert_eq!(kept.len(), 1 + MAX_DATA_ATTRS);
    assert_eq!(kept.get("data-a-big"), None);
    assert!(kept.get("data-k31").is_some());
    assert_eq!(kept.get("data-k32"), None);
}

/// Only a leaf node type may keep data attributes: a container would lose them
/// on every copy and paste.
#[test]
#[should_panic(expected = "sets `data_attrs`")]
fn a_container_that_opts_in_is_refused() {
    use rinch_editor_core::NodeSpec;
    let _ = Schema::builder()
        .node("doc", NodeSpec::builder("doc").content("block+").build())
        .node(
            "paragraph",
            NodeSpec::builder("paragraph")
                .content("text*")
                .group("block")
                .data_attrs(true)
                .build(),
        )
        .node(
            "text",
            NodeSpec::builder("text").group("inline").inline().build(),
        )
        .build();
}

/// The bound is the model's, not only the serializers': a node built by
/// `create_node` (which validates nothing else) and one changed by a raw
/// `SetNodeAttrStep` hold only the data attributes HTML, `DocNode` and the view
/// keep. Other attributes are as they always were; a type that does not opt in
/// is untouched.
#[test]
fn the_model_holds_only_the_data_attrs_a_node_keeps() {
    use rinch_editor_core::{EditorState, SetNodeAttrStep};
    let schema = Schema::starter_kit();
    let mut attrs: Vec<(String, AttrValue)> = vec![
        ("src".into(), AttrValue::from("a.png")),
        (
            "caption".into(),
            AttrValue::from("undeclared, kept as before"),
        ),
        ("data-rid".into(), AttrValue::from("1")),
        ("data-n".into(), AttrValue::Int(3)),
    ];
    attrs.extend((0..40).map(|i| (format!("data-k{i:02}"), AttrValue::from("v"))));
    let img = schema
        .create_node("image", Attrs::from_iter(attrs), Fragment::empty())
        .unwrap();
    let data = |n: &Node| {
        n.attrs()
            .iter()
            .filter(|(k, _)| k.starts_with("data-"))
            .count()
    };
    assert_eq!(data(&img), MAX_DATA_ATTRS);
    assert_eq!(img.attrs().get("data-rid"), None);
    assert_eq!(img.attrs().get("data-n"), None);
    assert!(img.attrs().get("caption").is_some());

    // A step: a reserved name, an over-long value and one past the count are not
    // held; the rest of the image is.
    let small = image(&schema, &[("data-ref", AttrValue::from("r1"))]);
    let para = schema
        .branch("paragraph", Fragment::from_node(small))
        .unwrap();
    let doc = schema.branch("doc", Fragment::from_node(para)).unwrap();
    let state = EditorState::create(std::rc::Rc::new(schema.clone()), doc, vec![]);
    let mut tr = state.tr();
    for (name, value) in [
        ("data-pm-type".to_string(), "paragraph".to_string()),
        ("data-long".to_string(), "x".repeat(MAX_DATA_ATTR_VALUE + 1)),
        ("data-ok".to_string(), "fine".to_string()),
    ] {
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            name,
            AttrValue::from(value),
        )))
        .unwrap();
    }
    let next = state.apply(tr);
    let img = next.doc.child(0).child(0);
    assert_eq!(img.attrs().get_str("data-ok"), Some("fine"));
    assert_eq!(img.attrs().get_str("data-ref"), Some("r1"));
    assert_eq!(img.attrs().get("data-pm-type"), None);
    assert_eq!(img.attrs().get("data-long"), None);

    // A paragraph does not opt in: `create_node` keeps what it is given.
    let p = schema
        .create_node(
            "paragraph",
            Attrs::from_iter([("data-rid", AttrValue::from("1"))]),
            Fragment::empty(),
        )
        .unwrap();
    assert!(p.attrs().get("data-rid").is_some());
}
