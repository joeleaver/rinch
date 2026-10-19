//! Node views C: copy and paste of the atoms an app draws. A copied `embed` and a
//! copied claimed image paste back as the same atoms with their attributes (the
//! copy-out marks them with `data-rinch-clip`, the default paste keeps them
//! beside it), and the new nodes get views of their own. An app that keys state
//! to a data attribute (a sheet id) re-keys the pasted copy in
//! `Plugin::handle_paste`, so two atoms never share one key.

use std::cell::Cell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, RenderScope};
use rinch_editor_core::serialize::slice_from_pasted_html;
use rinch_editor_core::{
    AttrValue, EditorState, Fragment, Node, PasteContent, Plugin, PluginKey, Pos, Schema,
    Selection, Slice, Transaction,
};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};
use std::cell::RefCell;

const DOC: &str = r#"<p>ab<img src="a.png" data-badge="b1">cd</p><div data-pm-type="embed" data-sheet="s1"></div><p>ef</p>"#;

struct Page {
    _doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
}

fn page(handle: EditorHandle) -> Page {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page {
        _doc: doc,
        _scope: scope,
        handle,
    }
}

/// Count the views mounted for images and for embeds.
fn views(h: &EditorHandle) -> (Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let (images, embeds) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let i = images.clone();
    h.register_node_view(
        NodeViewSpec::new("image", move |s: &mut RenderScope, _: NodeViewCtx| {
            i.set(i.get() + 1);
            s.create_element("b")
        })
        .matches(|a| a.get_str("data-badge").is_some()),
    )
    .keep();
    let e = embeds.clone();
    h.register_node_view(NodeViewSpec::new(
        "embed",
        move |s: &mut RenderScope, _: NodeViewCtx| {
            e.set(e.get() + 1);
            s.create_element("div")
        },
    ))
    .keep();
    (images, embeds)
}

/// Every `(type, data-badge or data-sheet)` of the document's atoms, in order.
fn keys(doc: &Node) -> Vec<(String, String)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content().size(), &mut |n, _, _| {
        if let Some(k) = n
            .attrs()
            .get_str("data-badge")
            .or(n.attrs().get_str("data-sheet"))
        {
            out.push((n.type_name().to_string(), k.to_string()));
        }
        true
    });
    out
}

/// Copy the whole document, put the caret at the end, and paste.
fn copy_all_and_paste(h: &EditorHandle) {
    let doc = h.doc();
    h.set_selection(Selection::text(Pos(1), Pos(doc.content().size() - 1)));
    let (html, text) = h.selection_clipboard().expect("something to copy");
    assert!(html.contains("data-rinch-clip"), "{html}");
    let end = h.doc().content().size() - 1;
    h.set_selection(Selection::cursor(Pos(end)));
    assert!(h.paste(&PasteContent {
        text: Some(text),
        html: Some(html),
    }));
}

#[test]
fn a_copied_embed_and_claimed_image_paste_back_as_the_atoms() {
    let h = create_editor();
    assert!(h.load_html(DOC));
    let p = page(h);
    let (images, embeds) = views(&p.handle);
    assert_eq!((images.get(), embeds.get()), (1, 1));
    copy_all_and_paste(&p.handle);
    assert_eq!(
        keys(&p.handle.doc()),
        [
            ("image".into(), "b1".into()),
            ("embed".into(), "s1".into()),
            ("image".into(), "b1".into()),
            ("embed".into(), "s1".into()),
        ],
        "the copies keep their attributes: {:?}",
        p.handle.doc()
    );
    assert_eq!(
        (images.get(), embeds.get()),
        (2, 2),
        "each pasted atom has a view of its own"
    );
}

/// Re-keys every pasted atom's `data-badge` / `data-sheet` (`-copy`), as an app
/// whose views key state to it would: a pasted sheet is a new sheet.
struct ReKey(Rc<Cell<usize>>);

impl ReKey {
    fn rekey(schema: &Schema, n: &Node) -> Node {
        if n.is_leaf() && !n.is_text() {
            let mut attrs = n.attrs().clone();
            for key in ["data-badge", "data-sheet"] {
                if let Some(v) = attrs.get_str(key) {
                    let v = format!("{v}-copy");
                    attrs = attrs.with(key, AttrValue::from(v));
                }
            }
            return schema
                .create_node(n.type_name(), attrs, Fragment::empty())
                .unwrap()
                .with_marks(n.marks().to_vec());
        }
        if n.is_text() {
            return n.clone();
        }
        let children = n
            .content()
            .children()
            .iter()
            .map(|c| ReKey::rekey(schema, c))
            .collect();
        n.copy_with_content(Fragment::from_children(children))
    }
}

impl Plugin for ReKey {
    fn key(&self) -> PluginKey {
        PluginKey("test.rekey")
    }

    fn handle_paste(&self, state: &EditorState, paste: &PasteContent) -> Option<Transaction> {
        let html = paste.html.as_deref()?;
        let slice = slice_from_pasted_html(state.schema(), html).ok()?;
        let content: Vec<Node> = slice
            .content
            .children()
            .iter()
            .map(|n| ReKey::rekey(state.schema(), n))
            .collect();
        let slice = Slice::new(
            Fragment::from_children(content),
            slice.open_start,
            slice.open_end,
        );
        let mut tr = state.tr();
        tr.replace_selection(slice).ok()?;
        self.0.set(self.0.get() + 1);
        Some(tr)
    }
}

#[test]
fn an_app_re_keys_the_pasted_atoms_in_handle_paste() {
    let h = create_editor();
    let claimed = Rc::new(Cell::new(0));
    assert!(h.add_plugin(Rc::new(ReKey(claimed.clone()))));
    assert!(h.load_html(DOC));
    let p = page(h);
    let (images, embeds) = views(&p.handle);
    copy_all_and_paste(&p.handle);
    assert_eq!(claimed.get(), 1, "the plugin took the paste");
    assert_eq!(
        keys(&p.handle.doc()),
        [
            ("image".into(), "b1".into()),
            ("embed".into(), "s1".into()),
            ("image".into(), "b1-copy".into()),
            ("embed".into(), "s1-copy".into()),
        ]
    );
    assert_eq!((images.get(), embeds.get()), (2, 2));
    // One undo step takes the whole paste back.
    assert!(p.handle.command("undo"));
    assert_eq!(keys(&p.handle.doc()).len(), 2);
}
