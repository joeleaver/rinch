//! `serialize::set_html_fallback` for an inline atom (node views C): an `image`
//! an app draws with a node view (a claimed image) can be given exported markup
//! too. An `<img>` cannot hold children, so an image the fallback answers for is
//! written as a `<span data-pm-type="image">` carrying the picture's attributes
//! (`src`, `alt`, `title`, `width`, its app data attributes) with the markup
//! **inside** it, in place of the `<img>` — what another application shows is
//! the app's markup, as the editor showed the app's component. rinch reads the
//! span back as the same image and ignores what is inside it; the default paste
//! does so only beside the clipboard mark, as for an `embed`.

use rinch_editor_core::serialize::{
    node_to_html, set_html_fallback, slice_from_html, slice_from_pasted_html, slice_to_html,
};
use rinch_editor_core::{AttrValue, Attrs, Fragment, Mark, Node, Schema, Slice};

fn image(schema: &Schema, badge: Option<&str>) -> Node {
    let mut attrs = vec![
        ("src", AttrValue::from("pic.png")),
        ("alt", AttrValue::from("a pic")),
        ("width", AttrValue::Int(40)),
    ];
    if let Some(b) = badge {
        attrs.push(("data-badge", AttrValue::from(b)));
    }
    schema
        .create_node("image", Attrs::from_iter(attrs), Fragment::empty())
        .unwrap()
}

fn para(schema: &Schema, children: Vec<Node>) -> Node {
    schema
        .branch("paragraph", Fragment::from_children(children))
        .unwrap()
}

fn doc(schema: &Schema, blocks: Vec<Node>) -> Node {
    schema
        .branch("doc", Fragment::from_children(blocks))
        .unwrap()
}

/// A fallback for the images carrying `data-badge`: a chip naming the badge.
fn chip(node: &Node) -> Option<String> {
    let badge = node.attrs().get_str("data-badge")?;
    Some(format!("<b class=\"chip\">{badge}</b>"))
}

#[test]
fn a_claimed_image_is_written_as_a_span_holding_the_apps_markup() {
    let s = Schema::starter_kit();
    let d = doc(
        &s,
        vec![para(
            &s,
            vec![
                s.text("a").unwrap(),
                image(&s, Some("new")),
                s.text("b").unwrap(),
                image(&s, None),
            ],
        )],
    );
    let bare = node_to_html(&d);
    assert!(!bare.contains("<span"), "{bare}");
    let _guard = set_html_fallback(chip);
    let html = node_to_html(&d);
    assert_eq!(
        html,
        r#"<p>a<span data-pm-type="image" alt="a pic" data-badge="new" src="pic.png" width="40"><b class="chip">new</b></span>b<img alt="a pic" src="pic.png" width="40"></p>"#
    );
    // Read back by rinch it is the same document: the span is the image.
    assert_eq!(
        slice_from_html(&s, &html).unwrap().content,
        d.content().clone()
    );
}

#[test]
fn the_copy_out_carries_the_mark_and_pastes_back_as_the_image() {
    let s = Schema::starter_kit();
    let p = para(
        &s,
        vec![
            s.text("a").unwrap(),
            image(&s, Some("hot")),
            s.text("b").unwrap(),
        ],
    );
    let slice = Slice::new(Fragment::from_node(p), 1, 1);
    let _guard = set_html_fallback(chip);
    let copied = slice_to_html(&slice);
    assert!(
        copied.contains(r#"<span data-pm-type="image" alt="a pic" data-badge="hot" src="pic.png" width="40" data-rinch-clip="">"#),
        "{copied}"
    );
    assert_eq!(
        slice_from_pasted_html(&s, &copied).unwrap().content,
        slice.content
    );
    // Another application's span naming an image (a browser's copy of a page
    // showing the export) is the markup it shows, not an image.
    let foreign = copied.replace(" data-rinch-clip=\"\"", "");
    let pasted = slice_from_pasted_html(&s, &foreign).unwrap();
    fn walk(n: &Node, t: &mut String) {
        assert_ne!(n.type_name(), "image");
        if let Some(x) = n.text() {
            t.push_str(x);
        }
        for c in n.content().children() {
            walk(c, t);
        }
    }
    let mut text = String::new();
    for n in pasted.content.children() {
        walk(n, &mut text);
    }
    assert_eq!(text, "ahotb");
}

#[test]
fn a_linked_claimed_image_keeps_its_link() {
    let s = Schema::starter_kit();
    let mt = s.mark_type("link").unwrap();
    let link = Mark::new(
        mt.clone(),
        mt.compute_attrs(&Attrs::from_iter([(
            "href",
            AttrValue::from("https://x.test"),
        )]))
        .unwrap(),
    );
    let img: Node = image(&s, Some("l")).with_marks(vec![link]);
    let d = doc(&s, vec![para(&s, vec![img])]);
    let _guard = set_html_fallback(chip);
    let html = node_to_html(&d);
    assert!(
        html.starts_with(r#"<p><a href="https://x.test""#) && html.contains("<span data-pm-type"),
        "{html}"
    );
    assert_eq!(
        slice_from_html(&s, &html).unwrap().content,
        d.content().clone()
    );
}
