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

// ── review of #1595 ─────────────────────────────────────────────────────────

fn texts(n: &Node, t: &mut String, images: &mut usize) {
    if n.type_name() == "image" {
        *images += 1;
    }
    if let Some(x) = n.text() {
        t.push_str(x);
    }
    for c in n.content().children() {
        texts(c, t, images);
    }
}

/// A `<span data-pm-type="image">` that names no picture (no `src`, an unsafe
/// one) is not an image — but its text must not be lost either (the reader is
/// total about text, #1397). The editor's own rendered DOM of a claimed image is
/// exactly such a span (marker and data attributes, no `src`).
#[test]
fn review_1595_a_span_naming_an_image_without_a_src_keeps_its_text_on_load() {
    let s = Schema::starter_kit();
    for html in [
        r#"<p>a<span data-pm-type="image" data-badge="new" data-pm-node-view=""><b>chip</b></span>b</p>"#,
        r#"<p>a<span data-pm-type="image" src="javascript:alert(1)"><b>chip</b></span>b</p>"#,
    ] {
        let slice = slice_from_html(&s, html).unwrap();
        let (mut t, mut n) = (String::new(), 0);
        for c in slice.content.children() {
            texts(c, &mut t, &mut n);
        }
        assert_eq!((t.as_str(), n), ("achipb", 0), "{html}");
    }
}

/// A fallback that answers a block (documented as not allowed): does the image
/// still read back? Informational: prints what a round trip gives.
#[test]
fn review_1595_block_markup_in_an_image_fallback_round_trip() {
    let s = Schema::starter_kit();
    let d = doc(
        &s,
        vec![para(
            &s,
            vec![s.text("a").unwrap(), image(&s, Some("x")), s.text("b").unwrap()],
        )],
    );
    let _guard = set_html_fallback(|_n: &Node| Some("<div>block</div>".to_string()));
    let html = node_to_html(&d);
    let back = slice_from_html(&s, &html).unwrap();
    let (mut t, mut n) = (String::new(), 0);
    for c in back.content.children() {
        texts(c, &mut t, &mut n);
    }
    eprintln!("REVIEW-1595 block fallback: {html} -> blocks={} text={t:?} images={n}", back.content.child_count());
    assert_eq!(n, 1, "the image survives its own export: {html}");
}
