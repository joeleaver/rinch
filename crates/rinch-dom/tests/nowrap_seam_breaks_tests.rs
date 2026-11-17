//! Break opportunities at the edges of a run that does not wrap (`nowrap`,
//! `pre`), and inside one (#2390, #2353, #2352). Every expectation is Chrome
//! 156's for the same markup.
//!
//! - #2390: the opportunity between a `nowrap`/`pre` element's last
//!   character and wrapping text right after it (no space between) belongs
//!   to the element both sides share, which wraps, so the character pair
//!   decides it (`abc-|123`). parley allows no break after text that does
//!   not wrap, and the seams pass did not look at such a paragraph.
//! - #2353: a collapsible space after an overflowing `nowrap` run hangs at
//!   the end of the run's line: the opportunity is after the space, not
//!   before it.
//! - #2352: a float inside a `nowrap` run makes no opportunity of its own:
//!   the run stays on one line, overflowing.
//!
//! Text is the bundled Inter registered as `ProbeFace`, `font: 16px/20px`,
//! `* { margin: 0 }`; boxes are sized `vertical-align: top` inline-blocks in
//! 20px lines.

#![cfg(feature = "software-renderer")]

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

fn html(markup: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css("* { margin: 0; padding: 0 } body { font: 16px/20px ProbeFace }");
    let body = d.body();
    d.set_inner_html(body, markup);
    d.resolve_layout(800.0, 800.0);
    d
}

fn by_id(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

/// The text of each line of `#id`'s inline layout.
fn lines(d: &RinchDocument, id: &str) -> Vec<String> {
    let n = d.tree.get(by_id(d, id).0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    il.layout
        .lines()
        .map(|l| il.text_content[l.text_range()].to_string())
        .collect()
}

/// The border box of `#id`, relative to `#rel`'s.
fn rel(d: &RinchDocument, id: &str, to: &str) -> (f32, f32, f32, f32) {
    let page = |id: &str| {
        let n = by_id(d, id);
        let l = d.tree.get(n.0).unwrap().layout;
        let (mut x, mut y) = (0.0, 0.0);
        let mut cur = Some(n.0);
        while let Some(i) = cur {
            let node = d.tree.get(i).unwrap();
            x += node.layout.x;
            y += node.layout.y;
            cur = node.parent;
        }
        (x, y, l.width, l.height)
    };
    let (a, b) = (page(id), page(to));
    (a.0 - b.0, a.1 - b.1, a.2, a.3)
}

#[track_caller]
fn near(got: (f32, f32, f32, f32), chrome: (f32, f32, f32, f32)) {
    let off = [
        got.0 - chrome.0,
        got.1 - chrome.1,
        got.2 - chrome.2,
        got.3 - chrome.3,
    ];
    assert!(
        off.iter().all(|d| d.abs() <= 0.5),
        "rinch {got:?}, Chrome {chrome:?}"
    );
}

fn ib(w: u32, id: &str) -> String {
    let id = if id.is_empty() {
        String::new()
    } else {
        format!("id={id} ")
    };
    format!(
        "<span {id}style='display:inline-block;vertical-align:top;width:{w}px;height:20px'></span>"
    )
}

// --- #2390: the end of a run that does not wrap ------------------------

/// The issue's rows: a `pre` element's `abc-` then `123`. rinch kept
/// `abc-123` together at both widths.
#[test]
fn a_break_after_a_pre_elements_hyphen_before_wrapping_digits() {
    let row = "Say <span style='white-space:pre'>abc-</span>123 ok";
    let d = html(&format!(
        "<div id=a style='width:1px'>{row}</div><div id=b style='width:72px'>{row}</div>"
    ));
    assert_eq!(lines(&d, "a"), ["Say ", "abc-", "123 ", "ok"]);
    assert_eq!(lines(&d, "b"), ["Say abc-", "123 ok"]);
}

/// A `nowrap` element alone before the text: the paragraph holds no space
/// and no box, and the seam is its only opportunity.
#[test]
fn a_break_after_a_nowrap_elements_hyphen() {
    let d =
        html("<div id=a style='width:1px'><span style='white-space:nowrap'>abc-</span>123</div>");
    assert_eq!(lines(&d, "a"), ["abc-", "123"]);
}

/// The same seam with a space inside the `nowrap` run: the run stays whole.
#[test]
fn a_nowrap_run_with_a_space_breaks_only_at_its_end() {
    let d = html(
        "<div id=a style='width:1px'>Say <span style='white-space:nowrap'>x abc-</span>123 ok</div>",
    );
    assert_eq!(lines(&d, "a"), ["Say ", "x abc-", "123 ", "ok"]);
}

/// Two `nowrap` siblings in a wrapping root: their common ancestor wraps.
#[test]
fn a_break_between_two_nowrap_siblings() {
    let d = html(
        "<div id=a style='width:1px'><span style='white-space:nowrap'>abc-</span><span style='white-space:nowrap'>123</span></div>",
    );
    assert_eq!(lines(&d, "a"), ["abc-", "123"]);
}

/// Where the pair has no opportunity (`c1`) or the shared element does not
/// wrap (a `nowrap` root), the text stays together, as before.
#[test]
fn no_break_where_the_pair_or_the_shared_element_forbids_it() {
    let d = html(concat!(
        "<div id=a style='width:1px'><span style='white-space:nowrap'>abc</span>123</div>",
        "<div id=b style='width:1px'><span style='white-space:nowrap'>abc/</span>def</div>",
        "<div id=c style='width:1px;white-space:nowrap'>abc-<span style='white-space:normal'>123</span></div>",
    ));
    assert_eq!(lines(&d, "a"), ["abc123"]);
    assert_eq!(lines(&d, "b"), ["abc/def"]);
    assert_eq!(lines(&d, "c"), ["abc-123"]);
}

// --- #2353: a space after an overflowing run ---------------------------

/// The issue's shape: the space after the 603px run hangs on the run's
/// line, and `b` starts the next at x 0 (rinch: x 4.5, after the space).
#[test]
fn a_space_after_an_overflowing_nowrap_run_hangs() {
    let d = html(&format!(
        "<div id=d style='width:560px'><span style='white-space:nowrap'>{}{}</span> {}</div>",
        ib(300, ""),
        ib(303, ""),
        ib(50, "b")
    ));
    near(rel(&d, "b", "d"), (0.0, 20.0, 50.0, 20.0));
    near(rel(&d, "d", "d"), (0.0, 0.0, 560.0, 40.0));
}

/// The same beside a float: the run's line goes below the float (#2331),
/// and `b` starts the line after it at x 0.
#[test]
fn a_space_after_an_overflowing_nowrap_run_beside_a_float_hangs() {
    let d = html(&format!(
        "<div id=d style='width:560px'><div style='float:left;width:100px;height:31px'></div><span style='white-space:nowrap'>{}{}</span> {}</div>",
        ib(300, ""),
        ib(303, ""),
        ib(50, "b")
    ));
    near(rel(&d, "b", "d"), (0.0, 51.0, 50.0, 20.0));
    near(rel(&d, "d", "d"), (0.0, 0.0, 560.0, 71.0));
}

// --- #2352: a float inside a run -----------------------------------------

/// The issue's shape: the float after `ib 50` goes to the line's start and
/// the run stays on its line, overflowing (rinch broke after the float and
/// put the second box below it, x 0 y 47, the div 67 tall).
#[test]
fn a_float_inside_a_nowrap_run_makes_no_break() {
    let d = html(&format!(
        "<div id=d style='width:400px'><span style='white-space:nowrap'>{}<div style='float:left;width:300px;height:47px'></div>{}</span></div>",
        ib(50, "a"),
        ib(300, "b")
    ));
    near(rel(&d, "a", "d"), (300.0, 0.0, 50.0, 20.0));
    near(rel(&d, "b", "d"), (350.0, 0.0, 300.0, 20.0));
    near(rel(&d, "d", "d"), (0.0, 0.0, 400.0, 20.0));
}
