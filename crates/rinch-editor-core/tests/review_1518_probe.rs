use rinch_editor_core::Schema;
use rinch_editor_core::serialize::{node_to_html, slice_from_html};

fn img_attrs(html: &str) -> Vec<(String, String)> {
    let schema = Schema::starter_kit();
    let s = slice_from_html(&schema, html).unwrap();
    let img = s.content.child(0).child(0).clone();
    img.attrs()
        .iter()
        .map(|(k, v)| (k.to_string(), format!("{v:?}")))
        .collect()
}

#[test]
fn dups_and_prefixes() {
    // HTML keeps the first of two attributes with one name.
    let dup = img_attrs(r#"<p><img src="a.png" data-a="first" data-a="second"></p>"#);
    assert!(format!("{dup:?}").contains("first"), "{dup:?}");
    assert!(!format!("{dup:?}").contains("second"), "{dup:?}");
    // `data-on…` is reserved by its exact names only: these are an app's.
    let on = img_attrs(
        r#"<p><img src="a.png" data-one="1" data-only="2" data-ontology="3" data-online="4"></p>"#,
    );
    for name in ["data-one", "data-only", "data-ontology", "data-online"] {
        assert!(format!("{on:?}").contains(name), "{name}: {on:?}");
    }
}

/// The opt-in on a node with no declared attrs (an `hr`, a `br`), which the
/// guide says HTML reads and writes: nothing in the PR's suite covers it.
#[test]
fn a_custom_hr_and_br_that_opt_in_keep_data_attrs_through_html() {
    use rinch_editor_core::{AttrValue, Attrs, Fragment, NodeSpec};
    let kit = Schema::starter_kit();
    let mut b = Schema::builder();
    for n in [
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
        "text",
        "table",
        "table_row",
        "table_cell",
        "table_header_cell",
        "image",
        "horizontal_rule",
        "hard_break",
    ] {
        let mut s = kit.node(n).unwrap().clone();
        if n == "horizontal_rule" || n == "hard_break" {
            s.data_attrs = true;
        }
        b = b.node(n, s);
    }
    for m in [
        "bold",
        "code",
        "highlight",
        "italic",
        "link",
        "strike",
        "subscript",
        "superscript",
        "text_color",
        "underline",
    ] {
        if let Some(s) = kit.mark(m) {
            b = b.mark(m, s.clone());
        }
    }
    let schema = b.build();
    let _ = NodeSpec::atom("x");
    let hr = schema
        .create_node(
            "horizontal_rule",
            Attrs::from_iter([("data-ref", AttrValue::from("r1"))]),
            Fragment::empty(),
        )
        .unwrap();
    assert_eq!(
        hr.attrs().get_str("data-ref"),
        Some("r1"),
        "validation keeps it"
    );
    let html = node_to_html(&hr);
    assert_eq!(html, r#"<hr data-ref="r1">"#);
    let s = slice_from_html(&schema, r#"<hr data-ref="r1"><p>a<br data-k="v">b</p>"#).unwrap();
    assert_eq!(s.content.child(0).attrs().get_str("data-ref"), Some("r1"));
    assert_eq!(
        s.content.child(1).child(1).attrs().get_str("data-k"),
        Some("v")
    );
}
