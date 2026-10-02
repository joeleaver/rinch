//! Two properties of line breaking that hold whatever the font and width,
//! checked over random text of letters, spaces, NBSPs, ZERO WIDTH SPACEs and
//! U+2028 (which rinch lays out as NBSP + ZWSP, #1181), under `normal` and
//! `pre-wrap`, at widths from 12 to 122px. Chrome 153 never violates either
//! (measured over 1,600 such paragraphs by the review of #1269):
//!
//! - **No line ends inside an NBSP's glue.** A line ending in an NBSP is
//!   followed by a space, or by nothing. A break after an NBSP and before a
//!   letter, a ZWSP or another NBSP is one UAX #14 forbids (LB12, LB7).
//! - **No `normal` line starts with a space.** A collapsible space at a
//!   soft wrap hangs at the end of the line before it.
//!
//! The first attempt at #1282 (commit 4489bacd, reverted) broke both, 80
//! times in 1,600 paragraphs, while fixing the case it was aimed at. This is
//! what catches that without Chrome.
#![cfg(feature = "software-renderer")]

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");

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

fn paragraph(rng: &mut Rng) -> String {
    let len = 4 + rng.below(24);
    (0..len)
        .map(|_| match rng.below(20) {
            0..=1 => ' ',
            2..=3 => '\u{a0}',
            4..=5 => '\u{200b}',
            6..=7 => '\u{2028}',
            n => (b'a' + (n as u8 % 6)) as char,
        })
        .collect()
}

fn esc(s: &str) -> String {
    s.replace('\u{a0}', "~")
        .replace('\u{200b}', "Z")
        .replace('\u{2028}', "L")
}

#[test]
fn no_line_ends_inside_nbsp_glue_and_no_normal_line_starts_with_a_space() {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut rng = Rng(0x1269_1181_2028_00a0);
    let mut bad = Vec::new();
    let mut lines_seen = 0usize;
    let mut d = RinchDocument::new();
    d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("ProbeFace"),
            ..Default::default()
        }),
    );
    let body = d.body();
    for case in 0..1600 {
        let text = paragraph(&mut rng);
        let width = 12 + rng.below(111);
        let ws = if case % 2 == 0 { "normal" } else { "pre-wrap" };
        let c = d.create_element("div");
        d.set_attribute(
            c,
            "style",
            &format!("width:{width}px;font:16px/25px ProbeFace;white-space:{ws}"),
        );
        d.append_child(body, c);
        let t = d.create_text(&text);
        d.append_child(c, t);
        d.resolve_layout(800.0, 600.0);
        let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
        let flat = &il.text_content;
        let lines: Vec<&str> = il.layout.lines().map(|l| &flat[l.text_range()]).collect();
        lines_seen += lines.len();
        for (i, line) in lines.iter().enumerate() {
            let next = lines.get(i + 1).and_then(|n| n.chars().next());
            if line.ends_with('\u{a0}') && next.is_some_and(|n| n != ' ') {
                bad.push(format!(
                    "{ws} @{width}px {:?}: a line ends inside an NBSP's glue: {:?}",
                    esc(&text),
                    lines.iter().map(|l| esc(l)).collect::<Vec<_>>()
                ));
            }
            if ws == "normal" && i > 0 && line.starts_with(' ') {
                bad.push(format!(
                    "{ws} @{width}px {:?}: a line starts with a space: {:?}",
                    esc(&text),
                    lines.iter().map(|l| esc(l)).collect::<Vec<_>>()
                ));
            }
        }
        d.remove_node(c);
    }
    // Positive control: the generator makes paragraphs that wrap.
    assert!(
        lines_seen > 3000,
        "only {lines_seen} lines in 1600 paragraphs"
    );
    assert!(
        bad.is_empty(),
        "{} violations, first:\n{}",
        bad.len(),
        bad.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
    );
}
