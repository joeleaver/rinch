//! An oracle for the NBSP re-break (#1218, from the review of PR #1257): in `white-space: normal` text of ASCII words,
//! single spaces and NBSPs, CSS line breaking is greedy over space-separated
//! tokens (an NBSP glues: LB12/LB12a give no break around it but after a
//! space), the space at a soft wrap hangs, and a token wider than the line
//! overflows alone. Compare rinch's lines with that greedy result, using the
//! cluster advances rinch shaped.
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

fn paragraph(rng: &mut Rng, multi: bool) -> String {
    let mut s = String::new();
    let words = 2 + rng.below(14);
    for w in 0..words {
        if w > 0 {
            // A space, an NBSP, a space: two break opportunities in one gap
            // between words, of which CSS takes the first (round 2 of the
            // review of #1257).
            if rng.below(6) == 0 {
                s.push_str(" \u{a0} ");
            } else if rng.below(3) == 0 {
                s.push('\u{a0}');
            } else {
                for _ in 0..if multi { 1 + rng.below(3) } else { 1 } {
                    s.push(' ');
                }
            }
        }
        if rng.below(8) == 0 {
            s.push('\u{a0}');
        }
        for _ in 0..1 + rng.below(6) {
            s.push(b"aWkmnopqiyx"[rng.below(11) as usize] as char);
        }
        if rng.below(8) == 0 {
            s.push('\u{a0}');
        }
    }
    s
}

#[test]
fn nbsp_lines_match_the_greedy_token_oracle() {
    run("", false);
}

#[test]
fn nbsp_lines_match_the_greedy_token_oracle_pre_wrap() {
    run("white-space: pre-wrap;", false);
}

#[test]
fn nbsp_lines_match_the_greedy_token_oracle_pre_wrap_multi_space() {
    run("white-space: pre-wrap;", true);
}

fn run(ws: &str, multi: bool) {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut rng = Rng(0x1257_0000_0000_0001);
    let mut checked = 0;
    let mut mismatches = Vec::new();
    for case in 0..600 {
        let text = paragraph(&mut rng, multi);
        let width = (12 + rng.below(120)) as f32;
        let mut d = RinchDocument::new();
        d.font_cx.collection.register_fonts(
            Blob::new(std::sync::Arc::new(FACE)),
            Some(FontInfoOverride {
                family_name: Some("ProbeFace"),
                ..Default::default()
            }),
        );
        let body = d.body();
        let c = d.create_element("div");
        d.set_attribute(
            c,
            "style",
            &format!("font: 16px/25px ProbeFace; width: {width}px; {ws}"),
        );
        d.append_child(body, c);
        let t = d.create_text(&text);
        d.append_child(c, t);
        d.resolve_layout(800.0, 600.0);
        let il = d.tree.get(c.0).unwrap().text_layout.as_ref().unwrap();
        let content = &il.text_content;
        assert_eq!(content, &text);
        // Advance of each byte offset's cluster.
        let mut adv = vec![0.0f32; content.len() + 1];
        for line in il.layout.lines() {
            for item in line.items() {
                if let parley::layout::PositionedLayoutItem::GlyphRun(gr) = item {
                    for cl in gr.run().clusters() {
                        adv[cl.text_range().start] = cl.advance();
                    }
                }
            }
        }
        let width_of = |s: usize, e: usize| -> f32 { (s..e).map(|i| adv[i]).sum() };
        // Greedy over tokens (maximal runs of non-spaces).
        let mut toks: Vec<(usize, usize)> = Vec::new();
        let bytes = content.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b' ' {
                i += 1;
                continue;
            }
            let st = i;
            while i < bytes.len() && bytes[i] != b' ' {
                i += 1;
            }
            toks.push((st, i));
        }
        let mut want = Vec::new();
        let mut line_start = 0;
        for &(s, e) in &toks[1..] {
            let w = width_of(line_start, e);
            if w <= width - 0.02 {
            } else if w <= width + 0.02 {
                line_start = usize::MAX;
                break;
            } else {
                want.push(content[line_start..s].to_string());
                line_start = s;
            }
        }
        if line_start == usize::MAX {
            continue;
        }
        want.push(content[line_start..].to_string());
        let got: Vec<String> = il
            .layout
            .lines()
            .map(|l| content[l.text_range()].to_string())
            .collect();
        checked += 1;
        if got != want {
            mismatches.push(format!(
                "case {case} at {width}: {:?}\n  got  {:?}\n  want {:?}",
                text.replace('\u{a0}', "~"),
                got.iter()
                    .map(|s| s.replace('\u{a0}', "~"))
                    .collect::<Vec<_>>(),
                want.iter()
                    .map(|s| s.replace('\u{a0}', "~"))
                    .collect::<Vec<_>>()
            ));
        }
    }
    println!("checked {checked}, mismatches {}", mismatches.len());
    for m in mismatches.iter().take(15) {
        println!("{m}");
    }
    assert!(checked > 400, "positive control: {checked}");
    assert!(mismatches.is_empty(), "{} mismatches", mismatches.len());
}
