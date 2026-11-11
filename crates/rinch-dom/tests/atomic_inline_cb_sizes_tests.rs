//! An atomic inline (`inline-block`, `inline-flex`, `inline-grid`, an inline
//! `<img>`) is laid out as a **detached Taffy root**, and a root takes the
//! available space it is computed under as its percentage basis. That space
//! is max-content on the block axis always, and on the inline axis for an
//! `auto` box that fits, so the box's own percentage height, `calc()`
//! height and `height: stretch` had no containing-block height (#1277,
//! #1420), and its own percentage padding no containing-block width when it
//! fit (#1480). The fix hands the root the sizes an in-flow child gets from
//! its parent: the containing block's width, and its content height where
//! that height is definite (`RinchDocument::own_containing_block_sizes`).
//!
//! Every number in `CHROME` is **Chrome 157**'s (headless, standards mode,
//! `file://`, the bundled Inter registered as `ProbeFace` through
//! `@font-face`, `16px/20px`), read with `getBoundingClientRect`. The text
//! `short text` is 71.84 wide. Every containing block is 400 wide, and 300
//! tall where it has a height, so a percentage of either axis, the content
//! height (20) and the containing block's own size are different answers.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn mount(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut doc = RinchDocument::new();
    let registered = doc.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "style", "font: 16px/20px ProbeFace");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc
}

/// `WxH` of every `[data-m]`, in document order, space-separated.
fn dump(doc: &RinchDocument) -> String {
    query_selector(&doc.tree, "[data-m]")
        .into_iter()
        .map(|id| {
            let l = doc.tree.get(id).unwrap().layout;
            format!("{:.2}x{:.2}", l.width, l.height)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn close(want: &str, got: &str) -> bool {
    let p = |s: &str| -> (f32, f32) {
        let (a, b) = s.split_once('x').unwrap();
        (a.parse().unwrap(), b.parse().unwrap())
    };
    want.split(' ').count() == got.split(' ').count()
        && want.split(' ').zip(got.split(' ')).all(|(c, r)| {
            let (c, r) = (p(c), p(r));
            // rinch lays boxes out on whole pixels.
            (c.0 - r.0).abs() <= 0.6 && (c.1 - r.1).abs() <= 0.6
        })
}

const H300: &str = "width: 400px; height: 300px";
const AUTO_H: &str = "width: 400px";

/// (name, containing block style, inner html, Chrome 157's sizes).
const CHROME: &[(&str, &str, &str, &str)] = &[
    // #1420's "probably any atomic inline".
    (
        "pct_height",
        H300,
        r#"<span data-m style="display: inline-block; height: 50%">short text</span>"#,
        "71.84x150.00",
    ),
    // #1277 (1).
    (
        "stretch_height",
        H300,
        r#"<span data-m style="display: inline-block; height: stretch">short text</span>"#,
        "71.84x300.00",
    ),
    (
        "stretch_height_less_margins",
        H300,
        r#"<span data-m style="display: inline-block; height: stretch; margin: 7px 0">short text</span>"#,
        "71.84x286.00",
    ),
    (
        "stretch_height_content_box_padding",
        H300,
        r#"<span data-m style="display: inline-block; height: stretch; padding: 4px">short text</span>"#,
        "79.84x300.00",
    ),
    // The control: an `auto` height is not definite, and the percentage
    // computes to `auto` (CSS 2.1 §10.5).
    (
        "pct_height_indefinite_cb",
        AUTO_H,
        r#"<span data-m style="display: inline-block; height: 50%">short text</span>"#,
        "71.84x20.00",
    ),
    // A `calc()` with a percentage is `auto` there too, not its length part.
    (
        "calc_height_indefinite_cb",
        AUTO_H,
        r#"<span data-m style="display: inline-block; height: calc(50% + 10px)">short text</span>"#,
        "71.84x20.00",
    ),
    // A stretched flex item's height is definite.
    (
        "pct_height_in_stretched_flex_item",
        "display: flex; width: 400px; height: 300px",
        r#"<div style="width: 400px"><span data-m style="display: inline-block; height: 50%">short text</span></div>"#,
        "71.84x150.00",
    ),
    // Inside a box resolved so: its height is the inner box's basis.
    (
        "pct_height_nested",
        H300,
        r#"<span data-m style="display: inline-block; height: 50%"><span data-m style="display: inline-block; height: 50%">x</span></span>"#,
        "8.73x150.00 8.73x75.00",
    ),
    // Shorter than its content (60): a column `inline-flex` is not stretched
    // to its content by the pin.
    (
        "inline_flex_column_pct_height",
        H300,
        r#"<span data-m style="display: inline-flex; flex-direction: column; height: 10%"><i>a</i><i>b</i><i>c</i></span>"#,
        "9.80x30.00",
    ),
    // The box's height is now definite for a percentage inside it.
    (
        "pct_height_inside",
        H300,
        r#"<span data-m style="display: inline-block; height: 50%"><div data-m style="height: 50%"></div></span>"#,
        "0.00x150.00 0.00x75.00",
    ),
    (
        "inline_flex_pct_height",
        H300,
        r#"<span data-m style="display: inline-flex; height: 50%">short text</span>"#,
        "71.84x150.00",
    ),
    (
        "inline_grid_pct_height",
        H300,
        r#"<span data-m style="display: inline-grid; height: 50%">short text</span>"#,
        "71.84x150.00",
    ),
    (
        "calc_height",
        H300,
        r#"<span data-m style="display: inline-block; height: calc(50% + 10px)">short text</span>"#,
        "71.84x160.00",
    ),
    (
        "min_max_height",
        H300,
        r#"<span data-m style="display: inline-block; min-height: 30%; max-height: 10%">short text</span>"#,
        "71.84x90.00",
    ),
    // #1420: from CSS and from the attributes.
    (
        "img_pct_height",
        H300,
        r#"<img data-m style="width: 50%; height: 50%"><img data-m width="50%" height="50%">"#,
        "200.00x150.00 200.00x150.00",
    ),
    // Of the containing block's content box.
    (
        "pct_height_padded_cb",
        "width: 400px; height: 300px; padding: 10px; box-sizing: border-box",
        r#"<span data-m style="display: inline-block; height: 50%; padding: 5px; border: 2px solid">short text</span>"#,
        "85.84x154.00",
    ),
    // #1480.
    (
        "pct_padding_fits",
        AUTO_H,
        r#"<span data-m style="display: inline-block; padding: 0 10%">short text</span>"#,
        "151.84x20.00",
    ),
    // A vertical percentage padding is of the containing block's width too.
    (
        "pct_padding_all_sides",
        AUTO_H,
        r#"<span data-m style="display: inline-block; padding: 5% 10%; border: 1px solid">short text</span>"#,
        "153.84x62.00",
    ),
];

#[test]
fn an_atomic_inline_resolves_against_its_containing_block_as_chrome() {
    let mut bad = vec![];
    for &(name, cb, inner, want) in CHROME {
        let mut doc = mount(&format!(r#"<div style="{cb}">{inner}</div>"#));
        doc.resolve_layout(800.0, 600.0);
        let got = dump(&doc);
        if !close(want, &got) {
            bad.push(format!("{name}: rinch {got}, Chrome 157 {want}"));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

/// A second layout with nothing changed moves nothing, and the containing
/// block's height changing moves the box with it: the size a box was
/// resolved at is recorded with the height it was resolved against.
#[test]
fn the_box_follows_its_containing_blocks_height() {
    let mut doc = mount(
        r#"<div data-cb style="width: 400px; height: 300px"><span data-m style="display: inline-block; height: 50%">short text</span></div>"#,
    );
    doc.resolve_layout(800.0, 600.0);
    assert!(close("71.84x150.00", &dump(&doc)), "{}", dump(&doc));
    doc.resolve_layout(800.0, 601.0);
    assert!(
        close("71.84x150.00", &dump(&doc)),
        "relayout: {}",
        dump(&doc)
    );
    let cb = query_selector(&doc.tree, "[data-cb]")[0];
    doc.set_attribute(
        rinch_core::dom::NodeId(cb),
        "style",
        "width: 400px; height: 200px",
    );
    doc.resolve_layout(800.0, 600.0);
    assert!(close("71.84x100.00", &dump(&doc)), "200px: {}", dump(&doc));
    // And back to indefinite: the percentage is `auto` again.
    doc.set_attribute(rinch_core::dom::NodeId(cb), "style", "width: 400px");
    doc.resolve_layout(800.0, 600.0);
    assert!(close("71.84x20.00", &dump(&doc)), "auto: {}", dump(&doc));
}
