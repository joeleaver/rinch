//! #848: the inline-decoration pass under a peer's edits. A guest editor with a
//! decorating plugin and an image node view receives the host's changes; after
//! each one its DOM must equal a fresh editor loaded with the same document,
//! plugin and node view.
#![cfg(feature = "collaboration")]

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{Attrs, EditorState, Plugin, PluginKey, Pos, Selection};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

/// Every "helo" (class `s`), and — first in draw order though it starts later
/// than some — one decoration from the second "helo" to the last (class `w`),
/// across blocks.
struct Helo;
impl Plugin for Helo {
    fn key(&self) -> PluginKey {
        PluginKey("test848.helo")
    }
    fn decorations(&self, st: &EditorState) -> DecorationSet {
        let mut hits: Vec<usize> = Vec::new();
        st.doc
            .nodes_between(0, st.doc.content_size(), &mut |n, pos, _| {
                if let Some(t) = n.text() {
                    let chars: Vec<char> = t.chars().collect();
                    for i in 0..chars.len().saturating_sub(3) {
                        if chars[i..i + 4] == ['h', 'e', 'l', 'o'] {
                            hits.push(pos + i);
                        }
                    }
                }
                true
            });
        let mut out = Vec::new();
        if hits.len() >= 3 {
            out.push(Decoration::inline(
                Pos(hits[1] + 2),
                Pos(hits[hits.len() - 1] + 2),
                Attrs::new().with("class", "w"),
            ));
        }
        for h in hits {
            out.push(Decoration::inline(
                Pos(h),
                Pos(h + 4),
                Attrs::new().with("class", "s"),
            ));
        }
        DecorationSet::new(out)
    }
}

fn spec() -> NodeViewSpec {
    NodeViewSpec::new("image", |scope: &mut RenderScope, _ctx: NodeViewCtx| {
        scope.create_element("b")
    })
}

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeId,
}

fn page(html: &str) -> Page {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    assert!(handle.load_html(html));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page {
        doc,
        _scope: scope,
        container: container.node_id(),
        handle,
    }
}

fn decorated(p: &Page) {
    p.handle.register_node_view(spec()).keep();
    assert!(p.handle.add_plugin(Rc::new(Helo)));
}

fn shape(p: &Page) -> String {
    let d = p.doc.borrow();
    fn walk(d: &MockDomDocument, id: NodeId, out: &mut String) {
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
    walk(&d, p.container, &mut out);
    out
}

fn html(h: &EditorHandle) -> String {
    rinch_editor_core::serialize::html::node_to_html(&h.doc())
}

const START: &str = "<p>one helo two</p><blockquote><p>helo <strong>three</strong></p>\
     <p>four</p></blockquote><ul><li><p>five helo<img src=\"x.png\">six</p></li></ul><p>last helo</p>";

#[test]
fn a_peers_edits_leave_the_decorations_exactly_as_a_fresh_render() {
    let a = page(START);
    let b = page("<p></p>");
    let to_b: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let sink = to_b.clone();
    let snapshot = a
        .handle
        .start_collaboration_host(move |d| sink.borrow_mut().push(d))
        .unwrap();
    b.handle
        .start_collaboration_guest(&snapshot, |_| {})
        .unwrap();
    decorated(&b);

    // Each edit on the host, as (from, to, text): replace from..to with text.
    let last = a.handle.doc().content_size() - 2;
    let edits: Vec<Box<dyn Fn(&EditorState) -> (usize, usize, &'static str)>> = vec![
        // Type at the end of the last paragraph, away from every decoration.
        Box::new(move |_| (last, last, "x")),
        // Type before everything: every decoration shifts.
        Box::new(|_| (1, 1, "helo ")),
        // Delete the first "helo" of the blockquote and the text around it.
        Box::new(|st| {
            let s = html_text_pos(st, "helo", 2);
            (s - 1, s + 5, "")
        }),
        // Make a new "helo" out of two halves (a decoration appears).
        Box::new(|st| {
            let s = html_text_pos(st, "four", 0);
            (s, s + 4, "hel")
        }),
        Box::new(|st| {
            let s = html_text_pos(st, "hel", 0);
            (s + 3, s + 3, "o")
        }),
    ];
    for (i, edit) in edits.iter().enumerate() {
        let (from, to, text) = edit(&a.handle.state());
        assert!(a.handle.update(|st| {
            let mut tr = st.tr();
            if to > from {
                tr.delete(from, to).ok()?;
            }
            if !text.is_empty() {
                tr.set_selection(Selection::cursor(Pos(from)));
                tr.insert_text(text).ok()?;
            }
            Some(tr)
        }));
        for d in to_b.borrow_mut().drain(..) {
            b.handle.collab_receive(&d);
            assert!(b.handle.collab_take_error().is_none());
        }
        assert_eq!(
            html(&b.handle),
            html(&a.handle),
            "edit {i}: the peer diverged"
        );
        let fresh = page(&html(&b.handle));
        decorated(&fresh);
        assert_eq!(
            shape(&b),
            shape(&fresh),
            "edit {i}: peer DOM != fresh render"
        );
        assert!(
            shape(&b).contains("data-pm-deco"),
            "edit {i}: the fixture decorates something"
        );
    }
}

/// The position of the `nth` "word" (counted from 0) among `st`'s text nodes.
fn html_text_pos(st: &EditorState, word: &str, nth: usize) -> usize {
    let mut found = Vec::new();
    st.doc
        .nodes_between(0, st.doc.content_size(), &mut |n, pos, _| {
            if let Some(t) = n.text() {
                let chars: Vec<char> = t.chars().collect();
                let w: Vec<char> = word.chars().collect();
                for i in 0..(chars.len() + 1).saturating_sub(w.len()) {
                    if chars[i..i + w.len()] == w[..] {
                        found.push(pos + i);
                    }
                }
            }
            true
        });
    found[nth]
}
