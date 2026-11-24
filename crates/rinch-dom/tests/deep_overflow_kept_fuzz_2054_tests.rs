//! #2054: a scroll container's kept descendant walk, brought up to date in
//! place, against a fresh document — over random sequences of edits.
//!
//! Each sequence starts from one document, makes random edits under the
//! scroller (rows inserted, removed, resized, restyled and moved; padding,
//! border, margin, filter, position, insets, overflow, display, transform),
//! with a layout after each, and reads the ranges between them in a random
//! order (a read, a paint consuming the tree, both, or neither). At every
//! read, each element's range under the scroller (and the scroller's own)
//! must equal what a fresh document given the same edits, never read before,
//! answers. The hand-written cases in `deep_overflow_kept_2054_tests.rs` pin
//! the rules one at a time; this finds the combinations they miss.
//!
//! The sequence is reproducible from its seed: a failure prints it and the
//! edits made. `RINCH_KEPT_FUZZ_SEEDS=<n>` runs more seeds.

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;

const START: &str = concat!(
    r#"<div id="s" style="width:200px;height:100px;overflow:auto">"#,
    r#"<div style="width:180px;height:30px"><div style="width:260px;height:10px"></div></div>"#,
    r#"<div style="position:relative;width:150px;height:40px">"#,
    r#"<div style="width:50px;height:120px"></div>"#,
    r#"<div style="position:absolute;left:120px;top:10px;width:150px;height:10px"></div>"#,
    r#"<div style="position:absolute;left:10px;top:20px;width:40px;height:10px"><div style="width:330px;height:20px"></div></div></div>"#,
    r#"<div style="display:contents"><div style="width:90px;height:20px"></div></div>"#,
    r#"<div style="width:120px;height:30px;overflow:auto"><div style="width:300px;height:80px"></div></div>"#,
    r#"<div style="height:20px">ab cd <span>ef<div style="width:230px;height:5px"></div>gh</span></div>"#,
    r#"</div>"#,
);

/// A small PRNG (xorshift64*): the sequences must be the same on every host.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, of: &[&'a str]) -> &'a str {
        of[self.below(of.len())]
    }
}

/// One edit, naming elements by their index in document order under the
/// scroller ([`elements`]), so it replays the same on a fresh document.
#[derive(Debug, Clone)]
enum Edit {
    /// A new `<div>` with this style, before element `at`'s child `index`
    /// (or last).
    Insert {
        parent: usize,
        index: usize,
        style: String,
    },
    /// Text, or a `<span>` holding text and a block (a split inline).
    InsertInline {
        parent: usize,
        block: bool,
    },
    Remove(usize),
    /// Element `node` taken out and put under element `parent`.
    Move {
        node: usize,
        parent: usize,
        index: usize,
    },
    Style {
        node: usize,
        property: &'static str,
        value: &'static str,
    },
}

const PROPERTIES: &[(&str, &[&str])] = &[
    ("width", &["0px", "20px", "80px", "150px", "260px", "420px"]),
    ("height", &["0px", "5px", "30px", "60px", "140px"]),
    ("padding", &["0px", "7px", "20px"]),
    ("border", &["none", "3px solid black", "11px solid black"]),
    ("margin-left", &["0px", "35px", "250px", "-30px"]),
    ("margin-top", &["0px", "15px", "90px"]),
    ("filter", &["none", "blur(0px)"]),
    ("position", &["static", "relative", "absolute"]),
    ("left", &["auto", "0px", "50px", "230px"]),
    ("top", &["auto", "0px", "70px", "160px"]),
    ("overflow", &["visible", "hidden", "clip", "auto"]),
    (
        "display",
        &[
            "block",
            "none",
            "contents",
            "inline-block",
            "inline",
            "flex",
        ],
    ),
    (
        "transform",
        &["none", "translateX(120px)", "translateY(90px)"],
    ),
    ("background-color", &["red", "blue"]),
];

fn new_doc() -> RinchDocument {
    let mut d = RinchDocument::new();
    d.load_css("body { margin: 0 } div { margin: 0; padding: 0 }");
    let body = d.body();
    d.set_inner_html(body, START);
    d.resolve_layout(800.0, 600.0);
    d
}

fn scroller(d: &RinchDocument) -> NodeId {
    d.query_selector("#s").expect("#s")
}

/// The scroller's descendant elements, in document order.
fn elements(d: &RinchDocument) -> Vec<NodeId> {
    fn walk(d: &RinchDocument, id: NodeId, out: &mut Vec<NodeId>) {
        for child in d.get_children(id) {
            if !d.is_text_node(child) {
                out.push(child);
                walk(d, child, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(d, scroller(d), &mut out);
    out
}

/// Every range a read takes: the scroller's, then each element's below it.
fn ranges(d: &RinchDocument) -> Vec<(f64, f64)> {
    std::iter::once(scroller(d))
        .chain(elements(d))
        .map(|id| (d.scroll_width(id), d.scroll_height(id)))
        .collect()
}

fn is_inside(d: &RinchDocument, node: NodeId, of: NodeId) -> bool {
    let mut at = Some(node);
    while let Some(a) = at {
        if a == of {
            return true;
        }
        at = d.parent_node(a);
    }
    false
}

/// A random edit for document `d`'s elements, or `None` when the one drawn
/// does not apply (a move into its own subtree).
fn draw(rng: &mut Rng, d: &RinchDocument) -> Option<Edit> {
    let els = elements(d);
    // Element `n` of the list, or `n == len` for the scroller itself.
    let parent = rng.below(els.len() + 1);
    Some(match rng.below(10) {
        0 | 1 => Edit::Insert {
            parent,
            index: rng.below(4),
            style: format!(
                "width:{};height:{}",
                rng.pick(&["0px", "40px", "130px", "330px"]),
                rng.pick(&["0px", "10px", "45px", "150px"])
            ),
        },
        2 => Edit::InsertInline {
            parent,
            block: rng.below(2) == 0,
        },
        3 if !els.is_empty() => Edit::Remove(rng.below(els.len())),
        4 if !els.is_empty() => {
            let node = rng.below(els.len());
            let target = if parent == els.len() {
                scroller(d)
            } else {
                els[parent]
            };
            if is_inside(d, target, els[node]) {
                return None;
            }
            Edit::Move {
                node,
                parent,
                index: rng.below(4),
            }
        }
        _ => {
            let (property, values) = PROPERTIES[rng.below(PROPERTIES.len())];
            Edit::Style {
                // The scroller itself too: its own style decides which
                // absolute boxes below it are its overflow.
                node: rng.below(els.len() + 1),
                property,
                value: values[rng.below(values.len())],
            }
        }
    })
}

fn put(d: &mut RinchDocument, parent: NodeId, child: NodeId, index: usize) {
    match d.get_children(parent).get(index) {
        Some(&reference) => d.insert_before(parent, child, reference),
        None => d.append_child(parent, child),
    }
}

fn apply(d: &mut RinchDocument, edit: &Edit) {
    let els = elements(d);
    let el = |n: usize| if n == els.len() { scroller(d) } else { els[n] };
    match edit {
        Edit::Insert {
            parent,
            index,
            style,
        } => {
            let parent = el(*parent);
            let div = d.create_element("div");
            d.set_attribute(div, "style", style);
            put(d, parent, div, *index);
        }
        Edit::InsertInline { parent, block } => {
            let parent = el(*parent);
            if *block {
                let span = d.create_element("span");
                d.set_inner_html(span, r#"ij <div style="width:270px;height:15px"></div> kl"#);
                d.append_child(parent, span);
            } else {
                let text = d.create_text("mn op qr st");
                d.append_child(parent, text);
            }
        }
        Edit::Remove(node) => {
            let node = els[*node];
            let parent = d.parent_node(node).expect("a parent");
            d.remove_child(parent, node);
        }
        Edit::Move {
            node,
            parent,
            index,
        } => {
            let (node, target) = (els[*node], el(*parent));
            let from = d.parent_node(node).expect("a parent");
            d.remove_child(from, node);
            put(d, target, node, *index);
        }
        Edit::Style {
            node,
            property,
            value,
        } => {
            let node = el(*node);
            d.set_style(node, property, value);
        }
    }
}

/// The ranges a fresh document answers after `edits`, each with a layout.
fn fresh(edits: &[Edit]) -> Vec<(f64, f64)> {
    let mut d = new_doc();
    for edit in edits {
        apply(&mut d, edit);
        d.resolve_layout(800.0, 600.0);
    }
    ranges(&d)
}

fn run(seed: u64, steps: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut d = new_doc();
    let _ = ranges(&d);
    d.tree.consume_paint_dirty();
    let mut edits = Vec::new();
    for _ in 0..steps {
        let Some(edit) = draw(&mut rng, &d) else {
            continue;
        };
        apply(&mut d, &edit);
        d.resolve_layout(800.0, 600.0);
        edits.push(edit);
        let got = match rng.below(4) {
            0 => continue,
            1 => ranges(&d),
            2 => {
                d.tree.consume_paint_dirty();
                ranges(&d)
            }
            _ => {
                let _ = ranges(&d);
                d.tree.consume_paint_dirty();
                ranges(&d)
            }
        };
        let want = fresh(&edits);
        assert_eq!(
            got,
            want,
            "seed {seed}: kept ranges differ from a fresh document's after {} edits:\n{edits:#?}",
            edits.len()
        );
    }
}

/// Random edit sequences: the kept ranges equal a fresh document's at every
/// read.
#[test]
fn kept_ranges_match_a_fresh_document_over_random_edits() {
    let seeds = std::env::var("RINCH_KEPT_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(48);
    for seed in 0..seeds {
        run(seed, 24);
    }
}
