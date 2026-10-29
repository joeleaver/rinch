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

/// `<div style="font:16px/25px ProbeFace;{style}">{inner}</div>`, laid out.
fn lay_out(style: &str, inner: &str) -> (RinchDocument, rinch_core::dom::NodeId) {
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
    (d, c)
}

/// Each line's text and the x of its first character (its caret, as
/// Chrome's character rects are read: an inline box or an element's start
/// padding before it on the line is counted), for [`lay_out`].
fn lines(style: &str, inner: &str) -> Vec<(String, f32)> {
    use rinch_core::dom::CaretAffinity;
    use rinch_dom::text_query as tq;
    let (d, c) = lay_out(style, inner);
    let n = d.tree.get(c.0).unwrap();
    let il = n.text_layout.as_ref().expect("the div is an IFC root");
    il.layout
        .lines()
        .map(|l| {
            let r = l.text_range();
            let x = tq::caret_position_for_offset_layout_with_affinity(
                &il.layout,
                r.start,
                CaretAffinity::Downstream,
            )
            .0;
            (il.text_content[r].to_string(), x)
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

/// A collapsible space then `break-spaces` spaces break between them in
/// Chrome 153: the first hangs, the `break-spaces` run starts the next line
/// (a paragraph that holds `pre` spaces, so it takes the seams path):
/// `("", 65.5), ("gggcccc", 4.0), ("b", 1.70), ("", 65.5)`, the third line
/// starting with all eight `break-spaces` spaces.
///
/// **Not Chrome's since parley 0.12, pinned.** parley now lays `break-spaces`
/// out itself (`WhiteSpaceCollapse::BreakSpaces`): it breaks after the first
/// `break-spaces` space, not before it, so that space and the collapsible one
/// before it stay on the second line, which overflows (75px, start-aligned),
/// and the third line fits the rest. The seams pass agrees with parley there:
/// the line is the same without it.
#[test]
fn a_wrapping_space_then_break_spaces_break_between_them() {
    let l = lines(
        "width:70px;text-align:right;white-space:break-spaces",
        " ggg<span style=\"white-space:normal\">cccc  </span>        \
         <span style=\"white-space:pre\">b   </span>   ",
    );
    assert_lines(&l, &[("", 65.5), ("gggcccc", 0.0), ("b", 1.70)]);
    assert!(l[2].0.starts_with("       b"), "{l:?}");
}

/// A word in a plain `<span>`, then its parent's collapsible space, then
/// `pre` spaces: Chrome finds that run as if it were in the word's text node
/// (review of #1897), and breaks before the `pre` spaces whether or not
/// the word fits. Main and the first head of #1897 kept the first two words
/// on one overflowing line.
#[test]
fn a_collapsible_space_after_a_closing_span_breaks_before_pre_spaces() {
    let l = lines(
        "width:47px;text-align:right;white-space:normal",
        "<span>dd</span> <span style=\"white-space:pre\">    dd</span> ee",
    );
    assert_lines(&l, &[("dd", 27.41), ("dd", 9.41), ("ee", 28.34)]);
    // With an inline box before the word, in a `pre-line` root: the first
    // head of #1897 put the box on a line of its own.
    let l = lines(
        "width:48px;text-align:right;white-space:pre-line",
        "<b style=\"display:inline-block;width:12px;height:10px\"></b>\
         <span>bbb</span> <span style=\"white-space:pre\">    dd</span>",
    );
    assert_lines(&l, &[("bbb", 18.61), ("dd", 10.41)]);
}

/// A paragraph with no `pre` spaces is laid out as main lays it out
/// (review of #1897): the wrapping-space-then-`break-spaces` break belongs
/// to the `pre`-space seams only. Here, on the #1426 edge-box path, the
/// first head of #1897 broke between the padded span's `pre-wrap` spaces
/// and the root's `break-spaces` ones, so the `pre-wrap` spaces stopped
/// hanging (line 1 at 3.38) and the caret at the start of line 2 was drawn
/// at the end of line 1. Main's answer is kept: 18.88 (Chrome 16.88, a
/// separate difference), `   cc` at 64.22 after the span's end padding, and
/// the caret at the start of line 2 is on line 2.
#[test]
fn break_spaces_after_a_padded_span_without_pre_spaces_are_main_s() {
    use rinch_core::dom::CaretAffinity;
    use rinch_dom::text_query as tq;
    let style = "width:96px;text-align:right;white-space:break-spaces";
    let html = "aaaa<span style=\"white-space:pre-wrap;padding:0 2px\">bbbb   </span>   cc";
    let l = lines(style, html);
    assert_lines(&l, &[("aaaabbbb", 18.88), ("cc", 64.22)]);
    let (d, c) = lay_out(style, html);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let second = il.layout.lines().nth(1).unwrap();
    let (y0, y1) = (
        second.metrics().block_min_coord,
        second.metrics().block_max_coord,
    );
    let (_, y) = tq::caret_position_for_offset_layout_with_affinity(
        &il.layout,
        second.text_range().start,
        CaretAffinity::Downstream,
    );
    assert!(
        y >= y0 - 0.5 && y <= y1 + 0.5,
        "caret y {y} on line 2 ({y0}..{y1})"
    );
}

/// A line whose trailing run is `pre` spaces then a collapsible one: the
/// collapsible space hangs, the `pre` spaces are content (#1243's second
/// item). parley counted the whole run as hanging and nothing sent the
/// paragraph through the hang pass (main: 11.81).
#[test]
fn pre_spaces_before_a_hanging_space_are_content() {
    let l = lines(
        "width:60px;text-align:right;white-space:normal",
        "<span style=\"white-space:pre\">  dddd  </span>  ccccd",
    );
    assert_lines(&l, &[("dddd", 2.81), ("ccccd", 13.64)]);
}

/// A word of a `pre` element, then the root's collapsible spaces, then `pre`
/// spaces: the word does not wrap, so the run after it breaks only when the
/// line already overflows — here it does not, and Chrome keeps one line.
#[test]
fn a_collapsible_space_after_a_pre_word_breaks_only_after_an_overflow() {
    let l = lines(
        &format!("{RIGHT_50};white-space:normal"),
        "<span style=\"white-space:pre\">aaaa</span>  <span style=\"white-space:pre\">  dd</span>",
    );
    assert_lines(&l, &[("aaaa   dd", 0.0)]);
}

/// A known difference, tracked in #1898 (review of #1897): the word and a
/// collapsible space split into two text nodes, after an inline box. Chrome
/// keeps the box with `mmmmm` and breaks before the `pre` space (`mmmmm` at
/// 8.92, ` ee` at 69.34); rinch takes the run as a text node of its own,
/// which the line does not overflow before, so it breaks after the box.
/// Main had `mmmmm` at 17.92. Pinned so a change shows.
#[test]
fn known_difference_split_word_after_a_box() {
    use rinch_core::dom::DomDocument;
    let (mut d, c) = lay_out(
        "width:88px;text-align:right;white-space:normal",
        "<b style=\"display:inline-block;width:12px;height:10px\"></b>mmmmm \
         <span style=\"white-space:pre\"> </span> ee",
    );
    // Split `mmmmm ` into `mmmmm` and ` `.
    let t = d.get_children(c)[1];
    d.set_text_content(t, "mmmmm");
    let sp = d.create_text(" ");
    let pre = d.get_children(c)[2];
    d.insert_before(c, sp, pre);
    d.resolve_layout(801.0, 600.0);
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    let texts: Vec<String> = il
        .layout
        .lines()
        // A line holding only the box has no text range of its own.
        .map(|l| {
            il.text_content
                .get(l.text_range())
                .unwrap_or("")
                .trim()
                .to_string()
        })
        .collect();
    assert_eq!(texts, ["", "mmmmm", "ee"], "Chrome: [\"mmmmm\", \"ee\"]");
}

/// No `pre` run here starts right after wrapping text, so only parley's
/// lines tell that the paragraph needs the seams (`ifc::pre_seams_needed`):
/// parley went back to its break after `hh ` when the `pre` text overflowed,
/// so its line ends right before the `pre` text. Chrome lets the `pre-wrap`
/// spaces after it hang and keeps `hh eeee` together.
#[test]
fn a_line_parley_ends_before_pre_text_takes_the_seams() {
    let l = lines(
        "width:70px;text-align:left;white-space:nowrap",
        "<span style=\"white-space:normal\">hh </span><span style=\"white-space:normal\">  </span>\
         <span style=\"white-space:pre\">eeee  </span><span style=\"white-space:pre-wrap\">    c</span>",
    );
    assert_lines(&l, &[("hh eeee", 0.0), ("c", 0.0)]);
}

/// parley broke after a `pre` run, before an inline box (main: the five
/// `pre` spaces, then the box and three spaces on a line of their own), where
/// `white-space: pre` allows no break; Chrome keeps the run, the box and the
/// trailing `pre-wrap` spaces on one line, the spaces hanging. A line parley
/// ended in a `pre` space sends the paragraph to the seams.
#[test]
fn a_pre_run_before_an_inline_box_is_not_broken() {
    let (d, c) = lay_out(
        "width:115px;text-align:left;white-space:pre-wrap",
        "<span style=\"white-space:pre\">     </span>\
         <b style=\"display:inline-block;font-weight:normal\">dmqksqymr</b>   ",
    );
    let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
    assert_eq!(il.layout.len(), 1, "one line, as in Chrome 155");
}

/// A `pre` element ending in `!`, glued to the word after it (review of
/// #1897). Chrome 155 has no break between `!` and a letter, and keeps
/// `cccc!dddd` together (overflowing); the first two heads of #1897 broke
/// it as `cccc!` / `dddd`, the seams' break opportunity coming from UAX #14
/// (ICU4X), which has one there.
#[test]
fn a_pre_word_ending_in_an_exclamation_mark_stays_glued() {
    for white_space in ["normal", "pre-wrap"] {
        let l = lines(
            &format!("width:60px;text-align:left;white-space:{white_space}"),
            "aaaa bbbb <span style=\"white-space:pre\">cccc!</span>dddd eeee \
             <span style=\"white-space:pre\"> ff</span> gg",
        );
        let texts: Vec<&str> = l.iter().map(|(t, _)| t.trim()).collect();
        assert!(texts.contains(&"cccc!dddd"), "{white_space}: {texts:?}");
    }
    let l = lines(
        "width:76px;text-align:left;white-space:normal",
        "Say <code style=\"white-space:pre\">hello!</code>world, then \
         <code style=\"white-space:pre\"> x</code> ok",
    );
    let texts: Vec<&str> = l.iter().map(|(t, _)| t.trim()).collect();
    assert!(texts.contains(&"hello!world,"), "{texts:?}");
}
