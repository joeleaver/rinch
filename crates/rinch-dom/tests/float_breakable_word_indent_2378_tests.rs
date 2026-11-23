//! #2378: two lines beside a float that rinch placed apart from Chrome.
//!
//! 1. **A word the line may break.** Under `overflow-wrap: anywhere` or
//!    `break-word`, `word-break: break-all` or `break-word`, a word wider
//!    than the room beside a float is broken there, not moved below the
//!    float: those values give it break opportunities inside it (CSS Text 3
//!    §5.2, §5.5), so its first piece fits the room and the line stays. rinch
//!    took the emergency break parley breaks such a word with for content
//!    that did not fit, and moved it below the float.
//! 2. **`text-indent`** is part of the line's content when the room is
//!    checked: a first line whose indent and first word together do not fit
//!    beside the float is moved below it, and a negative indent gives the
//!    room back.
//!
//! Every number is Chrome 158's for the same markup (bundled Inter,
//! 16px/20px, a 300px paragraph holding a 200x37 float, in a 560px column):
//! the paragraph's height, and each line's glyphs' left edge and top from a
//! `Range`'s client rects.
use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const FACE: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const CSS: &str = "body { margin: 0; font-family: InterT; font-size: 16px; line-height: 20px; }";
const WORD: &str = "Supercalifragilisticexpialidocious word";
const FLOAT_L: &str = "<div style='float:left;width:200px;height:37px'></div>";
const FLOAT_R: &str = "<div style='float:right;width:200px;height:37px'></div>";

fn doc(html: &str) -> RinchDocument {
    use parley::fontique::{Blob, FontInfoOverride};
    let mut d = RinchDocument::new();
    let registered = d.font_cx.collection.register_fonts(
        Blob::new(std::sync::Arc::new(FACE)),
        Some(FontInfoOverride {
            family_name: Some("InterT"),
            ..Default::default()
        }),
    );
    assert_eq!(registered.len(), 1, "one file, one family");
    d.load_css(CSS);
    let body = d.body();
    d.set_inner_html(body, &format!("<div style='width:560px'>{html}</div>"));
    d.resolve_layout(800.0, 800.0);
    d
}

fn node(d: &RinchDocument, id: &str) -> NodeId {
    d.query_selector(&format!("#{id}")).expect(id)
}

/// Each line's glyphs' left edge and top, from `#a`'s border box (which is
/// at the page origin), rounded to the pixel; and `#a`'s height.
fn lines(d: &RinchDocument) -> (Vec<(f32, f32)>, f32) {
    let n = d.tree.get(node(d, "a").0).unwrap();
    let il = n.text_layout.as_ref().expect("an IFC root");
    let out = il
        .layout
        .lines()
        .map(|line| {
            let mut left = f32::INFINITY;
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(g) = item {
                    let run = g.run();
                    if il.text_content[run.text_range()].trim().is_empty() {
                        continue;
                    }
                    let mut x = g.offset();
                    for c in run.visual_clusters() {
                        if il.text_content[c.text_range()].trim().is_empty() {
                            x += c.advance();
                        } else {
                            break;
                        }
                    }
                    left = left.min(x);
                }
            }
            (left.round(), line.metrics().block_min_coord.round())
        })
        .collect();
    (out, n.layout.height)
}

/// `(name, markup, Chrome's line starts, Chrome's height)`.
type Case = (&'static str, String, &'static [(f32, f32)], f32);

const BROKEN_L: &[(f32, f32)] = &[(200.0, 0.0), (200.0, 20.0), (0.0, 40.0)];

fn cases() -> Vec<Case> {
    let p = |style: &str, float: &str, text: &str| {
        format!("<div id=a style='width:300px;{style}'>{float}{text}</div>")
    };
    vec![
        (
            "anywhere",
            p("overflow-wrap:anywhere", FLOAT_L, WORD),
            BROKEN_L,
            60.0,
        ),
        (
            "break_word",
            p("overflow-wrap:break-word", FLOAT_L, WORD),
            BROKEN_L,
            60.0,
        ),
        (
            "break_all",
            p("word-break:break-all", FLOAT_L, WORD),
            BROKEN_L,
            60.0,
        ),
        (
            "wb_break_word",
            p("word-break:break-word", FLOAT_L, WORD),
            BROKEN_L,
            60.0,
        ),
        (
            "anywhere_right",
            p("overflow-wrap:anywhere", FLOAT_R, WORD),
            &[(0.0, 0.0), (0.0, 20.0), (0.0, 40.0)],
            60.0,
        ),
        (
            "anywhere_span",
            p(
                "",
                FLOAT_L,
                &format!("<span style='overflow-wrap:anywhere'>{WORD}</span>"),
            ),
            BROKEN_L,
            60.0,
        ),
        // The control: a word nothing lets break goes below the float.
        ("normal", p("", FLOAT_L, WORD), &[(0.0, 37.0)], 57.0),
        (
            "indent30",
            p("text-indent:30px", FLOAT_L, "abcdefgh ij"),
            &[(30.0, 37.0)],
            57.0,
        ),
        (
            "indent60",
            p("text-indent:60px", FLOAT_L, "abcdefgh ij"),
            &[(60.0, 37.0)],
            57.0,
        ),
        // A negative indent gives room: the 128px line fits the 100px beside
        // the float with 30px of it back.
        (
            "indent_neg",
            p("text-indent:-30px", FLOAT_L, "abcdefghijklmn ij"),
            &[(170.0, 0.0)],
            20.0,
        ),
    ]
}

/// Every case against Chrome 158.
#[test]
fn lines_beside_a_float_break_and_indent_as_chrome_158_does() {
    let mut wrong = Vec::new();
    for (name, markup, chrome, height) in cases() {
        let d = doc(&markup);
        let (got, h) = lines(&d);
        if got != chrome || (h - height).abs() > 0.5 {
            wrong.push(format!(
                "{name}: rinch {got:?} {h} tall, Chrome {chrome:?} {height} tall"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
