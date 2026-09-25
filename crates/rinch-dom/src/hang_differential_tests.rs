//! `break_lines_hanging_spaces` against the algorithm it replaced (review of
//! #1018, F1).
//!
//! The first version fixed one line per pass and re-broke the whole paragraph
//! from line 0 each time: quadratic, but obviously right, because every pass
//! reads the lines parley actually committed. The linear version follows line
//! ends through a table of units instead ([`super::line_end`]), and a table that
//! drifts from the breaker would put a line's spaces on the wrong line with
//! nothing else noticing. So the old loop is kept here, verbatim apart from
//! NBSP (which the review's F2 made not hang in both), as the oracle: on
//! random paragraphs of words, spaces, tabs, NBSPs, newlines and inline boxes
//! at random widths, both must commit the same lines.

use peniko::Brush;

use super::{HangStats, break_lines_hanging_spaces, is_hanging_space};

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

/// The pre-#1018-review loop: restart the break from line 0 for every line it
/// fixes.
fn reference(layout: &mut parley::Layout<Brush>, text: &str, max: f32) {
    layout.break_all_lines(Some(max));
    let mut widened: Vec<(usize, f32)> = Vec::new();
    // Searching from the last fixed line itself, not the one after it, is
    // the one change: a widened line can end at a hang again (an NBSP after
    // its spaces, which parley hangs itself), and is then widened further.
    while let Some(fix) = first_unhung(layout, text, widened.last().map_or(0, |&(i, _)| i)) {
        if widened.last().is_some_and(|&(i, _)| i == fix.0) {
            widened.pop();
        }
        widened.push(fix);
        let mut breaker = layout.break_lines();
        let mut next = widened.iter().peekable();
        let mut line = 0usize;
        loop {
            let line_max = match next.peek() {
                Some(&&(i, m)) if i == line => {
                    next.next();
                    m
                }
                _ => max,
            };
            let state = breaker.state_mut();
            state.set_layout_max_advance(line_max);
            state.set_line_max_advance(line_max);
            if breaker.break_next().is_none() {
                break;
            }
            if line_max != max {
                breaker.set_prior_line_width(max);
            }
            line += 1;
        }
        breaker.finish();
    }
}

fn first_unhung(layout: &parley::Layout<Brush>, text: &str, from: usize) -> Option<(usize, f32)> {
    use parley::layout::{BreakReason, Cluster};
    for (i, line) in layout.lines().enumerate().skip(from) {
        if line.break_reason() != BreakReason::Regular {
            continue;
        }
        let mut hanging = 0.0f32;
        let mut cluster = Cluster::from_byte_index(layout, line.text_range().end);
        match &cluster {
            None => {}
            Some(c) if c.is_hard_line_break() => {}
            Some(c) if is_hanging_space(text, c) => {
                while let Some(c) = cluster.filter(|c| is_hanging_space(text, c)) {
                    hanging += c.advance();
                    cluster = c.next_logical();
                }
            }
            Some(_) => continue,
        }
        return Some((i, line.metrics().advance + hanging + 0.01));
    }
    None
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// `(text, byte offsets of inline boxes and their widths)`.
fn paragraph(rng: &mut Rng, with_boxes: bool) -> (String, Vec<(usize, f32)>) {
    const PIECES: &[&str] = &[
        "a",
        "i",
        "word",
        "river",
        "longerword",
        " ",
        " ",
        " ",
        "  ",
        "   ",
        "\t",
        "\n",
        "\u{a0}",
        "-",
        "x\u{a0}\u{a0}",
    ];
    let mut s = String::new();
    let mut boxes = Vec::new();
    for _ in 0..rng.below(60) + 1 {
        if with_boxes && rng.below(12) == 0 {
            boxes.push((s.len(), 3.0 + rng.below(40) as f32));
        }
        s.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]);
    }
    (s, boxes)
}

fn build(
    fcx: &mut parley::FontContext,
    lcx: &mut parley::LayoutContext<Brush>,
    text: &str,
    boxes: &[(usize, f32)],
) -> parley::Layout<Brush> {
    let mut b = lcx.ranged_builder(fcx, text, 1.0, true);
    b.push_default(parley::style::StyleProperty::FontFamily(
        parley::style::FontFamily::Source("HangFace".into()),
    ));
    b.push_default(parley::style::StyleProperty::FontSize(16.0));
    // A ranged builder collapses nothing: every space is preserved, as in
    // `pre-wrap`.
    for (i, &(index, width)) in boxes.iter().enumerate() {
        b.push_inline_box(parley::InlineBox {
            id: i as u64,
            index,
            width,
            height: 10.0,
            kind: parley::InlineBoxKind::InFlow,
        });
    }
    b.build(text)
}

type Lines = Vec<(
    std::ops::Range<usize>,
    parley::layout::BreakReason,
    f32,
    f32,
)>;

fn lines(layout: &parley::Layout<Brush>) -> Lines {
    layout
        .lines()
        .map(|l| {
            let m = l.metrics();
            (
                l.text_range(),
                l.break_reason(),
                m.advance,
                m.inline_max_coord,
            )
        })
        .collect()
}

#[test]
fn the_linear_pass_commits_the_lines_the_restarting_loop_did() {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut fcx = crate::fonts::new_font_context();
    fcx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("HangFace"),
            ..Default::default()
        }),
    );
    let mut lcx = parley::LayoutContext::new();
    let mut rng = Rng(0x1018_f1f1_dead_beef);
    let (mut fixed, mut lines_fixed) = (0u32, 0u32);
    for case in 0..1200 {
        let (text, boxes) = paragraph(&mut rng, false);
        let max = 8.0 + rng.below(160) as f32 + rng.below(100) as f32 / 100.0;
        let mut want = build(&mut fcx, &mut lcx, &text, &boxes);
        reference(&mut want, &text, max);
        let mut got = build(&mut fcx, &mut lcx, &text, &boxes);
        let stats: HangStats = break_lines_hanging_spaces(&mut got, &text, Some(max), true);
        fixed += stats.passes;
        lines_fixed += stats.lines;
        assert!(stats.passes <= 1, "case {case}: {stats:?}");
        assert_eq!(
            lines(&got),
            lines(&want),
            "case {case}: {text:?} with boxes {boxes:?} at {max}px"
        );
        assert_eq!(got.height(), want.height(), "case {case}");
    }
    // Positive control: the sample reaches the fix, and fixes several lines of
    // one paragraph in one pass.
    assert!(
        fixed > 350 && lines_fixed > 2 * fixed,
        "{fixed} passes, {lines_fixed} lines"
    );
}

/// With inline boxes the restarting loop is no oracle: a line holding only a
/// box reports an inverted text range (`MAX..0`), and a box at a line's end
/// byte is invisible to a walk over clusters, so it fixed lines it should not
/// have and missed ones it should. What must hold instead: the unit table
/// accounts for every line exactly (line by line, in step), and no line is
/// left with spaces or tabs that should have hung. Not "no taller than
/// parley's own break": a line that keeps its spaces moves where every later
/// line starts, and they can wrap into one more line than parley's did.
#[test]
fn with_inline_boxes_every_line_is_in_step_and_none_is_left_unhung() {
    use parley::fontique::{Blob, FontInfoOverride};
    use parley::layout::BreakReason;
    let mut fcx = crate::fonts::new_font_context();
    fcx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("HangFace"),
            ..Default::default()
        }),
    );
    let mut lcx = parley::LayoutContext::new();
    let mut rng = Rng(0x1018_b0c5_0000_0001);
    let mut fixed_lines = 0u32;
    for case in 0..1200 {
        let (text, boxes) = paragraph(&mut rng, true);
        let max = 8.0 + rng.below(160) as f32 + rng.below(100) as f32 / 100.0;
        let mut got = build(&mut fcx, &mut lcx, &text, &boxes);
        let stats = break_lines_hanging_spaces(&mut got, &text, Some(max), true);
        fixed_lines += stats.lines;
        let units = super::logical_units(&got, &text);
        let mut cursor = 0;
        for (i, line) in got.lines().enumerate() {
            let m = line.metrics();
            if line.break_reason() == BreakReason::None {
                // The last line. An empty one after a final newline copies
                // the line before's metrics, advance included.
                break;
            }
            let end = super::line_end(&units, cursor, m.advance).unwrap_or_else(|| {
                panic!("case {case} line {i}: out of step, {text:?} {boxes:?} at {max}")
            });
            if line.break_reason() == BreakReason::Regular && m.advance > max {
                assert_eq!(
                    super::hanging_after(&units, end),
                    None,
                    "case {case} line {i} left unhung: {text:?} {boxes:?} at {max}"
                );
            }
            cursor = end;
        }
    }
    assert!(
        fixed_lines > 800,
        "positive control: {fixed_lines} lines fixed"
    );
}
