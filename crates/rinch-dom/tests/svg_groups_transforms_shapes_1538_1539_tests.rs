//! Inline SVG: `<g>` groups and the `transform` attribute (#1539), `<ellipse>`
//! and `<rect rx/ry>` (#1538).
//!
//! `paint/svg.rs` used to walk only the `<svg>`'s direct children and read no
//! `transform`, so a shape inside a `<g>` was not painted, a group's `fill`/
//! `stroke` reached nothing, a `transform` was ignored, an `<ellipse>` fell
//! through the shape match and a rounded `<rect>` painted square.
//!
//! **The oracle is Chrome 156**: the same markup in a 100x100 `<svg
//! viewBox="0 0 100 100">`, each probe asked with `document.elementFromPoint`
//! at the probe pixel's centre (a shape, or the `<svg>` behind it). Every
//! probe here is at least 1.5px from any edge of the geometry, so its pixel is
//! entirely inside (exactly the fill colour) or entirely outside (untouched)
//! in the software painter too. Probes are placed off the fixed points: a
//! translation that is not zero, a rotation that is not a multiple of 90°,
//! unequal corner radii, each where the broken (or a plausibly wrong) reading
//! paints the opposite answer.

#![cfg(feature = "software-renderer")]

use peniko::Brush;
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::skia_painter::TinySkiaPainter;

const GREEN: [u8; 4] = [0, 128, 0, 255];
const EMPTY: [u8; 4] = [0, 0, 0, 0];

/// A probe pixel and whether Chrome paints the shape there.
const INK: bool = true;
const NONE: bool = false;

/// Paint `markup` inside a 100x100 `<svg viewBox="{viewbox}">` at the
/// window's origin, on the software painter.
fn paint(viewbox: &str, markup: &str) -> TinySkiaPainter {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let host = doc.create_element("div");
    doc.append_child(body, host);
    doc.set_inner_html(
        host,
        &format!(
            "<svg viewBox=\"{viewbox}\" style=\"position: absolute; left: 0; top: 0; \
             display: block; width: 100px; height: 100px\">{markup}</svg>"
        ),
    );
    doc.resolve_layout(100.0, 100.0);
    let mut painter = TinySkiaPainter::new(100, 100);
    let mut layout_cx: parley::LayoutContext<Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (100.0, 100.0),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    painter
}

fn pixel(p: &TinySkiaPainter, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
}

/// Paint `markup` and check each `(x, y, ink)` probe against Chrome 156's
/// answer.
#[track_caller]
fn check(markup: &str, probes: &[(u32, u32, bool)]) {
    check_in("0 0 100 100", markup, probes);
}

#[track_caller]
fn check_in(viewbox: &str, markup: &str, probes: &[(u32, u32, bool)]) {
    let p = paint(viewbox, markup);
    let mut wrong = Vec::new();
    for &(x, y, ink) in probes {
        let got = pixel(&p, x, y);
        let want = if ink { GREEN } else { EMPTY };
        if got != want {
            wrong.push(format!(
                "({x},{y}): want {} got {got:?}",
                if ink { "ink" } else { "empty" }
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "{markup}\nprobes that differ from Chrome 156: {wrong:#?}"
    );
}

// ── #1539: groups ───────────────────────────────────────────────────────────

#[test]
fn shapes_inside_groups_are_painted_at_any_depth() {
    check(
        r#"<g><rect x="10" y="10" width="30" height="30" fill="green"/></g>
           <g><g><circle cx="70" cy="70" r="15" fill="green"/></g></g>"#,
        &[(25, 25, INK), (70, 70, INK), (50, 50, NONE)],
    );
}

#[test]
fn a_groups_fill_is_inherited_and_a_child_overrides_it() {
    check(
        r#"<g fill="green">
             <rect x="10" y="10" width="30" height="30"/>
             <g><rect x="60" y="10" width="30" height="30"/></g>
           </g>
           <g fill="red">
             <rect x="10" y="60" width="30" height="30" fill="green"/>
             <rect x="60" y="60" width="30" height="30" fill="none"/>
           </g>"#,
        &[(25, 25, INK), (75, 25, INK), (25, 75, INK), (75, 75, NONE)],
    );
}

/// The stroke, its width (10, where the painter's own default is 2) and its
/// cap all come from the group.
#[test]
fn a_groups_stroke_width_and_cap_are_inherited() {
    check(
        r#"<g stroke="green" stroke-width="10" fill="none">
             <rect x="20" y="20" width="60" height="50"/>
           </g>
           <g stroke="green" stroke-width="10" stroke-linecap="square">
             <line x1="30" y1="90" x2="70" y2="90"/>
           </g>"#,
        &[
            // The rect's left edge is stroked 15..25 wide.
            (19, 45, INK),
            (23, 45, INK),
            (12, 45, NONE),
            (50, 45, NONE),
            // The square cap reaches x = 25.
            (50, 90, INK),
            (27, 90, INK),
            (22, 90, NONE),
        ],
    );
}

/// `fill: currentColor` inherits as the keyword, so it resolves against the
/// shape's own `color`, not the group's.
#[test]
fn a_groups_current_color_fill_resolves_against_the_childs_color() {
    check(
        r#"<g fill="currentColor" style="color: rgb(255, 0, 0)">
             <rect x="10" y="10" width="30" height="30" style="color: rgb(0, 128, 0)"/>
           </g>"#,
        &[(25, 25, INK)],
    );
}

// ── #1539: transform ────────────────────────────────────────────────────────

#[test]
fn translate_moves_a_shape() {
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="translate(30,40)"/>"#,
        &[(40, 50, INK), (10, 10, NONE)],
    );
}

/// 30° clockwise about (50, 50): the bar's top end swings from (50, 10) to
/// about (70, 15).
#[test]
fn rotate_about_a_centre() {
    check(
        r#"<rect x="45" y="10" width="10" height="80" fill="green" transform="rotate(30 50 50)"/>"#,
        &[(69, 16, INK), (49, 15, NONE), (35, 76, INK), (50, 50, INK)],
    );
}

/// A stroke is in user space: `scale(3)` makes a 2-wide stroke 6 wide.
#[test]
fn a_scale_scales_the_stroke_width() {
    check(
        r#"<g transform="scale(3)">
             <line x1="5" y1="10" x2="30" y2="10" stroke="green" stroke-width="2"/>
           </g>"#,
        &[(50, 31, INK), (50, 28, INK), (50, 25, NONE), (50, 35, NONE)],
    );
}

/// A list applies right to left: `translate(50,10) scale(2)` scales first.
#[test]
fn a_transform_list_composes_in_order() {
    check(
        r#"<rect x="0" y="0" width="10" height="10" fill="green" transform="translate(50,10) scale(2)"/>"#,
        &[(65, 25, INK), (5, 5, NONE)],
    );
}

#[test]
fn group_transforms_compose_with_the_shapes_own() {
    check(
        r#"<g transform="translate(20,0)"><g transform="translate(0,30)">
             <rect x="0" y="0" width="20" height="20" fill="green" transform="scale(1.5)"/>
           </g></g>"#,
        &[
            (45, 55, INK),
            (22, 35, INK),
            (15, 35, NONE),
            (25, 25, NONE),
            (55, 70, NONE),
        ],
    );
}

/// The transform composes inside the `viewBox` mapping (here a scale of 2):
/// `translate(10,5)` is 20px right and 10px down, the rect then 20px wide.
#[test]
fn a_transform_is_in_viewbox_units() {
    check_in(
        "0 0 50 50",
        r#"<rect x="0" y="0" width="10" height="10" fill="green" transform="translate(10,5)"/>"#,
        &[(35, 27, INK), (22, 12, INK), (12, 7, NONE)],
    );
}

#[test]
fn matrix_skew_x_and_skew_y() {
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="matrix(1 0 0 1 60 10)"/>
           <rect x="10" y="50" width="20" height="40" fill="green" transform="skewX(30)"/>"#,
        &[(75, 25, INK), (41, 52, INK), (15, 85, NONE), (70, 85, INK)],
    );
    check(
        r#"<rect x="10" y="10" width="40" height="20" fill="green" transform="skewY(20)"/>"#,
        &[(45, 40, INK), (45, 15, NONE)],
    );
}

/// SVG's number grammar: a sign or a second `.` starts the next number, and
/// functions need no separator between them.
#[test]
fn transform_numbers_need_no_separator() {
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="translate(60-20)translate(0,60)scale(1e0)"/>"#,
        &[(70, 50, INK), (10, 10, NONE)],
    );
}

/// A `transform` that does not parse is ignored whole (the shape is drawn
/// untransformed), as Chrome does; a skew of 90° draws nothing.
#[test]
fn an_invalid_transform_is_ignored() {
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="translate(30,40) bogus(1)"/>"#,
        &[(10, 10, INK), (40, 50, NONE)],
    );
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="translate(30,40,5)"/>"#,
        &[(10, 10, INK), (40, 50, NONE)],
    );
    for t in ["translate(30,40),", "translate(30px,40px)", "rotate(0deg)"] {
        check(
            &format!(r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="{t}"/>"#),
            &[(10, 10, INK), (40, 50, NONE)],
        );
    }
    check(
        r#"<rect x="0" y="0" width="20" height="20" fill="green" transform="skewX(90)"/>"#,
        &[(10, 10, NONE), (40, 50, NONE)],
    );
}

// ── #1538: ellipse ──────────────────────────────────────────────────────────

/// Covers its centre and its ends, not its bounding box's corner; `ry` is
/// not `rx` (a circle of radius 40 would cover (50, 26)).
#[test]
fn an_ellipse_is_painted() {
    check(
        r#"<ellipse cx="50" cy="50" rx="40" ry="20" fill="green"/>"#,
        &[
            (50, 50, INK),
            (85, 50, INK),
            (50, 33, INK),
            (50, 26, NONE),
            (13, 33, NONE),
        ],
    );
}

/// SVG 2: an absent (or `auto`) radius takes the other one. A negative one
/// is an error, ignored, so it is `auto` too; a value that is no number is
/// `0` (Chrome 156), and an ellipse with a zero radius draws nothing.
#[test]
fn an_ellipses_auto_radius_takes_the_other() {
    check(
        r#"<ellipse cx="50" cy="50" rx="20" fill="green"/>"#,
        &[(50, 50, INK), (50, 35, INK), (50, 25, NONE)],
    );
    check(
        r#"<ellipse cx="50" cy="50" rx="-5" ry="20" fill="green"/>"#,
        &[(50, 50, INK), (35, 50, INK), (25, 50, NONE)],
    );
    check(
        r#"<ellipse cx="50" cy="50" rx="abc" ry="20" fill="green"/>"#,
        &[(50, 50, NONE), (35, 50, NONE)],
    );
    // **Not Chrome 156**, which draws nothing for the attribute keyword
    // `auto` (it reads it as `0`) though it draws this circle for
    // `style="rx: auto"`; SVG 2 makes the keyword `auto` in either.
    check(
        r#"<ellipse cx="50" cy="50" rx="auto" ry="20" fill="green"/>"#,
        &[(50, 50, INK), (35, 50, INK), (25, 50, NONE)],
    );
}

// ── #1538: rect rx/ry ───────────────────────────────────────────────────────

#[test]
fn a_rect_with_rx_has_round_corners() {
    check(
        r#"<rect x="10" y="10" width="80" height="80" rx="20" fill="green"/>"#,
        &[(14, 14, NONE), (50, 11, INK), (11, 50, INK), (50, 50, INK)],
    );
    // `ry` alone: `rx` is auto and takes it.
    check(
        r#"<rect x="10" y="10" width="80" height="80" ry="20" fill="green"/>"#,
        &[(14, 14, NONE), (50, 11, INK), (11, 50, INK)],
    );
}

/// `rx="30" ry="10"`: each corner is a quarter ellipse. (15, 12) is inside a
/// round corner of radius 10 and outside the ellipse; (12, 18) is inside the
/// ellipse and outside a round corner of radius 30.
#[test]
fn unequal_radii_make_elliptical_corners() {
    check(
        r#"<rect x="10" y="10" width="80" height="80" rx="30" ry="10" fill="green"/>"#,
        &[(15, 12, NONE), (12, 18, INK), (84, 87, NONE), (87, 81, INK)],
    );
}

/// `rx="100"` on an 80x40 rect: `rx` clamps to 40, the auto `ry` takes 100
/// and clamps to 20, so the corners are 40x20 ellipses (a round corner of 20
/// would cover (25, 32)).
#[test]
fn radii_clamp_to_half_the_size_each() {
    check(
        r#"<rect x="10" y="30" width="80" height="40" rx="100" fill="green"/>"#,
        &[(13, 33, NONE), (50, 50, INK), (12, 50, INK), (25, 32, NONE)],
    );
}

/// A negative radius is an error and reads as `auto`; one that is no number
/// is `0`, which squares the corners (Chrome 156).
#[test]
fn a_negative_radius_is_auto() {
    check(
        r#"<rect x="10" y="10" width="80" height="80" rx="-5" ry="20" fill="green"/>"#,
        &[(14, 14, NONE), (11, 50, INK)],
    );
    check(
        r#"<rect x="10" y="10" width="80" height="80" rx="abc" ry="20" fill="green"/>"#,
        &[(14, 14, INK), (11, 50, INK)],
    );
}

/// A rect of zero width renders nothing, its stroke included (Chrome 156).
#[test]
fn a_zero_width_rect_is_not_stroked() {
    check(
        r#"<rect x="50" y="10" width="0" height="80" stroke="green" stroke-width="6"/>"#,
        &[(50, 50, NONE)],
    );
}

#[test]
fn a_rounded_rect_is_stroked_round() {
    check(
        r#"<rect x="20" y="20" width="60" height="60" rx="20" fill="none" stroke="green" stroke-width="4"/>"#,
        &[(21, 21, NONE), (25, 25, INK), (50, 20, INK), (20, 50, INK)],
    );
}

/// From the #1994 review: a `<g>` hidden by the `display` attribute or the
/// property paints none of its content (Chrome 156). Inkscape saves a hidden
/// layer this way; before groups were walked nothing in one painted, after
/// they were it all did.
#[test]
fn a_hidden_group_paints_nothing() {
    let rect = r#"<rect x="20" y="20" width="60" height="60" fill="green"/>"#;
    check(
        &format!(r#"<g display="none">{rect}</g>"#),
        &[(50, 50, NONE)],
    );
    check(
        &format!(r#"<g style="display: none"><g>{rect}</g></g>"#),
        &[(50, 50, NONE)],
    );
    // Control: the same group shown paints.
    check(
        &format!(r#"<g display="inline">{rect}</g>"#),
        &[(50, 50, INK)],
    );
}

/// `rx="30" ry="100"` on a 40x80 rect: `rx` clamps to 20 (half the width) and
/// `ry` to 40 (half the height), so the rect is the inscribed ellipse
/// centred on (30, 50). A clamp to the whole side (30x80) draws something
/// else at the corner probes.
#[test]
fn radii_clamp_to_half_not_the_whole_side() {
    check(
        r#"<rect x="10" y="10" width="40" height="80" rx="30" ry="100" fill="green"/>"#,
        &[
            (30, 50, INK),
            (30, 13, INK),
            (12, 12, NONE),
            (13, 20, NONE),
            (47, 86, NONE),
        ],
    );
}

/// `stroke-linejoin` is inherited from a group. Under a 20-wide stroke the
/// apex of `20,80 50,20 80,80` is a miter (the default; ratio 2.24 is under
/// the limit of 4) whose tip reaches y = -2.4, or a bevel whose edge is at
/// y = 15.5. (50, 8) is inside the miter and outside the bevel.
#[test]
fn a_groups_stroke_linejoin_is_inherited() {
    let line = r#"<polyline points="20,80 50,20 80,80"/>"#;
    check(
        &format!(
            r#"<g stroke="green" stroke-width="20" fill="none" stroke-linejoin="bevel">{line}</g>"#
        ),
        &[(50, 8, NONE), (50, 20, INK)],
    );
    // Control: the probe sees a miter.
    check(
        &format!(
            r#"<g stroke="green" stroke-width="20" fill="none" stroke-linejoin="miter">{line}</g>"#
        ),
        &[(50, 8, INK)],
    );
}
