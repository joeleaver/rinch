//! Node views C: a spec for the inline `image` atom claims the images it
//! matches. A claimed image's host is an inline-block `<span>` holding the app's
//! component instead of the `<img>`; every other image keeps its `<img>`.
//! Pinned on `MockDomDocument` (the platform halves are in `rinch` and
//! `rinch-web`):
//!
//! - only a matching image is claimed, and only while it matches;
//! - the claimed image is still one atomic inline of the model and the view:
//!   the caret map, `pos_beside_inline_leaf` and the node selection name its span;
//! - an attribute change (local, undo, a collaborator's) keeps the mount, a move
//!   remounts;
//! - releasing the registration, or the image no longer matching, gives the
//!   `<img>` back, with its source.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomDocument, NodeHandle, NodeId, RenderScope};
use rinch_core::reactive::{Signal, on_cleanup};
use rinch_editor_core::{AttrValue, Fragment, NodeSelection, Pos, Selection, Slice};
use rinch_editor_view::{
    EditorHandle, NODE_VIEW_ATTR, NodeViewCtx, NodeViewSpec, create_editor,
};

#[derive(Default)]
struct Seen {
    renders: Cell<usize>,
    cleanups: Cell<usize>,
    state: RefCell<Vec<Signal<i32>>>,
    ctx: RefCell<Vec<NodeViewCtx>>,
    badges: RefCell<Vec<String>>,
    roots: RefCell<Vec<NodeHandle>>,
}

/// A view of the images carrying `data-badge`.
fn badge_spec(seen: &Rc<Seen>) -> NodeViewSpec {
    let s = seen.clone();
    NodeViewSpec::new("image", move |scope: &mut RenderScope, ctx: NodeViewCtx| {
        s.renders.set(s.renders.get() + 1);
        let root = scope.create_element("b");
        let label = scope.create_text("chip");
        root.append_child(&label);
        s.state.borrow_mut().push(Signal::new(0));
        s.ctx.borrow_mut().push(ctx.clone());
        s.roots.borrow_mut().push(root.clone());
        let attrs = ctx.attrs;
        let seen = s.clone();
        scope.create_effect(move || {
            let badge = attrs.get().get_str("data-badge").unwrap_or("").to_string();
            seen.badges.borrow_mut().push(badge);
        });
        let c = s.clone();
        on_cleanup(move || c.cleanups.set(c.cleanups.get() + 1));
        root
    })
    .matches(|attrs| attrs.get_str("data-badge").is_some())
}

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeHandle,
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
        handle,
        container,
    }
}

/// `ab` 1..3, the badged image at 3, `cd` 4..6, a plain image at 6, `ef` 7..9.
const DOC: &str = r#"<p>ab<img src="a.png" data-badge="new">cd<img src="b.png">ef</p>"#;
const BADGED: usize = 3;
const PLAIN: usize = 6;

/// Every element under `id` carrying `data-pm-type="image"`, depth first.
fn image_hosts(p: &Page) -> Vec<NodeId> {
    fn walk(d: &MockDomDocument, id: NodeId, out: &mut Vec<NodeId>) {
        for c in d.get_children(id) {
            if d.get_attribute(c, "data-pm-type").as_deref() == Some("image") {
                out.push(c);
            }
            walk(d, c, out);
        }
    }
    let mut out = Vec::new();
    walk(&p.doc.borrow(), p.container.node_id(), &mut out);
    out
}

fn tag(p: &Page, id: NodeId) -> String {
    p.doc.borrow().tag_name(id).unwrap_or_default()
}

fn attr(p: &Page, id: NodeId, name: &str) -> Option<String> {
    p.doc.borrow().get_attribute(id, name)
}

fn register(p: &Page) -> Rc<Seen> {
    let seen = Rc::new(Seen::default());
    p.handle.register_node_view(badge_spec(&seen)).keep();
    seen
}

#[test]
fn only_a_matching_image_is_claimed_and_its_host_is_an_inline_block_span() {
    let p = page(DOC);
    let seen = register(&p);
    assert_eq!(seen.renders.get(), 1, "one image matches");
    let hosts = image_hosts(&p);
    assert_eq!(hosts.len(), 2);
    let (claimed, plain) = (hosts[0], hosts[1]);
    assert_eq!(tag(&p, claimed), "span", "a claimed image is a span");
    assert_eq!(attr(&p, claimed, NODE_VIEW_ATTR).as_deref(), Some(""));
    assert_eq!(attr(&p, claimed, "data-badge").as_deref(), Some("new"));
    assert!(
        attr(&p, claimed, "style")
            .unwrap_or_default()
            .contains("display: inline-block"),
        "sized by its content, on the line: {:?}",
        attr(&p, claimed, "style")
    );
    assert_eq!(attr(&p, claimed, "src"), None, "no picture attributes");
    assert_eq!(
        p.doc.borrow().get_children(claimed),
        vec![seen.roots.borrow()[0].node_id()],
        "the component is in the span"
    );
    assert_eq!(tag(&p, plain), "img", "an image it does not match is unchanged");
    assert_eq!(attr(&p, plain, "src").as_deref(), Some("b.png"));
    assert_eq!(attr(&p, plain, NODE_VIEW_ATTR), None);
    assert!(p.handle.is_node_view_host(claimed.0));
    assert!(!p.handle.is_node_view_host(plain.0));
    assert_eq!(seen.ctx.borrow()[0].pos(), Some(Pos(BADGED)));
}

/// The span is the image's host for everything that names a host: a press on
/// its side resolves to the positions beside the image, and a node selection
/// of it is the image.
#[test]
fn the_span_is_the_one_atomic_inline_of_the_image() {
    let p = page(DOC);
    let _seen = register(&p);
    let span = image_hosts(&p)[0];
    assert_eq!(
        p.handle.pos_beside_inline_leaf(span.0, false),
        Some(Pos(BADGED))
    );
    assert_eq!(
        p.handle.pos_beside_inline_leaf(span.0, true),
        Some(Pos(BADGED + 1))
    );
    assert_eq!(
        p.handle.node_selection_at_host(span.0),
        Selection::node_at(&p.handle.doc(), Pos(BADGED)),
    );
    // Arrows step over it as over any image: one position.
    p.handle.set_selection(Selection::cursor(Pos(BADGED)));
    assert!(
        p.handle
            .move_cursor(rinch_editor_core::CursorMotion::CharRight, false)
    );
    assert_eq!(p.handle.selection().head(), Pos(BADGED + 1));
    // A whole-block selection still takes it, and the text around it is one block.
    assert_eq!(p.handle.doc().child(0).content().size(), 8);
}

#[test]
fn an_attr_change_keeps_the_mount_and_undo_redo_too() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(4);
    let ctx = seen.ctx.borrow()[0].clone();
    assert!(ctx.set_attrs(|a| *a = a.with("data-badge", AttrValue::from("hot"))));
    assert!(p.handle.command("undo"));
    assert!(p.handle.command("redo"));
    assert_eq!(*seen.badges.borrow(), ["new", "hot", "new", "hot"]);
    assert_eq!(seen.renders.get(), 1, "one mount throughout");
    let span = image_hosts(&p)[0];
    assert_eq!(tag(&p, span), "span");
    assert_eq!(attr(&p, span, "data-badge").as_deref(), Some("hot"));
    assert_eq!(attr(&p, span, "src"), None, "the span never takes a src");
    assert_eq!(seen.state.borrow()[0].get(), 4);
    // Typing beside it keeps it too.
    p.handle.set_selection(Selection::cursor(Pos(2)));
    assert!(p.handle.insert_text("x"));
    assert_eq!(seen.renders.get(), 1);
    assert_eq!(ctx.pos(), Some(Pos(BADGED + 1)));
}

/// An image that stops matching gets its `<img>` back, with its picture; one
/// that starts matching is claimed.
#[test]
fn an_image_that_stops_matching_is_an_img_again() {
    let p = page(DOC);
    let seen = register(&p);
    let ctx = seen.ctx.borrow()[0].clone();
    assert!(ctx.set_attrs(|a| *a = a.without("data-badge")));
    assert_eq!(seen.cleanups.get(), 1);
    let hosts = image_hosts(&p);
    assert_eq!(tag(&p, hosts[0]), "img");
    assert_eq!(attr(&p, hosts[0], "src").as_deref(), Some("a.png"));
    assert_eq!(attr(&p, hosts[0], NODE_VIEW_ATTR), None);
    assert!(p.doc.borrow().get_children(hosts[0]).is_empty());
    // The plain image starts matching.
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.step(Box::new(rinch_editor_core::SetNodeAttrStep::new(
            PLAIN,
            "data-badge",
            AttrValue::from("b"),
        )))
        .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 2);
    let hosts = image_hosts(&p);
    assert_eq!(tag(&p, hosts[1]), "span");
    assert_eq!(attr(&p, hosts[1], NODE_VIEW_ATTR).as_deref(), Some(""));
}

#[test]
fn releasing_the_registration_gives_every_img_back() {
    let p = page(DOC);
    let seen = Rc::new(Seen::default());
    let reg = p.handle.register_node_view(badge_spec(&seen));
    assert_eq!(tag(&p, image_hosts(&p)[0]), "span");
    drop(reg);
    assert_eq!(seen.cleanups.get(), 1);
    let hosts = image_hosts(&p);
    assert_eq!(
        hosts.iter().map(|&h| tag(&p, h)).collect::<Vec<_>>(),
        ["img", "img"]
    );
    assert_eq!(attr(&p, hosts[0], "src").as_deref(), Some("a.png"));
    assert_eq!(attr(&p, hosts[0], "data-badge").as_deref(), Some("new"));
}

/// A linked image: the span stands inside the link's `<a>`, where the `<img>`
/// stood.
#[test]
fn a_linked_claimed_image_keeps_its_link_wrapper() {
    let p = page(r#"<p>ab<a href="https://x.test"><img src="a.png" data-badge="l"></a>cd</p>"#);
    let seen = register(&p);
    assert_eq!(seen.renders.get(), 1);
    let span = image_hosts(&p)[0];
    assert_eq!(tag(&p, span), "span");
    let parent = p.doc.borrow().parent_node(span).unwrap();
    assert_eq!(tag(&p, parent), "a");
    // And back.
    let ctx = seen.ctx.borrow()[0].clone();
    assert!(ctx.set_attrs(|a| *a = a.without("data-badge")));
    let img = image_hosts(&p)[0];
    assert_eq!(tag(&p, img), "img");
    assert_eq!(tag(&p, p.doc.borrow().parent_node(img).unwrap()), "a");
}

/// Decision (d), as for an `embed`: the very same image node moved within one
/// change keeps its view (it is the same node), and an equal copy put in its
/// place — a cut and paste, a replace — is another node, and another mount.
#[test]
fn a_claimed_image_replaced_by_an_equal_copy_is_remounted() {
    let p = page(DOC);
    let seen = register(&p);
    seen.state.borrow()[0].set(9);
    let image = p.handle.doc().node_at(BADGED).unwrap().clone();
    // The same node, moved after "ef" in one transaction.
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.delete(BADGED, BADGED + 1).ok()?;
        tr.replace(8, 8, Slice::new(Fragment::from_node(image.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 1, "the same node keeps its view");
    assert_eq!(seen.ctx.borrow()[0].pos(), Some(Pos(8)));
    // An equal copy in its place.
    let copy = image.copy_with_content(image.content().clone());
    assert!(!copy.same_ref(&image) && copy == image);
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        tr.replace(8, 9, Slice::new(Fragment::from_node(copy.clone()), 0, 0))
            .ok()?;
        Some(tr)
    }));
    assert_eq!(seen.renders.get(), 2);
    assert_eq!(seen.cleanups.get(), 1);
    assert_eq!(seen.state.borrow()[1].get(), 0, "fresh state");
    assert_eq!(seen.ctx.borrow()[1].pos(), Some(Pos(8)));
    assert_eq!(tag(&p, image_hosts(&p)[1]), "span");
}

/// The node selection outlines the span, and the view's `selected` memo follows.
#[test]
fn a_node_selected_claimed_image_tells_its_view() {
    let p = page(DOC);
    let seen = register(&p);
    p.handle.set_selection(Selection::Node(NodeSelection {
        anchor: Pos(BADGED),
        head: Pos(BADGED + 1),
    }));
    assert!(seen.ctx.borrow()[0].selected.get());
    p.handle.set_selection(Selection::cursor(Pos(1)));
    assert!(!seen.ctx.borrow()[0].selected.get());
}

/// `set_image_source` still picks the source of the images no spec claims, and
/// is not asked to write one on a span.
#[test]
fn set_image_source_still_reaches_unclaimed_images() {
    rinch_editor_view::set_image_source(|attrs| {
        attrs
            .get_str("src")
            .map(|s| format!("https://cdn.test/{s}"))
    });
    let p = page(DOC);
    let _seen = register(&p);
    let hosts = image_hosts(&p);
    assert_eq!(
        attr(&p, hosts[1], "src").as_deref(),
        Some("https://cdn.test/b.png")
    );
    assert_eq!(attr(&p, hosts[0], "src"), None);
    rinch_editor_view::clear_image_source();
}
