//! An image keeps an app's `data-*` attributes (HTML's custom data attributes):
//! through the model's attribute validation, copy and paste (HTML), the durable
//! `DocNode` shape and Markdown's HTML tables. A node keeps them only when its
//! schema opts in (`NodeSpec::data_attrs`); the starter kit's `image` does.
//!
//! What is kept is a *valid* custom data attribute name (`data-` + at least one
//! character, no ASCII uppercase, XML-compatible) that rinch does not reserve
//! for itself (`data-pm-*`, `data-rid`, `data-on*`, …), with a string value.

use rinch_editor_core::serialize::{node_to_html, slice_from_html};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Node, Schema};

fn image(schema: &Schema, attrs: &[(&str, AttrValue)]) -> Node {
    let mut all = vec![("src", AttrValue::from("a.png"))];
    all.extend(attrs.iter().cloned());
    schema
        .create_node("image", Attrs::from_iter(all), Fragment::empty())
        .unwrap()
}

fn pasted_image(html: &str) -> Node {
    let schema = Schema::starter_kit();
    let slice = slice_from_html(&schema, html).unwrap();
    fn find(n: &Node) -> Option<Node> {
        if n.type_name() == "image" {
            return Some(n.clone());
        }
        n.content().children().iter().find_map(find)
    }
    slice.content.children().iter().find_map(find).expect("an image")
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
        ("board", AttrValue::from("b1")),
        ("onerror", AttrValue::from("alert(1)")),
        ("data-", AttrValue::from("no name")),
        ("data-Ref", AttrValue::from("uppercase")),
        ("data-a:b", AttrValue::from("colon")),
        ("data-a b", AttrValue::from("space")),
        ("data-pm-type", AttrValue::from("paragraph")),
        ("data-rid", AttrValue::from("7")),
        ("data-onclick", AttrValue::from("x")),
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

/// The starter kit's image declares no `board` any more.
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
        r#"<p><img src="a.png" data-ref="r1" data-annotation-id="" DATA-UPPER="u" data-pm-type="paragraph" data-rid="12" data-onclick="x" data-nofocus="" onerror="alert(1)"></p>"#,
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
        let para = schema.branch("paragraph", Fragment::from_node(img)).unwrap();
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
    /// attribute (`board`, which builds between #1500 and this change declared)
    /// is dropped on load, as any unknown attribute is.
    #[test]
    fn an_unknown_image_attr_is_dropped_on_load() {
        let schema = Schema::starter_kit();
        let json = r#"{"type":"doc","content":[{"type":"paragraph","content":[
            {"type":"image","attrs":{"src":"a.png","board":"b1","data-ref":"r1"}}]}]}"#;
        let wire: rinch_editor_core::serialize::DocNode = serde_json::from_str(json).unwrap();
        let doc = schema.node_from_doc(&wire).unwrap();
        let img = doc.child(0).child(0);
        assert_eq!(img.attrs().get("board"), None);
        assert_eq!(img.attrs().get_str("data-ref"), Some("r1"));
    }
}

#[cfg(feature = "markdown")]
mod markdown {
    use super::*;
    use rinch_editor_core::serialize::{
        MarkdownError, doc_from_markdown_strict, doc_to_markdown,
    };

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
