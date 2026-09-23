//! Incremental restyling against a fresh document, for every kind of change a
//! selector can see.
//!
//! An attribute, class, id or interaction-state change is now invalidated by
//! Stylo's own invalidator (snapshot against the invalidation maps), and a
//! child-list change by the selector flags matching leaves on the parent
//! (`style_resolution/invalidation.rs`); the cascade then re-cascades a node's
//! children only when something they inherit moved (`resolve::child_cascade`).
//! Each of those is a way to restyle **less** than before, so each is a way to
//! leave a style stale. The oracle here is the twin one (`style_twin`): after
//! every change the incrementally restyled document must compute, node for
//! node and longhand for longhand, what a fresh document built directly in the
//! final state computes.
//!
//! The document is described by a [`Spec`] — a tree of elements with classes,
//! attributes, text and interaction state — which the fresh document is built
//! from in one pass (every element styled from scratch on insertion), while the
//! incremental one is mutated through the ordinary `DomDocument` verbs and
//! `update_hover` / `update_focus` / `update_active`.
//!
//! Two layers:
//! - named scenarios, one per selector shape × mutation that the previous
//!   invalidation got wrong or that the new one could plausibly get wrong;
//! - a seeded random differential over a stylesheet that covers descendant,
//!   child, `+`, `~`, `:nth-child(an+b)`, `:nth-child(… of S)`,
//!   `:nth-last-child`, `:first-/:last-/:only-child`, `:nth-of-type`,
//!   `:empty`, `:not()`, `:is()`, attribute operators and the `i` flag, ids,
//!   `:hover` / `:focus` / `:active`, `:checked` / `:disabled`, generated
//!   content through `attr()`, and inherited properties, under random class,
//!   attribute, id, text, insert, remove, move and state mutations.
//!
//! Every rule sets a custom property of its own (`--rN`), which the
//! fingerprint compares, so a rule that stopped (or started) matching shows up
//! even where it would change nothing visible.

mod style_twin;

use std::collections::BTreeMap;

use rinch_core::dom::{DomDocument, NodeId};
use rinch_dom::RinchDocument;
use style_twin::{assert_twin, assert_twin_opts};

const CSS: &str = r#"
    body { font-size: 16px; }
    .a { --r1: 1; }
    .a .b { --r2: 1; }
    .a > .b { --r3: 1; }
    .a + .b { --r4: 1; }
    .a ~ .c { --r5: 1; }
    :nth-child(2) { --r6: 1; }
    :nth-child(2n+1 of .b) { --r7: 1; }
    :nth-last-child(1) { --r8: 1; }
    :first-child { --r9: 1; }
    :last-child { --r10: 1; }
    :only-child { --r11: 1; }
    :empty { --r12: 1; }
    :not(.a) > .d { --r13: 1; }
    :is(.b, .c) .d { --r14: 1; }
    [data-x] { --r15: 1; }
    [data-x="2"] .a { --r16: 1; }
    [data-x^="1"] + * { --r17: 1; }
    [data-x="y" i] { --r18: 1; }
    #i1 .b { --r19: 1; }
    #i2 { --r20: 1; }
    .b:hover { --r21: 1; }
    .a:hover .c { --r22: 1; }
    :hover + .d { --r23: 1; }
    .c:focus { --r24: 1; }
    input:checked + .b { --r25: 1; }
    input:disabled ~ .c { --r26: 1; }
    :disabled { --r27: 1; }
    .a:active .b { --r28: 1; }
    span:nth-of-type(2) { --r29: 1; }
    li:last-of-type { --r30: 1; }
    :not(:first-child).b { --r31: 1; }
    .a:focus-within { --r32: 1; }
    .d { color: rgb(1, 2, 3); }
    .a.b { background-color: rgb(200, 0, 0); }
    .c .a { font-size: 20px; }
    .b { width: 2em; }
    .e { display: none; }
    .a::before { content: attr(data-x); }
    .c:hover { color: rgb(0, 90, 0); }
    .d:empty { padding-left: 3px; }
    .f { position: relative; width: 50px; }
    .g { position: absolute; left: 0; right: 0; top: 0; height: 5px; }
    .i { padding-top: 9px; }
    .j { padding-top: inherit; }
    .k .b { --r40: 1; }
"#;

/// One element of a [`Spec`].
#[derive(Clone, Debug)]
struct El {
    tag: &'static str,
    classes: Vec<&'static str>,
    attrs: BTreeMap<&'static str, String>,
    /// `Some(text)`: a text child first (possibly empty).
    text: Option<String>,
    children: Vec<usize>,
    parent: Option<usize>,
}

/// A document description: elements by spec id, `0` the container under
/// `<body>`, plus the interaction state.
#[derive(Clone, Debug, Default)]
struct Spec {
    els: BTreeMap<usize, El>,
    next: usize,
    hover: Option<usize>,
    focus: Option<usize>,
    active: Option<usize>,
}

impl Spec {
    fn new() -> Self {
        let mut s = Spec::default();
        s.els.insert(
            0,
            El {
                tag: "div",
                classes: vec![],
                attrs: BTreeMap::new(),
                text: None,
                children: vec![],
                parent: None,
            },
        );
        s.next = 1;
        s
    }

    fn add(
        &mut self,
        parent: usize,
        index: usize,
        tag: &'static str,
        classes: &[&'static str],
    ) -> usize {
        let id = self.next;
        self.next += 1;
        self.els.insert(
            id,
            El {
                tag,
                classes: classes.to_vec(),
                attrs: BTreeMap::new(),
                text: None,
                children: vec![],
                parent: Some(parent),
            },
        );
        let kids = &mut self.els.get_mut(&parent).unwrap().children;
        let index = index.min(kids.len());
        kids.insert(index, id);
        id
    }

    fn subtree(&self, id: usize, out: &mut Vec<usize>) {
        out.push(id);
        for &c in &self.els[&id].children {
            self.subtree(c, out);
        }
    }

    fn class_string(&self, id: usize) -> String {
        self.els[&id].classes.join(" ")
    }
}

/// A live document mirroring a spec, with the spec-id → node map.
struct Live {
    doc: RinchDocument,
    map: BTreeMap<usize, NodeId>,
    texts: BTreeMap<usize, NodeId>,
}

fn create(
    doc: &mut RinchDocument,
    spec: &Spec,
    id: usize,
    map: &mut BTreeMap<usize, NodeId>,
    texts: &mut BTreeMap<usize, NodeId>,
) -> NodeId {
    let el = &spec.els[&id];
    let n = doc.create_element(el.tag);
    if !el.classes.is_empty() {
        doc.set_attribute(n, "class", &spec.class_string(id));
    }
    for (k, v) in &el.attrs {
        doc.set_attribute(n, k, v);
    }
    map.insert(id, n);
    if let Some(t) = &el.text {
        let tn = doc.create_text(t);
        doc.append_child(n, tn);
        texts.insert(id, tn);
    }
    n
}

/// A fresh document built directly in `spec`'s final state: every element
/// created with its attributes and state, then inserted, so each is styled
/// from scratch.
fn fresh(spec: &Spec) -> RinchDocument {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let mut map = BTreeMap::new();
    let mut texts = BTreeMap::new();
    // Create every element first, then set state, then attach top-down, so
    // the state is in place before any element is first styled.
    let mut order = vec![];
    spec.subtree(0, &mut order);
    for &id in &order {
        create(&mut doc, spec, id, &mut map, &mut texts);
    }
    // Interaction state, along the would-be ancestor chains (the spec's).
    let spec_chain = |mut id: usize| {
        let mut out = vec![id];
        while let Some(p) = spec.els[&id].parent {
            out.push(p);
            id = p;
        }
        out
    };
    let html = doc.tree.html_id;
    let body = doc.tree.body_id;
    if let Some(h) = spec.hover {
        for id in spec_chain(h) {
            doc.tree.nodes[map[&id].0].is_hovered = true;
        }
        doc.tree.nodes[html].is_hovered = true;
        doc.tree.nodes[body].is_hovered = true;
        doc.tree.hovered_node = Some(map[&h].0);
    }
    if let Some(a) = spec.active {
        for id in spec_chain(a) {
            doc.tree.nodes[map[&id].0].is_active = true;
        }
        doc.tree.nodes[html].is_active = true;
        doc.tree.nodes[body].is_active = true;
        doc.tree.active_node = Some(map[&a].0);
    }
    if let Some(f) = spec.focus {
        doc.tree.nodes[map[&f].0].is_focused = true;
        doc.tree.focused_node = Some(map[&f].0);
    }
    fn attach(doc: &mut RinchDocument, spec: &Spec, id: usize, map: &BTreeMap<usize, NodeId>) {
        for &c in &spec.els[&id].children {
            doc.append_child(map[&id], map[&c]);
            attach(doc, spec, c, map);
        }
    }
    let body_n = doc.body();
    doc.append_child(body_n, map[&0]);
    attach(&mut doc, spec, 0, &map);
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
    doc
}

fn live(spec: &Spec) -> Live {
    let mut doc = RinchDocument::new();
    doc.load_css(CSS);
    let mut map = BTreeMap::new();
    let mut texts = BTreeMap::new();
    let mut order = vec![];
    spec.subtree(0, &mut order);
    for &id in &order {
        create(&mut doc, spec, id, &mut map, &mut texts);
    }
    fn attach(doc: &mut RinchDocument, spec: &Spec, id: usize, map: &BTreeMap<usize, NodeId>) {
        for &c in &spec.els[&id].children {
            doc.append_child(map[&id], map[&c]);
            attach(doc, spec, c, map);
        }
    }
    let body_n = doc.body();
    doc.append_child(body_n, map[&0]);
    attach(&mut doc, spec, 0, &map);
    doc.resolve_layout(800.0, 600.0);
    doc.resolve_layout(800.0, 600.0);
    let mut l = Live { doc, map, texts };
    // Interaction state through the real entry points.
    if let Some(h) = spec.hover {
        let mut c = false;
        let n = l.map[&h].0;
        l.doc.update_hover(Some(n), &mut c);
    }
    if let Some(f) = spec.focus {
        let n = l.map[&f].0;
        l.doc.update_focus(Some(n));
    }
    if let Some(a) = spec.active {
        let n = l.map[&a].0;
        l.doc.update_active(Some(n));
    }
    l.doc.resolve_layout(800.0, 600.0);
    l
}

/// A change to apply to both the spec and the live document.
#[derive(Clone, Debug)]
enum Op {
    ToggleClass(usize, &'static str),
    SetAttr(usize, &'static str, &'static str),
    RemoveAttr(usize, &'static str),
    SetText(usize, &'static str),
    Insert {
        parent: usize,
        index: usize,
        tag: &'static str,
        classes: Vec<&'static str>,
    },
    Remove(usize),
    Move {
        node: usize,
        parent: usize,
        index: usize,
    },
    Hover(Option<usize>),
    Focus(Option<usize>),
    Active(Option<usize>),
}

fn apply(spec: &mut Spec, l: &mut Live, op: &Op) {
    match op {
        Op::ToggleClass(id, c) => {
            let el = spec.els.get_mut(id).unwrap();
            if let Some(i) = el.classes.iter().position(|x| x == c) {
                el.classes.remove(i);
            } else {
                el.classes.push(c);
            }
            let s = spec.class_string(*id);
            if s.is_empty() {
                l.doc.remove_attribute(l.map[id], "class");
            } else {
                l.doc.set_attribute(l.map[id], "class", &s);
            }
        }
        Op::SetAttr(id, k, v) => {
            spec.els.get_mut(id).unwrap().attrs.insert(k, v.to_string());
            l.doc.set_attribute(l.map[id], k, v);
        }
        Op::RemoveAttr(id, k) => {
            spec.els.get_mut(id).unwrap().attrs.remove(k);
            l.doc.remove_attribute(l.map[id], k);
        }
        Op::SetText(id, t) => {
            let el = spec.els.get_mut(id).unwrap();
            let had = el.text.is_some();
            el.text = Some(t.to_string());
            if had {
                let tn = l.texts[id];
                l.doc.set_text_content(tn, t);
            } else {
                // A text child goes first, before any element child.
                let tn = l.doc.create_text(t);
                let n = l.map[id];
                match l.doc.tree.nodes[n.0]
                    .children
                    .iter()
                    .copied()
                    .find(|&c| !l.doc.tree.nodes[c].is_pseudo_element)
                {
                    Some(first) => l.doc.insert_before(n, tn, NodeId(first)),
                    None => l.doc.append_child(n, tn),
                }
                l.texts.insert(*id, tn);
            }
        }
        Op::Insert {
            parent,
            index,
            tag,
            classes,
        } => {
            let id = spec.add(*parent, *index, tag, classes);
            let n = create(&mut l.doc, spec, id, &mut l.map, &mut l.texts);
            insert_at(l, *parent, *index, n);
        }
        Op::Remove(id) => {
            let mut sub = vec![];
            spec.subtree(*id, &mut sub);
            for s in [&mut spec.hover, &mut spec.focus, &mut spec.active] {
                if s.is_some_and(|h| sub.contains(&h)) {
                    *s = None;
                }
            }
            // The live document keeps its (detached) hover/focus/active node;
            // clear them through the real entry points first, so both agree.
            clear_state_in(l, &sub);
            let p = spec.els[id].parent.unwrap();
            spec.els.get_mut(&p).unwrap().children.retain(|c| c != id);
            for s in &sub {
                spec.els.remove(s);
            }
            let n = l.map[id];
            l.doc.remove_node(n);
            for s in &sub {
                l.map.remove(s);
                l.texts.remove(s);
            }
        }
        Op::Move {
            node,
            parent,
            index,
        } => {
            let old = spec.els[node].parent.unwrap();
            spec.els
                .get_mut(&old)
                .unwrap()
                .children
                .retain(|c| c != node);
            spec.els.get_mut(node).unwrap().parent = Some(*parent);
            let kids = &mut spec.els.get_mut(parent).unwrap().children;
            let index = (*index).min(kids.len());
            kids.insert(index, *node);
            // The hovered / active chain moves with the node in both.
            let n = l.map[node];
            let sub = {
                let mut v = vec![];
                spec.subtree(*node, &mut v);
                v
            };
            clear_state_in(l, &sub);
            for s in [&mut spec.hover, &mut spec.focus, &mut spec.active] {
                if s.is_some_and(|h| sub.contains(&h)) {
                    *s = None;
                }
            }
            // Detach first so the index is the spec's.
            l.doc.remove_node(n);
            insert_at(l, *parent, index, n);
        }
        Op::Hover(h) => {
            spec.hover = *h;
            let mut c = false;
            let n = h.map(|h| l.map[&h].0);
            l.doc.update_hover(n, &mut c);
        }
        Op::Focus(f) => {
            spec.focus = *f;
            let n = f.map(|f| l.map[&f].0);
            l.doc.update_focus(n);
        }
        Op::Active(a) => {
            spec.active = *a;
            let n = a.map(|a| l.map[&a].0);
            l.doc.update_active(n);
        }
    }
    l.doc.resolve_layout(800.0, 600.0);
}

/// Clear, through the real entry points, any interaction state held by a node
/// in `sub` (about to be removed or moved).
fn clear_state_in(l: &mut Live, sub: &[usize]) {
    let raw: Vec<usize> = sub
        .iter()
        .filter_map(|s| l.map.get(s))
        .map(|n| n.0)
        .collect();
    if l.doc.tree.hovered_node.is_some_and(|h| raw.contains(&h)) {
        let mut c = false;
        l.doc.update_hover(None, &mut c);
    }
    if l.doc.tree.focused_node.is_some_and(|h| raw.contains(&h)) {
        l.doc.update_focus(None);
    }
    if l.doc.tree.active_node.is_some_and(|h| raw.contains(&h)) {
        l.doc.update_active(None);
    }
}

/// Insert live node `n` as element child number `index` of spec `parent`
/// (text children and generated boxes are skipped when counting).
fn insert_at(l: &mut Live, parent: usize, index: usize, n: NodeId) {
    let p = l.map[&parent];
    let element_kids: Vec<usize> = l.doc.tree.nodes[p.0]
        .children
        .iter()
        .copied()
        .filter(|&c| l.doc.tree.nodes[c].is_element() && !l.doc.tree.nodes[c].is_pseudo_element)
        .collect();
    // Generated content in this stylesheet is `::before` only, which sits
    // first, so an append after the last element child is an append.
    match element_kids.get(index) {
        Some(&before) => l.doc.insert_before(p, n, NodeId(before)),
        None => l.doc.append_child(p, n),
    }
}

#[track_caller]
fn check(spec: &Spec, l: &Live, label: &str) {
    let f = fresh(spec);
    assert_twin(&l.doc, &f, label);
}

/// The random differential compares **styles** — every longhand, custom
/// property, rinch `ComputedStyle` and generated box — and leaves the layout
/// boxes out. Random documents reach layout-invalidation defects that have
/// nothing to do with how styles are invalidated (reproduced with every
/// descendant re-cascaded, and filed separately: a font change beside an
/// anonymous block or inside a split inline, and an IFC whose last inline goes
/// `display: none`). The named scenarios above compare the boxes too.
#[track_caller]
fn check_random(spec: &Spec, l: &Live, label: &str) {
    let f = fresh(spec);
    assert_twin_opts(&l.doc, &f, label, false);
}

/// Run `ops` from `spec`, checking the twin after every one.
#[track_caller]
fn scenario(name: &str, mut spec: Spec, ops: &[Op]) {
    let mut l = live(&spec);
    check(&spec, &l, &format!("{name}: initial"));
    for (i, op) in ops.iter().enumerate() {
        apply(&mut spec, &mut l, op);
        check(&spec, &l, &format!("{name}: after op {i} {op:?}"));
    }
}

/// `container > a, b, c, d` with the given classes.
fn row(classes: &[&[&'static str]]) -> Spec {
    let mut s = Spec::new();
    for (i, c) in classes.iter().enumerate() {
        s.add(0, i, "div", c);
    }
    s
}

// ── Named scenarios ────────────────────────────────────────────────────────

/// `.a + .b` and `.a ~ .c`: a class change on an earlier sibling restyles the
/// later ones (Chrome: the `+` target flips with the class). Siblings were
/// never reached before.
#[test]
fn a_class_change_reaches_later_siblings() {
    scenario(
        "sibling class",
        row(&[&[], &["b"], &["c"], &["c"]]),
        &[
            Op::ToggleClass(1, "a"),
            Op::ToggleClass(1, "a"),
            Op::ToggleClass(2, "a"),
        ],
    );
}

/// Inserting, removing and moving elements moves `:nth-child`, `:first-child`,
/// `:last-child`, `:only-child`, `:nth-last-child` and `+` targets among the
/// siblings that stay (Chrome: `li:last-child` moves to an appended `li`).
#[test]
fn insertions_and_removals_restyle_structural_siblings() {
    scenario(
        "structure",
        row(&[&["b"], &["b"]]),
        &[
            Op::Insert {
                parent: 0,
                index: 0,
                tag: "div",
                classes: vec!["a"],
            },
            Op::Insert {
                parent: 0,
                index: 9,
                tag: "div",
                classes: vec!["b"],
            },
            Op::Insert {
                parent: 0,
                index: 2,
                tag: "span",
                classes: vec![],
            },
            Op::Remove(1),
            Op::Move {
                node: 2,
                parent: 0,
                index: 0,
            },
            Op::Remove(2),
        ],
    );
}

/// `:empty` flips with a text child's text and with an element child
/// (Chrome: an empty text node keeps an element `:empty`).
#[test]
fn empty_follows_text_and_children() {
    scenario(
        "empty",
        row(&[&["d"], &["d"]]),
        &[
            Op::SetText(1, ""),
            Op::SetText(1, "t"),
            Op::SetText(1, ""),
            Op::Insert {
                parent: 2,
                index: 0,
                tag: "span",
                classes: vec![],
            },
            Op::Remove(3),
        ],
    );
}

/// `.a:hover .c`: hover on an ancestor restyles a matching descendant, and
/// `.b:hover` only the element itself; `:hover + .d` a sibling.
#[test]
fn hover_reaches_exactly_what_depends_on_it() {
    let mut s = row(&[&["a"], &["d"]]);
    let c = s.add(1, 0, "div", &["c"]);
    s.add(c, 0, "span", &["b"]);
    scenario(
        "hover",
        s,
        &[
            Op::Hover(Some(1)),
            Op::Hover(Some(4)),
            Op::Hover(Some(2)),
            Op::Hover(None),
            // Re-cascading the `.a` element alone (a class that changes only
            // reset properties, so nothing under it is re-matched) must not
            // lose the `:hover` sensitivity its descendant's matching recorded
            // on it.
            Op::ToggleClass(1, "f"),
            Op::Hover(Some(1)),
            Op::Hover(None),
        ],
    );
}

/// `input:checked + .b`, `input:disabled ~ .c`, `:disabled`, and a disabled
/// `<fieldset>` disabling the controls inside it.
#[test]
fn checked_and_disabled_reach_siblings_and_descendants() {
    let mut s = Spec::new();
    let inp = s.add(0, 0, "input", &[]);
    s.add(0, 1, "div", &["b"]);
    s.add(0, 2, "div", &["c"]);
    let fs = s.add(0, 3, "fieldset", &[]);
    s.add(fs, 0, "input", &[]);
    scenario(
        "checked/disabled",
        s,
        &[
            Op::SetAttr(inp, "checked", ""),
            Op::SetAttr(inp, "disabled", ""),
            Op::RemoveAttr(inp, "checked"),
            Op::SetAttr(fs, "disabled", ""),
            Op::RemoveAttr(fs, "disabled"),
        ],
    );
}

/// Attribute selectors: presence, `=`, `^=`, the `i` flag, an ancestor
/// attribute, `attr()` in generated content, and ids on either side.
#[test]
fn attribute_and_id_selectors() {
    let mut s = row(&[&["a"], &[]]);
    s.add(1, 0, "div", &["a"]);
    s.add(1, 1, "div", &["b"]);
    scenario(
        "attributes",
        s,
        &[
            Op::SetAttr(1, "data-x", "1"),
            Op::SetAttr(1, "data-x", "2"),
            Op::SetAttr(1, "data-x", "Y"),
            Op::RemoveAttr(1, "data-x"),
            Op::SetAttr(2, "id", "i1"),
            Op::SetAttr(4, "id", "i2"),
            Op::RemoveAttr(2, "id"),
            // An inline style is the element's own declarations: no selector
            // names it, and its element restyles (and, since `color`
            // inherits, its children).
            Op::SetAttr(1, "style", "color: rgb(9, 9, 9)"),
            Op::SetAttr(1, "style", "color: rgb(9, 9, 8); padding-left: 2px"),
            Op::RemoveAttr(1, "style"),
        ],
    );
}

/// An inherited property set on a container reaches its children through the
/// cascade even though no selector names them (`child_cascade`), and a
/// `display: none` toggle on an ancestor keeps the hidden subtree current.
#[test]
fn inheritance_follows_a_restyled_ancestor() {
    let mut s = row(&[&[], &[]]);
    let x = s.add(1, 0, "div", &["b"]);
    s.add(x, 0, "span", &["a"]);
    scenario(
        "inheritance",
        s,
        &[
            Op::ToggleClass(1, "d"),
            Op::ToggleClass(1, "c"),
            Op::ToggleClass(1, "e"),
            Op::ToggleClass(x, "a"),
            Op::ToggleClass(1, "e"),
            // A reset-only change on the parent reaches a child only through
            // an explicit `inherit` (`INHERITS_RESET_STYLE`).
            Op::ToggleClass(x, "j"),
            Op::ToggleClass(1, "i"),
            Op::ToggleClass(1, "i"),
        ],
    );
}

/// A class change whose descendant rule reaches **through** a `display: none`
/// element: rinch styles hidden subtrees, so the invalidation has to walk into
/// one where Stylo's own processor would stop (`EveryDescendant`).
#[test]
fn invalidation_walks_into_a_hidden_subtree() {
    let mut s = row(&[&[]]);
    let w = s.add(1, 0, "div", &["e"]);
    s.add(w, 0, "div", &["b"]);
    scenario(
        "hidden subtree",
        s,
        &[
            // `.k` matches nothing on its own, so the only way to `.b` is the
            // descendant walk — not a re-cascade inherited from above.
            Op::ToggleClass(1, "k"),
            Op::ToggleClass(1, "k"),
            Op::ToggleClass(1, "k"),
            Op::ToggleClass(w, "e"),
        ],
    );
}

/// `[attr=v i]` matches case-insensitively, as in every browser (Selectors 4
/// §6.3; Chrome: `[data-x="y" i]` matches `data-x="Y"`). The twin comparison
/// cannot see this one — both documents would share a wrong matcher — so it is
/// asserted directly.
#[test]
fn the_attribute_case_flag_is_honoured() {
    let mut s = row(&[&[]]);
    s.els
        .get_mut(&1)
        .unwrap()
        .attrs
        .insert("data-x", "Y".into());
    let doc = fresh(&s);
    let n = doc.tree.nodes[doc.tree.body_id].children[0];
    let el = doc.tree.nodes[n].children[0];
    let data = doc.tree.nodes[el].stylo_element_data.borrow();
    let cv = data.as_ref().unwrap().styles.primary.clone().unwrap();
    let fp = style_twin::stylo_fingerprint(&cv);
    assert!(
        fp.contains("--r18=1"),
        "[data-x=\"y\" i] must match data-x=\"Y\":\n{fp}"
    );
}

/// `:focus`, `:active`, `.a:active .b`.
#[test]
fn focus_and_active() {
    let mut s = row(&[&["c"], &["a"]]);
    s.add(2, 0, "span", &["b"]);
    scenario(
        "focus/active",
        s,
        &[
            Op::Focus(Some(1)),
            Op::Active(Some(3)),
            Op::Focus(Some(3)),
            Op::Active(None),
            Op::Focus(None),
        ],
    );
}

/// `:nth-child(2n+1 of .b)` and `:nth-of-type` / `:last-of-type`: a class or
/// tag change on one child moves its siblings' indices.
#[test]
fn nth_of_and_of_type() {
    let mut s = Spec::new();
    for i in 0..5 {
        s.add(0, i, if i % 2 == 0 { "span" } else { "li" }, &["b"]);
    }
    scenario(
        "nth-of",
        s,
        &[
            Op::ToggleClass(1, "b"),
            Op::ToggleClass(3, "b"),
            Op::Insert {
                parent: 0,
                index: 1,
                tag: "span",
                classes: vec!["b"],
            },
            Op::Remove(5),
            Op::Insert {
                parent: 0,
                index: 9,
                tag: "li",
                classes: vec![],
            },
        ],
    );
}

// ── Random differential ────────────────────────────────────────────────────

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

const CLASSES: &[&str] = &["a", "b", "c", "d", "e"];
const TAGS: &[&str] = &["div", "span", "li", "input", "fieldset", "p"];

/// Whether an element of `tag` may be put under a `parent_tag` element: no
/// children for an `<input>`, and no blocks inside a `<span>`.
fn may_nest(parent_tag: &str, tag: &str) -> bool {
    parent_tag != "input" && (parent_tag != "span" || matches!(tag, "span" | "input"))
}

fn contains_block(s: &Spec, id: usize) -> bool {
    let mut sub = vec![];
    s.subtree(id, &mut sub);
    sub.iter()
        .any(|i| !matches!(s.els[i].tag, "span" | "input"))
}

fn random_spec(rng: &mut Rng) -> Spec {
    let mut s = Spec::new();
    let n = 4 + rng.below(8);
    for _ in 0..n {
        let tag = rng.pick(TAGS);
        let parents: Vec<usize> = s
            .els
            .keys()
            .copied()
            .filter(|&p| may_nest(s.els[&p].tag, tag))
            .collect();
        let p = rng.pick(&parents);
        let idx = rng.below(4);
        let mut classes = vec![];
        for &c in CLASSES {
            if c != "e" && rng.below(3) == 0 {
                classes.push(c);
            }
        }
        let id = s.add(p, idx, tag, &classes);
        if rng.below(4) == 0 {
            s.els.get_mut(&id).unwrap().text = Some(if rng.below(2) == 0 {
                String::new()
            } else {
                "t".into()
            });
        }
    }
    s
}

fn random_op(rng: &mut Rng, s: &Spec) -> Op {
    let ids: Vec<usize> = s.els.keys().copied().filter(|&i| i != 0).collect();
    if ids.len() < 2 {
        return Op::Insert {
            parent: 0,
            index: rng.below(4),
            tag: rng.pick(TAGS),
            classes: vec![rng.pick(CLASSES)],
        };
    }
    let any = |rng: &mut Rng| rng.pick(&ids);
    match rng.below(12) {
        0 | 1 => Op::ToggleClass(any(rng), rng.pick(CLASSES)),
        2 => Op::SetAttr(any(rng), "data-x", rng.pick(&["1", "2", "y", "Y", "12"])),
        3 => Op::RemoveAttr(any(rng), rng.pick(&["data-x", "id", "checked", "disabled"])),
        4 => Op::SetAttr(any(rng), "id", rng.pick(&["i1", "i2", "i3"])),
        5 => Op::SetAttr(any(rng), rng.pick(&["checked", "disabled"]), ""),
        6 => {
            let tag = rng.pick(TAGS);
            let parents: Vec<usize> = s
                .els
                .keys()
                .copied()
                .filter(|&p| may_nest(s.els[&p].tag, tag))
                .collect();
            Op::Insert {
                parent: rng.pick(&parents),
                index: rng.below(4),
                tag,
                classes: vec![rng.pick(CLASSES)],
            }
        }
        7 if ids.len() > 2 => Op::Remove(any(rng)),
        8 if ids.len() > 2 => {
            let node = any(rng);
            let mut sub = vec![];
            s.subtree(node, &mut sub);
            let parents: Vec<usize> = s
                .els
                .keys()
                .copied()
                .filter(|p| {
                    !sub.contains(p)
                        && s.els[p].tag != "input"
                        && (s.els[p].tag != "span" || !contains_block(s, node))
                })
                .collect();
            Op::Move {
                node,
                parent: rng.pick(&parents),
                index: rng.below(4),
            }
        }
        9 => Op::Hover(if rng.below(4) == 0 {
            None
        } else {
            Some(any(rng))
        }),
        10 => {
            if rng.below(2) == 0 {
                Op::Focus(if rng.below(3) == 0 {
                    None
                } else {
                    Some(any(rng))
                })
            } else {
                Op::Active(if rng.below(3) == 0 {
                    None
                } else {
                    Some(any(rng))
                })
            }
        }
        _ => Op::SetText(any(rng), rng.pick(&["", "t"])),
    }
}

/// Seeded random documents and mutation sequences; every step compared with a
/// fresh document. Deterministic: the seeds are fixed, and a failure names the
/// seed, the step and the op.
#[test]
fn random_mutation_sequences_match_a_fresh_document() {
    let seeds: u64 = std::env::var("RINCH_TWIN_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let first: u64 = std::env::var("RINCH_TWIN_FIRST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    for seed in first..=seeds {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0x1000_0001));
        let mut spec = random_spec(&mut rng);
        let mut l = live(&spec);
        check_random(&spec, &l, &format!("seed {seed}: initial"));
        for step in 0..10 {
            let op = random_op(&mut rng, &spec);
            if std::env::var_os("RINCH_TWIN_DEBUG").is_some() {
                eprintln!("seed {seed} step {step}: {op:?}\n  spec before: {spec:?}");
            }
            apply(&mut spec, &mut l, &op);
            check_random(&spec, &l, &format!("seed {seed} step {step}: {op:?}"));
        }
    }
}

/// An absolutely positioned box whose containing block is the initial one has
/// the viewport baked into its Taffy style; an ancestor that starts or stops
/// being a containing block (`position: relative` by class) moves it to its
/// parent's box and back, though nothing about the absolute box's own style
/// changed.
#[test]
fn an_ancestor_becoming_a_containing_block_resizes_an_absolute() {
    let mut s = Spec::new();
    let w = s.add(0, 0, "div", &[]);
    s.add(w, 0, "div", &["g"]);
    scenario(
        "containing block",
        s,
        &[
            Op::ToggleClass(w, "f"),
            Op::ToggleClass(w, "f"),
            Op::ToggleClass(0, "f"),
        ],
    );
}
