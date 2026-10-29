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

/// A declared ratio relates the sizes of the `box-sizing` box. (rinch
/// sizes every box as `border-box`, #1278: the `content-box` row is in
/// [`known_differences_from_chrome`].)
#[test]
fn the_ratio_is_of_the_box_sizing_box() {
    check(
        "width: 100px; padding: 10px; box-sizing: border-box; aspect-ratio: 2",
        (100.0, 50.0),
    );
    check(
        "width: 120px; padding: 10px 30px 20px 10px; box-sizing: border-box; aspect-ratio: 3",
        (120.0, 40.0),
    );
}

#[test]
fn min_and_max_clamp_the_transferred_size() {
    check("aspect-ratio: 2; max-width: 120px", (120.0, 60.0));
    // The other three Chrome rows (a definite size and the other axis's
    // clamp) are in `known_differences_from_chrome` (#1561).
}

#[test]
fn a_flex_item_takes_the_ratio() {
    const ROW: &str = "display: flex";
    // Stretched to the line, which is the container's height: the declared
    // width stays and the stretch wins over the ratio.
    check_in(
        ROW,
        "div",
        &[],
        "width: 100px; aspect-ratio: 2",
        (100.0, 300.0),
    );
    check_in(
        ROW,
        "div",
        &[],
        "height: 40px; aspect-ratio: 2",
        (80.0, 40.0),
    );
    check_in(
        "display: flex; align-items: flex-start",
        "div",
        &[],
        "width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
    const COLUMN: &str = "display: flex; flex-direction: column";
    check_in(COLUMN, "div", &[], "aspect-ratio: 2", (400.0, 200.0));
    check_in(
        COLUMN,
        "div",
        &[],
        "width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
}

#[test]
fn a_grid_item_takes_the_ratio() {
    const GRID: &str = "display: grid; grid-template-columns: 200px";
    check_in(GRID, "div", &[], "aspect-ratio: 2", (200.0, 100.0));
    check_in(
        GRID,
        "div",
        &[],
        "width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
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
    check(
        "display: inline-block; width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
    check(
        "display: inline-block; height: 40px; aspect-ratio: 2",
        (80.0, 40.0),
    );
    check(
        "display: inline-block; width: 50%; aspect-ratio: 4",
        (200.0, 50.0),
    );
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

/// A fixed box with one vertical inset (or none) keeps the height Taffy
/// gave it from the ratio or the replaced measure. It used to be replaced by
/// a sum of the children's heights after the compute — 0 for every row here
/// (#1569, gone with #893). Chrome 155's numbers, from #1569.
#[test]
fn a_fixed_box_takes_the_ratio() {
    for (tag, attrs, style, want) in [
        (
            "div",
            &[][..],
            "position: fixed; top: 0; left: 0; width: 100px; aspect-ratio: 2",
            (100.0, 50.0),
        ),
        (
            "div",
            &[],
            "position: fixed; bottom: 16px; right: 16px; width: 320px; aspect-ratio: 16/9",
            (320.0, 180.0),
        ),
        (
            "video",
            &[],
            "position: fixed; bottom: 16px; right: 16px; width: 320px; aspect-ratio: 16/9",
            (320.0, 180.0),
        ),
        (
            "div",
            &[],
            "position: fixed; left: 0; right: 0; top: 0; aspect-ratio: 4",
            (800.0, 200.0),
        ),
        (
            "img",
            &[("src", FORTY_BY_THIRTY)],
            "position: fixed; top: 0; left: 0; width: 80px",
            (80.0, 60.0),
        ),
        (
            "video",
            &[],
            "position: fixed; top: 0; left: 0; width: 320px",
            (320.0, 150.0),
        ),
    ] {
        check_in("", tag, attrs, style, want);
    }
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
    check_in(
        "",
        "video",
        &[],
        "height: 60px; aspect-ratio: 2",
        (120.0, 60.0),
    );
    // A declared ratio is the `box-sizing` box's: the iframe's 2px UA border
    // is inside the 100x50.
    check_in(
        "",
        "iframe",
        &[],
        "box-sizing: border-box; width: 100px; aspect-ratio: 2",
        (100.0, 50.0),
    );
    // `auto <ratio>`'s is the content box's, whatever `box-sizing` says.
    check_in(
        "",
        "iframe",
        &[],
        "box-sizing: border-box; width: 100px; aspect-ratio: auto 2",
        (100.0, 52.0),
    );
    const PADDED: &str = "display: block; box-sizing: border-box; width: 100px; padding: 5px";
    check_in(
        "",
        "video",
        &[],
        &format!("{PADDED}; aspect-ratio: auto 2"),
        (100.0, 55.0),
    );
    check_in(
        "",
        "video",
        &[],
        &format!("{PADDED}; aspect-ratio: 2"),
        (100.0, 50.0),
    );
    // So is a natural ratio: a 300x150 canvas with a 10px border is 120 + 20
    // wide, 60 + 20 tall.
    check_in(
        "",
        "canvas",
        &[],
        "display: block; box-sizing: border-box; width: 140px; border: 10px solid; aspect-ratio: auto 1",
        (140.0, 80.0),
    );
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
    check_in(
        "",
        "canvas",
        &[],
        "height: 60px; aspect-ratio: 3",
        (180.0, 60.0),
    );
    // No size declared: the natural width, and the declared ratio.
    check_in("", "canvas", &[], "aspect-ratio: 1", (300.0, 300.0));
    check_in(
        "",
        "canvas",
        &[],
        "aspect-ratio: 1; max-width: 100px",
        (100.0, 100.0),
    );
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
    check_in(
        "",
        "img",
        &[],
        "aspect-ratio: 1; width: 200px",
        (200.0, 200.0),
    );
    check_in(
        "",
        "video",
        BOTH,
        "aspect-ratio: 1; height: auto",
        (100.0, 100.0),
    );
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
    loaded(
        &[],
        "width: 100px; aspect-ratio: 2; max-height: 30px",
        (100.0, 30.0),
    );
}

#[test]
fn a_box_holding_text_takes_the_ratio() {
    let text = |style: &str, want: (f32, f32)| {
        let (doc, _, e) = build("", "div", &[], style, "aaaaaaaa bbbbbbbb cccccccc");
        assert_eq!(size(&doc, e), want, "{style} (Chrome 155, Inter 16px/20px)");
    };
    // Three 20px lines overflow 25px. A scroll container's automatic minimum
    // height, and an explicit `min-height: 0`, are not its content's, so the
    // ratio holds. (A visible box grows to its content in Chrome:
    // `known_differences_from_chrome`, #1562.)
    text(
        "width: 100px; aspect-ratio: 4; overflow: hidden",
        (100.0, 25.0),
    );
    text(
        "width: 100px; aspect-ratio: 4; min-height: 0",
        (100.0, 25.0),
    );
    let (doc, _, e) = build("", "div", &[], "width: 100px; aspect-ratio: 1", "aa");
    assert_eq!(size(&doc, e), (100.0, 100.0));
}

/// (style, container, rinch, Chrome 155, issue)
type KnownRow = (&'static str, &'static str, (f32, f32), (f32, f32), u32);

/// Where rinch is not Chrome yet. Each row asserts rinch's answer, with
/// Chrome 155's beside it, so a fix shows up here.
#[test]
fn known_differences_from_chrome() {
    // (style, container, rinch, Chrome, issue)
    let rows: &[KnownRow] = &[
        // `box-sizing` is never read: every box is `border-box` (#1278).
        (
            "width: 100px; padding: 10px; aspect-ratio: 2",
            "",
            (100.0, 50.0),
            (120.0, 70.0),
            1278,
        ),
        // Taffy transfers the other axis's min/max onto a definite size.
        (
            "width: 100px; aspect-ratio: 2; max-height: 30px",
            "",
            (60.0, 30.0),
            (100.0, 30.0),
            1561,
        ),
        (
            "width: 100px; aspect-ratio: 2; min-height: 80px",
            "",
            (160.0, 80.0),
            (100.0, 80.0),
            1561,
        ),
        (
            "height: 20px; aspect-ratio: 2; min-width: 60px",
            "",
            (60.0, 30.0),
            (60.0, 20.0),
            1561,
        ),
        // No automatic minimum size from the ratio: a stretched flex item
        // 300 tall is 600 wide in Chrome, and does not shrink.
        (
            "aspect-ratio: 2",
            "display: flex",
            (400.0, 300.0),
            (600.0, 300.0),
            1562,
        ),
        // Taffy applies `auto <ratio>` to a non-replaced box's border box
        // like a plain `<ratio>`; Chrome uses the content box (120 x
        // 56.65625). Only visible with padding or a border under
        // `border-box`, which is every rinch box (#1278).
        (
            "width: 120px; padding: 10px 30px 20px 10px; box-sizing: border-box; aspect-ratio: auto 3",
            "",
            (120.0, 40.0),
            (120.0, 57.0),
            1278,
        ),
        // A flex item grown from a 0 basis does not take its cross size
        // from the ratio. #1571.
        (
            "flex: 1; aspect-ratio: 4",
            "display: flex; align-items: center",
            (400.0, 0.0),
            (400.0, 100.0),
            1571,
        ),
    ];
    for &(style, container, rinch, chrome, issue) in rows {
        let (doc, _, e) = build(container, "div", &[], style, "");
        assert_eq!(
            size(&doc, e),
            rinch,
            "{style} in \"{container}\": rinch's known answer (Chrome 155: {chrome:?}, #{issue})"
        );
    }
    let (doc, _, e) = build("", "iframe", &[], "width: 100px; aspect-ratio: 2", "");
    // Chrome 155: 104x54 (content-box: the 2px border outside 100x50). #1278.
    assert_eq!(size(&doc, e), (100.0, 50.0));
    // Content taller than the ratio: Chrome 155 grows a visible box to its
    // three 20px lines, 100x60, block and inline-block alike. #1562.
    for style in [
        "width: 100px; aspect-ratio: 4",
        "display: inline-block; width: 100px; aspect-ratio: 4",
    ] {
        let (doc, _, e) = build("", "div", &[], style, "aaaaaaaa bbbbbbbb cccccccc");
        assert_eq!(size(&doc, e), (100.0, 25.0), "{style}");
    }
    // A shrink-to-fit box whose width comes from its content does not take
    // its height from that width: a 30x10 block inside. Chrome 155: 30x30
    // for each. #1570.
    for html in [
        r#"<span id=t style="display:inline-block;aspect-ratio:1"><div style="width:30px;height:10px"></div></span>"#,
        r#"<div style="display:flex"><div><span id=t style="display:inline-block;aspect-ratio:1"><div style="width:30px;height:10px"></div></span></div></div>"#,
        r#"<div style="position:relative"><div id=t style="position:absolute;top:0;left:0;aspect-ratio:1"><div style="width:30px;height:10px"></div></div></div>"#,
    ] {
        assert_eq!(measure_html(html), (30.0, 10.0), "{html}");
    }
}

/// `body > div(400x300) > html`, laid out; the size of `#t`.
fn measure_html(html: &str) -> (f32, f32) {
    let mut doc = document();
    let body = doc.body();
    let c = doc.create_element("div");
    doc.set_attribute(
        c,
        "style",
        "width: 400px; height: 300px; font: 16px/20px ProbeFace",
    );
    doc.append_child(body, c);
    doc.set_inner_html(c, html);
    doc.resolve_layout(800.0, 600.0);
    let t = doc.query_selector_all("#t")[0];
    size(&doc, t)
}

/// A restyle that changes only `aspect-ratio` moves no other Taffy input:
/// it has to relayout by itself.
#[test]
fn a_change_of_ratio_relays_out() {
    for (tag, before, after) in [
        ("div", (100.0, 50.0), (100.0, 25.0)),
        ("canvas", (100.0, 50.0), (100.0, 25.0)),
        ("video", (100.0, 50.0), (100.0, 25.0)),
        ("img", (100.0, 50.0), (100.0, 25.0)),
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
        // No ratio left: a div and an unloaded image are empty, a canvas
        // has its own, a video the default object height.
        let back = if tag == "div" || tag == "img" {
            (100.0, 0.0)
        } else if tag == "canvas" {
            (100.0, 50.0)
        } else {
            (100.0, 150.0)
        };
        assert_eq!(size(&doc, e), back, "<{tag}> after aspect-ratio: auto");
    }
}

/// `aspect-ratio` does not apply to internal table boxes (css-sizing-4 §5.1;
/// css-tables-3: a row, a row group, a cell, a column, a column group). A
/// caption is not one, and neither is the table: both take the ratio.
/// Chrome 155 for each row; the element `#t` in a 400x300 div.
#[test]
fn internal_table_boxes_take_no_ratio() {
    let rows: &[(&str, (f32, f32))] = &[
        (
            r#"<div style="display:table"><div id=t style="display:table-cell;width:100px;aspect-ratio:2"></div></div>"#,
            (100.0, 0.0),
        ),
        (
            r#"<div style="display:table"><div id=t style="display:table-cell;height:40px;aspect-ratio:2"></div></div>"#,
            (0.0, 40.0),
        ),
        (
            r#"<div style="display:table;width:100px"><div id=t style="display:table-row;aspect-ratio:2"><div style="display:table-cell">x</div></div></div>"#,
            (100.0, 20.0),
        ),
        (
            r#"<div style="display:table;width:100px"><div id=t style="display:table-row-group;aspect-ratio:2"><div style="display:table-row"><div style="display:table-cell">x</div></div></div></div>"#,
            (100.0, 20.0),
        ),
        // Not internal: a caption and the table itself keep the ratio.
        (
            r#"<div style="display:table"><div id=t style="display:table-caption;width:100px;aspect-ratio:2"></div><div style="display:table-cell">x</div></div>"#,
            (100.0, 50.0),
        ),
        (
            r#"<div id=t style="display:table;width:100px;aspect-ratio:2"></div>"#,
            (100.0, 50.0),
        ),
    ];
    for &(html, want) in rows {
        assert_eq!(measure_html(html), want, "{html}");
    }
}
