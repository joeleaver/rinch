//! #1833 (from the review of PR #1862): a property check — random documents
//! and random edits (marks added and removed, typing with stored marks,
//! deletions, inline leaves, jumps back to an earlier state as an undo or a
//! peer's change would make); the incrementally updated DOM must equal a fresh
//! render of the same document, with no orphaned or empty wrapper (the mock's
//! live node count equal too). The review ran 3000 and 2000 seeds clean; CI
//! runs fewer (`RINCH_PROP_SEEDS` raises it).

use super::*;
use rinch_core::dom::NodeId;
use rinch_core::dom::mock::MockDomDocument;
use rinch_editor_core::model::Fragment;
use rinch_editor_core::{Attrs, EditorState, Pos, Schema, Selection, default_plugins};
use std::cell::RefCell;
use std::rc::Rc;

struct H {
    doc: Rc<RefCell<dyn DomDocument>>,
    mock: Rc<RefCell<MockDomDocument>>,
    container: NodeHandle,
    container_id: NodeId,
}

fn harness() -> H {
    let mock = Rc::new(RefCell::new(MockDomDocument::new()));
    let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
    let container_id = doc.borrow_mut().create_element("div");
    let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
    H {
        doc,
        mock,
        container,
        container_id,
    }
}

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
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

fn marks_pool(s: &Schema) -> Vec<Mark> {
    let m = |name: &str, attrs: Attrs| Mark::new(s.mark_type(name).unwrap().clone(), attrs);
    vec![
        m("bold", Attrs::default()),
        m("italic", Attrs::default()),
        m("code", Attrs::default()),
        m("strike", Attrs::default()),
        m("highlight", Attrs::default()),
        m("highlight", Attrs::default().with("color", "red")),
        m("link", Attrs::default().with("href", "https://a.test/")),
        m("link", Attrs::default().with("href", "https://b.test/")),
        m("text_color", Attrs::default().with("color", "blue")),
        m("text_color", Attrs::default().with("color", "green")),
    ]
}

/// A random mark set, at most one mark per type, canonical order.
fn rand_marks(r: &mut Rng, pool: &[Mark]) -> Vec<Mark> {
    let mut out: Vec<Mark> = Vec::new();
    for m in pool {
        if r.chance(30) && !out.iter().any(|o| o.type_name() == m.type_name()) {
            out.push(m.clone());
        }
    }
    out.sort_by(|a, b| a.type_name().cmp(b.type_name()));
    out
}

fn rand_inline(r: &mut Rng, s: &Schema, pool: &[Mark]) -> Node {
    let marks = rand_marks(r, pool);
    match r.below(10) {
        0 => s
            .create_node("hard_break", Attrs::new(), Fragment::empty())
            .unwrap()
            .with_marks(marks),
        1 => s
            .create_node(
                "image",
                Attrs::new().with("src", "x.png"),
                Fragment::empty(),
            )
            .unwrap()
            .with_marks(marks),
        _ => {
            let len = 1 + r.below(3);
            let text: String = (0..len)
                .map(|_| (b'a' + r.below(26) as u8) as char)
                .collect();
            s.text_with_marks(&text, marks).unwrap()
        }
    }
}

fn rand_doc(r: &mut Rng, s: &Rc<Schema>, pool: &[Mark]) -> EditorState {
    let blocks: Vec<Node> = (0..1 + r.below(2))
        .map(|_| {
            let kids: Vec<Node> = (0..r.below(7)).map(|_| rand_inline(r, s, pool)).collect();
            s.branch("paragraph", Fragment::from_children(kids))
                .unwrap()
        })
        .collect();
    let doc = s.branch("doc", Fragment::from_children(blocks)).unwrap();
    EditorState::create(s.clone(), doc, default_plugins())
}

/// One random edit. `None` when the step was refused.
fn rand_edit(r: &mut Rng, s: &Rc<Schema>, pool: &[Mark], st: &EditorState) -> Option<EditorState> {
    let size = st.doc.content_size();
    let a = r.below(size + 1);
    let b = r.below(size + 1);
    let (from, to) = (a.min(b), a.max(b));
    let m = pool[r.below(pool.len())].clone();
    let mut tr = st.tr();
    let ok = match r.below(7) {
        0 | 1 => tr.add_mark(from, to, m).is_ok(),
        2 => tr.remove_mark(from, to, m).is_ok(),
        3 | 4 => {
            tr.set_selection(Selection::cursor(Pos(from)));
            if r.chance(60) {
                tr.set_stored_marks(Some(rand_marks(r, pool)));
            }
            let len = 1 + r.below(2);
            let text: String = (0..len)
                .map(|_| (b'A' + r.below(26) as u8) as char)
                .collect();
            tr.insert_text(&text).is_ok()
        }
        5 => tr.delete(from, to).is_ok(),
        _ => {
            tr.set_selection(Selection::cursor(Pos(from)));
            let node = rand_inline(r, s, pool);
            tr.replace_selection_with(node).is_ok()
        }
    };
    if !ok {
        return None;
    }
    Some(st.apply(tr))
}

const ATTRS: &[&str] = &[
    "href",
    "style",
    "class",
    "src",
    "data-pm-mark",
    "data-pm-deco",
    "data-pm-deco-run",
];

fn shape(h: &H, id: NodeId) -> String {
    let d = h.doc.borrow();
    fn walk(d: &dyn DomDocument, id: NodeId, out: &mut String) {
        match d.tag_name(id) {
            Some(tag) => {
                out.push_str(&format!("<{tag}"));
                for a in ATTRS {
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

/// No empty mark wrapper anywhere under `id`.
fn empty_wrappers(h: &H, id: NodeId) -> usize {
    let d = h.doc.borrow();
    fn walk(d: &dyn DomDocument, id: NodeId) -> usize {
        let kids = d.get_children(id);
        let me = usize::from(d.get_attribute(id, "data-pm-mark").is_some() && kids.is_empty());
        me + kids.into_iter().map(|c| walk(d, c)).sum::<usize>()
    }
    walk(&*d, id)
}

fn rand_decos(r: &mut Rng, st: &EditorState) -> Vec<InlineDeco> {
    let size = st.doc.content_size();
    (0..r.below(3))
        .filter_map(|_| {
            let a = r.below(size + 1);
            let b = r.below(size + 1);
            (a != b).then(|| InlineDeco {
                from: a.min(b),
                to: a.max(b),
                class: Rc::from(if r.chance(50) { "d1" } else { "d2" }),
            })
        })
        .collect()
}

fn fresh(st: &EditorState, decos: &[InlineDeco]) -> (String, usize) {
    let h = harness();
    let mut view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), st);
    if !decos.is_empty() {
        let doc = Rc::downgrade(&h.doc);
        sync_inline_decos(&mut view.root, decos, 0, &doc);
    }
    let out = (shape(&h, h.container_id), h.mock.borrow().__node_count());
    drop(view);
    out
}

fn run_seed(seed: u64, steps: usize, with_decos: bool) {
    let s = Rc::new(Schema::starter_kit());
    let pool = marks_pool(&s);
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut st = rand_doc(&mut r, &s, &pool);
    let h = harness();
    let mut view = RinchDomEditorView::new(h.container.clone(), Rc::downgrade(&h.doc), &st);
    let doc_ref = Rc::downgrade(&h.doc);
    let mut decos: Vec<InlineDeco> = Vec::new();
    let mut history: Vec<EditorState> = vec![st.clone()];
    for step in 0..steps {
        let next = if history.len() > 1 && r.chance(15) {
            // Undo-like: go back to an earlier state (the view diffs it as any
            // other transaction; collab remote changes take the same path).
            let i = r.below(history.len() - 1);
            history[i].clone()
        } else {
            match rand_edit(&mut r, &s, &pool, &st) {
                Some(n) => n,
                None => continue,
            }
        };
        view.update_dom(&st, &next);
        if with_decos {
            decos = if r.chance(50) {
                rand_decos(&mut r, &next)
            } else {
                // keep old decos if still in range
                let size = next.doc.content_size();
                decos.into_iter().filter(|d| d.to <= size).collect()
            };
            sync_inline_decos(&mut view.root, &decos, 0, &doc_ref);
        }
        let got = shape(&h, h.container_id);
        let (want, want_count) = fresh(&next, if with_decos { &decos } else { &[] });
        let html = rinch_editor_core::serialize::node_to_html(&next.doc);
        assert_eq!(
            got,
            want,
            "seed {seed} step {step}: incremental DOM != fresh render\nprev doc: {}\nnext doc: {html}",
            rinch_editor_core::serialize::node_to_html(&st.doc)
        );
        assert_eq!(
            h.mock.borrow().__node_count(),
            want_count,
            "seed {seed} step {step}: live node count differs from a fresh render (orphans?)\n{html}"
        );
        assert_eq!(
            empty_wrappers(&h, h.container_id),
            0,
            "seed {seed} step {step}: empty wrapper\n{got}"
        );
        history.push(next.clone());
        st = next;
    }
}

fn seeds(default: u64) -> u64 {
    std::env::var("RINCH_PROP_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn incremental_equals_fresh_marks_only() {
    for seed in 1..=seeds(300) {
        run_seed(seed, 25, false);
    }
}

#[test]
fn incremental_equals_fresh_with_decorations() {
    for seed in 1..=seeds(200) {
        run_seed(seed, 25, true);
    }
}
