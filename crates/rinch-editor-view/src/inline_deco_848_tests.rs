//! #848: the inline-decoration pass must cost O(blocks + decorations touching
//! what it visits), not O(blocks × decorations) — and must project exactly what
//! the old whole-list scan projected.
//!
//! The oracle is [`naive_runs`]: for every text run, `run_decos` over the
//! **whole** decoration list in draw order — the definition the pass had before
//! #848 — checked against the run's projected `decos` and `has_deco` on every
//! level, plus the DOM against a fresh render of the same state.

use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::model::Fragment;
use rinch_editor_core::serialize::slice_from_html;
use rinch_editor_core::{Attrs, EditorState, Plugin, PluginKey, Pos, Schema, Selection};
use std::cell::RefCell;
use std::rc::Rc;

struct H {
    doc: Rc<RefCell<dyn DomDocument>>,
    container: NodeHandle,
    container_id: NodeId,
}

fn harness() -> H {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock;
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    H {
        doc,
        container,
        container_id,
    }
}

/// The decorations a test puts in it, in the order it puts them (draw order).
struct Decos(RefCell<Vec<(usize, usize, &'static str)>>);
impl Plugin for Decos {
    fn key(&self) -> PluginKey {
        PluginKey("test848.decos")
    }
    fn decorations(&self, _s: &EditorState) -> DecorationSet {
        DecorationSet::new(
            self.0
                .borrow()
                .iter()
                .map(|&(a, b, c)| Decoration::inline(Pos(a), Pos(b), Attrs::new().with("class", c)))
                .collect(),
        )
    }
}

fn state(s: &Rc<Schema>, doc: Node, plugin: &Rc<Decos>) -> EditorState {
    EditorState::create(s.clone(), doc, vec![plugin.clone() as Rc<dyn Plugin>])
}

fn doc_from_html(s: &Schema, html: &str) -> Node {
    let slice = slice_from_html(s, html).unwrap();
    s.branch("doc", slice.content).unwrap()
}

fn shape(h: &H, id: NodeId) -> String {
    let d = h.doc.borrow();
    fn walk(d: &dyn DomDocument, id: NodeId, out: &mut String) {
        match d.tag_name(id) {
            Some(tag) => {
                out.push_str(&format!("<{tag}"));
                for a in ["class", "data-pm-mark", "data-pm-deco", "data-pm-deco-run"] {
                    if let Some(v) = d.get_attribute(id, a) {
                        out.push_str(&format!(" {a}={v:?}"));
                    }
                }
                out.push('>');
                for c in d.get_children(id) {
                    walk(d, c, out);
                }
                out.push_str(&format!("</{tag}>"));
            }
            None => out.push_str(&format!("{:?}", d.text_content(id).unwrap_or_default())),
        }
    }
    let mut out = String::new();
    walk(&*d, id, &mut out);
    out
}

fn deco_segments(h: &H, id: NodeId) -> usize {
    let d = h.doc.borrow();
    fn walk(d: &dyn DomDocument, id: NodeId) -> usize {
        let me = usize::from(d.get_attribute(id, "data-pm-deco").is_some());
        me + d
            .get_children(id)
            .into_iter()
            .map(|c| walk(d, c))
            .sum::<usize>()
    }
    walk(&*d, id)
}

/// Check every run under `desc` against the whole-list definition; returns the
/// subtree's expected `has_deco`.
fn naive_runs(desc: &ViewDesc, decos: &[InlineDeco], content_start: usize, ctx: &str) -> bool {
    let mut pos = content_start;
    let mut any = false;
    for c in &desc.children {
        let has = if c.is_text {
            let len = c.node.text().map_or(0, |t| t.chars().count());
            let want = run_decos(decos, pos, len);
            assert_eq!(c.decos, want, "{ctx}: run at {pos} (len {len})");
            !want.is_empty()
        } else {
            naive_runs(c, decos, pos + 1, ctx)
        };
        assert_eq!(c.has_deco, has, "{ctx}: has_deco of the child at {pos}");
        any |= has;
        pos += c.node.node_size();
    }
    any
}

fn as_inline(list: &[(usize, usize, &'static str)]) -> Vec<InlineDeco> {
    list.iter()
        .filter(|(a, b, _)| a < b)
        .map(|&(from, to, c)| InlineDeco {
            from,
            to,
            class: Rc::from(c),
        })
        .collect()
}

/// The pass's result on `view` for `st` equals the oracle and a fresh render.
fn check(h: &H, view: &RinchDomEditorView, st: &EditorState, decos: &[InlineDeco], ctx: &str) {
    let any = naive_runs(&view.root, decos, 0, ctx);
    assert_eq!(view.root.has_deco, any, "{ctx}: root has_deco");
    let fresh_h = harness();
    let fresh = RinchDomEditorView::new(fresh_h.container.clone(), Rc::downgrade(&fresh_h.doc), st);
    assert_eq!(
        shape(h, h.container_id),
        shape(&fresh_h, fresh_h.container_id),
        "{ctx}: incremental DOM != fresh render"
    );
    drop(fresh);
}

// ── cost ─────────────────────────────────────────────────────────────────────

/// Range comparisons one keystroke in the last paragraph costs, with one
/// decoration in each of the first `decos` paragraphs of `blocks`.
fn keystroke_range_tests(blocks: usize, decos: usize) -> usize {
    let s = Rc::new(Schema::starter_kit());
    let text = "the quick brown fox";
    let paras: Vec<_> = (0..blocks)
        .map(|_| {
            s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
                .unwrap()
        })
        .collect();
    let doc = s.branch("doc", Fragment::from_children(paras)).unwrap();
    let bs = text.chars().count() + 2;
    let plugin = Rc::new(Decos(RefCell::new(
        (0..decos)
            .map(|b| (b * bs + 5, b * bs + 10, "pm-spell-error"))
            .collect(),
    )));
    let st = state(&s, doc, &plugin);
    let h = harness();
    let mut view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), &st);
    let mut tr = st.tr();
    tr.set_selection(Selection::cursor(Pos(blocks * bs - 1)));
    let cur = st.apply(tr);
    view.update_dom(&st, &cur);
    let mut tr = cur.tr();
    tr.insert_text("x").unwrap();
    let next = cur.apply(tr);
    DECO_RANGE_TESTS.with(|c| c.set(0));
    view.update_dom(&cur, &next);
    let n = DECO_RANGE_TESTS.with(|c| c.get());
    // The keystroke changed nothing the decorations draw: still one segment each.
    assert_eq!(deco_segments(&h, h.container_id), decos);
    n
}

#[test]
fn a_keystroke_away_from_the_decorations_costs_linear_range_tests() {
    // Old pass: every child of every visited level tested against the whole
    // list — about blocks × decorations / 2 here (the scan stops at the first
    // hit, and block i's hit is decoration i). The sorted cursor tests each
    // decoration about once per level it reaches, plus one check per child.
    for (blocks, decos) in [(100, 100), (200, 200), (400, 400)] {
        let n = keystroke_range_tests(blocks, decos);
        assert!(
            n <= 2 * (blocks + decos),
            "{blocks} blocks × {decos} decorations: {n} range tests for one keystroke"
        );
    }
    // And it scales linearly: doubling both doubles the work, not quadruples it.
    let (a, b) = (
        keystroke_range_tests(200, 200),
        keystroke_range_tests(400, 400),
    );
    assert!(b <= 2 * a + 8, "200²: {a}, 400²: {b}");
}

// ── correctness: hand-picked shapes ──────────────────────────────────────────

const NESTED: &str = "<p>alpha beta</p><blockquote><p>gamma delta</p><p>eps</p></blockquote>\
     <ul><li><p>zeta <strong>eta</strong> theta</p></li><li><p>iota</p></li></ul>\
     <hr><p>kappa<img src=\"x.png\">lambda</p>";

/// Drive `view` through `steps` (each a decoration list in draw order),
/// checking each against the oracle and a fresh render.
fn drive(steps: &[&[(usize, usize, &'static str)]]) {
    let s = Rc::new(Schema::starter_kit());
    let plugin = Rc::new(Decos(RefCell::new(Vec::new())));
    let mut st = state(&s, doc_from_html(&s, NESTED), &plugin);
    let h = harness();
    let mut view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), &st);
    let size = st.doc.content_size();
    for (i, list) in steps.iter().enumerate() {
        assert!(
            list.iter().all(|&(_, b, _)| b <= size),
            "step {i} out of range"
        );
        *plugin.0.borrow_mut() = list.to_vec();
        // A selection-only transaction re-runs the decoration pass.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(Pos(1 + i % 3)));
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        st = next;
        check(&h, &view, &st, &as_inline(list), &format!("step {i}"));
    }
}

#[test]
fn overlapping_decorations_keep_draw_order_whatever_their_positions() {
    let size = {
        let s = Schema::starter_kit();
        doc_from_html(&s, NESTED).content_size()
    };
    // Drawn later but starting earlier, and the reverse; one reaching from the
    // first paragraph into the list, across the blockquote; one over the image.
    drive(&[
        &[(4, 9, "b"), (2, 6, "a"), (1, 30, "c")],
        &[(1, 30, "c"), (4, 9, "b"), (2, 6, "a")],
        &[
            (20, size - 2, "wide"),
            (3, 5, "x"),
            (25, 27, "y"),
            (size - 9, size - 5, "img"),
        ],
    ]);

    // The same first step spelled out, independently of `run_decos`: "alpha
    // beta" is chars 1..11, and every stretch lists its classes in draw order
    // (b, a, c) — not in order of position (c, a, b).
    let s = Rc::new(Schema::starter_kit());
    let plugin = Rc::new(Decos(RefCell::new(vec![
        (4, 9, "b"),
        (2, 6, "a"),
        (1, 30, "c"),
    ])));
    let st = state(&s, doc_from_html(&s, NESTED), &plugin);
    let h = harness();
    let view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), &st);
    let run = &view.root.children[0].children[0];
    let got: Vec<(usize, usize, &str)> = run
        .decos
        .iter()
        .map(|d| (d.start, d.end, &*d.class))
        .collect();
    assert_eq!(
        got,
        vec![
            (0, 1, "c"),
            (1, 3, "a c"),
            (3, 5, "b a c"),
            (5, 8, "b c"),
            (8, 10, "c")
        ]
    );
}

#[test]
fn decorations_moving_between_blocks_leave_nothing_behind() {
    // On the first paragraph, then only in the blockquote, then only in the
    // list, then none: each step must take the earlier segments off.
    drive(&[
        &[(2, 5, "a")],
        &[(15, 19, "a"), (22, 24, "b")],
        &[(33, 36, "a")],
        &[],
    ]);
}

// ── correctness: random nested documents ─────────────────────────────────────

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn rand_word(r: &mut Rng) -> String {
    (0..1 + r.below(5))
        .map(|_| (b'a' + r.below(26) as u8) as char)
        .collect()
}

fn rand_para(r: &mut Rng) -> String {
    let mut out = String::from("<p>");
    for _ in 0..r.below(4) {
        match r.below(6) {
            0 => out.push_str(&format!("<strong>{}</strong>", rand_word(r))),
            1 => out.push_str(&format!("<em>{} </em>", rand_word(r))),
            2 => out.push_str("<img src=\"x.png\">"),
            3 => out.push_str("<br>"),
            _ => out.push_str(&format!("{} ", rand_word(r))),
        }
    }
    out.push_str("</p>");
    out
}

fn rand_html(r: &mut Rng) -> String {
    (0..1 + r.below(6))
        .map(|_| match r.below(5) {
            0 => format!("<blockquote>{}{}</blockquote>", rand_para(r), rand_para(r)),
            1 => format!(
                "<ul><li>{}</li><li>{}</li></ul>",
                rand_para(r),
                rand_para(r)
            ),
            2 => "<hr>".to_string(),
            _ => rand_para(r),
        })
        .collect()
}

const CLASSES: &[&str] = &["a", "b", "c"];

fn rand_decos(r: &mut Rng, size: usize) -> Vec<(usize, usize, &'static str)> {
    (0..r.below(9))
        .map(|_| {
            let a = r.below(size + 1);
            // Mostly short, sometimes long enough to cross several blocks.
            let len = if r.below(4) == 0 {
                r.below(size + 1)
            } else {
                r.below(6)
            };
            (a, (a + len).min(size), CLASSES[r.below(3)])
        })
        .collect()
}

fn rand_edit(r: &mut Rng, st: &EditorState) -> EditorState {
    let size = st.doc.content_size();
    let a = r.below(size + 1);
    let b = r.below(size + 1);
    let mut tr = st.tr();
    let ok = match r.below(3) {
        0 => tr.delete(a.min(b), a.max(b)).is_ok(),
        _ => {
            tr.set_selection(Selection::cursor(Pos(a)));
            tr.insert_text(&rand_word(r)).is_ok()
        }
    };
    if ok {
        st.apply(tr)
    } else {
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(Pos(0)));
        st.apply(tr)
    }
}

#[test]
fn random_nested_documents_project_exactly_the_whole_list_definition() {
    let seeds = std::env::var("RINCH_PROP_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150u64);
    for seed in 1..=seeds {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let s = Rc::new(Schema::starter_kit());
        let plugin = Rc::new(Decos(RefCell::new(Vec::new())));
        let mut st = state(&s, doc_from_html(&s, &rand_html(&mut r)), &plugin);
        let h = harness();
        let mut view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), &st);
        for step in 0..20 {
            // Half the steps change the decorations, half only the document
            // (stale decorations clipped to the new size, as a plugin that did
            // not map them would leave them).
            let next = rand_edit(&mut r, &st);
            let size = next.doc.content_size();
            let list = if r.below(2) == 0 {
                rand_decos(&mut r, size)
            } else {
                plugin
                    .0
                    .borrow()
                    .iter()
                    .filter(|&&(_, b, _)| b <= size)
                    .copied()
                    .collect()
            };
            *plugin.0.borrow_mut() = list.clone();
            view.update_dom(&st, &next);
            st = next;
            let ctx = format!(
                "seed {seed} step {step}: {list:?} over {}",
                rinch_editor_core::serialize::node_to_html(&st.doc)
            );
            check(&h, &view, &st, &as_inline(&list), &ctx);
        }
    }
}
