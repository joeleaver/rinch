//! Bench from the review of #836: the per-transaction inline-decoration pass on a
//! long document. `cargo test --release ... -- --ignored --nocapture`.
use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle};
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::view::EditorView;
use rinch_editor_core::{
    Attrs, EditorState, Fragment, Plugin, PluginKey, Pos, Schema, Selection, default_plugins,
};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

struct Spell(Vec<(usize, usize)>);
impl Plugin for Spell {
    fn key(&self) -> PluginKey {
        PluginKey("review836.spell")
    }
    fn decorations(&self, _s: &EditorState) -> DecorationSet {
        DecorationSet::new(
            self.0
                .iter()
                .map(|&(a, b)| {
                    Decoration::inline(Pos(a), Pos(b), Attrs::new().with("class", "pm-spell-error"))
                })
                .collect(),
        )
    }
}

fn run(blocks: usize, decos: usize) -> f64 {
    let s = Rc::new(Schema::starter_kit());
    let text = "the quick brown fox jumps over the lazy dog again";
    let paras: Vec<_> = (0..blocks)
        .map(|_| {
            s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
                .unwrap()
        })
        .collect();
    let doc = s.branch("doc", Fragment::from_children(paras)).unwrap();
    let bs = text.chars().count() + 2;
    // One decoration on "quick" in every (blocks/decos)-th paragraph.
    let step = blocks.checked_div(decos).unwrap_or(1).max(1);
    let ranges: Vec<(usize, usize)> = (0..decos)
        .map(|i| {
            let b = (i * step).min(blocks - 1);
            (b * bs + 1 + 4, b * bs + 1 + 9)
        })
        .collect();
    let mut plugins = default_plugins();
    plugins.push(Rc::new(Spell(ranges)));
    let st = EditorState::create(s, doc, plugins);
    let d: Rc<RefCell<dyn DomDocument>> = Rc::new(RefCell::new(MockDomDocument::new()));
    let cid = d.borrow_mut().create_element("div");
    let container = NodeHandle::new(cid, Rc::downgrade(&d));
    let mut view = rinch_editor_view::RinchDomEditorView::new(container, Rc::downgrade(&d), &st);
    // Type into the LAST paragraph, away from the decorations.
    let at = blocks * bs - 1;
    let mut cur = {
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(Pos(at)));
        st.apply(tr)
    };
    let n = 200;
    let t0 = Instant::now();
    for _ in 0..n {
        let mut tr = cur.tr();
        tr.insert_text("x").unwrap();
        let next = cur.apply(tr);
        view.update_dom(&cur, &next);
        cur = next;
    }
    let total = t0.elapsed().as_secs_f64() * 1e6 / n as f64;
    let t1 = Instant::now();
    for _ in 0..n {
        std::hint::black_box(cur.decorations());
    }
    let deco_only = t1.elapsed().as_secs_f64() * 1e6 / n as f64;
    eprintln!("   (one state.decorations() call: {deco_only:.1} us)");
    total
}

#[test]
#[ignore]
fn bench_decoration_pass() {
    for (b, dcount) in [
        (2000, 0),
        (2000, 1),
        (2000, 200),
        (2000, 2000),
        (10000, 0),
        (10000, 1000),
    ] {
        // best of 5
        let best = (0..5).map(|_| run(b, dcount)).fold(f64::INFINITY, f64::min);
        eprintln!("blocks {b:>6} decos {dcount:>5}: {best:>9.1} us / keystroke");
    }
}
