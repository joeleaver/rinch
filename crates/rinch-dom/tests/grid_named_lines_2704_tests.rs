//! #2704 — a grid container's `grid-template-areas` and the `[names]` of its
//! track lists are read, so an item placed by a name lands where Chrome puts
//! it: `grid-area: e`, an area's implicit `e-start` / `e-end` lines, an area
//! name used as a line (`grid-column: b / c`), named lines (`[m] 213px [e]`),
//! a repeated name picked by its count (`grid-column: a 2`, `a -1`), names
//! inside `repeat()`, the `grid-template` shorthand, and an area template
//! that alone sets the size of the explicit grid. Absolutely positioned items
//! placed by a name are in `abs_grid_area_2377_tests.rs` and in the `p` rows
//! here.
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

/// The row's markup: the grid and its items, as the Chrome page built it.
fn html(grid: &str, items: &str) -> String {
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
    format!(r#"<div style="{pos}{G};{grid}">{kids}</div>"#)
}

fn rows() -> Vec<(&'static str, String, &'static str)> {
    CHROME
        .iter()
        .map(|(n, g, i, w)| (*n, html(g, i), *w))
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
    assert_eq!(rows.len(), 17, "the table is being read");
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
