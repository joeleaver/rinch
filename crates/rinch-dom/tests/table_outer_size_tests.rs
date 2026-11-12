//! A table's outer size in other formatting contexts (#2403, #2395, #2328).
//!
//! A table sizes itself: it is never narrower than its columns' minimum nor
//! shorter than its rows, whatever width or height its parent or its own
//! `width`/`height` give it (CSS 2.1 §17.5.2.2, §17.5.3), and its insets, as
//! an absolutely positioned box, only place it. Within that, a flex or grid
//! container stretches an auto-width table as any item, where a block
//! container shrinks it to fit.
//!
//! Every expected number is Chrome 156's for the same markup inside
//! `<div style='position:relative;width:800px'>` (`getBoundingClientRect`
//! relative to that wrapper, `body { margin: 0; font: 16px/20px sans-serif }
//! td { padding: 0 } table { border-spacing: 0 }` at 800x600). The cells hold
//! fixed-size blocks and no text, so no number depends on the host's fonts.
//! rinch places boxes on whole pixels: `rowflex-w-shrink`'s 166.671875 is 167,
//! so the comparison allows half a pixel (and the expectation is 166.67).

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const CSS: &str =
    "body { margin: 0; font: 16px/20px sans-serif } td { padding: 0 } table { border-spacing: 0 }";

/// `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// A case: its name, its markup, and Chrome's box for each `#id` in it.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, Rect)]);

/// `(x, y, width, height)` of `#id`'s border box in the document.
fn rect(doc: &RinchDocument, id: &str) -> Rect {
    let n = doc.query_selector(&format!("#{id}")).expect(id);
    let (x, y) = rinch_dom::paint::compute_absolute_position(&doc.tree, n.0, 1.0);
    let l = doc.tree.get(n.0).unwrap().layout;
    (x as f32, y as f32, l.width, l.height)
}

/// Lay each case out in its own document and compare every named box with
/// Chrome's; report every case that differs, not only the first.
fn check(cases: &[Case]) {
    let mut failures = Vec::new();
    for (name, html, expected) in cases {
        let mut doc = RinchDocument::new();
        doc.load_css(CSS);
        let body = doc.body();
        doc.set_inner_html(
            body,
            &format!("<div style='position:relative;width:800px'>{html}</div>"),
        );
        doc.resolve_layout(800.0, 600.0);
        for (id, want) in *expected {
            let have = rect(&doc, id);
            let near = |a: f32, b: f32| (a - b).abs() <= 0.5;
            if !(near(have.0, want.0)
                && near(have.1, want.1)
                && near(have.2, want.2)
                && near(have.3, want.3))
            {
                failures.push(format!("{name}: #{id} {have:?}, Chrome {want:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_absolutely_positioned_table_is_not_sized_by_its_insets() {
    // #2403. A table's insets place it; it is as tall as its rows (or its `height`) and as wide as its
    // columns (or its `width`), between `top`/`bottom` or `left`/`right` alike, in a block, grid or
    // flex container, against a containing block that is not its parent, and as a fixed box. rinch
    // was: `abs-tb` 280 tall, `abs-grid-lr` 780 wide, `abs-w-narrow` 50 wide. Between `left` and
    // `right` it shrinks to fit the room they leave, so a long line of boxes wraps there
    // (`wrap-*`; review of #2600: laid out in the whole width, it was 300 wide).
    check(&[
        (
            "abs-tb",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0'><tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 10.0, 0.0, 60.0))],
        ),
        (
            "abs-tb-grid",
            "<div style='display:grid;position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0'><tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 10.0, 0.0, 60.0))],
        ),
        (
            "abs-tb-flex",
            "<div style='display:flex;position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0'><tr style='height:30px'><td></td></tr><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 10.0, 0.0, 60.0))],
        ),
        (
            "abs-tb-mauto",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0;margin:auto 0'><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 135.0, 0.0, 30.0))],
        ),
        (
            "abs-tb-sp",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0;border-spacing:3px;padding:2px'><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 10.0, 10.0, 40.0))],
        ),
        (
            "abs-tb-minh",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:10px;bottom:10px;left:0;min-height:100px'><tr style='height:30px'><td></td></tr></table></div>",
            &[("t", (0.0, 10.0, 0.0, 100.0))],
        ),
        (
            "abs-lr",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:10px;left:10px;right:10px'><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div>",
            &[("t", (10.0, 10.0, 50.0, 30.0))],
        ),
        (
            "abs-lr-mauto",
            "<div style='position:relative;height:300px'><table id=t style='position:absolute;top:0;left:10px;right:10px;margin:0 auto'><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div>",
            &[("t", (375.0, 0.0, 50.0, 30.0))],
        ),
        (
            "abs-grid-lr",
            "<div style='display:grid;position:relative;height:100px'><table id=t style='position:absolute;left:10px;right:10px;top:0'><tr><td><div style='width:30px;height:20px'></div></td></tr></table></div>",
            &[("t", (10.0, 0.0, 30.0, 20.0))],
        ),
        (
            "abs-flex-lr",
            "<div style='display:flex;position:relative;height:100px'><table id=t style='position:absolute;left:10px;right:10px;top:0'><tr><td><div style='width:30px;height:20px'></div></td></tr></table></div>",
            &[("t", (10.0, 0.0, 30.0, 20.0))],
        ),
        (
            "abs-cb-grand",
            "<div style='position:relative;height:300px;width:600px'><div style='height:50px'><table id=t style='position:absolute;top:10px;bottom:10px;left:10px;right:10px'><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div></div>",
            &[("t", (10.0, 10.0, 50.0, 30.0))],
        ),
        (
            "abs-cb-grand-flex",
            "<div style='position:relative;height:300px;width:600px'><div style='display:flex;height:50px'><table id=t style='position:absolute;top:10px;bottom:10px;left:10px;right:10px'><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div></div>",
            &[("t", (10.0, 10.0, 50.0, 30.0))],
        ),
        (
            "fixed-tb",
            "<table id=t style='position:fixed;top:10px;bottom:10px;left:10px;right:10px'><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table>",
            &[("t", (10.0, 10.0, 50.0, 30.0))],
        ),
        (
            "abs-w-narrow",
            "<div style='position:relative;height:100px'><table id=t style='position:absolute;left:10px;top:0;width:50px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (10.0, 0.0, 300.0, 20.0))],
        ),
        (
            "wrap-block-10",
            "<div style='position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:10px;right:10px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (10.0, 0.0, 280.0, 20.0))],
        ),
        (
            "wrap-block-50",
            "<div style='position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:50px;right:50px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (50.0, 0.0, 200.0, 30.0))],
        ),
        (
            "wrap-flex-10",
            "<div style='display:flex;position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:10px;right:10px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (10.0, 0.0, 280.0, 20.0))],
        ),
        (
            "wrap-flex-50",
            "<div style='display:flex;position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:50px;right:50px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (50.0, 0.0, 200.0, 30.0))],
        ),
        (
            "wrap-grid-10",
            "<div style='display:grid;position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:10px;right:10px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (10.0, 0.0, 280.0, 20.0))],
        ),
        (
            "wrap-grid-50",
            "<div style='display:grid;position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:50px;right:50px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (50.0, 0.0, 200.0, 30.0))],
        ),
        (
            "wrap-block-margin",
            "<div style='position:relative;width:300px;height:200px'><table id=t style='position:absolute;top:0;left:10px;right:10px;margin:0 20px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div>",
            &[("t", (30.0, 0.0, 240.0, 30.0))],
        ),
        (
            "wrap-grand",
            "<div style='position:relative;width:300px;height:200px'><div><table id=t style='position:absolute;top:0;left:50px;right:50px'><tr><td style='font-size:0;line-height:0'><span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span> <span style='display:inline-block;width:50px;height:10px'></span></td></tr></table></div></div>",
            &[("t", (50.0, 0.0, 200.0, 30.0))],
        ),
    ]);
}

#[test]
fn an_auto_width_table_is_stretched_as_a_flex_or_grid_item() {
    // #2395. An auto-width table is stretched to its column flex container or grid area, never
    // below its columns' minimum; `start`/`center` alignment and `width: fit-content` keep it at its
    // content's width. rinch was: `colflex` and `grid` 0 wide, `grid-col` 50.
    check(&[
        (
            "colflex",
            "<div style='display:flex;flex-direction:column;height:300px'><table id=t style='height:40px'><tr style='height:30px'><td id=c></td></tr><tr style='height:30px'><td></td></tr></table></div>",
            &[
                ("t", (0.0, 0.0, 800.0, 60.0)),
                ("c", (0.0, 0.0, 800.0, 30.0)),
            ],
        ),
        (
            "colflex-sp2",
            "<div style='display:flex;flex-direction:column;height:300px'><table id=t style='border-spacing:2px'><tr style='height:30px'><td id=c></td></tr></table></div>",
            &[
                ("t", (0.0, 0.0, 800.0, 34.0)),
                ("c", (2.0, 2.0, 796.0, 30.0)),
            ],
        ),
        (
            "colflex-2col",
            "<div style='display:flex;flex-direction:column'><table id=t><tr style='height:30px'><td id=c><div style='width:50px;height:5px'></div></td><td id=c2><div style='width:150px;height:5px'></div></td></tr></table></div>",
            &[
                ("t", (0.0, 0.0, 800.0, 30.0)),
                ("c", (0.0, 0.0, 200.0, 30.0)),
                ("c2", (200.0, 0.0, 600.0, 30.0)),
            ],
        ),
        (
            "grid",
            "<div style='display:grid;grid-template-rows:40px;height:300px'><table id=t><tr style='height:30px'><td id=c></td></tr><tr style='height:30px'><td></td></tr></table></div>",
            &[
                ("t", (0.0, 0.0, 800.0, 60.0)),
                ("c", (0.0, 0.0, 800.0, 30.0)),
            ],
        ),
        (
            "grid-col",
            "<div style='display:grid;grid-template-columns:300px 1fr'><table id=t><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table><div></div></div>",
            &[("t", (0.0, 0.0, 300.0, 30.0))],
        ),
        (
            "colflex-narrow",
            "<div style='display:flex;flex-direction:column;width:100px'><table id=t><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "grid-narrow",
            "<div style='display:grid;grid-template-columns:100px'><table id=t><tr><td><div style='width:300px;height:20px'></div></td></tr></table><div id=s style='height:5px'></div></div>",
            &[
                ("t", (0.0, 0.0, 300.0, 20.0)),
                ("s", (0.0, 20.0, 100.0, 5.0)),
            ],
        ),
        (
            "colflex-start",
            "<div style='display:flex;flex-direction:column;align-items:start'><table id=t><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 50.0, 30.0))],
        ),
        (
            "grid-jstart",
            "<div style='display:grid;justify-items:start'><table id=t><tr style='height:30px'><td><div style='width:50px;height:5px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 50.0, 30.0))],
        ),
        (
            "grid-jcenter",
            "<div style='display:grid;justify-items:center;width:400px'><table id=t><tr><td><div style='width:50px;height:5px'></div></td></tr></table></div>",
            &[("t", (175.0, 0.0, 50.0, 5.0))],
        ),
        (
            "colflex-fc",
            "<div style='display:flex;flex-direction:column'><table id=t style='width:fit-content'><tr><td><div style='width:30px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 30.0, 20.0))],
        ),
    ]);
}

#[test]
fn a_table_with_a_width_widens_to_its_columns_in_every_context() {
    // #2328. A table's columns' minimum is a floor under its `width` as a flex item (no `flex-basis`,
    // `min-width: 0` or shrinking lowers it), a grid item (its track contribution too), an
    // absolutely positioned box, and in a shrink-to-fit parent's contribution. A fixed table keeps
    // its width. rinch was: `rowflex-w` 100, `ib-w`'s wrapper 100.
    check(&[
        (
            "rowflex-w",
            "<div style='display:flex;width:600px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "rowflex-w-wide",
            "<div style='display:flex;width:600px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td><td><div style='width:10px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 310.0, 20.0))],
        ),
        (
            "rowflex-w-pad",
            "<div style='display:flex;width:600px'><table id=t style='width:100px;border-spacing:2px;padding:3px;border:1px solid'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 312.0, 32.0))],
        ),
        (
            "rowflex-w-basis",
            "<div style='display:flex;width:600px'><table id=t style='flex-basis:50px;width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "rowflex-w-minw0",
            "<div style='display:flex;width:600px'><table id=t style='width:100px;min-width:0'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "rowflex-w-grow",
            "<div style='display:flex;width:600px'><table id=t style='width:100px;flex-grow:1'><tr><td><div style='width:300px;height:20px'></div></td></tr></table><div id=s style='width:100px;height:5px'></div></div>",
            &[
                ("t", (0.0, 0.0, 500.0, 20.0)),
                ("s", (500.0, 0.0, 100.0, 5.0)),
            ],
        ),
        (
            "rowflex-w-shrink",
            "<div style='display:flex;width:200px'><table id=t style='width:500px'><tr><td><div style='width:100px;height:20px'></div></td></tr></table><div id=s style='width:100px;height:10px'></div></div>",
            &[
                ("t", (0.0, 0.0, 166.67, 20.0)),
                ("s", (166.67, 0.0, 33.33, 10.0)),
            ],
        ),
        (
            "rowflex-fixed",
            "<div style='display:flex;width:600px'><table id=t style='width:100px;table-layout:fixed'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 100.0, 20.0))],
        ),
        (
            "rowflex-pct",
            "<div style='display:flex;width:400px'><table id=t style='width:50%'><tr><td><div style='width:300px;height:5px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 5.0))],
        ),
        (
            "colflex-w",
            "<div style='display:flex;flex-direction:column;width:600px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "colflex-w-narrow",
            "<div style='display:flex;flex-direction:column;width:100px'><table id=t style='width:50px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "colflex-center",
            "<div style='display:flex;flex-direction:column;align-items:center;width:400px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:5px'></div></td></tr></table></div>",
            &[("t", (50.0, 0.0, 300.0, 5.0))],
        ),
        (
            "grid-w",
            "<div style='display:grid;width:600px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "grid-w-col",
            "<div style='display:grid;grid-template-columns:200px 100px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table><div id=s style='height:5px'></div></div>",
            &[
                ("t", (0.0, 0.0, 300.0, 20.0)),
                ("s", (200.0, 0.0, 100.0, 5.0)),
            ],
        ),
        (
            "grid-auto-cols",
            "<div style='display:grid;grid-template-columns:auto auto;width:600px'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table><div id=s style='height:5px'></div></div>",
            &[
                ("t", (0.0, 0.0, 300.0, 20.0)),
                ("s", (450.0, 0.0, 150.0, 5.0)),
            ],
        ),
        (
            "ib-w",
            "<div id=w style='vertical-align:top;display:inline-block'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[
                ("w", (0.0, 0.0, 300.0, 20.0)),
                ("t", (0.0, 0.0, 300.0, 20.0)),
            ],
        ),
        (
            "ib-w-pad",
            "<div id=w style='vertical-align:top;display:inline-block'><table id=t style='width:100px;border-spacing:2px;padding:3px;border:1px solid'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[
                ("w", (0.0, 0.0, 312.0, 32.0)),
                ("t", (0.0, 0.0, 312.0, 32.0)),
            ],
        ),
        (
            "float-w",
            "<div id=w style='float:left'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[
                ("w", (0.0, 0.0, 300.0, 20.0)),
                ("t", (0.0, 0.0, 300.0, 20.0)),
            ],
        ),
        (
            "abs-w-contrib",
            "<div style='position:relative;height:100px'><div id=w style='position:absolute'><table id=t style='width:100px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div></div>",
            &[
                ("w", (0.0, 0.0, 300.0, 20.0)),
                ("t", (0.0, 0.0, 300.0, 20.0)),
            ],
        ),
        (
            "ib-colflex-w",
            "<div id=w style='vertical-align:top;display:inline-flex;flex-direction:column'><table id=t style='width:100px'><tr><td><div style='width:300px;height:5px'></div></td></tr></table></div>",
            &[("w", (0.0, 0.0, 300.0, 5.0)), ("t", (0.0, 0.0, 300.0, 5.0))],
        ),
        (
            "ib-rowflex-w",
            "<div id=w style='vertical-align:top;display:inline-flex'><table id=t style='width:100px'><tr><td><div style='width:300px;height:5px'></div></td></tr></table><div id=s style='width:10px;height:5px'></div></div>",
            &[
                ("w", (0.0, 0.0, 310.0, 5.0)),
                ("t", (0.0, 0.0, 300.0, 5.0)),
                ("s", (300.0, 0.0, 10.0, 5.0)),
            ],
        ),
        (
            "ib-grid-w",
            "<div id=w style='vertical-align:top;display:inline-grid'><table id=t style='width:100px'><tr><td><div style='width:300px;height:5px'></div></td></tr></table></div>",
            &[("w", (0.0, 0.0, 300.0, 5.0)), ("t", (0.0, 0.0, 300.0, 5.0))],
        ),
    ]);
}

#[test]
fn an_auto_width_table_in_a_block_still_shrinks_to_fit() {
    // Unchanged: a block container does not stretch an auto-width table (it shrinks to fit its
    // room, and moves below a float it does not fit beside), whose `auto` width no longer reaches
    // Taffy as `fit-content`.
    check(&[
        (
            "block-mauto",
            "<div style='width:400px'><table id=t style='margin:0 auto'><tr><td><div style='width:30px;height:20px'></div></td></tr></table></div>",
            &[("t", (185.0, 0.0, 30.0, 20.0))],
        ),
        (
            "block-narrow",
            "<div style='width:100px'><table id=t><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 300.0, 20.0))],
        ),
        (
            "beside-float",
            "<div style='width:400px'><div id=s style='float:left;width:300px;height:50px'></div><table id=t><tr><td><div style='width:50px;height:20px'></div></td></tr></table></div>",
            &[
                ("s", (0.0, 0.0, 300.0, 50.0)),
                ("t", (300.0, 0.0, 50.0, 20.0)),
            ],
        ),
        (
            "beside-float-wide",
            "<div style='width:400px'><div id=s style='float:left;width:300px;height:50px'></div><table id=t><tr><td><div style='width:150px;height:20px'></div></td></tr></table></div>",
            &[
                ("s", (0.0, 0.0, 300.0, 50.0)),
                ("t", (0.0, 50.0, 150.0, 20.0)),
            ],
        ),
        (
            "float-auto",
            "<div style='width:400px'><table id=t style='float:left'><tr><td><div style='width:30px;height:20px'></div></td></tr></table><div id=s style='height:5px'></div></div>",
            &[("t", (0.0, 0.0, 30.0, 20.0)), ("s", (0.0, 0.0, 400.0, 5.0))],
        ),
        (
            "ib-nowidth",
            "<div id=w style='display:inline-block'><table id=t><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[
                ("w", (0.0, 0.0, 300.0, 20.0)),
                ("t", (0.0, 0.0, 300.0, 20.0)),
            ],
        ),
        (
            "ib-w-wide",
            "<div id=w style='display:inline-block'><table id=t style='width:400px'><tr><td><div style='width:300px;height:20px'></div></td></tr></table></div>",
            &[
                ("w", (0.0, 0.0, 400.0, 20.0)),
                ("t", (0.0, 0.0, 400.0, 20.0)),
            ],
        ),
        (
            "pct-w",
            "<div style='width:400px'><table id=t style='width:50%'><tr><td><div style='width:30px;height:5px'></div></td></tr></table></div>",
            &[("t", (0.0, 0.0, 200.0, 5.0))],
        ),
        (
            "anon-host",
            "<div style='display:flex;flex-direction:column;width:400px'><div id=t style='display:table-row'><div id=c style='display:table-cell'><div style='width:30px;height:5px'></div></div></div></div>",
            &[("t", (0.0, 0.0, 400.0, 5.0)), ("c", (0.0, 0.0, 30.0, 5.0))],
        ),
    ]);
}
