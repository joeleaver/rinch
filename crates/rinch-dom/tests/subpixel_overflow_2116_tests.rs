//! #2116: a scroll container whose content overflows only by rounding shows no
//! scrollbar, takes no wheel and has no range — and one whose content
//! overflows by half a pixel or more does, as in Chrome.
//!
//! Taffy rounds each box's *location* on its own (`round(location)`) and its
//! *size* from its absolute edges (`round(cum + size) - round(cum)`), so the
//! rounded rects of a box's children need not add up to its rounded size: the
//! menu bar's two-item dropdown (border 1, padding 4, two 32.8px rows and a
//! separator) came out with its last row ending a pixel past its content box at
//! some vertical positions, and painted a bar over content that fits.
//!
//! **Chrome's rule, measured in Chrome 156** (`scrollTop = 1e6`, then the
//! bar's width and the `scrollTop` it kept), for a `height: H` scroller holding
//! one `height: H + d` child, after a `10 + f` px spacer:
//!
//! | `H` | first `d` (0.01 steps) with a bar | `scrollHeight` / `clientHeight` there |
//! |---|---|---|
//! | 100 | 0.50 | 101 / 100 |
//! | 100.3 | 0.20 | 101 / 100 |
//! | 100.7 | 0.80 | 102 / 101 |
//!
//! The same for every `f` in {0, .25, .4, .5, .6, .75} and at a device pixel
//! ratio of 1, 1.25 and 1.5, monotonic in `d`, and the kept `scrollTop` is 1
//! each time: Chrome rounds the scrollable overflow and the client box each to
//! a whole CSS pixel, independently of the box's position and of the device
//! scale, and shows a bar when the first is larger. So `scrollHeight` and
//! `clientHeight` are what the fixtures below compare, at three scales and ten
//! positions — off the fixed point, since a box on a whole pixel rounds nothing.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use rinch_dom::paint::scrollbar::{ScrollbarAxis, can_scroll_toward, overflows, scrollbars};

const VIEWPORT: (f32, f32) = (400.0, 600.0);
const SCALES: [f64; 3] = [1.0, 1.25, 1.5];
const OFFSETS: [f32; 10] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];

/// A `10 + f` px spacer, then a scroll container with `container` style holding
/// one `div` per entry of `children`, laid out at `scale`.
fn build(f: f32, scale: f64, container: &str, children: &[&str]) -> (RinchDocument, NodeId) {
    let mut doc = RinchDocument::new();
    doc.set_device_pixel_ratio(scale as f32);
    let body = doc.body();
    let spacer = doc.create_element("div");
    doc.set_attribute(spacer, "style", &format!("height: {}px", 10.0 + f));
    doc.append_child(body, spacer);
    let c = doc.create_element("div");
    doc.set_attribute(c, "style", container);
    doc.append_child(body, c);
    for style in children {
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", style);
        doc.append_child(c, child);
    }
    doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
    (doc, c)
}

/// What the container answers on the vertical axis: a bar, the wheel, its
/// overflow, and `scrollHeight - clientHeight`.
#[derive(Debug, PartialEq)]
struct Answer {
    bar: bool,
    wheel: bool,
    overflows: bool,
    range: f64,
}

fn answer(doc: &RinchDocument, c: NodeId, scale: f64) -> Answer {
    let tree = &doc.tree;
    Answer {
        bar: scrollbars(tree, c.0, scale).vertical.is_some(),
        wheel: can_scroll_toward(tree, c.0, 0.0, 1.0),
        overflows: overflows(tree, c.0, ScrollbarAxis::Vertical),
        // `scrollHeight` is never below `clientHeight` in a browser; here a
        // scroller emptier than its box reports what it holds.
        range: (doc.scroll_height(c) - doc.client_height(c)).max(0.0),
    }
}

const FITS: Answer = Answer {
    bar: false,
    wheel: false,
    overflows: false,
    range: 0.0,
};

/// The issue's dropdown: `overflow-y: auto`, border 1, padding 4, two rows of
/// `height: 24.8px; padding: 4px 10px` (a 16px line at `line-height: 1.55`,
/// declared so no font is measured) and a 1px separator with `margin: 4px 0`.
/// 74.6px of content in a 74.6px content box, at every position and scale.
#[test]
fn the_menu_dropdown_fits_at_every_position_and_scale() {
    let row = "height: 24.8px; padding: 4px 10px; box-sizing: content-box";
    let mut wrong = Vec::new();
    for scale in SCALES {
        for f in OFFSETS {
            let (doc, c) = build(
                f,
                scale,
                "overflow-y: auto; padding: 4px; border: 1px solid black; width: 200px",
                &[row, "height: 1px; margin: 4px 0", row],
            );
            let got = answer(&doc, c, scale);
            if got != FITS {
                wrong.push(format!("scale {scale} offset {f}: {got:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a bar over content that fits:\n{}",
        wrong.join("\n")
    );
}

/// An IFC root's lines are measured unrounded (three 24.8px lines are 74.4px)
/// while its box is rounded: an `auto`-height one holding them fits.
#[test]
fn an_auto_height_text_scroller_fits_its_own_lines() {
    let mut wrong = Vec::new();
    for scale in SCALES {
        for f in OFFSETS {
            let mut doc = RinchDocument::new();
            doc.set_device_pixel_ratio(scale as f32);
            let body = doc.body();
            let spacer = doc.create_element("div");
            doc.set_attribute(spacer, "style", &format!("height: {}px", 10.0 + f));
            doc.append_child(body, spacer);
            let c = doc.create_element("div");
            doc.set_attribute(
                c,
                "style",
                "overflow-y: auto; width: 200px; padding: 3px; font-size: 16px; \
                 line-height: 24.8px; white-space: pre",
            );
            doc.append_child(body, c);
            let t = doc.create_text("a\nb\nc");
            doc.append_child(c, t);
            doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
            let got = answer(&doc, c, scale);
            if got != FITS {
                wrong.push(format!("scale {scale} offset {f}: {got:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a bar over lines that fit:\n{}",
        wrong.join("\n")
    );
}

/// Chrome's threshold, from the table at the top: `(H, d, bar)`. Below it no
/// bar and no range; at or past it a bar, the wheel, and a range of one pixel.
#[test]
fn sub_pixel_overflow_follows_chromes_rounding() {
    let cases: [(f32, f32, bool); 8] = [
        (100.0, 0.3, false),
        (100.0, 0.45, false),
        (100.0, 0.6, true),
        (100.0, 0.9, true),
        (100.3, 0.1, false),
        (100.3, 0.3, true),
        (100.7, 0.7, false),
        (100.7, 0.9, true),
    ];
    let mut wrong = Vec::new();
    for scale in SCALES {
        for f in OFFSETS {
            for (h, d, bar) in cases {
                let (doc, c) = build(
                    f,
                    scale,
                    &format!("overflow-y: auto; width: 200px; height: {h}px"),
                    &[&format!("height: {}px", h + d)],
                );
                let got = answer(&doc, c, scale);
                let want = if bar {
                    Answer {
                        bar: true,
                        wheel: true,
                        overflows: true,
                        range: 1.0,
                    }
                } else {
                    FITS
                };
                if got != want {
                    wrong.push(format!("scale {scale} offset {f} H {h} d {d}: {got:?}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} differ from Chrome:\n{}",
        wrong.len(),
        SCALES.len() * OFFSETS.len() * cases.len(),
        wrong.join("\n")
    );
}

/// A whole pixel of overflow is a range of one pixel wherever the box is,
/// though the box, its padding and its child are all fractional, and ten are
/// ten (Chrome: `scrollHeight` 101 / 110 against `clientHeight` 100). The
/// rounded rects measured 8.3 for the ten at some positions.
#[test]
fn whole_pixel_overflow_keeps_its_range() {
    for scale in SCALES {
        for f in OFFSETS {
            for d in [1.0_f32, 10.0] {
                let (doc, c) = build(
                    f,
                    scale,
                    "overflow-y: auto; width: 200px; height: 100.4px; padding: 2.3px; box-sizing: border-box",
                    &[&format!("height: {}px", 100.4 - 4.6 + d)],
                );
                let got = answer(&doc, c, scale);
                assert!(
                    got.bar && got.wheel && got.overflows,
                    "{scale} {f} {d}: {got:?}"
                );
                assert_eq!(got.range, d as f64, "scale {scale} offset {f} d {d}");
            }
        }
    }
}

/// A scroller emptier than its box still reports what it holds, as before
/// #2116 (a browser's `scrollHeight` would answer the client height): the
/// whole-pixel rule moves only an overflowing axis.
#[test]
fn an_emptier_scroller_reports_what_it_holds() {
    for scale in SCALES {
        for f in OFFSETS {
            let (doc, c) = build(
                f,
                scale,
                "overflow-y: auto; width: 200px; height: 100.4px",
                &["height: 30.3px"],
            );
            assert!((doc.scroll_height(c) - 30.3).abs() < 0.01, "{scale} {f}");
            assert_eq!(answer(&doc, c, scale), FITS, "{scale} {f}");
        }
    }
}

/// The thresholds themselves, on both sides (Chrome 156, every position and
/// scale): content that rounds up from exactly half a pixel is a range of
/// one, a hundredth less is none. `(H, content, bar)`, written as CSS so no
/// float sum lands beside the half. Round-half-down, round-half-to-even and a
/// `> 0.5` compare each fail a row here.
const THRESHOLDS: [(&str, &str, bool); 6] = [
    ("100", "100.49", false),
    ("100", "100.5", true),
    ("100.3", "100.49", false),
    ("100.3", "100.5", true),
    ("100.7", "101.49", false),
    ("100.7", "101.5", true),
];

fn want(bar: bool) -> Answer {
    if bar {
        Answer {
            bar: true,
            wheel: true,
            overflows: true,
            range: 1.0,
        }
    } else {
        FITS
    }
}

#[test]
fn the_half_pixel_threshold_is_exact() {
    let mut wrong = Vec::new();
    for scale in SCALES {
        for f in OFFSETS {
            for (h, content, bar) in THRESHOLDS {
                let (doc, c) = build(
                    f,
                    scale,
                    &format!("overflow-y: auto; width: 200px; height: {h}px"),
                    &[&format!("height: {content}px")],
                );
                let got = answer(&doc, c, scale);
                if got != want(bar) {
                    wrong.push(format!("scale {scale} offset {f} H {h} {content}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The same thresholds across: a `W` wide `overflow-x: auto` box, `10 + f`
/// px from the left, holding a `content` wide child. Chrome 156 answers as it
/// does vertically (`scrollWidth`/`clientWidth` 101/100, 102/101 at the bar),
/// at every position and scale.
#[test]
fn the_horizontal_axis_follows_the_same_rule() {
    let mut wrong = Vec::new();
    for scale in SCALES {
        for f in OFFSETS {
            for (w, content, bar) in THRESHOLDS {
                let mut doc = RinchDocument::new();
                doc.set_device_pixel_ratio(scale as f32);
                let body = doc.body();
                let c = doc.create_element("div");
                doc.set_attribute(
                    c,
                    "style",
                    &format!(
                        "margin-left: {}px; overflow-x: auto; overflow-y: hidden; \
                         width: {w}px; height: 50px",
                        10.0 + f
                    ),
                );
                doc.append_child(body, c);
                let child = doc.create_element("div");
                doc.set_attribute(child, "style", &format!("width: {content}px; height: 20px"));
                doc.append_child(c, child);
                doc.resolve_layout(VIEWPORT.0, VIEWPORT.1);
                let tree = &doc.tree;
                let got = Answer {
                    bar: scrollbars(tree, c.0, scale).horizontal.is_some(),
                    wheel: can_scroll_toward(tree, c.0, 1.0, 0.0),
                    overflows: overflows(tree, c.0, ScrollbarAxis::Horizontal),
                    range: (doc.scroll_width(c) - doc.client_width(c)).max(0.0),
                };
                if got != want(bar) {
                    wrong.push(format!("scale {scale} offset {f} W {w} {content}: {got:?}"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
