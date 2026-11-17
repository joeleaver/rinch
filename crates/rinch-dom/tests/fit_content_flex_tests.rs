//! A flex container sized to fit its content (#2370): a float, an
//! absolutely positioned box, an `inline-flex`, `width: fit-content`, a grid
//! item aligned to its start, a row in a column aligned to its start. Its
//! width is `min(max-content, max(min-content, room))` (CSS Sizing 3 §5.2);
//! it was the max-content width whatever the room, so wrapping text did not
//! wrap. The fix is in the Taffy fork (`tests/fit_content_flex.rs` there).
//!
//! Every number is Chrome 156's for the same markup: a 300px parent in a
//! 560px column, the bundled Inter at 16px/20px, and the container's margins
//! 23px and 17px, so its room is 260. `T` is 431.17px wide unwrapped and its
//! longest word, "epsilon", 54.38; `W` is one 139.7px word.
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const T: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
const W: &str = "Supercalifragilistic";
const M: &str = "margin:0 17px 0 23px";

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

/// The painted border box of `#id`, in page coordinates: x, y, width,
/// height. (An atomic inline's place on its line is not in its `layout.x`.)
fn abs(d: &mut RinchDocument, id: &str) -> (f32, f32, f32, f32) {
    let n = d.query_selector(&format!("#{id}")).expect(id);
    let b = d.bounding_rect(n).expect(id);
    (b.x, b.y, b.width, b.height)
}

/// A value [`near`] does not check.
const ANY: f32 = f32::NAN;

/// Within half a pixel on each of the given values but [`ANY`].
#[track_caller]
fn near(d: &mut RinchDocument, id: &str, chrome: [f32; 4]) {
    let got = abs(d, id);
    let got_a = [got.0, got.1, got.2, got.3];
    assert!(
        got_a
            .iter()
            .zip(chrome)
            .all(|(g, c)| c.is_nan() || (g - c).abs() <= 0.5),
        "#{id}: rinch (x, y, w, h) {got:?}, Chrome {chrome:?} (NaN: not checked)"
    );
}

/// The 300px parent, with `style` added, holding `inner`.
fn parent(style: &str, inner: &str) -> String {
    format!("<div style='width:300px;{style}'>{inner}</div>")
}

/// The container `#t`, with `style` and the margins, holding `items`.
fn container(style: &str, items: &str) -> String {
    format!("<div id=t style='{style};{M}'>{items}</div>")
}

/// Items `#i0`, `#i1`, ..., each with its style and text.
fn items(list: &[(&str, &str)]) -> String {
    list.iter()
        .enumerate()
        .map(|(n, (style, text))| format!("<div id=i{n} style='{style}'>{text}</div>"))
        .collect()
}

/// A float holding `items` with the container style `style`.
fn float(style: &str, list: &[(&str, &str)]) -> RinchDocument {
    doc(&parent(
        "",
        &container(&format!("float:left;display:flex;{style}"), &items(list)),
    ))
}

// --- A float -----------------------------------------------------------------

/// The issue's shape: the text wraps in the room (rinch: 431 x 20).
#[test]
fn a_float_row_of_one_wrapping_item() {
    let mut d = float("", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 260.0, 40.0]);
}

/// Two items share the room (rinch: 862 wide).
#[test]
fn a_float_row_of_two_items() {
    let mut d = float("", &[("", T), ("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [153.0, ANY, 130.0, 80.0]);
}

/// Content narrower than the room: the max-content width (a control; it
/// passed before).
#[test]
fn a_float_row_narrower_than_its_room() {
    let mut d = float("", &[("", "alpha beta")]);
    near(&mut d, "t", [23.0, 0.0, 79.03, 20.0]);
}

/// Three unbreakable words: a single-line container is never narrower than
/// the sum of its items' min-content contributions, 3 x 139.7.
#[test]
fn a_float_row_overflows_to_its_min_content() {
    let mut d = float("", &[("", W), ("", W), ("", W)]);
    near(&mut d, "t", [23.0, 0.0, 419.11, 20.0]);
}

/// Gaps count toward both sizes: 3 x 139.7 + 2 x 15, and with two wrapping
/// items, (260 - 15) / 2 each.
#[test]
fn gaps() {
    let mut d = float("gap:15px", &[("", W), ("", W), ("", W)]);
    near(&mut d, "t", [23.0, 0.0, 449.11, 20.0]);
    let mut d = float("column-gap:15px", &[("", T), ("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [160.5, ANY, 122.5, 80.0]);
}

/// An item's padding and margins count toward its contributions.
#[test]
fn item_padding_and_margins() {
    let item = "padding:0 7px;margin:0 4px 0 9px";
    let mut d = float("", &[(item, T), (item, T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 100.0]);
    near(&mut d, "i0", [32.0, ANY, 117.0, 100.0]);
    let mut d = float("", &[(item, W), (item, W), (item, W)]);
    near(&mut d, "t", [23.0, 0.0, 500.11, 20.0]);
}

/// The container's own padding and border come on top of its items.
#[test]
fn container_padding_and_border() {
    let mut d = float("padding:0 6px 0 11px", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    near(&mut d, "i0", [34.0, ANY, 243.0, 40.0]);
    let mut d = float("border:3px solid", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 46.0]);
    // Two words' 279.4 and 70 of padding pass the room.
    let mut d = float("padding:0 35px", &[("", W), ("", W)]);
    near(&mut d, "t", [23.0, 0.0, 349.41, 20.0]);
    near(&mut d, "i0", [58.0, ANY, 139.7, 20.0]);
}

/// A container's max-width and min-width clamp the result.
#[test]
fn container_min_and_max_width() {
    let mut d = float("max-width:210px", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 210.0, 60.0]);
    let mut d = float("min-width:290px", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 290.0, 40.0]);
}

/// A multi-line container's min-content is its largest item's: each item
/// on its own line of 260, and a 280px item makes it 280.
#[test]
fn a_float_wrapping_row() {
    let mut d = float("flex-wrap:wrap", &[("", T), ("", T), ("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 120.0]);
    let mut d = float("flex-wrap:wrap", &[("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    let mut d = float("flex-wrap:wrap", &[("", T), ("width:280px", "")]);
    near(&mut d, "t", [23.0, 0.0, 280.0, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 280.0, 40.0]);
    // Lines that each fit, and padding: the room, padding included (it was
    // the room less the padding).
    let fixed = "width:100px;height:20px";
    let mut d = float(
        "flex-wrap:wrap;padding:0 6px 0 11px",
        &[(fixed, ""), (fixed, ""), (fixed, "")],
    );
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    near(&mut d, "i2", [34.0, 20.0, 100.0, 20.0]);
}

/// An item that does not shrink overflows the float: its min-content
/// contribution is its min-content size, not its flex base size.
#[test]
fn an_item_that_does_not_shrink() {
    let mut d = float("", &[("flex-shrink:0", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 20.0]);
    near(&mut d, "i0", [23.0, ANY, 431.17, 20.0]);
}

/// `overflow: hidden` items contribute their content's min-content width.
#[test]
fn clipped_items() {
    let mut d = float("", &[("overflow:hidden", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    let clip = "overflow:hidden";
    let mut d = float("", &[(clip, W), (clip, W), (clip, W)]);
    near(&mut d, "t", [23.0, 0.0, 419.11, 20.0]);
}

/// `flex: 1 1 0`: the bases sum to 0, the max-content contributions to 862.
#[test]
fn items_of_zero_basis() {
    let mut d = float("", &[("flex:1 1 0", T), ("flex:1 1 0", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [153.0, ANY, 130.0, 80.0]);
}

/// A fixed item beside a wrapping one, and items with a min-width (which
/// floors their min-content contributions: 2 x 200).
#[test]
fn fixed_and_floored_items() {
    let mut d = float("", &[("width:120px;flex-shrink:0", ""), ("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [143.0, ANY, 140.0, 80.0]);
    let mut d = float("", &[("min-width:200px", T), ("min-width:200px", T)]);
    near(&mut d, "t", [23.0, 0.0, 400.0, 60.0]);
}

/// A column's width is its cross size, fit to its items (it was already).
#[test]
fn a_float_column() {
    let mut d = float("flex-direction:column", &[("", T), ("", T)]);
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    let mut d = float(
        "flex-direction:column",
        &[("", "alpha beta"), ("", "gamma")],
    );
    near(&mut d, "t", [23.0, 0.0, 79.03, 40.0]);
}

// --- Other boxes sized to fit their content -----------------------------------

#[test]
fn absolutely_positioned() {
    for dir in ["row", "column"] {
        let mut d = doc(&parent(
            "position:relative",
            &container(
                &format!("position:absolute;display:flex;flex-direction:{dir}"),
                &items(&[("", T)]),
            ),
        ));
        near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    }
}

/// Chrome puts each at x 23; rinch ignores an atomic inline's margins on its
/// line (#1405), so x is not checked (nor an inline-flex after text, which
/// Chrome moves to the next line and rinch, without the margins, keeps).
/// These passed before: rinch sizes an atomic inline itself.
#[test]
fn inline_flex() {
    for dir in ["row", "column"] {
        let mut d = doc(&parent(
            "",
            &container(
                &format!("display:inline-flex;flex-direction:{dir}"),
                &items(&[("", T)]),
            ),
        ));
        near(&mut d, "t", [ANY, 0.0, 260.0, 40.0]);
    }
    let mut d = doc(&parent(
        "",
        &container("display:inline-flex", &items(&[("", T), ("", T)])),
    ));
    near(&mut d, "t", [ANY, 0.0, 260.0, 80.0]);
}

#[test]
fn width_fit_content() {
    for dir in ["row", "column"] {
        let mut d = doc(&parent(
            "",
            &container(
                &format!("width:fit-content;display:flex;flex-direction:{dir}"),
                &items(&[("", T)]),
            ),
        ));
        near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    }
}

#[test]
fn a_start_aligned_grid_item() {
    for dir in ["row", "column"] {
        let mut d = doc(&parent(
            "display:grid;justify-items:start",
            &container(
                &format!("display:flex;flex-direction:{dir}"),
                &items(&[("", T)]),
            ),
        ));
        near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    }
    let mut d = doc(&parent(
        "display:grid;justify-items:start",
        &container(
            "display:flex;flex-wrap:wrap;gap:10px",
            &items(&[("", T), ("", T)]),
        ),
    ));
    near(&mut d, "t", [23.0, 0.0, 260.0, 90.0]);
}

/// A row in a column aligned to its start.
#[test]
fn a_row_in_a_start_aligned_column() {
    let mut d = doc(&parent(
        "display:flex;flex-direction:column;align-items:flex-start",
        &container("display:flex", &items(&[("", T)])),
    ));
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
}

// --- Nested in a float ---------------------------------------------------------

#[test]
fn a_flex_row_in_a_float_flex_row() {
    let inner = format!("<div id=n style='display:flex'>{}</div>", items(&[("", T)]));
    let mut d = doc(&parent("", &container("float:left;display:flex", &inner)));
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 260.0, 40.0]);
    let inner = format!(
        "<div id=n style='display:flex;gap:8px'>{}</div>",
        items(&[("", T), ("", T)])
    );
    let mut d = doc(&parent("", &container("float:left;display:flex", &inner)));
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [157.0, ANY, 126.0, 80.0]);
}

#[test]
fn a_flex_row_in_a_float_block() {
    let inner = format!("<div id=n style='display:flex'>{}</div>", items(&[("", T)]));
    let mut d = doc(&parent("", &container("float:left", &inner)));
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 260.0, 40.0]);
    let inner = format!(
        "<div><div style='display:flex'>{}</div></div>",
        items(&[("", T)])
    );
    let mut d = doc(&parent("", &container("float:left;display:flex", &inner)));
    near(&mut d, "t", [23.0, 0.0, 260.0, 40.0]);
}

#[test]
fn a_flex_row_in_a_float_column() {
    let inner = format!(
        "<div id=n style='display:flex'>{}</div>",
        items(&[("", T), ("", T)])
    );
    let mut d = doc(&parent(
        "",
        &container("float:left;display:flex;flex-direction:column", &inner),
    ));
    near(&mut d, "t", [23.0, 0.0, 260.0, 80.0]);
    near(&mut d, "i1", [153.0, ANY, 130.0, 80.0]);
}

// --- Review of #2389 ---------------------------------------------------------

/// An item's max-width caps its max-content contribution: 80 + 41.11, not
/// the room (rinch was 300), in a float, an absolutely positioned box and a
/// wrapping float alike; a min-width above the max wins (120 + 41.11). No
/// margins on the container here.
#[test]
fn an_item_max_width_caps_the_max_content() {
    let words = "alpha beta gamma delta epsilon zeta";
    for (parent_style, container_style, item_style, w, h) in [
        (
            "",
            "float:left;display:flex",
            "max-width:80px",
            121.11,
            100.0,
        ),
        (
            "position:relative",
            "position:absolute;display:flex",
            "max-width:80px",
            121.11,
            100.0,
        ),
        (
            "",
            "float:left;display:flex;flex-wrap:wrap",
            "max-width:80px",
            121.11,
            100.0,
        ),
        (
            "",
            "float:left;display:flex",
            "max-width:80px;min-width:120px",
            161.11,
            60.0,
        ),
    ] {
        let mut d = doc(&parent(
            parent_style,
            &format!(
                "<div id=t style='{container_style}'><div id=i0 style='{item_style}'>{words}</div><div id=i1>alpha</div></div>"
            ),
        ));
        near(&mut d, "t", [0.0, 0.0, w, h]);
        near(&mut d, "i1", [w - 41.11, ANY, 41.11, h]);
    }
}

// --- Percentages against the final width (#2392) ------------------------------
//
// The container sizes itself with its items' percentages behaving as `auto`
// (css-sizing-3 §5.2.1), then lays them out with the percentages resolved
// against that width. rinch resolved them against nothing in a float or a
// start-aligned grid item, so a `width: 50%` took its content's width: "alpha
// beta" (79.03) and "gamma delta" (97.85) stayed side by side on one line.
// Chrome 157's numbers, the same 300px parent (no margins where none are
// written). A second item's width is not checked where rinch snaps both its
// edges to pixels (111.44 to 199.88 paints 111 to 200, 89 wide).

/// Two `width: 50%` items split the 176.88 their contents sum to, and wrap.
/// (rinch: 79 and 98 wide, 20 tall.)
#[test]
fn a_float_row_resolves_percentage_widths_against_its_width_2392() {
    for prop in ["width", "flex-basis"] {
        let mut d = float(
            "",
            &[
                (&format!("{prop}:50%"), "alpha beta"),
                (&format!("{prop}:50%"), "gamma delta"),
            ],
        );
        near(&mut d, "t", [23.0, 0.0, 176.88, 40.0]);
        near(&mut d, "i0", [23.0, ANY, 88.44, 40.0]);
        near(&mut d, "i1", [111.44, ANY, ANY, 40.0]);
    }
}

/// One percentage item beside an auto one: 50% of 172.77 is 86.39, and the
/// two shrink by their bases to fit. (rinch: 79 / 94, one line.)
#[test]
fn a_float_row_shrinks_a_resolved_percentage_basis_2392() {
    let mut d = float("", &[("width:50%", "alpha beta"), ("", "beta gamma")]);
    near(&mut d, "t", [23.0, 0.0, 172.77, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 82.86, 40.0]);
    near(&mut d, "i1", [105.86, ANY, 89.91, 40.0]);
}

/// Unequal percentages, with the container's padding and border and an
/// item's margin: 30% and 60% of the 244.75 content box, 4px margin.
/// (rinch: 79 / 157.)
#[test]
fn a_float_row_with_insets_resolves_percentages_against_its_content_box_2392() {
    let mut d = float(
        "padding:0 5px;border-left:3px solid",
        &[
            ("width:30%;margin-left:4px", "alpha beta"),
            ("flex-basis:60%", "gamma delta epsilon"),
        ],
    );
    near(&mut d, "t", [23.0, 0.0, 252.75, 40.0]);
    near(&mut d, "i0", [35.0, ANY, 71.92, 40.0]);
    near(&mut d, "i1", [106.92, ANY, 143.84, 40.0]);
}

/// A column: the item's `width: 50%` is its cross size, half of the 97.84
/// the widest item makes the float. (rinch: 79.)
#[test]
fn a_float_column_resolves_a_percentage_cross_width_2392() {
    let mut d = float(
        "flex-direction:column",
        &[("width:50%", "alpha beta"), ("", "gamma delta")],
    );
    near(&mut d, "t", [23.0, 0.0, 97.84, 60.0]);
    near(&mut d, "i0", [23.0, 0.0, 48.92, 40.0]);
    near(&mut d, "i1", [23.0, 40.0, 97.84, 20.0]);
}

/// A flex row that is a start-aligned grid item, fit-content sized the same
/// way. (rinch: 79 / 98, one line.)
#[test]
fn a_start_aligned_grid_item_row_resolves_percentage_widths_2392() {
    let mut d = doc(&parent(
        "display:grid;justify-items:start",
        &format!(
            "<div id=t style='display:flex'>{}</div>",
            items(&[("width:50%", "alpha beta"), ("width:50%", "gamma delta")])
        ),
    ));
    near(&mut d, "t", [0.0, 0.0, 176.88, 40.0]);
    near(&mut d, "i0", [0.0, 0.0, 88.44, 40.0]);
    near(&mut d, "i1", [88.44, 0.0, ANY, 40.0]);
}

/// Controls that passed before: an absolutely positioned row and an
/// `inline-flex` are laid out at their final width already.
#[test]
fn an_abspos_and_an_inline_flex_row_resolve_percentage_widths_2392() {
    let mut d = doc(&parent(
        "position:relative;height:40px",
        &format!(
            "<div id=t style='position:absolute;display:flex;left:23px'>{}</div>",
            items(&[("width:50%", "alpha beta"), ("width:50%", "gamma delta")])
        ),
    ));
    near(&mut d, "t", [23.0, 0.0, 176.88, 40.0]);
    near(&mut d, "i0", [23.0, 0.0, 88.44, 40.0]);
    near(&mut d, "i1", [111.44, 0.0, ANY, 40.0]);
    let mut d = doc(&parent(
        "",
        &format!(
            "<div id=t style='display:inline-flex'>{}</div>",
            items(&[("width:40%", "alpha beta"), ("", "gamma")])
        ),
    ));
    near(&mut d, "t", [0.0, 0.0, 134.84, 40.0]);
    near(&mut d, "i0", [0.0, 0.0, 53.94, 40.0]);
    near(&mut d, "i1", [53.94, 0.0, 55.81, 40.0]);
}

/// A percentage margin on an item, and a percentage `column-gap`, count as 0
/// toward the width and then resolve against it: 10% of 176.88 is 17.69.
/// (rinch: 0 each, one line.)
#[test]
fn a_float_row_resolves_a_percentage_margin_and_gap_2392() {
    let mut d = float(
        "",
        &[("margin-left:10%", "alpha beta"), ("", "gamma delta")],
    );
    near(&mut d, "t", [23.0, 0.0, 176.88, 40.0]);
    near(&mut d, "i0", [40.69, ANY, 71.13, 40.0]);
    near(&mut d, "i1", [111.81, ANY, ANY, 40.0]);
    let mut d = float("column-gap:10%", &[("", "alpha beta"), ("", "gamma delta")]);
    near(&mut d, "t", [23.0, 0.0, 176.88, 40.0]);
    near(&mut d, "i0", [23.0, ANY, 71.13, 40.0]);
    near(&mut d, "i1", [111.81, ANY, ANY, 40.0]);
}
