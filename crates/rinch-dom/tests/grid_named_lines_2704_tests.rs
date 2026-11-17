//! #2704 — a grid container's `grid-template-areas` and the `[names]` of its
//! track lists are read, so an item placed by a name lands where Chrome puts
//! it: `grid-area: e`, an area's implicit `e-start` / `e-end` lines, an area
//! name used as a line (`grid-column: b / c`), named lines (`[m] 213px [e]`),
//! a repeated name picked by its count (`grid-column: a 2`, `a -1`), names
//! inside `repeat()`, the `grid-template` shorthand, and an area template
//! that alone sets the size of the explicit grid. Absolutely positioned items
//! placed by a name are in `abs_grid_area_2377_tests.rs` and in the `p` rows
//! here. A table ignores the grid names (it is laid out as a grid of its
//! own columns and rows). #2717: a negative index past the named lines, a
//! missing name with a negative index, and `span <name>` against a line count
//! as Chrome does (the `c07`, `c09`, `c25` and `k` rows).
//!
//! Every number in `CHROME` is **Chrome 153**'s (Playwright's
//! `chrome-headless-shell` 153.0.8010.12, standards mode,
//! `* { box-sizing: border-box }`, an 800x600 viewport, device scale factor
//! 1, the bundled Inter registered as `ProbeFace` through `@font-face`,
//! `body { margin: 0; font: 16px/20px ProbeFace }`), `getBoundingClientRect`
//! in the viewport: `mark=x,y,width,height`.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::testing::query_selector;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const VIEWPORT: (f32, f32) = (800.0, 600.0);

const TEXT: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";

/// The grid every row shares: three columns and two rows of off-round sizes,
/// unequal gaps, padding and a border, so a line misplaced by one track, one
/// gap or the padding edge moves a box by a distinct amount.
const G: &str = "display:grid;column-gap:7px;row-gap:5px;width:431px;height:259px;padding:11px 13px;border:3px solid";

/// (name, grid style, items as `mark:style` joined by `|`, Chrome 153). An
/// item whose style starts with `abs;` is absolutely positioned (the grid is
/// then `position: relative`), holds `TEXT` and has horizontal margins of
/// 17 and 23 px.
const CHROME: &[(&str, &str, &str, &str)] = &[
    (
        "n01_area",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-area:e",
        "a=120,80,213,149",
    ),
    (
        "n02_area_spans",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a a c' 'd e e'",
        "a:grid-area:a|b:grid-area:e|c:grid-area:d",
        "a=16,14,317,61 b=120,80,291,149 c=16,80,97,149",
    ),
    (
        "n03_area_rows_and_dot",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'. b c' 'd b .'",
        "a:grid-area:b|b:grid-area:c|c:|d:",
        "a=120,14,213,215 b=340,14,71,61 c=16,14,97,61 d=16,80,97,149",
    ),
    (
        "n04_named_lines",
        "grid-template-columns:[s] 97px [m] 213px [e] 71px [f];grid-template-rows:[t] 61px [u] 149px [v]",
        "a:grid-column:m / f;grid-row:u / v|b:grid-column:s / m;grid-row:t",
        "a=120,80,291,149 b=16,14,97,61",
    ),
    (
        "n05_implicit_area_lines",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-column:b-start / c-end;grid-row:e-start|b:grid-column:a-end;grid-row:d-start / d-end",
        "a=120,80,291,149 b=120,80,213,149",
    ),
    (
        "n06_area_name_as_lines",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-column:b / c;grid-row:e|b:grid-column:d;grid-row:a / d",
        "a=120,80,291,149 b=16,14,97,215",
    ),
    (
        "n07_repeated_names",
        "grid-template-columns:[a] 1fr [a] 1fr [a] 1fr [a];grid-template-rows:61px 149px",
        "a:grid-column:a 2;grid-row:2|b:grid-column:a 3 / a 4|c:grid-column:a -2 / a -1;grid-row:2",
        "a=151.33,80,128.33,149 b=286.66,14,128.33,61 c=286.66,80,128.33,149",
    ),
    (
        "n08_names_in_repeat",
        "grid-template-columns:[x] 41px repeat(2, [r] 59px [q] 83px) [z];grid-template-rows:61px 149px",
        "a:grid-column:q 2 / z|b:grid-column:r / q;grid-row:2|c:grid-column:x / r 2;grid-row:2",
        "a=286,14,83,61 b=64,80,59,149 c=16,80,197,149",
    ),
    (
        "n09_name_with_area_and_line",
        "grid-template-columns:[b-start] 97px [mid] 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a a b' 'd e f'",
        "a:grid-column:b-start / mid|b:grid-area:b;grid-row:2",
        "a=16,14,97,61 b=16,80,395,149",
    ),
    (
        "n10_missing_name",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-column:nosuch;grid-row:1|b:grid-column:2;grid-row:2",
        "a=425,14,0,61 b=120,80,213,149",
    ),
    (
        "n11_template_shorthand",
        "grid-template:[top] 'h h h' 61px [mid] 's m n' 149px [bot] / 97px 213px 71px",
        "a:grid-area:m|b:grid-area:h|c:grid-column:2;grid-row:mid / bot",
        "a=120,80,213,149 b=16,14,395,61 c=120,80,213,149",
    ),
    (
        // No track list: the template alone makes the explicit grid four
        // columns by two rows, its empty fourth column included, and the four
        // `auto` columns share the width (94.5 each).
        "n12_areas_size_the_explicit_grid",
        "grid-template-areas:'a b c .' 'd e f .'",
        "a:grid-area:f|b:",
        "a=219,132,94.5,113 b=16,14,94.5,113",
    ),
    (
        "n13_auto_flow_around_area",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:grid-area:a|b:|c:grid-area:e|d:",
        "a=16,14,97,61 b=120,14,213,61 c=120,80,213,149 d=340,14,71,61",
    ),
    (
        // Names only inside `repeat()`, none at the top level.
        "n14_names_only_in_repeat",
        "grid-template-columns:41px repeat(2, [r] 59px [q] 83px);grid-template-rows:61px 149px",
        "a:grid-column:r 2 / q 2;grid-row:2|b:grid-column:q / span 2",
        "a=220,80,59,149 b=130,14,149,61",
    ),
    // The review's rows (Chrome 153): several areas, invalid templates
    // (which Stylo drops, keeping an earlier valid declaration), dots, a name
    // on several lines, `span <name>`, names missing from the grid,
    // `auto-fill` / `auto-fit` with names, the `grid-template` and `grid`
    // shorthands, four-value `grid-area`, dense and column auto-flow around
    // areas, names at a `repeat()`'s edges, an area template bigger than the
    // track lists, `inherit`, negative indices, and an explicit `a-start`
    // beside an area `a`.
    (
        "c01_multi_area",
        "grid-template-columns:97px 213px 71px 53px;grid-template-rows:61px 109px 37px;grid-template-areas:'a a b c' 'a a b c' 'd e e e'",
        "a:grid-area:a|b:grid-area:e|c:grid-area:b|d:grid-area:c",
        "a=16,14,317,175 b=120,194,351,37 c=340,14,71,175 d=418,14,53,175",
    ),
    (
        "c02_invalid_nonrect",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a a .' 'a b .'",
        "a:grid-area:a|b:grid-area:b|c:",
        "a=425,242,0,3 b=425,242,0,3 c=16,14,97,61",
    ),
    (
        "c03_invalid_ragged",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e'",
        "a:grid-area:e|b:",
        "a=425,242,0,3 b=16,14,97,61",
    ),
    (
        "c04_invalid_keeps_earlier",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f';grid-template-areas:'x x' 'x y'",
        "a:grid-area:e|b:grid-area:x",
        "a=120,80,213,149 b=425,242,0,3",
    ),
    (
        "c05_dots",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'. ... a' 'b .. .'",
        "a:grid-area:a|b:grid-area:b|c:|d:|e:",
        "a=340,14,71,61 b=16,80,97,149 c=16,14,97,61 d=120,14,213,61 e=120,80,213,149",
    ),
    (
        "c06_dup_names",
        "grid-template-columns:[a] 97px [a b] 213px [a] 71px;grid-template-rows:61px 149px",
        "x:grid-column:a 3|y:grid-column:b / a -1;grid-row:2|z:grid-column:a 2 / span a 2;grid-row:2",
        "x=340,14,71,61 y=120,80,213,149 z=120,80,298,149",
    ),
    (
        "c07_span_name",
        "grid-template-columns:[s] 97px [m] 213px [m] 71px [e];grid-template-rows:61px 149px",
        "a:grid-column:s / span m|b:grid-column:span m / e;grid-row:2|c:grid-column:2 / span m 2;grid-row:1",
        "a=16,14,97,61 b=340,80,71,149 c=120,14,298,61",
    ),
    (
        "c08_span_missing",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px",
        "a:grid-column:2 / span nosuch;grid-row:2|b:grid-column:1",
        "a=120,80,298,149 b=16,14,97,61",
    ),
    (
        "c09_implicit_names",
        "grid-template-columns:97px 213px;grid-template-rows:61px 149px",
        "a:grid-column:foo;grid-row:1|b:grid-column:foo 2 / span 1;grid-row:2|c:grid-row:bar -1;grid-column:1",
        "a=367.33,30,20.33,61 b=394.66,96,20.34,149 c=16,14,97,11",
    ),
    (
        "c10_autofill",
        "grid-template-columns:repeat(auto-fill, [a] 100px);grid-template-rows:61px 149px",
        "a:grid-column:a 2|b:grid-column:a -1;grid-row:2|c:grid-column:a 3 / span 1;grid-row:2",
        "a=123,14,100,61 b=230,80,100,149 c=230,80,100,149",
    ),
    (
        "c11_autofill_mixed",
        "grid-template-columns:[x] 41px repeat(auto-fill, [a] 89px [b]) [z] 37px;grid-template-rows:61px 149px",
        "a:grid-column:a 2 / b 2|b:grid-column:z;grid-row:2|c:grid-column:b -1 / z;grid-row:1|d:grid-column:x / a;grid-row:2",
        "a=160,14,89,61 b=352,80,37,149 c=352,14,37,61 d=16,80,41,149",
    ),
    (
        "c12_autofit_names",
        "grid-template-columns:repeat(auto-fit, [a] 89px [b]);grid-template-rows:61px 149px",
        "a:grid-column:a 2 / b 2|b:grid-column:b 1;grid-row:2",
        "a=16,14,89,61 b=16,80,89,149",
    ),
    (
        "c13_template_shorthand",
        "grid-template:[h1] 'a a b' 61px [h2] 'c d d' 149px [h3] / [c1] 97px [c2] 213px [c3] 71px [c4]",
        "a:grid-row:h2 / h3;grid-column:c2 / c4|b:grid-area:b|c:grid-area:c|d:grid-row:h1;grid-column:c1 / c3",
        "a=120,80,291,149 b=340,14,71,61 c=16,80,97,149 d=16,14,317,61",
    ),
    (
        "c14_grid_shorthand",
        "grid:'a b' 61px 'c d' 149px / 97px 213px",
        "a:grid-area:d|b:grid-area:a|c:",
        "a=120,80,213,149 b=16,14,97,61 c=120,14,213,61",
    ),
    (
        "c15_grid_shorthand_flow",
        "grid:auto-flow dense / [x] 97px [y] 213px [z]",
        "a:grid-column:y / z|b:grid-column:x / z|c:|d:grid-column:span 1",
        "a=120,14,213,73.66 b=16,92.66,317,73.67 c=16,14,97,73.66 d=16,171.33,97,73.67",
    ),
    (
        "c16_grid_area_four",
        "grid-template-columns:[c1] 97px [c2] 213px [c3] 71px [c4];grid-template-rows:[r1] 61px [r2] 149px [r3]",
        "a:grid-area:r1 / c2 / r3 / c4|b:grid-area:r2 / c1 / span 1 / c2",
        "a=120,14,291,215 b=16,80,97,149",
    ),
    (
        "c17_grid_area_four_areas",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "x:grid-area:a / b / e / f|y:grid-area:d / a / d / a",
        "x=120,14,291,215 y=16,80,97,149",
    ),
    (
        "c18_dense_names",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 109px 37px;grid-template-areas:'a b c' 'd e f' 'g h i';grid-auto-flow:dense",
        "p:grid-column:span 2|q:grid-area:b|r:|s:grid-area:d|t:|u:grid-area:i",
        "p=120,80,291,109 q=120,14,213,61 r=16,14,97,61 s=16,80,97,109 t=340,14,71,61 u=340,194,71,37",
    ),
    (
        "c19_column_flow_areas",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f';grid-auto-flow:column",
        "p:grid-area:d|q:|r:|s:grid-area:b|t:",
        "p=16,80,97,149 q=16,14,97,61 r=120,80,213,149 s=120,14,213,61 t=340,14,71,61",
    ),
    (
        "c20_repeat_boundary_names",
        "grid-template-columns:[x] repeat(2, [r] 59px [q]) [z] 83px;grid-template-rows:61px 149px",
        "a:grid-column:z / span 1|b:grid-column:q 2;grid-row:2|c:grid-column:r 2 / q 2;grid-row:1|d:grid-column:x / q;grid-row:2",
        "a=148,14,83,61 b=148,80,83,149 c=82,14,59,61 d=16,80,59,149",
    ),
    (
        "c21_row_names_repeat",
        "grid-template-columns:97px 213px;grid-template-rows:[t] 41px repeat(2, [r] 37px [q]) [z] 53px",
        "a:grid-row:r 2 / z;grid-column:1|b:grid-row:t / q;grid-column:2|c:grid-row:z;grid-column:1 / 3",
        "a=16,102,97,37 b=120,14,213,83 c=16,144,317,53",
    ),
    (
        "c22_areas_bigger_than_tracks",
        "grid-template-columns:97px;grid-template-rows:61px;grid-template-areas:'a b c' 'd e f'",
        "x:grid-area:f|y:grid-area:b",
        "x=271,80,144,165 y=120,14,144,61",
    ),
    (
        "c23_inherit",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:inherit",
        "a:grid-area:e|b:grid-area:c",
        "a=120,80,213,149 b=340,14,71,61",
    ),
    (
        "c24_names_repeat_count1",
        "grid-template-columns:repeat(1, [r] 59px) 83px [r] 41px;grid-template-rows:61px 149px",
        "a:grid-column:r 2|b:grid-column:r;grid-row:2",
        "a=172,14,41,61 b=16,80,59,149",
    ),
    (
        "c25_negative_names",
        "grid-template-columns:[a] 97px [b] 213px [a] 71px [b];grid-template-rows:61px 149px",
        "a:grid-column:a -1 / b -1|b:grid-column:b -2 / a -1;grid-row:2|c:grid-column:a -3;grid-row:2",
        "a=347,14,71,61 b=127,80,213,149 c=16,80,0,149",
    ),
    (
        "c26_area_and_explicit_lines_conflict",
        "grid-template-columns:97px [a-start] 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "x:grid-column:a-start;grid-row:2|y:grid-column:a-start 2;grid-row:1",
        "x=16,80,97,149 y=120,14,213,61",
    ),
    // #2717's rows (Chrome 153): `span <name>` against a negative line or
    // from one, its count taken from the nearest line, past the named lines
    // on either side (an implicit line before the grid when it counts back,
    // after its own start line when that is already past the grid), a
    // negative index past the named lines in the row axis, a missing name
    // with `-2`, and an absolute item whose line is outside the grid.
    (
        "k01_span_name_to_negative_line",
        "grid-template-columns:[s] 97px [m] 213px [m] 71px [e];grid-template-rows:61px 149px",
        "a:grid-column:span m / -1|b:grid-column:span m / -2;grid-row:2",
        "a=340,14,71,61 b=120,80,213,149",
    ),
    (
        "k02_negative_line_to_span_name",
        "grid-template-columns:[s] 97px [m] 213px [m] 71px [e];grid-template-rows:61px 149px",
        "a:grid-column:-4 / span m|b:grid-column:-3 / span m;grid-row:2",
        "a=16,14,97,61 b=120,80,213,149",
    ),
    (
        "k03_span_name_count_before",
        "grid-template-columns:[m] 97px [m] 213px [m] 71px [e];grid-template-rows:61px 149px",
        "a:grid-column:span m 1 / e|b:grid-column:span m 2 / e;grid-row:2|c:grid-column:span m 4 / e;grid-row:1",
        "a=347,234,71,11 b=127,80,291,149 c=16,14,402,61",
    ),
    (
        "k04_span_name_past_lines_after",
        "grid-template-columns:97px [m] 213px [m] 71px;grid-template-rows:61px 149px",
        "a:grid-column:2 / span m 3|b:grid-column:1;grid-row:2",
        "a=120,14,305,61 b=16,80,97,149",
    ),
    (
        "k05_span_missing_from_implicit_start",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px",
        "a:grid-column:6 / span nosuch|b:grid-column:1;grid-row:2",
        "a=432,14,0,61 b=16,80,97,149",
    ),
    (
        "k06_span_missing_to_line_before_grid",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px",
        "a:grid-column:span nosuch / -6|b:grid-column:1;grid-row:2",
        "a=16,14,0,61 b=37,80,97,149",
    ),
    (
        "k07_row_negative_past_name",
        "grid-template-columns:97px 213px;grid-template-rows:61px [x] 149px",
        "a:grid-row:x -2;grid-column:1|b:grid-row:x -1;grid-column:2",
        "a=16,14,97,11 b=120,96,213,149",
    ),
    (
        "k08_positive_past_names",
        "grid-template-columns:[a] 97px 213px [a] 71px;grid-template-rows:61px 149px",
        "a:grid-column:a 3;grid-row:1|b:grid-column:a 2;grid-row:2",
        "a=425,14,0,61 b=340,80,71,149",
    ),
    (
        "k09_missing_negative_two",
        "grid-template-columns:97px 213px;grid-template-rows:61px 149px",
        "a:grid-column:nosuch -2;grid-row:1|b:grid-column:1;grid-row:2",
        "a=16,14,34,61 b=98,80,97,149",
    ),
    (
        "k10_span_name_from_named_start",
        "grid-template-columns:[m] 97px [m] 213px [n] 71px [m];grid-template-rows:61px 149px",
        "a:grid-column:span m / n|b:grid-column:n / span m;grid-row:2",
        "a=120,14,213,61 b=340,80,71,149",
    ),
    (
        "k11_abs_negative_past_names",
        "grid-template-columns:[a] 97px [b] 213px [a] 71px [b];grid-template-rows:61px 149px",
        "a:abs;grid-column:a -3 / a 2;grid-row:2",
        "a=26,80,290,60",
    ),
    (
        "p01_abs_area",
        "grid-template-columns:97px 213px 71px;grid-template-rows:61px 149px;grid-template-areas:'a b c' 'd e f'",
        "a:abs;grid-area:e",
        "a=143,80,173,120",
    ),
    (
        "p02_abs_repeated_name",
        "grid-template-columns:[a] 1fr [a] 1fr [a] 1fr [a];grid-template-rows:61px 149px",
        "a:abs;grid-column:a 2 / a 3;grid-row:2",
        "a=174.33,80,88.33,240",
    ),
    (
        "p03_abs_template_shorthand",
        "grid-template:'h h h' 61px 's m n' 149px / 97px 213px 71px",
        "a:abs;grid-area:m",
        "a=143,80,173,120",
    ),
];

/// The style of a block around the grid, for a row that inherits from it.
const PARENTS: &[(&str, &str)] = &[("c23_inherit", "grid-template-areas:'a b c' 'd e f'")];

/// The row's markup: the grid and its items, as the Chrome page built it.
fn html(name: &str, grid: &str, items: &str) -> String {
    let mut abs = false;
    let kids: String = items
        .split('|')
        .map(|item| {
            let (m, style) = item.split_once(':').unwrap();
            match style.strip_prefix("abs;") {
                Some(rest) => {
                    abs = true;
                    format!(
                        r#"<div data-m="{m}" style="position:absolute;margin:0 17px 0 23px;{rest}">{TEXT}</div>"#
                    )
                }
                None => format!(r#"<div data-m="{m}" style="{style}"></div>"#),
            }
        })
        .collect();
    let pos = if abs { "position:relative;" } else { "" };
    let grid = grid.replace('\'', "&quot;");
    let html = format!(r#"<div style="{pos}{G};{grid}">{kids}</div>"#);
    match PARENTS.iter().find(|p| p.0 == name) {
        Some((_, parent)) => {
            let parent = parent.replace('\'', "&quot;");
            format!(r#"<div style="{parent}">{html}</div>"#)
        }
        None => html,
    }
}

fn rows() -> Vec<(&'static str, String, &'static str)> {
    CHROME
        .iter()
        .map(|(n, g, i, w)| (*n, html(n, g, i), *w))
        .collect()
}

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
    assert_eq!(registered.len(), 1, "one file, one family");
    doc
}

fn lay_out(html: &str, viewport: (f32, f32)) -> RinchDocument {
    let mut doc = document();
    doc.load_css("* { box-sizing: border-box; }");
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0; font: 16px/20px ProbeFace");
    let wrap = doc.create_element("div");
    doc.set_inner_html(wrap, html);
    doc.append_child(body, wrap);
    doc.resolve_layout(viewport.0, viewport.1);
    doc
}

fn one(doc: &RinchDocument, selector: &str) -> u64 {
    let found = query_selector(&doc.tree, selector);
    assert_eq!(found.len(), 1, "{selector} names exactly one node");
    found[0]
}

/// `mark=x,y,width,height` for every mark `want` names, in the viewport.
fn dump(doc: &RinchDocument, want: &str) -> String {
    want.split(' ')
        .map(|part| {
            let m = part.split_once('=').unwrap().0;
            let id = one(doc, &format!("[data-m={m}]"));
            let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, id, 1.0);
            let l = doc.tree.get(id).unwrap().layout;
            format!(
                "{m}={},{},{},{}",
                num(x as f32),
                num(y as f32),
                num(l.width),
                num(l.height)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Whether two dumps agree to within the pixel grid: rinch puts every box
/// edge on a whole pixel, Chrome reports the unrounded box.
fn close(want: &str, got: &str) -> bool {
    let nums = |s: &str| -> Vec<f32> {
        s.split(' ')
            .flat_map(|p| p.split_once('=').unwrap().1.split(','))
            .map(|v| v.parse().unwrap())
            .collect()
    };
    let (w, g) = (nums(want), nums(got));
    w.len() == g.len() && w.iter().zip(&g).all(|(a, b)| (a - b).abs() <= 0.75)
}

#[test]
fn placement_by_name_matches_chrome_153() {
    let rows = rows();
    let bad: Vec<_> = rows
        .iter()
        .filter_map(|(n, h, want)| {
            let got = dump(&lay_out(h, VIEWPORT), want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert_eq!(rows.len(), 54, "the table is being read");
    assert!(bad.is_empty(), "{} of {}:\n{bad:#?}", bad.len(), rows.len());
}

/// Every row laid out at another viewport height first, then at the measured
/// one: an incremental layout lands where a fresh one does.
#[test]
fn a_relayout_lands_where_a_fresh_layout_does() {
    let bad: Vec<_> = rows()
        .iter()
        .filter_map(|(n, h, want)| {
            let mut doc = lay_out(h, (VIEWPORT.0, VIEWPORT.1 + 37.0));
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            let got = dump(&doc, want);
            (!close(want, &got)).then(|| format!("{n}: chrome[{want}] rinch[{got}]"))
        })
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// Changing only `grid-template-areas` (or only a line's name) re-places the
/// item on the next layout: the names reach Taffy's style, and a style that
/// differs in nothing else is still a layout change.
#[test]
fn renaming_an_area_or_a_line_moves_the_item() {
    // `e` moves from the middle of the second row to its right two columns.
    let (_, h, want) = rows().into_iter().find(|r| r.0 == "n01_area").unwrap();
    let mut doc = lay_out(&h, VIEWPORT);
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
    let a = one(&doc, "[data-m=a]");
    let grid = NodeId(doc.tree.get(a).unwrap().parent.unwrap());
    let style = doc.get_attribute(grid, "style").unwrap();
    let moved = style.replace("\"d e f\"", "\"d f e\"");
    assert_ne!(style, moved, "the template is rewritten: {style}");
    doc.set_attribute(grid, "style", &moved);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    // Chrome 153: the third column, `340,80,71,149`.
    let want = "a=340,80,71,149";
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));

    // `m` moves from the second line to the third.
    let (_, h, want) = rows()
        .into_iter()
        .find(|r| r.0 == "n04_named_lines")
        .unwrap();
    let mut doc = lay_out(&h, VIEWPORT);
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
    let a = one(&doc, "[data-m=a]");
    let grid = NodeId(doc.tree.get(a).unwrap().parent.unwrap());
    let style = doc.get_attribute(grid, "style").unwrap();
    let moved = style.replace("[m] 213px [e]", "213px [m e]");
    assert_ne!(style, moved);
    doc.set_attribute(grid, "style", &moved);
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    // Chrome 153: `a` spans the third column, `b` the first two.
    let want = "a=340,80,71,149 b=16,14,317,61";
    assert!(close(want, &dump(&doc, want)), "{}", dump(&doc, want));
}

/// A table is laid out as a grid, but Chrome ignores `grid-template-areas`
/// and line names on one: the table and its first cell are where they are
/// without them (Chrome 153: the table 130x39, the cell at 10,10, 53x19). An
/// area template reaching the table's grid added three tracks and widened it.
#[test]
fn a_table_ignores_grid_areas_and_line_names() {
    let cell = r#"<tr><td data-m="c" style="padding:0;width:53px;height:19px;{c}"></td><td style="padding:0;width:47px;height:19px"></td></tr>"#;
    for (table, c) in [
        ("", ""),
        (
            "grid-template-areas:&quot;a b c&quot; &quot;d e f&quot;",
            "grid-area:e",
        ),
        (
            "grid-template-columns:[a] 13px [b] 200px [c];grid-template-rows:[r] 70px [s]",
            "grid-column:b / c",
        ),
    ] {
        let html = format!(
            r#"<table data-m="t" style="border-spacing:10px;{table}">{}</table>"#,
            cell.replace("{c}", c)
        );
        let doc = lay_out(&html, VIEWPORT);
        let want = "t=0,0,130,39 c=10,10,53,19";
        assert!(
            close(want, &dump(&doc, want)),
            "{table}: {}",
            dump(&doc, want)
        );
    }
}
