//! Where a wrapping run of spaces meets a `pre` element's spaces (#1243).
//!
//! CSS Text 3 hangs a preserved space only where its text wraps, so a `pre`
//! element's spaces never hang at a soft wrap, and `white-space: pre` allows
//! no break inside or after them. Chrome 155 breaks **between** a run of
//! spaces that wraps and a `pre` one: the first run hangs at the end of the
//! line, the `pre` spaces start the next. parley 0.11.1 has no opportunity
//! there (UAX #14 has none between two spaces), so it kept such a paragraph
//! on one overflowing line, or hung the one `pre` space that overflowed and
//! broke the `pre` run after it. Such a paragraph now takes the seams path of
//! #1238 (`ifc::WrapSeams`, `ifc::css_line`).
//!
//! How Chrome takes that break depends on the text nodes the wrapping run
//! lies in (`ifc::pre_seam`): an ordinary break when the wrapping spaces'
//! element closes there into one that does not wrap, or when the run starts
//! in the word's text node and goes on into another; only when what is
//! before it fits when the run ends the word's text node; and only when the
//! line overflows before the `pre` spaces when the run is a text node of its
//! own.
//!
//! Every expectation is Chrome 155's on the same markup: the bundled Inter as
//! `ProbeFace`, `16px/25px`, each line read as the x of its first character
//! from the container's left edge.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// Each line's text and alignment offset, for `<div style="font:16px/25px
/// ProbeFace;{style}">{inner}</div>`.
fn lines(style: &str, inner: &str) -> Vec<(String, f32)> {
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
    let body = d.body();
    let c = d.create_element("div");
    d.set_attribute(c, "style", &format!("font:16px/25px ProbeFace;{style}"));
    d.append_child(body, c);
    d.set_inner_html(c, inner);
    d.resolve_layout(800.0, 600.0);
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    il.layout
        .lines()
        .map(|l| {
            (
                il.text_content[l.text_range()].to_string(),
                l.metrics().offset,
            )
        })
        .collect()
}

/// The lines' texts, trimmed, and offsets as Chrome reads them.
fn assert_lines(got: &[(String, f32)], want: &[(&str, f32)]) {
    let texts: Vec<&str> = got.iter().map(|(t, _)| t.trim()).collect();
    let want_texts: Vec<&str> = want.iter().map(|(t, _)| *t).collect();
    assert_eq!(texts, want_texts, "{got:?}");
    for ((_, off), (t, want)) in got.iter().zip(want) {
        assert!(
            (off - want).abs() < 0.06,
            "line {t:?} at {off}, Chrome {want}: {got:?}"
        );
    }
}

const RIGHT_50: &str = "width:50px;text-align:right";

/// The issue's case (c8 of the #1228 review). `aaaa` and its three
/// `pre-wrap` spaces fit; the first `pre` space does not. parley hung it and
/// broke the `pre` run after it (`aaaa` + 4 spaces at 0, ` bbbb` at 6.31);
/// Chrome breaks before the `pre` spaces: `aaaa` at 14.06 (its spaces hang),
/// `  bbbb` at 1.81.
#[test]
fn a_pre_space_after_hanging_spaces_starts_the_next_line() {
    let l = lines(
        &format!("{RIGHT_50};white-space:pre-wrap"),
        "aaaa   <span style=\"white-space:pre\">  </span>bbbb",
    );
    assert_lines(&l, &[("aaaa", 14.06), ("bbbb", 1.81)]);
    assert_eq!(l[1].0, "  bbbb", "the pre spaces start the line");
}

/// Nothing overflows into a hang: `aaaa` and one collapsed space fit, the
/// `pre` spaces and `bbbb` do not, and parley found no break between the
/// two runs of spaces, so the paragraph was one overflowing line.
#[test]
fn a_normal_space_then_pre_spaces_break_between_them() {
    let l = lines(
        &format!("{RIGHT_50};white-space:normal"),
        "aaaa <span style=\"white-space:pre\">  </span>bbbb",
    );
    assert_lines(&l, &[("aaaa", 14.06), ("bbbb", 1.81)]);
}

/// A `pre` run that parley broke in the middle: it hung the one `pre` space
/// that overflowed after `aaa` (`  aaa ` / `     aa`). There is no break
/// before or inside the run, so `aaa` and the whole `pre` text overflow on
/// one line (Chrome: `  ` at 40, then `aaa      aa` at 0).
#[test]
fn a_pre_run_is_not_broken_where_one_of_its_spaces_overflows() {
    let l = lines(
        "width:40px;text-align:right;white-space:pre-wrap",
        "  aaa<span style=\"white-space:pre\">      aa</span>",
    );
    assert_lines(&l, &[("", 40.0), ("aaa      aa", 0.0)]);
}

/// When a `pre` element opens after the wrapping spaces, Chrome takes the
/// break only if what is before it fits. `hhhhhhh` alone overflows 50px, so
/// the line runs on to the next opportunity, after `bb `.
#[test]
fn an_overflowing_word_keeps_the_pre_spaces_that_open_after_it() {
    let l = lines(
        &format!("{RIGHT_50};white-space:pre-wrap"),
        "hhhhhhh  <span style=\"white-space:pre\">  </span>bb cc",
    );
    assert_lines(&l, &[("hhhhhhh    bb", 0.0), ("cc", 31.72)]);
}

/// Where the wrapping spaces' element closes into a `pre` one, the break is
/// ordinary: taken even though `hhhhhhh` overflows. Into the `pre` root's
/// own spaces, and into it with a `pre` span opening right after.
#[test]
fn spaces_closing_into_pre_break_after_an_overflowing_word() {
    let l = lines(
        &format!("{RIGHT_50};white-space:pre"),
        "<span style=\"white-space:pre-wrap\">hhhhhhh  </span>  bbbb",
    );
    assert_lines(&l, &[("hhhhhhh", 0.0), ("bbbb", 1.81)]);
    let l = lines(
        "width:60px;text-align:right;white-space:pre",
        "fhhhhaaaa cccc<span style=\"white-space:pre-wrap\">aa</span>\
         <span style=\"white-space:pre-wrap\">   </span>\
         <span style=\"white-space:pre\">   gg</span>",
    );
    assert_lines(&l, &[("fhhhhaaaa ccccaa", 0.0), ("gg", 26.88)]);
}

/// parley breaks after the leading space, the last opportunity it sees
/// before the overflow; the break before the `pre` spaces comes later and
/// fits, and CSS ends the line at the last break that fits (Chrome:
/// ` eee    ` at 47.52, `   hhhbbbbb` at 0).
#[test]
fn the_last_break_that_fits_is_taken_not_parleys_earlier_one() {
    let l = lines(
        "width:80px;text-align:right;white-space:nowrap",
        "<span style=\"white-space:pre-wrap\"> eee    </span>\
         <span style=\"white-space:pre\">   </span>hhhbbbbb",
    );
    assert_lines(&l, &[("eee", 47.52), ("hhhbbbbb", 0.0)]);
}

/// The run of wrapping spaces is a text node of its own: Chrome takes the
/// break only when the line overflows before the `pre` spaces. `aaaa` and the
/// run fit, and the paragraph stays on one line; `hhhhhhh` overflows, and
/// the `pre` spaces start the next line.
#[test]
fn a_run_of_its_own_breaks_only_after_an_overflow() {
    let style = format!("{RIGHT_50};white-space:pre-wrap");
    let html = |w: &str| {
        format!(
            "{w}<span style=\"white-space:pre-wrap\">  </span>\
             <span style=\"white-space:pre\">  dd</span>"
        )
    };
    assert_lines(&lines(&style, &html("aaaa")), &[("aaaa    dd", 0.0)]);
    assert_lines(
        &lines(&style, &html("hhhhhhh")),
        &[("hhhhhhh", 0.0), ("dd", 21.41)],
    );
}

/// The run starts in the word's text node and goes on into another: an
/// ordinary break, taken after the overflowing `hhhhhhh`.
#[test]
fn a_run_across_two_text_nodes_breaks_after_an_overflow() {
    let l = lines(
        &format!("{RIGHT_50};white-space:pre-wrap"),
        "<span style=\"white-space:pre-wrap\">hhhhhhh   </span>\
         <span style=\"white-space:pre-wrap\">  </span>\
         <span style=\"white-space:pre\">  dd</span>",
    );
    assert_lines(&l, &[("hhhhhhh", 0.0), ("dd", 21.41)]);
}

/// A collapsible space then `break-spaces` spaces break between them: the
/// first hangs, the `break-spaces` run starts the next line (a paragraph
/// that holds `pre` spaces, so it takes the seams path).
#[test]
fn a_wrapping_space_then_break_spaces_break_between_them() {
    let l = lines(
        "width:70px;text-align:right;white-space:break-spaces",
        " ggg<span style=\"white-space:normal\">cccc  </span>        \
         <span style=\"white-space:pre\">b   </span>   ",
    );
    assert_lines(&l, &[("", 65.5), ("gggcccc", 4.0), ("b", 1.70), ("", 65.5)]);
    assert!(l[2].0.starts_with("        b"), "{l:?}");
}
