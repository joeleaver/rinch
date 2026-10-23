//! #1833 (from the review of PR #1862): random edits through `EditorHandle`
//! (node views on some images, undo/redo, a collaboration guest receiving
//! every change); the DOM of each editor must equal a fresh mount of the same
//! document. The review ran 400 seeds clean; CI runs fewer
//! (`RINCH_PROP_SEEDS` raises it).
#![cfg(feature = "collaboration")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_editor_core::*;
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeHandle,
}

fn spec() -> NodeViewSpec {
    NodeViewSpec::new(
        "image",
        move |scope: &mut RenderScope, _ctx: NodeViewCtx| {
            let root = scope.create_element("b");
            let label = scope.create_text("chip");
            root.append_child(&label);
            root
        },
    )
    .matches(|attrs| attrs.get_str("data-badge").is_some())
}

fn page_with(handle: EditorHandle) -> Page {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    handle.register_node_view(spec()).keep();
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page {
        doc,
        _scope: scope,
        handle,
        container,
    }
}

const ATTRS: &[&str] = &[
    "href",
    "style",
    "class",
    "src",
    "data-pm-mark",
    "data-badge",
    "data-pm-type",
];

fn shape(p: &Page) -> String {
    fn walk(d: &MockDomDocument, id: NodeId, out: &mut String) {
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
    walk(&p.doc.borrow(), p.container.node_id(), &mut out);
    out
}

fn fresh(doc: Node) -> (String, usize) {
    let h = create_editor();
    h.load_doc(doc);
    let p = page_with(h);
    let n = p.doc.borrow().__node_count();
    (shape(&p), n)
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

fn pool(s: &Schema) -> Vec<Mark> {
    let m = |name: &str, attrs: Attrs| Mark::new(s.mark_type(name).unwrap().clone(), attrs);
    vec![
        m("bold", Attrs::default()),
        m("italic", Attrs::default()),
        m("code", Attrs::default()),
        m("strike", Attrs::default()),
        m("highlight", Attrs::default()),
        m("link", Attrs::default().with("href", "https://a.test/")),
        m("link", Attrs::default().with("href", "https://b.test/")),
    ]
}

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
    match r.below(8) {
        0 => s
            .create_node("hard_break", Attrs::new(), Fragment::empty())
            .unwrap()
            .with_marks(marks),
        1 => {
            let mut a = Attrs::new().with("src", "x.png");
            if r.chance(60) {
                a = a.with("data-badge", "y");
            }
            s.create_node("image", a, Fragment::empty())
                .unwrap()
                .with_marks(marks)
        }
        _ => {
            let len = 1 + r.below(3);
            let text: String = (0..len)
                .map(|_| (b'a' + r.below(26) as u8) as char)
                .collect();
            s.text_with_marks(&text, marks).unwrap()
        }
    }
}

fn check(p: &Page, what: &str) {
    let got = shape(p);
    let n = p.doc.borrow().__node_count();
    let (want, want_n) = fresh(p.handle.doc());
    assert_eq!(got, want, "{what}: incremental != fresh");
    assert_eq!(n, want_n, "{what}: node count (orphans?)");
}

fn seeds(default: u64) -> u64 {
    std::env::var("RINCH_PROP_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
fn handles_incremental_equals_fresh() {
    for seed in 1..=seeds(15) {
        let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let host = page_with(create_editor());
        let s = host.handle.state().schema().clone();
        let pool = pool(&s);
        // A random starting document.
        let blocks: Vec<Node> = (0..1 + r.below(2))
            .map(|_| {
                let kids: Vec<Node> = (0..1 + r.below(6))
                    .map(|_| rand_inline(&mut r, &s, &pool))
                    .collect();
                s.branch("paragraph", Fragment::from_children(kids))
                    .unwrap()
            })
            .collect();
        assert!(host.handle.update(|st| {
            let mut tr = st.tr();
            tr.replace_with(
                0,
                st.doc.content_size(),
                Fragment::from_children(blocks.clone()),
            )
            .ok()?;
            Some(tr)
        }));
        check(&host, &format!("seed {seed} start"));
        let outbox: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let ob = outbox.clone();
        let snap = match host
            .handle
            .start_collaboration_host(move |d| ob.borrow_mut().push(d))
        {
            Ok(s) => s,
            Err(_) => continue,
        };
        let guest = page_with(create_editor());
        guest
            .handle
            .start_collaboration_guest(&snap, |_| {})
            .unwrap();
        check(&guest, &format!("seed {seed} guest join"));
        for step in 0..20 {
            let k = r.below(10);
            if k == 0 {
                host.handle.command("undo");
            } else if k == 1 {
                host.handle.command("redo");
            } else {
                let st = host.handle.state();
                let size = st.doc.content_size();
                let a = r.below(size + 1);
                let b = r.below(size + 1);
                let (from, to) = (a.min(b), a.max(b));
                let m = pool[r.below(pool.len())].clone();
                let op = r.below(6);
                let stored = rand_marks(&mut r, &pool);
                let node = rand_inline(&mut r, &s, &pool);
                let text: String = (0..1 + r.below(2))
                    .map(|_| (b'A' + r.below(26) as u8) as char)
                    .collect();
                host.handle.update(|st| {
                    let mut tr = st.tr();
                    match op {
                        0 | 1 => {
                            tr.add_mark(from, to, m.clone()).ok()?;
                        }
                        2 => {
                            tr.remove_mark(from, to, m.clone()).ok()?;
                        }
                        3 => {
                            tr.set_selection(Selection::cursor(Pos(from)));
                            tr.set_stored_marks(Some(stored.clone()));
                            tr.insert_text(&text).ok()?;
                        }
                        4 => {
                            tr.delete(from, to).ok()?;
                        }
                        _ => {
                            tr.set_selection(Selection::cursor(Pos(from)));
                            tr.replace_selection_with(node.clone()).ok()?;
                        }
                    }
                    Some(tr)
                });
            }
            check(&host, &format!("seed {seed} step {step} host"));
            for d in outbox.borrow_mut().drain(..) {
                guest.handle.collab_receive(&d);
            }
            check(&guest, &format!("seed {seed} step {step} guest"));
        }
    }
}
