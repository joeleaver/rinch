//! `Decoration::Node`: a plugin's class on a whole node's host element — which
//! ranges name a node, how classes merge with each other and with the
//! element's own, and that they stay right as the document, the hosts and the
//! decorations change.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::mock::MockDomDocument;
use rinch_core::dom::{DomContent, DomDocument, DomTree, NodeHandle, NodeId, RenderScope};
use rinch_editor_core::decoration::{Decoration, DecorationSet};
use rinch_editor_core::{Attrs, EditorState, Node, Plugin, PluginKey, Pos, Selection};
use rinch_editor_view::{EditorHandle, NodeViewCtx, NodeViewSpec, create_editor};

/// What a test asks to have decorated.
#[derive(Clone)]
enum Ask {
    /// Exactly this range, whatever stands there.
    Range(usize, usize, &'static str),
    /// Every node of this type, found in the document each time.
    Type(&'static str, &'static str),
    /// The `n`th node, in document order, that is not a text run.
    Nth(usize, &'static str),
}

#[derive(Default)]
struct Asks(RefCell<Vec<Ask>>);

/// Every non-text node of `doc` with its range, in document order.
fn nodes(doc: &Node) -> Vec<(usize, usize, Node)> {
    let mut out = Vec::new();
    doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
        if !n.is_text() {
            out.push((pos, pos + n.node_size(), n.clone()));
        }
        true
    });
    out
}

fn deco(from: usize, to: usize, class: &str) -> Decoration {
    Decoration::node(Pos(from), Pos(to), Attrs::new().with("class", class))
}

impl Plugin for Asks {
    fn key(&self) -> PluginKey {
        PluginKey("test.node-decorations")
    }
    fn decorations(&self, st: &EditorState) -> DecorationSet {
        let all = nodes(&st.doc);
        let mut out = Vec::new();
        for ask in self.0.borrow().iter() {
            match *ask {
                Ask::Range(a, b, c) => out.push(deco(a, b, c)),
                Ask::Type(t, c) => out.extend(
                    all.iter()
                        .filter(|(_, _, n)| n.type_name() == t)
                        .map(|(a, b, _)| deco(*a, *b, c)),
                ),
                Ask::Nth(n, c) => out.extend(all.get(n).map(|(a, b, _)| deco(*a, *b, c))),
            }
        }
        DecorationSet::new(out)
    }
}

struct Page {
    doc: Rc<RefCell<MockDomDocument>>,
    _scope: RenderScope,
    handle: EditorHandle,
    container: NodeId,
    asks: Rc<Asks>,
}

fn page(html: &str) -> Page {
    let doc = Rc::new(RefCell::new(MockDomDocument::new()));
    let body = doc.borrow().body();
    let dyn_doc: Rc<RefCell<dyn DomDocument>> = doc.clone();
    let mut scope = RenderScope::new(dyn_doc.clone(), body);
    let handle = create_editor();
    let asks = Rc::new(Asks::default());
    assert!(handle.add_plugin(asks.clone()));
    assert!(handle.load_html(html));
    let container = handle.mount(&mut scope);
    NodeHandle::new(body, Rc::downgrade(&dyn_doc)).append_child(&container);
    Page {
        doc,
        _scope: scope,
        container: container.node_id(),
        handle,
        asks,
    }
}

impl Page {
    /// Change what is decorated, with a transaction that changes nothing else.
    fn ask(&self, asks: &[Ask]) {
        *self.asks.0.borrow_mut() = asks.to_vec();
        assert!(self.handle.update(|st| Some(st.tr())));
    }

    fn handle_for(&self, id: NodeId) -> NodeHandle {
        let dyn_doc: Rc<RefCell<dyn DomDocument>> = self.doc.clone();
        NodeHandle::new(id, Rc::downgrade(&dyn_doc))
    }

    /// The node hosts (`data-pm-type`) under the container, in document order.
    fn hosts(&self) -> Vec<NodeId> {
        fn walk(d: &MockDomDocument, id: NodeId, out: &mut Vec<NodeId>) {
            for c in d.get_children(id) {
                if d.get_attribute(c, "data-pm-type").is_some() {
                    out.push(c);
                }
                walk(d, c, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.doc.borrow(), self.container, &mut out);
        out
    }

    /// The host of the `n`th node of type `ty`.
    fn host(&self, ty: &str, n: usize) -> NodeId {
        let d = self.doc.borrow();
        self.hosts()
            .into_iter()
            .filter(|id| d.get_attribute(*id, "data-pm-type").as_deref() == Some(ty))
            .nth(n)
            .unwrap_or_else(|| panic!("no {ty} #{n}"))
    }

    fn class(&self, id: NodeId) -> Option<String> {
        self.doc.borrow().get_attribute(id, "class")
    }

    /// `type=class` for every node host that has a class.
    fn classed(&self) -> Vec<String> {
        self.hosts()
            .into_iter()
            .filter_map(|id| {
                let class = self.class(id)?;
                let d = self.doc.borrow();
                let ty = d.get_attribute(id, "data-pm-type")?;
                Some(format!("{ty}={class}"))
            })
            .collect()
    }

    /// The editor's DOM: tags, node types and classes.
    fn shape(&self) -> String {
        fn walk(d: &MockDomDocument, id: NodeId, out: &mut String) {
            match d.tag_name(id) {
                Some(tag) => {
                    out.push_str(&format!("<{tag}"));
                    for a in ["class", "data-pm-type"] {
                        if let Some(v) = d.get_attribute(id, a) {
                            // A class list is a set: the words' order depends
                            // on which came first.
                            let mut words: Vec<&str> = v.split_whitespace().collect();
                            words.sort_unstable();
                            out.push_str(&format!(" {a}={:?}", words.join(" ")));
                        }
                    }
                    out.push('>');
                    for c in d.get_children(id) {
                        // Overlays are not the document's.
                        if d.get_attribute(c, "data-pm-selection").is_none() {
                            walk(d, c, out);
                        }
                    }
                    out.push_str(&format!("</{tag}>"));
                }
                None => out.push_str(&format!("{:?}", d.text_content(id).unwrap_or_default())),
            }
        }
        let d = self.doc.borrow();
        let mut out = String::new();
        for c in d.get_children(self.container) {
            if d.get_attribute(c, "data-pm-type").is_some() {
                walk(&d, c, &mut out);
            }
        }
        out
    }

    /// The incrementally kept DOM equals a fresh editor's of the same document.
    fn assert_fresh(&self, ctx: &str) {
        let fresh = page("<p></p>");
        *fresh.asks.0.borrow_mut() = self.asks.0.borrow().clone();
        fresh.handle.load_doc(self.handle.doc());
        assert_eq!(self.shape(), fresh.shape(), "{ctx}");
    }
}

const EVERYTHING: &str = "<h2>title</h2><p>a<br>b<img src=\"x.png\">c</p>\
     <blockquote><p>quoted</p></blockquote><ul><li><p>item</p></li></ul><hr>\
     <table><tr><td><p>cell</p></td></tr></table><pre><code>code</code></pre>";

#[test]
fn every_kind_of_node_takes_the_class_on_its_own_host() {
    let p = page(EVERYTHING);
    for ty in [
        "heading",
        "paragraph",
        "hard_break",
        "image",
        "blockquote",
        "bullet_list",
        "list_item",
        "horizontal_rule",
        "table",
        "table_row",
        "table_cell",
        "code_block",
    ] {
        p.ask(&[Ask::Type(ty, "on")]);
        let classed = p.classed();
        assert!(!classed.is_empty(), "{ty}: nothing classed");
        assert!(
            classed.iter().all(|c| *c == format!("{ty}=on")),
            "{ty}: {classed:?}"
        );
        p.ask(&[]);
        assert_eq!(p.classed(), Vec::<String>::new(), "{ty}: left behind");
    }
}

#[test]
fn a_range_that_does_not_span_one_node_draws_nothing() {
    let p = page("<p>one</p><blockquote><p>two</p></blockquote>");
    // p: 0..5, blockquote: 5..12, its p: 6..11.
    let end = p.handle.doc().content_size();
    assert_eq!(end, 12);
    for (a, b) in [
        (0, 4),   // short of the end
        (0, 6),   // past it
        (1, 5),   // from inside
        (1, 4),   // the textblock's content: a text run, which has no element
        (2, 3),   // inside a text run
        (0, 12),  // two siblings; also the document's whole content
        (5, 11),  // the container's start, its child's end
        (6, 12),  // the child's start, the container's end
        (12, 14), // past the document
        (40, 50), // far past it
    ] {
        p.ask(&[Ask::Range(a, b, "on")]);
        assert_eq!(p.classed(), Vec::<String>::new(), "{a}..{b}");
    }
    p.ask(&[Ask::Range(6, 11, "inner"), Ask::Range(5, 12, "outer")]);
    assert_eq!(p.classed(), ["blockquote=outer", "paragraph=inner"]);
    assert_eq!(p.class(p.host("paragraph", 0)), None);
}

#[test]
fn several_decorations_of_one_node_add_the_union_of_their_words() {
    let p = page("<p>one</p>");
    p.ask(&[
        Ask::Range(0, 5, "b a"),
        Ask::Range(0, 5, " c  a "),
        Ask::Range(0, 5, "b"),
    ]);
    assert_eq!(p.classed(), ["paragraph=b a c"]);
    // The words that stay keep their places on the element.
    p.ask(&[Ask::Range(0, 5, " c  a ")]);
    assert_eq!(p.classed(), ["paragraph=a c"]);
    p.ask(&[]);
    assert_eq!(p.class(p.host("paragraph", 0)), None, "attribute removed");
}

#[test]
fn the_elements_own_classes_are_never_taken_off() {
    let p = page("<p>one</p>");
    let host = p.handle_for(p.host("paragraph", 0));
    host.set_attribute("class", "own shared");
    p.ask(&[Ask::Range(0, 5, "shared mine")]);
    assert_eq!(p.classed(), ["paragraph=own shared mine"]);
    // A class put on the element while it is decorated stays too.
    host.add_class("later");
    p.ask(&[Ask::Range(0, 5, "mine other")]);
    assert_eq!(p.classed(), ["paragraph=own shared mine later other"]);
    p.ask(&[]);
    assert_eq!(p.classed(), ["paragraph=own shared later"]);
}

#[test]
fn a_decoration_only_change_rebuilds_nothing_and_owes_an_overlay_pass() {
    let p = page("<p>one</p><p>two</p>");
    let before = (p.hosts(), p.handle.doc());
    let text = p.doc.borrow().get_children(before.0[0]);
    let key = p.doc.borrow().doc_key();
    p.ask(&[Ask::Nth(0, "on")]);
    assert_eq!(p.classed(), ["paragraph=on"]);
    assert!(rinch_editor_view::overlay_pass_owed(key));
    assert_eq!(p.hosts(), before.0);
    assert_eq!(p.doc.borrow().get_children(before.0[0]), text);
    assert!(p.handle.doc().same_ref(&before.1));
    p.ask(&[]);
    assert_eq!(p.classed(), Vec::<String>::new());
    assert_eq!(p.hosts(), before.0);
}

#[test]
fn a_retagged_block_carries_the_class_on_its_new_host() {
    let p = page("<p>one</p><p>two</p>");
    p.ask(&[Ask::Nth(0, "on")]);
    p.handle.set_selection(Selection::cursor(Pos(2)));
    let old = p.host("paragraph", 0);
    assert!(p.handle.command("setHeading2"));
    assert_ne!(p.host("heading", 0), old);
    assert_eq!(p.classed(), ["heading=on"]);
    p.assert_fresh("retag");
    assert!(p.handle.command("undo"));
    assert_eq!(p.classed(), ["paragraph=on"]);
    p.assert_fresh("undo");
}

#[test]
fn a_host_reused_for_another_node_loses_the_class() {
    let p = page("<p>one</p><p>two</p><p>three</p>");
    p.ask(&[Ask::Range(0, 5, "at-start")]);
    let first = p.host("paragraph", 0);
    // A new first block: whatever host it gets, the class is the node's at the
    // range, and the node that moved down has none.
    assert!(p.handle.update(|st| {
        let mut tr = st.tr();
        let copy = rinch_editor_core::Fragment::from_node(st.doc.child(2).clone());
        tr.replace_with(0, 0, copy).ok()?;
        Some(tr)
    }));
    // "three" is 7 long, so the range spans no node now.
    assert_eq!(p.classed(), Vec::<String>::new());
    assert!(p.class(first).is_none());
    p.assert_fresh("insert before");
    p.ask(&[Ask::Nth(0, "first")]);
    for _ in 0..2 {
        assert!(p.handle.update(|st| {
            let mut tr = st.tr();
            tr.delete(0, st.doc.child(0).node_size()).ok()?;
            Some(tr)
        }));
        assert_eq!(p.classed(), ["paragraph=first"]);
        p.assert_fresh("delete first");
    }
}

#[test]
fn a_class_follows_containers_being_made_and_unmade() {
    let p = page("<p>one</p><p>two</p>");
    p.ask(&[Ask::Type("paragraph", "p"), Ask::Type("blockquote", "q")]);
    p.handle.set_selection(Selection::cursor(Pos(2)));
    for step in [
        "wrapInBlockquote",
        "enter",
        "undo",
        "wrapInBlockquote",
        "toggleBulletList",
        "enter",
        "undo",
        "undo",
    ] {
        assert!(p.handle.command(step), "{step}");
        p.assert_fresh(step);
    }
}

/// A fixed pseudo-random walk of edits and decoration changes: after each, the
/// DOM equals a fresh render.
#[test]
fn random_edits_and_decoration_changes_equal_a_fresh_render() {
    const CLASSES: [&str; 4] = ["a", "b", "a b", "c"];
    let p = page(EVERYTHING);
    let mut seed = 0x9e37_79b9_u64;
    let mut roll = move |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let mut ran = 0;
    for step in 0..600 {
        match roll(4) {
            0 => {
                let count = nodes(&p.handle.doc()).len().max(1);
                let asks: Vec<Ask> = (0..roll(5))
                    .map(|_| Ask::Nth(roll(count), CLASSES[roll(4)]))
                    .collect();
                p.ask(&asks);
            }
            1 => {
                let size = p.handle.doc().content_size();
                p.handle
                    .set_selection(Selection::cursor(Pos(roll(size) + 1)));
                p.handle.insert_text("z");
            }
            _ => {
                let size = p.handle.doc().content_size();
                p.handle
                    .set_selection(Selection::cursor(Pos(roll(size) + 1)));
                let command = [
                    "enter",
                    "backspace",
                    "setHeading1",
                    "setParagraph",
                    "wrapInBlockquote",
                    "toggleBulletList",
                    "undo",
                    "redo",
                ][roll(8)];
                if p.handle.command(command) {
                    ran += 1;
                }
            }
        }
        p.assert_fresh(&format!("step {step}"));
    }
    assert!(ran > 100, "the walk did little: {ran}");
}

fn view(node_type: &str) -> NodeViewSpec {
    NodeViewSpec::new(node_type, |scope: &mut RenderScope, _ctx: NodeViewCtx| {
        scope.create_element("b")
    })
}

#[test]
fn a_node_views_host_keeps_the_class_through_mount_and_unmount() {
    // A block leaf, whose host stays, and an inline leaf, whose `<img>` is
    // swapped for a `<span>` and back.
    let p = page("<p>a<img src=\"x.png\">b</p><hr>");
    p.ask(&[
        Ask::Type("image", "pic"),
        Ask::Type("horizontal_rule", "line"),
    ]);
    let want = ["image=pic", "horizontal_rule=line"];
    assert_eq!(p.classed(), want);
    let img = p.host("image", 0);
    let image_view = p.handle.register_node_view(view("image"));
    let rule_view = p.handle.register_node_view(view("horizontal_rule"));
    assert_ne!(p.host("image", 0), img, "precondition: the host is swapped");
    assert_eq!(p.classed(), want, "mounted");
    p.ask(&[Ask::Type("image", "other")]);
    assert_eq!(p.classed(), ["image=other"], "changed while mounted");
    p.ask(&[
        Ask::Type("image", "pic"),
        Ask::Type("horizontal_rule", "line"),
    ]);
    drop(image_view);
    drop(rule_view);
    assert_eq!(p.classed(), want, "unmounted");
    p.ask(&[]);
    assert_eq!(p.classed(), Vec::<String>::new());
}

#[test]
fn a_node_view_mounted_with_the_node_gets_the_class() {
    let p = page("<p>x</p>");
    p.handle.register_node_view(view("image")).keep();
    p.ask(&[Ask::Type("image", "pic")]);
    assert!(p.handle.load_html("<p>a<img src=\"x.png\">b</p>"));
    assert_eq!(p.classed(), ["image=pic"]);
}
