//! CSS `aspect-ratio` (#1286).
//!
//! rinch used to drop the property: `div { width: 100px; aspect-ratio: 2 }`
//! was 100x0 and the responsive-embed `video { width: 100%; aspect-ratio:
//! 16/9 }` took the default object height. It is now carried from Stylo into
//! `ComputedStyle::aspect_ratio` and from there into Taffy's
//! `Style::aspect_ratio` for a non-replaced box, and into the replaced
//! measure (`replaced::measure`) for an `<img>`, `<canvas>`, `<video>` and
//! `<iframe>`, where a `<ratio>` without `auto` replaces the natural ratio
//! and `auto <ratio>` is used only when there is no natural ratio. The
//! `width`/`height` attributes' ratio (#684) is the presentational hint
//! `aspect-ratio: auto w / h`, cascaded below every author rule, so an
//! author `aspect-ratio` replaces it.
//!
//! **Every number is Chrome 155's** (`--headless=new`, standards mode, `body
//! { margin: 0 }`, `getBoundingClientRect`), with the element alone in a
//! `width: 400px; height: 300px` div whose own style is the case's
//! container style. The loaded image is 40x30, off every ratio a case
//! declares.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

/// A 40x30 PNG.
const FORTY_BY_THIRTY: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAYAAABe3VzdAAAAPUlEQVR4nO3OIQEAIBAAMTJ9/wC0ggjIQ0zMb+2Z87NVBwQF64CgYB0QFKwDgoJ1QFCwDggK1gFBwTrwcgFD1ConI1jnigAAAABJRU5ErkJggg==";

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn document() -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1);
    doc
}

/// `body > div(400x300; container) > <tag attrs style>text`, laid out.
fn build(
    container: &str,
    tag: &str,
    attrs: &[(&str, &str)],
    style: &str,
    text: &str,
) -> (RinchDocument, NodeId, NodeId) {
    let mut doc = document();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        &format!("width: 400px; height: 300px; font: 16px/20px ProbeFace; {container}"),
    );
    doc.append_child(body, c);
    let e = doc.create_element(tag);
    for (k, v) in attrs {
        doc.set_attribute(e, k, v);
    }
    doc.set_attribute(e, "style", style);
    if !text.is_empty() {
        let t = doc.create_text(text);
        doc.append_child(e, t);
    }
    doc.append_child(c, e);
    doc.resolve_layout(800.0, 600.0);
    (doc, c, e)
}

fn size(doc: &RinchDocument, id: NodeId) -> (f32, f32) {
    let n = doc.tree.get(id.0).unwrap();
    (n.layout.width, n.layout.height)
}

#[track_caller]
fn check_in(container: &str, tag: &str, attrs: &[(&str, &str)], style: &str, want: (f32, f32)) {
    let (doc, _, e) = build(container, tag, attrs, style, "");
    assert_eq!(
        size(&doc, e),
        want,
        "<{tag} {attrs:?} style=\"{style}\"> in \"{container}\" (Chrome 155)"
    );
}

#[track_caller]
fn check(style: &str, want: (f32, f32)) {
    check_in("", "div", &[], style, want);
}

#[test]
fn a_block_takes_its_other_dimension_from_the_ratio() {
    check("width: 100px; aspect-ratio: 2", (100.0, 50.0));
    // An auto width stretches to the container, and the ratio gives the
    // height.
    check("aspect-ratio: 4/1", (400.0, 100.0));
    // A definite height and an auto width: the width comes from the ratio,
    // not from the container.
    check("height: 30px; aspect-ratio: 2", (60.0, 30.0));
    // `auto` matters only to a box with a natural ratio.
    check("width: 100px; aspect-ratio: auto 2", (100.0, 50.0));
    // 66.65625 in Chrome (its 1/64 px layout units); rinch rounds to whole
    // pixels.
    check("width: 100px; aspect-ratio: 3/2", (100.0, 67.0));
    // A degenerate ratio is `auto`.
    check("width: 100px; aspect-ratio: 0/1", (100.0, 0.0));
}

#[test]
fn the_ratio_is_of_the_box_sizing_box() {
    check("width: 100px; padding: 10px; aspect-ratio: 2", (120.0, 70.0));
    check(
        "width: 100px; padding: 10px; box-sizing: border-box; aspect-ratio: 2",
        (100.0, 50.0),
    );
}

#[test]
fn min_and_max_clamp_the_transferred_size() {
    check("width: 100px; aspect-ratio: 2; max-height: 30px", (100.0, 30.0));
    check("width: 100px; aspect-ratio: 2; min-height: 80px", (100.0, 80.0));
    check("aspect-ratio: 2; max-width: 120px", (120.0, 60.0));
    check("height: 20px; aspect-ratio: 2; min-width: 60px", (60.0, 20.0));
}

#[test]
fn a_flex_item_takes_the_ratio() {
    const ROW: &str = "display: flex";
    // Stretched to the line, which is the container's height: the declared
    // width stays and the stretch wins over the ratio.
    check_in(ROW, "div", &[], "width: 100px; aspect-ratio: 2", (100.0, 300.0));
    check_in(ROW, "div", &[], "height: 40px; aspect-ratio: 2", (80.0, 40.0));
    // Nothing declared: stretched to 300 tall, so 600 wide.
    check_in(ROW, "div", &[], "aspect-ratio: 2", (600.0, 300.0));
    check_in(
        "display: flex; align-items: flex-start",
        "div",
        &[],
        "width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
    const COLUMN: &str = "display: flex; flex-direction: column";
    check_in(COLUMN, "div", &[], "aspect-ratio: 2", (400.0, 200.0));
    check_in(COLUMN, "div", &[], "width: 100px; aspect-ratio: 2", (100.0, 50.0));
}

#[test]
fn a_grid_item_takes_the_ratio() {
    const GRID: &str = "display: grid; grid-template-columns: 200px";
    check_in(GRID, "div", &[], "aspect-ratio: 2", (200.0, 100.0));
    check_in(GRID, "div", &[], "width: 100px; aspect-ratio: 2", (100.0, 50.0));
    check_in(
        "display: grid; grid-template-columns: 200px; align-items: start",
        "div",
        &[],
        "aspect-ratio: 2",
        (200.0, 100.0),
    );
}

/// An atomic inline is computed as a Taffy root.
#[test]
fn an_inline_block_takes_the_ratio() {
    check("display: inline-block; width: 100px; aspect-ratio: 2", (100.0, 50.0));
    check("display: inline-block; height: 40px; aspect-ratio: 2", (80.0, 40.0));
    check("display: inline-block; width: 50%; aspect-ratio: 4", (200.0, 50.0));
}

#[test]
fn an_absolute_box_takes_the_ratio() {
    const REL: &str = "position: relative";
    check_in(
        REL,
        "div",
        &[],
        "position: absolute; width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
    check_in(
        REL,
        "div",
        &[],
        "position: absolute; left: 0; right: 100px; aspect-ratio: 3",
        (300.0, 100.0),
    );
}

#[test]
fn a_video_with_no_natural_ratio_takes_the_declared_one() {
    // The responsive embed.
    check_in(
        "",
        "video",
        &[],
        "display: block; width: 100%; aspect-ratio: 16/9",
        (400.0, 225.0),
    );
    check_in(
        "",
        "video",
        &[],
        "display: block; width: 100px; aspect-ratio: auto 2",
        (100.0, 50.0),
    );
    check_in("", "video", &[], "height: 60px; aspect-ratio: 2", (120.0, 60.0));
    // A 4px iframe border (the UA's `2px inset`) around a 100x50 content box.
    check_in("", "iframe", &[], "width: 100px; aspect-ratio: 2", (104.0, 54.0));
}

#[test]
fn a_ratio_without_auto_replaces_a_canvas_natural_ratio() {
    check_in(
        "",
        "canvas",
        &[],
        "display: block; width: 100px; aspect-ratio: 1",
        (100.0, 100.0),
    );
    // `auto` first: the 300x150 bitmap's own 2:1 wins.
    check_in(
        "",
        "canvas",
        &[],
        "display: block; width: 100px; aspect-ratio: auto 1",
        (100.0, 50.0),
    );
    check_in("", "canvas", &[], "height: 60px; aspect-ratio: 3", (180.0, 60.0));
    // No size declared: the natural width, and the declared ratio.
    check_in("", "canvas", &[], "aspect-ratio: 1", (300.0, 300.0));
    check_in("", "canvas", &[], "aspect-ratio: 1; max-width: 100px", (100.0, 100.0));
}

const BOTH: &[(&str, &str)] = &[("width", "100"), ("height", "50")];

/// The attributes' ratio is the hint `aspect-ratio: auto 100 / 50`, below
/// every author rule.
#[test]
fn an_author_aspect_ratio_replaces_the_attributes_ratio() {
    // `aspect-ratio: auto` switches the mapped ratio off: an unloaded image
    // has no ratio and no natural size.
    check_in(
        "",
        "img",
        BOTH,
        "aspect-ratio: auto; width: 200px; height: auto",
        (200.0, 0.0),
    );
    check_in(
        "",
        "img",
        BOTH,
        "aspect-ratio: 1; width: 200px; height: auto",
        (200.0, 200.0),
    );
    // 66.65625 in Chrome.
    check_in(
        "",
        "img",
        BOTH,
        "aspect-ratio: auto 3; width: 200px; height: auto",
        (200.0, 67.0),
    );
    check_in("", "img", &[], "aspect-ratio: 1; width: 200px", (200.0, 200.0));
    check_in("", "video", BOTH, "aspect-ratio: 1; height: auto", (100.0, 100.0));
    // No ratio: the default object height.
    check_in(
        "",
        "video",
        BOTH,
        "aspect-ratio: auto; width: 200px; height: auto",
        (200.0, 150.0),
    );
}

#[test]
fn a_loaded_image_keeps_its_ratio_unless_one_without_auto_is_declared() {
    let loaded = |attrs: &[(&str, &str)], style: &str, want: (f32, f32)| {
        let mut all = vec![("src", FORTY_BY_THIRTY)];
        all.extend_from_slice(attrs);
        check_in("", "img", &all, style, want);
    };
    loaded(&[], "width: 80px; aspect-ratio: auto 1", (80.0, 60.0));
    loaded(&[], "width: 80px; aspect-ratio: 1", (80.0, 80.0));
    // The natural width, through the declared ratio.
    loaded(&[], "aspect-ratio: 1", (40.0, 40.0));
    loaded(&[], "height: 50px; aspect-ratio: 2", (100.0, 50.0));
    loaded(BOTH, "aspect-ratio: 1; height: auto", (100.0, 100.0));
    loaded(&[], "width: 100px; aspect-ratio: 2; max-height: 30px", (100.0, 30.0));
}

#[test]
fn content_taller_than_the_ratio_grows_a_visible_box() {
    let text = |style: &str, want: (f32, f32)| {
        let (doc, _, e) = build("", "div", &[], style, "aaaaaaaa bbbbbbbb cccccccc");
        assert_eq!(size(&doc, e), want, "{style} (Chrome 155, Inter 16px/20px)");
    };
    // Three 20px lines: `min-height: auto` is the content height.
    text("width: 100px; aspect-ratio: 4", (100.0, 60.0));
    text("display: inline-block; width: 100px; aspect-ratio: 4", (100.0, 60.0));
    // A scroll container's, or an explicit `min-height: 0`, is not.
    text("width: 100px; aspect-ratio: 4; overflow: hidden", (100.0, 25.0));
    text("width: 100px; aspect-ratio: 4; min-height: 0", (100.0, 25.0));
    let (doc, _, e) = build("", "div", &[], "width: 100px; aspect-ratio: 1", "aa");
    assert_eq!(size(&doc, e), (100.0, 100.0));
}

/// A restyle that changes only `aspect-ratio` moves no other Taffy input:
/// it has to relayout by itself.
#[test]
fn a_change_of_ratio_relays_out() {
    for (tag, before, after) in [
        ("div", (100.0, 50.0), (100.0, 25.0)),
        ("canvas", (100.0, 50.0), (100.0, 25.0)),
        ("video", (100.0, 50.0), (100.0, 25.0)),
    ] {
        let (mut doc, _, e) = build(
            "",
            tag,
            &[],
            "display: block; width: 100px; aspect-ratio: 2",
            "",
        );
        assert_eq!(size(&doc, e), before, "<{tag}> before");
        doc.set_style(e, "aspect-ratio", "4");
        doc.resolve_layout(800.0, 600.0);
        assert_eq!(size(&doc, e), after, "<{tag}> after aspect-ratio: 4");
        doc.set_style(e, "aspect-ratio", "auto");
        doc.resolve_layout(800.0, 600.0);
        let back = if tag == "div" {
            (100.0, 0.0)
        } else if tag == "canvas" {
            (100.0, 50.0)
        } else {
            (100.0, 150.0)
        };
        assert_eq!(size(&doc, e), back, "<{tag}> after aspect-ratio: auto");
    }
}
