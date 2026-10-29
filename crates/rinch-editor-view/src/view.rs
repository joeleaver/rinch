//! [`RinchDomEditorView`] and its [`ViewDesc`] tree — the desktop projection of
//! `EditorState` onto rinch-dom (design §6).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use crate::handle::ScrollAlign;
use crate::node_views::{Kept, Registry, Slot, Work};
use rinch_core::dom::{CaretAffinity, DomDocument, GeneratedFlatRange, NodeFont, NodeHandle};
use rinch_editor_core::decoration::DecorationSet;
use rinch_editor_core::serialize::{mark_dom_tag, node_dom_tag};
use rinch_editor_core::transform::Mapping;
use rinch_editor_core::{EditorState, EditorView, Mark, Node, Pos, Selection, ViewRequest};

/// A weak handle to the host document. The view upgrades + borrows it briefly to
/// create/patch nodes; holding only a `Weak` lets the document drop normally.
type DocRef = Weak<RefCell<dyn DomDocument>>;

/// Create a host element and wrap it in a [`NodeHandle`]. `None` if the document
/// has been dropped.
fn create_element(doc: &DocRef, tag: &str) -> Option<NodeHandle> {
    let d = doc.upgrade()?;
    let id = d.borrow_mut().create_element(tag);
    Some(NodeHandle::new(id, doc.clone()))
}

/// Create a host text node and wrap it in a [`NodeHandle`].
fn create_text(doc: &DocRef, text: &str) -> Option<NodeHandle> {
    let d = doc.upgrade()?;
    let id = d.borrow_mut().create_text(text);
    Some(NodeHandle::new(id, doc.clone()))
}

/// Set the host element's attributes from `node`: a `data-pm-type` marker (its
/// schema type, a debugging/MCP/a11y aid) plus the node-specific attributes the
/// tag itself does not encode. Idempotent — clears attributes that no longer
/// apply, so an in-place update can't leave a stale `start`/`src`.
///
/// A node whose spec keeps an app's `data-*` attributes (`NodeSpec::data_attrs`)
/// gets each of them as itself; `old` is the node the host showed before an
/// in-place update, whose data attributes the new one no longer has are
/// removed. Only an app data attribute the node keeps is ever written or
/// removed ([`rinch_editor_core::app_data_attrs`]: unreserved, bounded in
/// number and size), so one can never overwrite or take away a name rinch
/// reserves (`data-pm-type`, `data-rid`, …).
fn apply_element_attrs(dom: &NodeHandle, node: &Node, old: Option<&Node>) {
    apply_marker_attrs(dom, node, old);
    apply_type_attrs(dom, node);
}

/// The attributes every host of `node` carries, its own element or the `<span>`
/// a node view of an inline leaf is mounted in ([`apply_view_host_attrs`]): the
/// `data-pm-type` marker and the app's `data-*` attributes the node keeps.
fn apply_marker_attrs(dom: &NodeHandle, node: &Node, old: Option<&Node>) {
    dom.set_attribute("data-pm-type", node.type_name());
    let kept: Vec<(&str, &str)> = if node.node_type().spec().data_attrs {
        rinch_editor_core::app_data_attrs(node.attrs()).collect()
    } else {
        Vec::new()
    };
    if let Some(old) = old {
        for (name, _) in rinch_editor_core::app_data_attrs(old.attrs()) {
            if !kept.iter().any(|(k, _)| *k == name) {
                dom.remove_attribute(name);
            }
        }
    }
    for (name, value) in kept {
        dom.set_attribute(name, value);
    }
}

/// The `<span>` a node view of an inline leaf (a claimed `image`) is mounted in:
/// the marker and the app's data attributes, none of the picture's own (`src`,
/// `alt`, `width` belong to an `<img>`), and `display: inline-block`, so the
/// span is one atomic inline on its line, sized by the app's component.
fn apply_view_host_attrs(dom: &NodeHandle, node: &Node, old: Option<&Node>) {
    apply_marker_attrs(dom, node, old);
    dom.set_style("display", "inline-block");
}

/// The attributes `node`'s type writes on its own element (an `<img>`'s source, a
/// list's `start`, a table's grid, …).
fn apply_type_attrs(dom: &NodeHandle, node: &Node) {
    match node.type_name() {
        "image" => {
            // The source the picture is loaded from: the node's `src`, unless
            // the app chose another from its attrs (`set_image_source`).
            dom.set_attribute("src", &crate::images::image_source(node.attrs()));
            match node.attrs().get_str("alt") {
                Some(alt) if !alt.is_empty() => dom.set_attribute("alt", alt),
                _ => dom.remove_attribute("alt"),
            }
            // A presentational hint, as HTML's own: an author `width` rule
            // (or `max-width`) still wins, and the height follows the
            // picture's aspect ratio.
            match node.attrs().get_int("width") {
                Some(width) if width > 0 => dom.set_attribute(
                    "width",
                    &width.min(rinch_editor_core::IMAGE_MAX_WIDTH).to_string(),
                ),
                _ => dom.remove_attribute("width"),
            }
        }
        "ordered_list" => match node.attrs().get_int("start") {
            Some(start) if start != 1 => dom.set_attribute("start", &start.to_string()),
            _ => dom.remove_attribute("start"),
        },
        // A table is laid out as a CSS grid (the default stylesheet sets
        // `display: grid` on `<table>` and `display: contents` on `<tr>`, so the
        // cells are the grid's items). The column count is data-dependent, so the
        // view writes it inline. A cell's span is not written here: it is its
        // rectangle in the table's map, which the table writes for every cell
        // (`ViewDesc::sync_table_spans`, #1182).
        "table" => {
            let cols = rinch_editor_core::tables::column_count(node).max(1);
            dom.set_style(
                "grid-template-columns",
                &format!("repeat({cols}, minmax(0, 1fr))"),
            );
        }
        // A task item carries its checkbox state out as `data-checked` so the default
        // stylesheet can render a ticked vs empty box (task_item has no native HTML tag,
        // so the box is a `::before` keyed off this attribute). Always reset when false.
        "task_item" => match node.attrs().get_bool("checked") {
            Some(true) => dom.set_attribute("data-checked", "true"),
            _ => dom.remove_attribute("data-checked"),
        },
        // The indent level (set by the indent / outdent commands) renders as a left
        // margin — 2em per level. Always written (resetting to 0) so outdent clears
        // the previous value, like the table spans above.
        "paragraph" | "heading" => {
            let indent = node.attrs().get_int("indent").unwrap_or(0).max(0);
            if indent > 0 {
                dom.set_style("margin-left", &format!("{}em", indent * 2));
            } else {
                dom.set_style("margin-left", "0");
            }
            // Horizontal alignment. Always written (resetting to "left") so switching
            // back to the default clears a previously applied alignment, like the
            // margin above. Only the three non-default values are honoured.
            match node.attrs().get_str("text_align") {
                Some(a @ ("center" | "right" | "justify")) => dom.set_style("text-align", a),
                _ => dom.set_style("text-align", "left"),
            }
        }
        _ => {}
    }
}

/// Set a mark wrapper element's attributes: a `data-pm-mark` marker plus the
/// attr-bearing specials (`link` → `href`/`title`/`target`+`rel`, `text_color` →
/// inline `color`, `highlight` → inline `background-color`). Mirrors the
/// schema-driven copy-out HTML in `serialize::html`.
fn apply_mark_attrs(dom: &NodeHandle, mark: &Mark) {
    dom.set_attribute("data-pm-mark", mark.type_name());
    match mark.type_name() {
        "link" => {
            dom.set_attribute("href", mark.attrs.get_str("href").unwrap_or(""));
            if let Some(title) = mark.attrs.get_str("title").filter(|t| !t.is_empty()) {
                dom.set_attribute("title", title);
            }
            if let Some(target) = mark.attrs.get_str("target").filter(|t| !t.is_empty()) {
                dom.set_attribute("target", target);
                if target != "_self" {
                    // Guard against reverse-tabnabbing, as copy-out HTML does.
                    dom.set_attribute("rel", "noopener noreferrer");
                }
            }
        }
        "text_color" => {
            if let Some(color) = mark.attrs.get_str("color").filter(|c| !c.is_empty()) {
                dom.set_style("color", color);
            }
        }
        "highlight" => {
            if let Some(color) = mark.attrs.get_str("color").filter(|c| !c.is_empty()) {
                dom.set_style("background-color", color);
            }
        }
        _ => {}
    }
}

/// Put `want` in `host`, in that order, moving each node only when it is not
/// already where it goes. Nodes of `host` that `want` does not name (one
/// another container takes; a wrapper about to be discarded; a box the host
/// generated itself) are stepped over, not touched.
fn place_children(host: &NodeHandle, want: &[NodeHandle]) {
    let actual = host.children();
    let wanted: HashSet<usize> = want.iter().map(|n| n.node_id().0).collect();
    let mut placed: HashSet<usize> = HashSet::with_capacity(want.len());
    let mut i = 0usize;
    for node in want {
        while actual
            .get(i)
            .is_some_and(|a| !wanted.contains(&a.node_id().0) || placed.contains(&a.node_id().0))
        {
            i += 1;
        }
        let id = node.node_id().0;
        placed.insert(id);
        match actual.get(i) {
            Some(a) if a.node_id().0 == id => i += 1,
            Some(a) => host.insert_before(node, a),
            None => host.append_child(node),
        }
    }
}

/// One [`Decoration::Inline`] flattened for the view: an absolute model range in
/// char positions plus the CSS class list to put on the host element(s) wrapping
/// it.
///
/// [`Decoration::Inline`]: rinch_editor_core::decoration::Decoration::Inline
#[derive(Clone, Debug, PartialEq)]
struct InlineDeco {
    from: usize,
    to: usize,
    class: Rc<str>,
}

/// A decorated stretch of **one** text run: local char offsets into the run's
/// text and the (possibly merged) class list for it. This is the view's
/// change-detection key for a run — [`ViewDesc::decos`] holds what is currently
/// projected, and a run whose recomputed list compares equal is not touched.
#[derive(Clone, Debug, PartialEq)]
struct RunDeco {
    start: usize,
    end: usize,
    class: Rc<str>,
}

/// The inline decorations of `set`, in draw order. Widgets (and inline
/// decorations with nothing to draw — an empty range or no `class`) drop out:
/// `Decoration::inline_class` is the core's own filter, so the view never
/// matches the enum.
fn inline_decos(set: &DecorationSet) -> Vec<InlineDeco> {
    set.iter()
        .filter_map(|d| {
            let class: Rc<str> = Rc::from(d.inline_class()?);
            let (from, to) = d.range();
            Some(InlineDeco {
                from: from.0,
                to: to.0,
                class,
            })
        })
        .collect()
}

/// Clip `decos` to the text run occupying model chars `[run_from, run_from + len)`
/// and flatten them into non-overlapping, ordered stretches in run-local char
/// offsets.
///
/// Two decorations that overlap (a misspelling inside a search hit) produce one
/// stretch per distinct sub-range carrying **both** classes, so a stretch is never
/// wrapped twice; abutting stretches with an identical class list are merged back
/// into one, so the projected DOM is the minimum that expresses the decorations.
fn run_decos<'a>(
    decos: impl IntoIterator<Item = &'a InlineDeco>,
    run_from: usize,
    len: usize,
) -> Vec<RunDeco> {
    let run_to = run_from + len;
    let mut clipped = decos.into_iter().filter_map(|d| {
        let a = d.from.max(run_from);
        let b = d.to.min(run_to);
        (a < b).then(|| (a - run_from, b - run_from, &d.class))
    });
    // One decoration over the run — a spellchecker's every squiggle — is its
    // own stretch and shares the decoration's class string (#848: this runs for
    // every decorated run on every transaction).
    let Some(first) = clipped.next() else {
        return Vec::new();
    };
    let Some(second) = clipped.next() else {
        return vec![RunDeco {
            start: first.0,
            end: first.1,
            class: first.2.clone(),
        }];
    };
    let clipped: Vec<(usize, usize, &Rc<str>)> =
        [first, second].into_iter().chain(clipped).collect();
    let mut bounds: Vec<usize> = clipped.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<RunDeco> = Vec::new();
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut classes: Vec<&Rc<str>> = clipped
            .iter()
            .filter(|(x, y, _)| *x <= a && *y >= b)
            .map(|(_, _, c)| *c)
            .collect();
        if classes.is_empty() {
            continue;
        }
        classes.dedup();
        let class: Rc<str> = match classes[..] {
            [one] => one.clone(),
            _ => Rc::from(
                classes
                    .iter()
                    .map(|c| &***c)
                    .collect::<Vec<&str>>()
                    .join(" "),
            ),
        };
        match out.last_mut() {
            Some(last) if last.end == a && last.class == class => last.end = b,
            _ => out.push(RunDeco {
                start: a,
                end: b,
                class,
            }),
        }
    }
    out
}

/// The UTF-8 byte offset of char index `i` in `text` (its length past the end).
fn byte_of_char(text: &str, i: usize) -> usize {
    text.char_indices().nth(i).map_or(text.len(), |(b, _)| b)
}

/// Build the host node a text run projects to, given the decorations over it —
/// the inner node a mark chain then wraps.
///
/// With no decorations that is a bare host **text node**, exactly as before
/// decorations existed: an undecorated document projects to byte-identical DOM.
/// With decorations it is a `<span data-pm-deco-run>` holding the run split into
/// plain text chunks and `<span data-pm-deco class="…">` segments.
///
/// **Why split the run rather than overlay it** (the alternative: absolutely
/// positioned underline divs measured from `query_selection_rects`, like the
/// selection wash). A wrapper span is what *both* backends already know how to
/// style — the web puts the consumer's CSS class straight on a real element, and
/// the native side gets the same cascade through rinch-dom — so one `class` attr
/// covers both with no per-backend painting code in the view. It also survives
/// reflow for free: an overlay would have to be re-measured after every layout,
/// on every line, and re-laid out and repainted whenever it moved (see
/// [`RinchDomEditorView::overlay_dirty`]). And it is invisible to the rest of the
/// view: an inline element contributes nothing to the inline formatting context's
/// flat text, so the caret map (`textblock_flat_byte`/`ifc_byte_to_char`, both
/// computed from the **model**) and both backends' hit-tests — which walk every
/// text node under the textblock — are unchanged by the extra nesting.
///
/// Returns the inner node and the decoration segments inside it (kept so a
/// pointer hit on one still resolves to the run: see [`find_node_by_host`]).
fn build_run_host(
    text: &str,
    decos: &[RunDeco],
    doc: &DocRef,
) -> Option<(NodeHandle, Vec<NodeHandle>)> {
    if decos.is_empty() {
        return Some((create_text(doc, text)?, Vec::new()));
    }
    let wrapper = create_element(doc, "span")?;
    wrapper.set_attribute("data-pm-deco-run", "true");
    let mut segments = Vec::with_capacity(decos.len());
    let mut at = 0usize;
    let chunk = |from: usize, to: usize| &text[byte_of_char(text, from)..byte_of_char(text, to)];
    for d in decos {
        if d.start > at
            && let Some(t) = create_text(doc, chunk(at, d.start))
        {
            wrapper.append_child(&t);
        }
        if let Some(span) = create_element(doc, "span") {
            span.set_attribute("data-pm-deco", "true");
            span.set_attribute("class", &d.class);
            if let Some(t) = create_text(doc, chunk(d.start, d.end)) {
                span.append_child(&t);
            }
            wrapper.append_child(&span);
            segments.push(span);
        }
        at = d.end;
    }
    let total = text.chars().count();
    if at < total
        && let Some(t) = create_text(doc, chunk(at, total))
    {
        wrapper.append_child(&t);
    }
    Some((wrapper, segments))
}

/// A descriptor mirroring one model [`Node`], owning the host node that projects
/// it. The conceptual successor to the old `BlockMap`, but a proper persistent
/// tree: because the model shares `Rc`s, [`Node::same_ref`] lets the diff skip an
/// unchanged subtree wholesale (design A12).
///
/// **Inline marks (M5.5a, #1833):** a marked inline run (`text("ab", [bold,
/// italic])`) sits in a *chain* of mark wrappers — `<em><strong>#text</strong></em>`.
/// [`Self::dom`] is the inner text/leaf node (caret geometry, the A15 position
/// map); the wrappers belong to the textblock, which places its inline children
/// in them ([`Self::regroup_inline`]) and shares a wrapper between adjacent runs
/// whose mark continues, so inline code split by bold is one `<code>`. A run's
/// chain is [`Self::wraps`].
pub(crate) struct ViewDesc {
    /// The model node this descriptor currently projects.
    node: Node,
    /// The inner host node projecting `node` itself — a host text node for a text
    /// run, or the element for a block/leaf. Caret and the A15 map address this.
    dom: NodeHandle,
    /// The mark wrappers this inline node sits in, outermost first, each with
    /// the mark it draws (#1833). Shared with the neighbours a mark continues
    /// into, so adjacent runs' wrappers can be the same elements. Written only
    /// by the parent's [`Self::regroup_inline`]; empty for a block.
    wraps: Vec<(Mark, NodeHandle)>,
    /// For a textblock: every mark wrapper element its inline children are
    /// placed in, as the last [`Self::regroup_inline`] left them — so a wrapper
    /// no child uses any more is found and discarded.
    inline_wrappers: Vec<NodeHandle>,
    /// For a textblock mid-diff: the inline children the diff dropped, retired
    /// by [`Self::regroup_inline`] once their successors are in place, so the
    /// block is never left empty in between (an `:empty` rule would restyle
    /// it) and a wrapper they sat in is not discarded with them inside.
    deferred: Vec<ViewDesc>,
    /// Child descriptors, positionally 1:1 with `node`'s children.
    children: Vec<ViewDesc>,
    /// Whether `node` is a model **text** node. For a decorated run [`Self::dom`]
    /// is then the `<span data-pm-deco-run>` standing in for the text node, not a
    /// host text node — see [`Self::decos`].
    is_text: bool,
    /// The inline decorations currently projected over this text run, in run-local
    /// char offsets (always empty for an element). The decoration pass's
    /// change-detection key: a run whose recomputed list compares equal is left
    /// alone, so a transaction that changes only the *document* never rewrites a
    /// squiggle's DOM, and one that changes only decorations never touches the
    /// document's.
    decos: Vec<RunDeco>,
    /// The `<span data-pm-deco>` segments inside a decorated run, so a host node
    /// lookup that lands on one resolves to this run ([`find_node_by_host`]).
    segments: Vec<NodeHandle>,
    /// Whether this subtree currently projects **any** decoration segment. Lets
    /// the decoration pass skip a subtree that neither holds one nor overlaps a
    /// decoration, so a per-keystroke pass costs one range test per top-level
    /// block rather than a walk of the whole document.
    has_deco: bool,
    /// The trailing-break placeholder — a `<br>` after a textblock's last
    /// child when that child is a hard break or text ending in a newline —
    /// while shown
    /// ([`Self::sync_trailing_break`]). Always the element's last child.
    trailing_break: Option<NodeHandle>,
    /// A table cell's grid placement as last written to its host, or `None`
    /// before its table has placed it (a freshly built cell, and every node
    /// that is not a cell). Written only by the table ([`Self::sync_table_spans`]),
    /// because a cell's placement is its [`TableMap`] rectangle, which the whole
    /// table decides (#1182).
    ///
    /// [`TableMap`]: rinch_editor_core::tables::TableMap
    grid_placement: Option<GridPlacement>,
    /// The node-view slot of a leaf node a registered
    /// [`NodeViewSpec`](crate::NodeViewSpec) may draw: the app's component is
    /// mounted inside [`Self::dom`] (see [`crate::node_views`]). `None` for every
    /// other node.
    slot: Option<Rc<Slot>>,
    /// Whether this subtree holds any [`Self::slot`]. A subtree that does is
    /// taken out of the host by [`retire`] (detached, its views disposed, then
    /// discarded) rather than discarded on the spot, and a changed stretch that
    /// holds one is diffed by node identity ([`ViewDesc::diff_children`]).
    has_view: bool,
}

/// What the descriptor diff needs beside the host document: the editor's node
/// views and where to queue their work, and which nodes a `SetNodeAttrStep`
/// proved to be the same node with other attributes.
pub(crate) struct Cx<'a> {
    doc: &'a DocRef,
    /// The editor's registered node views, while any is registered.
    views: Option<&'a Registry>,
    /// The nodes the transaction only changed the attributes of
    /// ([`crate::handle::kept_nodes`]) — the only nodes a node view keeps its
    /// mount through when they are not the same `Rc` (decision (d): never hand a
    /// view another node).
    kept: &'a [Kept],
    /// A **remote** transaction's mapping ([`RinchDomEditorView::set_remote_mapping`]):
    /// which old node survives where. Set only for a peer's change, whose steps the
    /// collaboration adapter built from the CRDT's own block identities, so a node
    /// no step touched is the same CRDT block. Read only where the model cannot
    /// tell two nodes apart: an `Rc` that stands more than once among one parent's
    /// children (an in-editor copy pasted beside its original), where the first
    /// match by `same_ref` could hand one copy's view the other's place (the
    /// round-3 review of #1595, F3).
    mapping: Option<&'a Mapping>,
    work: &'a mut Vec<Work>,
    slots: &'a mut Vec<Rc<Slot>>,
}

impl Cx<'_> {
    /// Whether the old child at `old_pos` may be taken as the new child at `new_pos`
    /// when both are one `Rc` that stands more than once among the parent's old or
    /// new children (`ambiguous`), for a descriptor holding a view: only when the
    /// remote mapping carries it there untouched. Without a remote mapping, a local
    /// edit keeps the first match by `same_ref`, which is the pairing the collaboration
    /// projection makes for the same edit (its identity runs).
    fn same_place(&self, ambiguous: bool, old_pos: usize, old_end: usize, new_pos: usize) -> bool {
        if !ambiguous {
            return true;
        }
        // REVIEW4: both ends, and the end checked for deletion (an insertion right
        // before the node sets `deleted_after` at its start).
        self.mapping.is_none_or(|m| {
            let s = m.map_result(old_pos, 1);
            let e = m.map_result(old_end, -1);
            s.pos == new_pos && e.pos == new_pos + (old_end - old_pos) && !e.deleted_before()
        })
    }
    /// Whether a registered spec draws `node`'s type: its descriptor needs a slot.
    fn claims(&self, node: &Node) -> bool {
        self.views.is_some_and(|views| {
            !node.is_text() && node.is_leaf() && views.claims(node.type_name())
        })
    }

    /// A slot for `node`'s host `dom`, when a registered spec draws its type.
    fn slot_for(&mut self, node: &Node, dom: &NodeHandle) -> Option<Rc<Slot>> {
        if !self.claims(node) {
            return None;
        }
        let slot = Slot::new(dom.clone(), self.doc.clone(), node.clone());
        self.work.push(Work::Sync(slot.clone()));
        self.slots.push(slot.clone());
        Some(slot)
    }
}

/// A node's identity as a number: the address of its shared allocation (its attrs
/// live in it), equal for two handles exactly when they are `same_ref`, while both
/// are alive.
fn node_key(node: &Node) -> usize {
    std::ptr::from_ref(node.attrs()) as usize
}

/// The nodes ([`node_key`]) that stand more than once among `old` (a parent's
/// descriptors) or among `new`'s children: one `Rc` in two places (an in-editor
/// copy pasted beside its original), which `same_ref` cannot tell apart. One pass
/// over each list per child-list diff, so a remote change costs O(children), not
/// O(views × children) (the round-4 review of #1595, G2).
fn twice_keys(old: &[ViewDesc], new: &Node) -> HashSet<usize> {
    let mut twice = HashSet::new();
    let mut seen = HashSet::new();
    for n in old.iter().map(|o| &o.node) {
        if !seen.insert(node_key(n)) {
            twice.insert(node_key(n));
        }
    }
    seen.clear();
    for j in 0..new.child_count() {
        let k = node_key(new.child(j));
        if !seen.insert(k) {
            twice.insert(k);
        }
    }
    twice
}

/// Take `desc`'s host out of the document for good, replacing it with
/// `replacement` when given. A subtree without node views is discarded at once
/// (issue #719); one with views is detached and its views marked removed, and the
/// discard waits for [`crate::node_views::run`] to dispose them first — their
/// cleanups run with no editor borrow held and still see their own nodes.
fn retire(desc: ViewDesc, cx: &mut Cx, replacement: Option<&NodeHandle>) {
    if let Some(r) = replacement {
        desc.dom.replace_with(r);
    }
    if !desc.has_view {
        desc.dom.discard();
        return;
    }
    fn mark(desc: &ViewDesc, work: &mut Vec<Work>) {
        if let Some(slot) = &desc.slot {
            slot.mark_removed();
            work.push(Work::Sync(slot.clone()));
        }
        for c in &desc.children {
            if c.has_view {
                mark(c, work);
            }
        }
    }
    mark(&desc, cx.work);
    if replacement.is_none() {
        desc.dom.remove();
    }
    cx.work.push(Work::Discard(desc.dom));
}

/// Where a table cell sits in its table's CSS grid, as the view writes it: its
/// [`TableMap`] rectangle in each axis, or [`Self::Outside`] for a cell the
/// bounded grid has no slot for. Two placements are equal exactly when they
/// write the same `grid-column` / `grid-row`, so a cell whose placement did not
/// change is not written (#1226).
///
/// [`TableMap`]: rinch_editor_core::tables::TableMap
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridPlacement {
    /// The rectangle: `(columns, rows)`, each on definite grid lines or as a
    /// span of tracks auto-placement puts where the map does.
    At(GridAxis, GridAxis),
    /// A cell no grid slot holds (past the capped width, or in a row the
    /// rowspans from above fill). No table command treats it as a cell; it is
    /// laid out as a band across the whole grid (`grid-column: 1 / -1`) on a
    /// row of its own, so its content stays visible without adding a column.
    /// Among cells placed by lines it is the first row the grid leaves empty —
    /// normally one after its last.
    Outside,
}

/// One axis of a [`GridPlacement::At`]: `Lines(start, end)` is `start + 1 /
/// end + 1`, `Span(n)` is `span n` (`auto` for one track).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GridAxis {
    Lines(usize, usize),
    Span(usize),
}

impl GridAxis {
    fn of(definite: bool, start: usize, end: usize) -> GridAxis {
        if definite {
            GridAxis::Lines(start, end)
        } else {
            GridAxis::Span(end - start)
        }
    }

    fn css(self) -> String {
        match self {
            GridAxis::Lines(start, end) => format!("{} / {}", start + 1, end + 1),
            GridAxis::Span(n) if n > 1 => format!("span {n}"),
            GridAxis::Span(_) => "auto".to_string(),
        }
    }
}

/// The most tracks a table's grid may have in an axis for its cells to be
/// placed on definite grid lines in that axis ([`GridLines`]): Stylo clamps a
/// grid line number to 10000 (`MAX_GRID_LINE`), so a cell starting past line
/// 10000 would be laid out on it, on top of its neighbours. A grid this size
/// has lines 1 to 10000. Past it the cells are placed in that axis by their
/// spans and auto-placement, as before #1209.
const MAX_DEFINITE_GRID_TRACKS: usize = 9999;

/// Which axes of a table's grid its cells are placed in by definite lines:
/// columns when there are at most [`MAX_DEFINITE_GRID_TRACKS`] of them, so a
/// table past 9999 rows keeps its cells in their columns; rows when both axes
/// are within it, because row lines over auto-placed columns can grow the
/// grid past Taffy's lines (see [`ViewDesc::sync_table_spans`]), and only when
/// auto-placement would put some cell in another row than the map does
/// ([`auto_rows_match_map`], #1226).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridLines {
    rows: bool,
    columns: bool,
}

impl GridPlacement {
    /// The `grid-column` / `grid-row` values that place a cell so.
    fn css(self) -> (String, String) {
        match self {
            GridPlacement::At(columns, rows) => (columns.css(), rows.css()),
            GridPlacement::Outside => ("1 / -1".to_string(), "auto".to_string()),
        }
    }
}

/// True when every cell of a table, placed on its map columns' definite lines
/// with its rows left to CSS grid auto-placement (`grid-row: span n`, in
/// document order, `grid-auto-flow: row`), lands in the rows the map gives it
/// (#1226). Row lines are then not needed, and they are worth leaving out: a
/// line names a row index, so a row inserted or deleted at the top of the table
/// would rewrite (and restyle) every cell below it; a span does not move.
///
/// `rects` is [`rinch_editor_core::tables::cell_rects`]. False for a table
/// with a cell outside the grid. This is css-grid-1 §8.5 step 4 for items
/// with a definite column position, which is every item here: the cursor's
/// column becomes the item's start line, its row moves down one when that is
/// before the previous item's, then down to the first row where the item
/// overlaps no item placed before it. It diverges from the map where a row a
/// rowspan from above leaves short holds a hole the cell would rise into, or a
/// ragged row lets the next row's cells move up.
///
/// Linear in the map's slots plus the rows each cell's search passes over.
fn auto_rows_match_map(rects: &[Vec<Option<rinch_editor_core::tables::Rect>>]) -> bool {
    let mut cells = Vec::new();
    for rect in rects.iter().flatten() {
        match rect {
            Some(r) => cells.push(*r),
            None => return false,
        }
    }
    let width = cells.iter().map(|r| r.right).max().unwrap_or(0);
    let height = cells.iter().map(|r| r.bottom).max().unwrap_or(0);
    // The first cell (in document order) covering each slot; a later cell
    // covering it too is placed elsewhere by auto-placement, which the walk
    // below finds.
    let mut owner = vec![usize::MAX; width * height];
    for (k, r) in cells.iter().enumerate() {
        for y in r.top..r.bottom {
            for x in r.left..r.right {
                let slot = &mut owner[y * width + x];
                if *slot == usize::MAX {
                    *slot = k;
                }
            }
        }
    }
    // Whether cell `k`'s columns are free of every cell before it over the
    // rows `top..top + h`.
    let free = |k: usize, r: &rinch_editor_core::tables::Rect, top: usize| {
        (top..top + (r.bottom - r.top))
            .all(|y| y >= height || (r.left..r.right).all(|x| owner[y * width + x] >= k))
    };
    let (mut row, mut column) = (0usize, 0usize);
    for (k, r) in cells.iter().enumerate() {
        if r.left < column {
            row += 1;
        }
        column = r.left;
        if row > r.top {
            return false;
        }
        while row < r.top {
            if free(k, r, row) {
                return false;
            }
            row += 1;
        }
        if !free(k, r, row) {
            return false;
        }
    }
    true
}

/// True when `a` and `b` give every cell the same [`GridPlacement`] because they
/// agree on everything a `TableMap` is computed from: the rows, each row's
/// cells, and each cell's `colspan` / `rowspan`.
fn same_table_spans(a: &Node, b: &Node) -> bool {
    let span = |c: &Node, name: &str| c.attrs().get_int(name);
    a.child_count() == b.child_count()
        && (0..a.child_count()).all(|r| {
            let (ra, rb) = (a.child(r), b.child(r));
            // An edit shares every row it did not touch by reference, so a
            // keystroke in a cell compares one row's cells, not the table's.
            ra.same_ref(rb)
                || ra.child_count() == rb.child_count()
                    && (0..ra.child_count()).all(|i| {
                        let (ca, cb) = (ra.child(i), rb.child(i));
                        span(ca, "colspan") == span(cb, "colspan")
                            && span(ca, "rowspan") == span(cb, "rowspan")
                    })
        })
}

impl ViewDesc {
    /// Build a descriptor (and its host subtree, including inline mark wrappers)
    /// for `node`.
    fn build(node: &Node, cx: &mut Cx) -> Option<ViewDesc> {
        let doc = cx.doc;
        let (dom, children, is_text) = if let Some(text) = node.text() {
            (create_text(doc, text)?, Vec::new(), true)
        } else {
            let el = create_element(doc, &node_dom_tag(node))?;
            apply_element_attrs(&el, node, None);
            let inline = node.is_textblock();
            let mut children = Vec::with_capacity(node.child_count());
            for i in 0..node.child_count() {
                if let Some(child) = ViewDesc::build(node.child(i), cx) {
                    // A textblock's inline children are placed with their
                    // mark wrappers by `regroup_inline`, below.
                    if !inline {
                        el.append_child(&child.dom);
                    }
                    children.push(child);
                }
            }
            (el, children, false)
        };
        let slot = if is_text {
            None
        } else {
            cx.slot_for(node, &dom)
        };
        let has_view = slot.is_some() || children.iter().any(|c| c.has_view);
        Some(ViewDesc {
            slot,
            has_view,
            node: node.clone(),
            dom,
            wraps: Vec::new(),
            inline_wrappers: Vec::new(),
            deferred: Vec::new(),
            children,
            is_text,
            // A freshly built run carries no decorations; the decoration pass that
            // runs right after the document diff puts them back on.
            decos: Vec::new(),
            segments: Vec::new(),
            has_deco: false,
            trailing_break: None,
            grid_placement: None,
        })
        .map(|mut desc| {
            if desc.node.is_textblock() {
                desc.regroup_inline(cx);
            }
            desc.sync_trailing_break(cx.doc);
            if rinch_editor_core::tables::is_table(&desc.node) {
                desc.sync_table_spans();
            }
            desc
        })
    }

    /// Place this textblock's inline children in their mark wrappers (#1833).
    ///
    /// Walking the children left to right with a stack of open wrappers
    /// (outermost first), a child keeps every open wrapper, from the outside
    /// in, whose mark it also carries, closes the rest, and opens its other
    /// marks inside them, the last in canonical (type-name) order outermost
    /// — the nesting a run alone always had. So a mark that continues from
    /// one run into the next is one element (inline code split by bold is
    /// one `<code>`, one pill), where each run used to have a wrapper chain of
    /// its own; a run's nesting therefore depends on the runs before it, and
    /// can differ from the copy-out HTML's, which nests each run alone.
    ///
    /// A run whose mark set changed has been rebuilt by the diff
    /// ([`Self::update`] refuses it), so its new node arrives detached. A
    /// wrapper is reused, for the same mark, from the child's own previous
    /// chain before one is made; a wrapper this pass makes is filled while
    /// still detached and then attached; a node already in place is not
    /// touched ([`place_children`]); the children the diff dropped are retired
    /// once their successors are in place ([`Self::deferred`]); and a wrapper
    /// no child uses any more is discarded. Run only when the children's
    /// structure changed ([`Self::diff_children`]): a text edit inside a run
    /// costs nothing here. The root runs it too when the schema's top node is
    /// a textblock.
    fn regroup_inline(&mut self, cx: &mut Cx) {
        let doc = cx.doc;
        // (container, the nodes it holds); index 0 is the textblock itself.
        let mut containers: Vec<(NodeHandle, Vec<NodeHandle>)> =
            vec![(self.dom.clone(), Vec::with_capacity(self.children.len()))];
        // The open wrappers, outermost first: (mark, host, container index).
        let mut stack: Vec<(Mark, NodeHandle, usize)> = Vec::new();
        let mut used: HashSet<usize> = HashSet::new();
        let mut reused_ids: HashSet<usize> = HashSet::new();
        for child in &mut self.children {
            let marks: Vec<&Mark> = child
                .node
                .marks()
                .iter()
                .filter(|m| mark_dom_tag(m).is_some())
                .collect();
            let keep = stack
                .iter()
                .take_while(|(m, _, _)| marks.contains(&m))
                .count();
            stack.truncate(keep);
            for &mark in marks.iter().rev() {
                if stack.iter().any(|(m, _, _)| m == mark) {
                    continue;
                }
                let reused = child
                    .wraps
                    .iter()
                    .find(|(m, w)| m == mark && !used.contains(&w.node_id().0))
                    .map(|(_, w)| w.clone());
                if let Some(w) = &reused {
                    reused_ids.insert(w.node_id().0);
                }
                let Some(wrapper) = reused.or_else(|| {
                    let el = create_element(doc, &mark_dom_tag(mark)?)?;
                    apply_mark_attrs(&el, mark);
                    Some(el)
                }) else {
                    continue;
                };
                used.insert(wrapper.node_id().0);
                let parent = stack.last().map_or(0, |(_, _, c)| *c);
                containers[parent].1.push(wrapper.clone());
                containers.push((wrapper.clone(), Vec::new()));
                stack.push((mark.clone(), wrapper, containers.len() - 1));
            }
            let parent = stack.last().map_or(0, |(_, _, c)| *c);
            containers[parent].1.push(child.dom.clone());
            child.wraps = stack
                .iter()
                .map(|(m, w, _)| (m.clone(), w.clone()))
                .collect();
        }
        if let Some(br) = &self.trailing_break {
            containers[0].1.push(br.clone());
        }
        let reused = reused_ids;
        // A wrapper made by this pass is filled first, innermost first, while
        // it is detached: building it costs the host's layout nothing. Then the
        // rest top-down: a container is in its final place before its own
        // nodes go in, so no node is ever put inside one of its descendants
        // (a made wrapper is empty but for what this pass put in it).
        for (host, want) in containers.iter().skip(1).rev() {
            if !reused.contains(&host.node_id().0) {
                place_children(host, want);
            }
        }
        for (i, (host, want)) in containers.iter().enumerate() {
            if i == 0 || reused.contains(&host.node_id().0) {
                place_children(host, want);
            }
        }
        // The dropped children go now: their successors are in place, and
        // the wrappers they sat in are still standing (a node view's host is
        // detached by `retire` before its wrapper is discarded below).
        for old in std::mem::take(&mut self.deferred) {
            retire(old, cx, None);
        }
        let stale: Vec<NodeHandle> = std::mem::take(&mut self.inline_wrappers)
            .into_iter()
            .filter(|w| !used.contains(&w.node_id().0))
            .collect();
        // Detach them all first: one may be inside another, and each is
        // discarded on its own (issue #719 — gone for good).
        for w in &stale {
            w.remove();
        }
        for w in stale {
            w.discard();
        }
        self.inline_wrappers = containers.into_iter().skip(1).map(|(w, _)| w).collect();
    }

    /// Place every cell of this `table` descriptor in the table's CSS grid by its
    /// rectangle in the table's [`TableMap`] — the bounded grid every table
    /// command edits — rather than by its raw `colspan` / `rowspan` (#1182). A
    /// table reaching the model by an app's own transaction can say
    /// `colspan = 3_000_000` or `rowspan = i64::MAX` (a load caps `colspan` at
    /// 1000 and `rowspan` at 65534, #1214, #1244); written raw, that
    /// asked the host for that many implicit grid tracks, and on the desktop a
    /// few such cells stacked past the `i16` lines Taffy 0.12 numbered a grid with,
    /// and layout panicked. The map cuts a span at the grid's edge and has no
    /// slot for a cell past its capped width, so no cell asks for more columns
    /// than the map has or more rows than the table has, and a cell with no
    /// slot is a [`GridPlacement::Outside`] band. A cell whose placement did
    /// not change is not written.
    ///
    /// [`TableMap`]: rinch_editor_core::tables::TableMap
    fn sync_table_spans(&mut self) {
        #[cfg(test)]
        table_span_tests::SYNCS.with(|n| n.set(n.get() + 1));
        let rects = rinch_editor_core::tables::cell_rects(&self.node);
        let columns =
            rinch_editor_core::tables::column_count(&self.node) <= MAX_DEFINITE_GRID_TRACKS;
        let lines = GridLines {
            // Row lines need column lines too: a cell locked to its row is
            // placed beside the cells before it, so a wide row of wide cells
            // (four of `colspan = 20000`, each span clamped to 10000 tracks)
            // grows the implicit grid past the 32767 lines Taffy 0.12 numbered
            // it with, and layout panicked; Taffy 0.14 (#1236) clamps the axis
            // at 10000 tracks and overlaps the cells past it instead.
            // Auto-placed, those cells wrap. And only where auto-placement
            // would not put every cell in its own row (#1226).
            rows: columns
                && self.node.child_count() <= MAX_DEFINITE_GRID_TRACKS
                && !auto_rows_match_map(&rects),
            columns,
        };
        for (row, row_rects) in self.children.iter_mut().zip(&rects) {
            for (cell, rect) in row.children.iter_mut().zip(row_rects) {
                let placement = match rect {
                    Some(r) => GridPlacement::At(
                        GridAxis::of(lines.columns, r.left, r.right),
                        GridAxis::of(lines.rows, r.top, r.bottom),
                    ),
                    None => GridPlacement::Outside,
                };
                if cell.grid_placement == Some(placement) {
                    continue;
                }
                let (column, grid_row) = placement.css();
                cell.dom.set_style("grid-column", &column);
                cell.dom.set_style("grid-row", &grid_row);
                cell.grid_placement = Some(placement);
            }
        }
    }

    /// Show or drop the **trailing-break placeholder**: a `<br
    /// data-pm-trailing-break>` after a textblock whose last child is a hard
    /// break or text ending in a newline, which is ProseMirror's rule for its
    /// `ProseMirror-trailingBreak`.
    ///
    /// A line box after a block's last forced break exists in CSS only when
    /// something comes after the break: `<p>a<br></p>` and a code block's
    /// `<pre>a\n</pre>` are one line, in every browser and on rinch-dom
    /// (#1172). So a paragraph ending in a hard break (Shift+Enter at its end),
    /// or a code block whose text ends in a newline (a load, a peer's edit),
    /// would show no line for the caret after the break to sit on. The second
    /// `<br>` is that something; the line it ends is the empty one.
    ///
    /// It is outside the model, after every model position, and outside the
    /// descriptors (`children` stays 1:1 with the node's): it carries no
    /// `data-pm-type`, so the web's DOM-point and mirror walks count nothing for
    /// it, and it is after the textblock's last position, so the caret map
    /// never reaches its byte — past the model's end, [`ifc_byte_to_char`]
    /// answers the block's end.
    fn sync_trailing_break(&mut self, doc: &DocRef) {
        let last = (!self.is_text && self.node.is_textblock() && self.node.child_count() > 0)
            .then(|| self.node.child(self.node.child_count() - 1));
        let wanted = last.is_some_and(|last| match last.text() {
            Some(text) => text.ends_with('\n'),
            None => node_dom_tag(last) == "br",
        });
        match (wanted, self.trailing_break.is_some()) {
            (true, false) => {
                if let Some(br) = create_element(doc, "br") {
                    br.set_attribute(TRAILING_BREAK_ATTR, "");
                    self.dom.append_child(&br);
                    self.trailing_break = Some(br);
                }
            }
            (false, true) => {
                // Gone for good — `discard`, not `remove` (issue #719).
                if let Some(br) = self.trailing_break.take() {
                    br.discard();
                }
            }
            _ => {}
        }
    }

    /// Patch this descriptor in place to project `new`. Returns `false` if the
    /// kinds are incompatible (text vs element, a different node type or element
    /// tag) or the mark set changed (the run wants other wrappers: it is
    /// rebuilt, and its textblock's [`Self::regroup_inline`] places the new
    /// node, #1833), and the caller must replace the whole subtree instead.
    ///
    /// `pos` is the position just before `new` in the new document. A node a
    /// view is mounted for ([`Self::slot`]) is patched in place only when it is
    /// the same node: the same `Rc`, or one whose attributes a `SetNodeAttrStep`
    /// changed ([`Cx::kept`]) — never a different node that happens to stand at
    /// its place (decision (d)); then `false` rebuilds it, and its view is
    /// remounted.
    ///
    /// `old_pos` is the position just before this descriptor's node in the old
    /// document, for [`Cx::same_place`].
    fn update(&mut self, new: &Node, cx: &mut Cx, pos: usize, old_pos: usize) -> bool {
        // Fast path: same persistent subtree — nothing changed (design A12).
        if self.node.same_ref(new) {
            return true;
        }
        // A descriptor with no view is never patched into a node a view draws: it
        // would get no slot, and the node no view (a task list, whose host is a
        // `div` as an embed's is, replaced by an embed at its place — review 2 of
        // #1575).
        if self.slot.is_none() && cx.claims(new) {
            return false;
        }
        if let Some(slot) = &self.slot {
            if self.node.type_name() != new.type_name()
                || self.node.marks() != new.marks()
                || !cx
                    .kept
                    .iter()
                    .any(|k| k.pos == pos && k.old_pos == old_pos && k.old.same_ref(&self.node))
            {
                return false;
            }
            if slot.has_view_host() {
                apply_view_host_attrs(&self.dom, new, Some(&self.node));
            } else {
                apply_element_attrs(&self.dom, new, Some(&self.node));
            }
            slot.set_node(new.clone());
            cx.work.push(Work::Sync(slot.clone()));
            self.node = new.clone();
            return true;
        }
        // A changed mark set wants other wrappers: rebuild, and the parent's
        // `regroup_inline` places the new node in them (#1833). A rebuilt
        // node is detached, so it goes into a new wrapper before that wrapper
        // is attached — fewer host mutations than moving the old one. (Blocks
        // are always unmarked.)
        if self.node.marks() != new.marks() {
            return false;
        }
        if new.is_text() {
            if !self.is_text {
                return false;
            }
            if self.node.text() != new.text() {
                // A *decorated* run's `dom` is the wrapper span holding its
                // segments, not a text node — the text has to be re-split, so
                // rebuild the run wholesale and let the decoration pass re-apply.
                if !self.decos.is_empty() {
                    return false;
                }
                self.dom.set_text(new.text().unwrap_or(""));
            }
            self.node = new.clone();
            return true;
        }
        // Element: the node type and the tag must match to patch in place. The
        // type, not only the tag: `div` hosts several types (a task list, a
        // task item, an embed no view claims, an app's block leaf), and a host
        // patched into another type would keep its `data-pm-type` — the attrs
        // are rewritten only when they differ, and are empty on both sides
        // (review 3 of #1575, F2).
        if self.is_text
            || self.node.type_name() != new.type_name()
            || node_dom_tag(&self.node) != node_dom_tag(new)
        {
            return false;
        }
        // Re-apply element attrs when the node's own attrs changed, OR when it is a
        // `table` whose column count changed: a table's `grid-template-columns` is
        // derived from its column count (its *content*), not its attrs, so a column
        // add/remove must refresh it. Gating on the count (rather than any table edit)
        // avoids a full table restyle on every keystroke inside a cell.
        let table_cols_changed = new.type_name() == "table"
            && rinch_editor_core::tables::column_count(&self.node)
                != rinch_editor_core::tables::column_count(new);
        if self.node.attrs() != new.attrs() || table_cols_changed {
            apply_element_attrs(&self.dom, new, Some(&self.node));
        }
        // A cell's placement is its rectangle in the whole table's map, so a
        // span or a row anywhere in the table can move it; and a cell host
        // built by this diff has no placement yet.
        let replace_spans =
            rinch_editor_core::tables::is_table(new) && !same_table_spans(&self.node, new);
        let regroup = self.diff_children(new, cx, pos + 1, old_pos + 1);
        self.node = new.clone();
        if regroup && new.is_textblock() {
            self.regroup_inline(cx);
        }
        self.has_view = self.slot.is_some() || self.children.iter().any(|c| c.has_view);
        if rinch_editor_core::tables::is_table(new)
            && (replace_spans
                || self
                    .children
                    .iter()
                    .flat_map(|row| &row.children)
                    .any(|cell| cell.grid_placement.is_none()))
        {
            self.sync_table_spans();
        }
        self.sync_trailing_break(cx.doc);
        true
    }

    /// Reconcile this descriptor's children against `new`'s children.
    ///
    /// The children `new` shares with the old node by reference (`same_ref`) at
    /// the **start** and at the **end** are matched first and left alone; only
    /// the stretch between them is diffed, positionally: a child patched in
    /// place where it can be, replaced where its kind changed, new children
    /// inserted before the unchanged suffix, surplus ones discarded.
    ///
    /// Matching the suffix is what keeps a structural edit local (issue #905).
    /// A purely positional diff re-pointed every block after a split or join
    /// at its neighbour's host, rewriting each one's text — so one Enter in a
    /// long document re-shaped every paragraph below the caret on the desktop.
    /// A keyed/LIS pass is still not needed: an edit typically changes one contiguous
    /// stretch of siblings, which is exactly what prefix + suffix isolate.
    ///
    /// `content_start` is the position just inside `new`'s content (0 for the
    /// document), so each child's position is known for [`Self::update`].
    ///
    /// A changed stretch holding a node view is diffed by **node identity**
    /// instead ([`Self::diff_stretch_keyed`]): a view's node that only moved
    /// within the stretch (a block inserted before it and another removed after
    /// it in one transaction) keeps its mount rather than being matched by
    /// position against another node and remounted.
    ///
    /// A textblock's inline children are not placed in the host here: a new
    /// one is left unplaced and the answer says whether the structure changed
    /// (a child built, replaced — a changed mark set included — removed or
    /// moved), when the
    /// caller owes [`Self::regroup_inline`] (#1833). A text edit inside a run
    /// changes none of that.
    fn diff_children(
        &mut self,
        new: &Node,
        cx: &mut Cx,
        content_start: usize,
        old_content_start: usize,
    ) -> bool {
        let inline = self.node.is_textblock();
        let new_count = new.child_count();
        let old_count = self.children.len();
        // Where each old child stood in the old document, for `Cx::same_place`.
        let mut old_at = Vec::with_capacity(old_count + 1);
        old_at.push(old_content_start);
        for c in &self.children {
            old_at.push(old_at[old_at.len() - 1] + c.node.node_size());
        }
        let mut new_at = Vec::with_capacity(new_count + 1);
        new_at.push(content_start);
        for i in 0..new_count {
            new_at.push(new_at[i] + new.child(i).node_size());
        }
        // Only a peer's change asks which copy is which, and only of a view's node.
        let twice = if cx.mapping.is_some() && self.children.iter().any(|c| c.has_view) {
            twice_keys(&self.children, new)
        } else {
            HashSet::new()
        };
        // A run matched by `same_ref` stops at a view's node that the model cannot
        // tell from another copy of it unless the remote mapping says it is this one
        // (`Cx::same_place`).
        let keeps = |this: &Self, i: usize, j: usize| {
            let old = &this.children[i];
            old.node.same_ref(new.child(j))
                && (!old.has_view
                    || cx.mapping.is_none()
                    || cx.same_place(
                        twice.contains(&node_key(&old.node)),
                        old_at[i],
                        old_at[i + 1],
                        new_at[j],
                    ))
        };
        let mut start = 0;
        while start < old_count && start < new_count && keeps(self, start, start) {
            start += 1;
        }
        let (mut old_end, mut new_end) = (old_count, new_count);
        while old_end > start && new_end > start && keeps(self, old_end - 1, new_end - 1) {
            old_end -= 1;
            new_end -= 1;
        }
        // The changed stretch: `old[start..old_end]` becomes `new[start..new_end]`.
        if self.children[start..old_end].iter().any(|c| c.has_view) {
            self.diff_stretch_keyed(new, cx, start, old_end, new_end, &old_at, &new_at, &twice);
            return true;
        }
        let mut regroup = false;
        let common = (old_end - start).min(new_end - start);
        for (i, (&at, &old_pos)) in new_at
            .iter()
            .zip(&old_at)
            .enumerate()
            .take(start + common)
            .skip(start)
        {
            let new_child = new.child(i);
            if self.children[i].update(new_child, cx, at, old_pos) {
                continue;
            }
            regroup = true;
            // Kind changed — build a replacement and swap it into the host
            // (inside the old one's mark wrappers; `regroup_inline` moves it
            // if its marks want others).
            if let Some(replacement) = ViewDesc::build(new_child, cx) {
                // `replace_with` *detaches* the node it displaces (issue
                // #719); the `ViewDesc` holding it is overwritten here, so
                // nothing can show it again. `retire` says so, or the
                // browser backend pins it for the life of the page — this
                // is a per-keystroke path.
                let old = std::mem::replace(&mut self.children[i], replacement);
                // A textblock's replacement is placed, and the old one retired,
                // by `regroup_inline`.
                if inline {
                    self.deferred.push(old);
                } else {
                    retire(old, cx, Some(&self.children[i].dom));
                }
            }
        }
        let at = start + common;
        if new_end - start > common {
            // New children go in before the unchanged suffix (or at the end).
            regroup = true;
            let mut built = Vec::with_capacity(new_end - at);
            for i in at..new_end {
                if let Some(new_desc) = ViewDesc::build(new.child(i), cx) {
                    if inline {
                        built.push(new_desc);
                        continue;
                    }
                    // At the end, still before the trailing-break placeholder,
                    // which stays last until `sync_trailing_break` drops it.
                    match self
                        .children
                        .get(old_end)
                        .map(|c| &c.dom)
                        .or(self.trailing_break.as_ref())
                    {
                        Some(next) => self.dom.insert_before(&new_desc.dom, next),
                        None => self.dom.append_child(&new_desc.dom),
                    }
                    built.push(new_desc);
                }
            }
            self.children.splice(at..at, built);
        } else if old_end > at {
            // Dropped for good — `discard`, not `remove` (issue #719).
            regroup = true;
            let extras: Vec<ViewDesc> = self.children.drain(at..old_end).collect();
            for extra in extras {
                if inline {
                    self.deferred.push(extra);
                } else {
                    retire(extra, cx, None);
                }
            }
        }
        regroup
    }

    /// [`Self::diff_children`]'s changed stretch `old[start..old_end]` →
    /// `new[start..new_end]` (the new children starting at `pos`), matched by node
    /// identity: a new child that is the same `Rc` as an old one anywhere in the
    /// stretch takes that descriptor (its host is moved, a view in it stays
    /// mounted); any other is patched over the old descriptor at its own index,
    /// if that one is free and [`Self::update`] accepts it, or built. The old
    /// descriptors nobody took are retired.
    #[allow(clippy::too_many_arguments)]
    fn diff_stretch_keyed(
        &mut self,
        new: &Node,
        cx: &mut Cx,
        start: usize,
        old_end: usize,
        new_end: usize,
        old_at: &[usize],
        new_at: &[usize],
        twice_set: &HashSet<usize>,
    ) {
        // Whether a view's node stands more than once among the old or the new
        // children (`twice_keys`, read before the drain): then `same_ref` alone does
        // not say which copy is which, and its descriptor goes where the remote
        // mapping carries it.
        let twice: Vec<bool> = self.children[start..old_end]
            .iter()
            .map(|o| o.has_view && twice_set.contains(&node_key(&o.node)))
            .collect();
        let mut old: Vec<Option<ViewDesc>> =
            self.children.drain(start..old_end).map(Some).collect();
        // An old descriptor some new child is the same node as is that child's,
        // and is never patched over another.
        // So is one a kept node was before the transaction (the same node with
        // other attributes, wherever it stands now).
        let reserved: Vec<bool> = old
            .iter()
            .map(|o| {
                o.as_ref().is_some_and(|o| {
                    (start..new_end).any(|j| o.node.same_ref(new.child(j)))
                        || cx.kept.iter().any(|k| k.old.same_ref(&o.node))
                })
            })
            .collect();
        let inline = self.node.is_textblock();
        let mut retired: Vec<ViewDesc> = Vec::new();
        let mut placed: Vec<ViewDesc> = Vec::with_capacity(new_end - start);
        for j in start..new_end {
            let child = new.child(j);
            let at = new_at[j];
            let same = (0..old.len()).find(|&k| {
                old[k].as_ref().is_some_and(|o| {
                    o.node.same_ref(child)
                        && cx.same_place(twice[k], old_at[start + k], old_at[start + k + 1], at)
                })
            });
            if let Some(k) = same {
                placed.extend(old[k].take());
                continue;
            }
            // The same node with other attributes, moved within the stretch (a
            // block inserted before it in the same transaction): the one that
            // stood where the transaction's `SetNodeAttrStep` found it.
            let was = cx.kept.iter().find(|k| k.pos == at).and_then(|kept| {
                (0..old.len()).find(|&k| {
                    old[k].as_ref().is_some_and(|o| {
                        o.node.same_ref(&kept.old) && old_at[start + k] == kept.old_pos
                    })
                })
            });
            if let Some(k) = was
                && let Some(mut desc) = old[k].take()
            {
                if desc.update(child, cx, at, old_at[start + k]) {
                    placed.push(desc);
                    continue;
                }
                retired.push(desc);
            }
            let free = reserved.get(j - start) == Some(&false);
            if let Some(mut desc) = old
                .get_mut(j - start)
                .filter(|_| free)
                .and_then(Option::take)
            {
                if desc.update(child, cx, at, old_at[j]) {
                    placed.push(desc);
                    continue;
                }
                retired.push(desc);
            }
            placed.extend(ViewDesc::build(child, cx));
        }
        for desc in old.into_iter().flatten() {
            retired.push(desc);
        }
        for desc in retired {
            if inline {
                self.deferred.push(desc);
            } else {
                retire(desc, cx, None);
            }
        }
        // Place them in order before the unchanged suffix (or the trailing-break
        // placeholder, or at the end); a kept host is moved, not rebuilt.
        let anchor = self
            .children
            .get(start)
            .map(|c| c.dom.clone())
            .or(self.trailing_break.clone());
        // A textblock places its inline children by `regroup_inline`, which
        // the caller runs.
        let placed_here = if self.node.is_textblock() {
            &placed[..0]
        } else {
            &placed[..]
        };
        for desc in placed_here {
            match &anchor {
                Some(next) => self.dom.insert_before(&desc.dom, next),
                None => self.dom.append_child(&desc.dom),
            }
        }
        self.children.splice(start..start, placed);
    }

    /// The placed host node of the first block child, if any (the placeholder is
    /// inserted before it so it overlays the empty editor).
    fn first_child_dom(&self) -> Option<&NodeHandle> {
        self.children.first().map(|c| &c.dom)
    }
}

/// The attribute marking a textblock's trailing-break placeholder
/// ([`ViewDesc::sync_trailing_break`], #1172).
pub(crate) const TRAILING_BREAK_ATTR: &str = "data-pm-trailing-break";

/// The desktop editor view: projects [`EditorState`] onto rinch-dom and keeps the
/// host in sync as transactions are applied (design §6). Implements the
/// renderer-agnostic [`EditorView`] seam.
pub struct RinchDomEditorView {
    doc: DocRef,
    /// The descriptor for the document root. Its [`ViewDesc::dom`] is the host
    /// container (created by the caller, e.g. the `Editor {}` component) and is
    /// **never replaced** — only its children diff.
    root: ViewDesc,
    /// The placeholder overlay node while shown — decoration-diff state (A4/A8).
    placeholder: Option<NodeHandle>,
    /// The caret overlay node — an absolutely-positioned bar the view owns and
    /// repositions from `state.selection` (design §6: caret is rendered *from*
    /// the selection, never via a `data-ce-cursor` attribute). It lives as a child
    /// of the **container** (never a textblock — a child node would disrupt the
    /// block's inline-formatting context), positioned in container space.
    caret: Option<NodeHandle>,
    /// The last rendered caret `(x, y, height)` (pixels rounded), so a re-run that
    /// lands on the same spot writes nothing — otherwise the caret's own style
    /// writes would re-dirty the tree and spin the repaint loop.
    last_caret: Option<(i32, i32, i32)>,
    /// The blink phase last applied to the caret (`Some(true)` = shown,
    /// `Some(false)` = hidden, `None` = no caret present). Guards
    /// [`Self::set_caret_blink_visible`] against redundant writes so a blink tick
    /// that doesn't change the phase doesn't re-dirty the tree.
    blink_shown: Option<bool>,
    /// Highlight rectangles for the current text selection (container children,
    /// reused across updates).
    selection_rects: Vec<NodeHandle>,
    /// The last rendered selection `(from, to)`, for change-detection.
    /// Change-detection key for the selection overlay (the wash). Tagged by kind
    /// (`0` = text range, `1` = cell rectangle) so a text and a cell selection that
    /// happen to share the same two positions don't alias and stale-skip a re-render.
    last_selection: Option<(u8, usize, usize)>,
    /// The outline overlay for a [`Selection::Node`] — a translucent box tracing the
    /// selected leaf node (image / horizontal rule), drawn from `state.selection`
    /// (design §6 node-views). `None` when the current selection is not a node
    /// selection.
    node_outline: Option<NodeHandle>,
    /// The last rendered node-selection box `(x, y, w, h)` (pixels rounded), so a
    /// re-run on the same geometry writes nothing (mirrors [`Self::last_caret`]).
    last_node_outline: Option<(i32, i32, i32, i32)>,
    /// Index into [`Self::selection_rects`] of the wash rectangle over the *head*
    /// cell of the current [`Selection::Cell`], or `None` when the selection is not
    /// a cell selection. It is the cell-selection arm's
    /// [scroll anchor](Self::scroll_anchor) — the cell the user is moving, so
    /// extending a cell rectangle follows the head rather than jumping to the
    /// anchor corner.
    ///
    /// [`Selection::Cell`]: rinch_editor_core::Selection::Cell
    cell_anchor_rect: Option<usize>,
    /// The IME composition (preedit) overlay: a span shown inline at the caret
    /// with the composing text underlined. The composition is **never** part of
    /// the document (design A5) — it is a transient view overlay, discarded on the
    /// next commit or clear. `preedit_text` is the span's text-node child.
    preedit_node: Option<NodeHandle>,
    preedit_text: Option<NodeHandle>,
    /// The active composition string, or `None` when not composing.
    preedit: Option<String>,
    /// Last applied preedit `(x, y, text)`, so a re-run on the same composition and
    /// caret writes nothing (mirrors [`Self::last_caret`]).
    last_preedit: Option<(i32, i32, String)>,
    /// The decoration set currently projected, for the next diff.
    decorations: DecorationSet,
    /// The inline decorations of [`Self::decorations`], flattened once per
    /// change of the set rather than once per transaction (#848).
    inline: Vec<InlineDeco>,
    /// Set whenever the post-layout pass writes an overlay's geometry (caret,
    /// selection rect, or node outline). The runtime reads it via
    /// [`Self::take_overlay_dirty`] to re-resolve layout and schedule a repaint,
    /// since the overlays' new boxes only exist after a layout pass. A
    /// dirty-region repaint: rinch-dom keeps each overlay's last *painted* rect
    /// until the paint consumes it, so the old caret is cleared however many
    /// resolves land first. (This used to force a full repaint per keystroke,
    /// because a second resolve overwrote that old rect.)
    overlay_dirty: bool,
    /// The invisible boxes [`Self::position_reveal`] places over a range's end
    /// and start for [`EditorHandle::scroll_into_view`] to scroll to. Empty
    /// until the first reveal; kept (and moved) after it, like the caret.
    ///
    /// [`EditorHandle::scroll_into_view`]: super::EditorHandle::scroll_into_view
    reveal_probes: Vec<NodeHandle>,
    /// The drop caret ([`Self::show_drop_caret`]): where an OS file drag over
    /// this editor will land, drawn while the drag hovers. Separate from the
    /// caret, so the live selection is untouched and an unfocused editor
    /// (whose caret the focus-aware pass hides) still shows it.
    drop_caret: Option<NodeHandle>,
    /// The drop caret's last `(x, y, height)` (pixels rounded), `None` while
    /// hidden, so a repeat with the same geometry writes nothing.
    last_drop_caret: Option<(i32, i32, i32)>,
    /// Which side of a soft wrap the caret at the selection's head is drawn on
    /// (#301) — the handle's hint for the current selection
    /// ([`EditorHandle::caret_affinity`](super::EditorHandle::caret_affinity)),
    /// set before every caret pass.
    caret_affinity: CaretAffinity,
    /// What a hard break and a tab occupy in the host's flat byte offsets
    /// ([`FlatWidths`]). Read once at construction; the caret map
    /// (`textblock_flat_byte` / `ifc_byte_to_char`) counts them so a caret after
    /// a break (#1099) or a tab (#1109) is drawn after it.
    flat: FlatWidths,
    /// The editor's node views ([`crate::node_views`]), shared with its handle.
    node_views: Rc<Registry>,
    /// Node-view work the diff queued, run by the handle once its borrow is
    /// released ([`Self::take_node_view_work`]).
    node_view_work: Vec<Work>,
    /// Every live node-view slot, in no particular order.
    slots: Vec<Rc<Slot>>,
    /// The slot of the node-selected node, while that node has one.
    selected_slot: Option<Rc<Slot>>,
    /// The next [`EditorView::update_dom`]'s proof of which nodes are the same
    /// node with other attributes ([`Cx::kept`]); taken by it.
    kept: Vec<Kept>,
    /// The mapping of a peer's change, for the next diff ([`Self::set_remote_mapping`]);
    /// taken by it.
    remote_mapping: Option<Mapping>,
    /// Set once the editor unmounted ([`Self::dispose_node_views`]): no slot is
    /// made after that.
    node_views_off: bool,
    /// Each mounted view's host box as the last caret pass saw it, so a view
    /// that grew or shrank re-runs the selection overlay
    /// ([`Self::node_view_hosts_moved`]).
    view_boxes: Vec<(usize, (i32, i32, i32, i32))>,
}

/// What a margin anchor is measured on ([`RinchDomEditorView::margin_target`]).
enum MarginTarget {
    /// A caret: its textblock's host id, its flat byte, its position.
    Text(usize, usize, Pos),
    /// A leaf block's host id.
    Leaf(usize),
}

/// How far above and below a revealed range [`RinchDomEditorView::position_reveal`]
/// keeps clear, in CSS px, so the words land a little inside the scroller's
/// edge rather than flush against it.
pub(crate) const REVEAL_MARGIN: f32 = 16.0;

impl RinchDomEditorView {
    /// Build a view over `state`, rendering its document into `container` (the host
    /// element standing in for the `doc` node). `container` is owned by the caller
    /// and never replaced.
    pub fn new(container: NodeHandle, doc: DocRef, state: &EditorState) -> RinchDomEditorView {
        Self::with_node_views(container, doc, state, Rc::default())
    }

    /// [`Self::new`] for an editor whose node views are `node_views`: a node a
    /// registered spec draws gets a slot, and its view is mounted by the work
    /// [`Self::take_node_view_work`] hands back.
    pub(crate) fn with_node_views(
        container: NodeHandle,
        doc: DocRef,
        state: &EditorState,
        node_views: Rc<Registry>,
    ) -> RinchDomEditorView {
        // Ship the default editor stylesheet (light + dark) so the editor looks
        // polished out of the box — injected once per document.
        super::styles::ensure_default_styles(&doc);
        container.set_attribute("data-pm-type", state.doc.type_name());
        // `position: relative` + `z-index: 0` (the containing block + stacking
        // context the caret/selection overlays need) live in the default stylesheet
        // on `[data-pm-editor]`, NOT as inline styles here — otherwise a consumer's
        // `style:` prop on the `Editor {}` component would replace the container's
        // inline `style` attribute and silently break overlay positioning. The
        // selection rects (`z-index: -1`) then paint *behind* the in-flow text and
        // the caret (`z-index: 1`) in front.
        let mut children = Vec::with_capacity(state.doc.child_count());
        let mut node_view_work = Vec::new();
        let mut slots = Vec::new();
        // A schema whose top node is itself a textblock (`doc: text*`): its
        // runs are placed in their mark wrappers, as any textblock's (#1833).
        let inline_root = state.doc.is_textblock();
        let mut cx = Cx {
            doc: &doc,
            views: (!node_views.is_empty()).then_some(&*node_views),
            kept: &[],
            mapping: None,
            work: &mut node_view_work,
            slots: &mut slots,
        };
        for i in 0..state.doc.child_count() {
            if let Some(child) = ViewDesc::build(state.doc.child(i), &mut cx) {
                if !inline_root {
                    container.append_child(&child.dom);
                }
                children.push(child);
            }
        }
        let has_view = children.iter().any(|c| c.has_view);
        let mut root = ViewDesc {
            slot: None,
            has_view,
            node: state.doc.clone(),
            wraps: Vec::new(),
            inline_wrappers: Vec::new(),
            deferred: Vec::new(),
            dom: container,
            children,
            is_text: false,
            decos: Vec::new(),
            segments: Vec::new(),
            has_deco: false,
            trailing_break: None,
            grid_placement: None,
        };
        if inline_root {
            root.regroup_inline(&mut cx);
        }
        let flat = doc
            .upgrade()
            .and_then(|d| d.try_borrow().ok().map(|d| FlatWidths::of(&*d)))
            .unwrap_or(FlatWidths::UTF8);
        let mut view = RinchDomEditorView {
            flat,
            doc,
            root,
            placeholder: None,
            caret: None,
            last_caret: None,
            blink_shown: None,
            selection_rects: Vec::new(),
            last_selection: None,
            node_outline: None,
            last_node_outline: None,
            cell_anchor_rect: None,
            caret_affinity: CaretAffinity::Downstream,
            preedit_node: None,
            preedit_text: None,
            preedit: None,
            last_preedit: None,
            decorations: DecorationSet::empty(),
            inline: Vec::new(),
            overlay_dirty: false,
            reveal_probes: Vec::new(),
            drop_caret: None,
            last_drop_caret: None,
            node_views,
            node_view_work,
            slots,
            selected_slot: None,
            kept: Vec::new(),
            remote_mapping: None,
            node_views_off: false,
            view_boxes: Vec::new(),
        };
        view.sync_decorations(state);
        view.sync_selected_slot(state);
        view
    }

    /// Diff `state`'s decorations against the projected set and patch what they
    /// render to — independent of the document diff (design A4): the placeholder
    /// overlay for widgets, and the wrapper spans over decorated text runs for
    /// inline decorations.
    fn sync_decorations(&mut self, state: &EditorState) {
        let next = state.decorations();
        if next != self.decorations {
            self.sync_widget_decorations(&next);
            self.inline = inline_decos(&next);
            self.decorations = next;
        }
        // Inline decorations are re-applied whenever any are present *or* any were
        // last time, even when the set itself compares equal. The document diff
        // runs first and may have rebuilt a decorated run — a fresh text node
        // carries no segments — so "the decorations did not change" does not imply
        // "what projects them is still there". The `has_deco` prune and the sorted
        // cursor keep that re-check to one test per top-level block plus one per
        // decoration it reaches (#848), and an unchanged run writes nothing.
        if !self.inline.is_empty() || self.root.has_deco {
            self.root.has_deco = sync_inline_decos(&mut self.root, &self.inline, 0, &self.doc);
        }
    }

    /// The widget half of [`Self::sync_decorations`]: today the empty-editor
    /// placeholder (design A8).
    fn sync_widget_decorations(&mut self, next: &DecorationSet) {
        let placeholder_text = next.iter().find_map(|d| d.as_placeholder());
        match (placeholder_text, self.placeholder.is_some()) {
            (Some(text), false) => {
                if let Some(node) = create_element(&self.doc, "div") {
                    node.set_attribute("data-pm-placeholder", "true");
                    node.set_attribute("class", "rinch-editor-placeholder");
                    if let Some(t) = create_text(&self.doc, text) {
                        node.append_child(&t);
                    }
                    // Overlay the (empty) first block; if none, append to the root.
                    match self.root.first_child_dom() {
                        Some(first) => self.root.dom.insert_before(&node, first),
                        None => self.root.dom.append_child(&node),
                    }
                    self.placeholder = Some(node);
                }
            }
            (None, true) => {
                if let Some(node) = self.placeholder.take() {
                    // `take`n, so the handle is gone: discard (issue #719). The
                    // arm above builds a fresh placeholder when one is needed.
                    node.discard();
                }
            }
            // (Some, true): placeholder stays (its text is fixed per editor).
            // (None, false): nothing to do.
            _ => {}
        }
    }
}

impl EditorView for RinchDomEditorView {
    fn update_dom(&mut self, _prev: &EditorState, next: &EditorState) -> Vec<ViewRequest> {
        // Phase 1 (before layout): diff the document and patch the host. The root's
        // host element is fixed, so only its children reconcile.
        let kept = std::mem::take(&mut self.kept);
        let mapping = self.remote_mapping.take();
        let registry = self.node_views.clone();
        {
            let mut cx = Cx {
                doc: &self.doc,
                views: (!self.node_views_off && !registry.is_empty()).then_some(&*registry),
                kept: &kept,
                mapping: mapping.as_ref(),
                work: &mut self.node_view_work,
                slots: &mut self.slots,
            };
            let regroup = self.root.diff_children(&next.doc, &mut cx, 0, 0);
            self.root.node = next.doc.clone();
            // A textblock root (`doc: text*`) places its runs as any
            // textblock does (#1833).
            if regroup && next.doc.is_textblock() {
                self.root.regroup_inline(&mut cx);
            }
        }
        self.root.has_view = self.root.children.iter().any(|c| c.has_view);
        self.root.node = next.doc.clone();
        if self.slots.iter().any(|s| s.is_removed()) {
            self.slots.retain(|s| !s.is_removed());
        }
        self.sync_selected_slot(next);
        // Decoration diff, independent of the document diff (A4).
        self.sync_decorations(next);
        Vec::new()
    }

    fn update_caret(&mut self, next: &EditorState) -> Vec<ViewRequest> {
        // A node view that grew or shrank moved everything below it: the
        // selection highlight is keyed by position, so it is drawn again.
        if self.node_view_hosts_moved() {
            self.last_selection = None;
        }
        // Phase 2 (after layout): render the selection from `next.selection`.
        //
        // Every request below is emitted only when the selection overlay actually
        // *moved* — the `position_*` helpers early-return on an unchanged geometry
        // key, so "did it move" is exactly what they report. That is a hint, not
        // the scroll decision: an overlay also moves under a resize or a remote
        // edit, which must not scroll. `EditorHandle::update_caret` decides from
        // what changed the state (its `ScrollGate`).
        //
        // A node selection (a selected image / horizontal rule) outlines the node
        // and shows neither a text highlight nor a caret (design §6 node-views).
        if let rinch_editor_core::Selection::Node(_) = &next.selection {
            self.clear_selection_rects();
            self.hide_caret();
            self.hide_preedit();
            self.cell_anchor_rect = None;
            return scroll_if(self.render_node_selection(next));
        }
        // A cell selection (a rectangle of table cells) washes each selected cell
        // and shows neither a caret nor a node outline.
        if let rinch_editor_core::Selection::Cell(_) = &next.selection {
            self.clear_node_selection();
            self.hide_caret();
            self.hide_preedit();
            return scroll_if(self.render_cell_selection(next));
        }
        self.clear_node_selection();
        self.cell_anchor_rect = None;
        // Otherwise: the text selection highlight (for a range) and the caret (at a
        // collapsed cursor).
        self.render_selection(next);
        // No caret while a range is selected — the highlight conveys the head, a
        // caret blinking over a highlight reads as noise, and with no caret the
        // blink loop idles (`set_caret_blink` reports "nothing to blink").
        if !next.selection.is_empty() {
            self.hide_caret();
            self.hide_preedit();
            return Vec::new();
        }
        let Some((block, flat_byte)) = self.caret_target(&next.doc, next.selection.head()) else {
            self.hide_caret();
            self.hide_preedit();
            return Vec::new();
        };
        let Some(doc) = self.doc.upgrade() else {
            return Vec::new();
        };
        let block_id = block.node_id().0;
        // Query the caret geometry (local to the textblock's border box), then
        // translate into the container's coordinate space. An *empty* textblock has
        // no Parley layout, so `query_caret_position` returns `None` there — fall
        // back to the block's content origin with a sensible line height, so the
        // caret is still visible on a blank line.
        // The composing block's font, so the preedit overlay (a container child that
        // would otherwise inherit only the container's default font) matches the text
        // it composes into — a 32px heading vs. 16px body. Queried only while composing.
        let mut preedit_font = None;
        let geometry = {
            let d = doc.borrow();
            if self.preedit.is_some() {
                preedit_font = d.node_font(block_id as u64);
            }
            self.caret_geometry(
                &*d,
                &next.doc,
                next.selection.head(),
                block_id,
                flat_byte,
                self.caret_affinity,
                next.stored_marks.as_deref(),
            )
            .map(|(x, y, height)| (x, self.keep_caret_in_block(&*d, block_id, x), y, height))
        };
        match geometry {
            Some((x, caret_x, y, height)) => {
                // While composing, the preedit overlay stands in for the caret at
                // the cursor — show it and hide the blinking caret. Otherwise place
                // the caret as usual.
                if let Some(text) = self.preedit.clone() {
                    self.position_preedit(x, y, height, &text, preedit_font.as_ref());
                    self.hide_caret();
                    Vec::new()
                } else {
                    self.hide_preedit();
                    scroll_if(self.position_caret(caret_x, y, height))
                }
            }
            None => {
                self.hide_preedit();
                self.hide_caret();
                Vec::new()
            }
        }
    }
}

/// `[ScrollSelectionIntoView]` when the selection overlay moved, else nothing —
/// the "overlay moved" hint every arm of [`RinchDomEditorView::update_caret`] returns
/// through.
fn scroll_if(moved: bool) -> Vec<ViewRequest> {
    if moved {
        vec![ViewRequest::ScrollSelectionIntoView]
    } else {
        Vec::new()
    }
}

impl RinchDomEditorView {
    /// The editor container (the `doc` node's element).
    pub(crate) fn container(&self) -> &NodeHandle {
        &self.root.dom
    }

    /// The node-view work queued since the last call ([`crate::node_views::run`]).
    pub(crate) fn take_node_view_work(&mut self) -> Vec<Work> {
        std::mem::take(&mut self.node_view_work)
    }

    /// Prove, for the next [`EditorView::update_dom`], which nodes are the same
    /// node with other attributes: positions in the new document ([`Cx::kept`]).
    pub(crate) fn set_kept(&mut self, kept: Vec<Kept>) {
        self.kept = kept;
    }

    /// For the next [`EditorView::update_dom`], which applies a **peer's** change:
    /// that transaction's mapping ([`Cx::mapping`]).
    #[cfg(feature = "collaboration")]
    pub(crate) fn set_remote_mapping(&mut self, mapping: Mapping) {
        self.remote_mapping = Some(mapping);
    }

    /// Re-read which nodes the registered specs draw (a spec was registered or
    /// released): a node of a newly drawn type gets a slot, and every slot is
    /// matched again.
    pub(crate) fn rescan_node_views(&mut self) {
        if self.node_views_off {
            return;
        }
        let registry = self.node_views.clone();
        fn walk(desc: &mut ViewDesc, cx: &mut Cx) {
            match &desc.slot {
                Some(slot) => cx.work.push(Work::Sync(slot.clone())),
                None if !desc.is_text => desc.slot = cx.slot_for(&desc.node, &desc.dom),
                None => {}
            }
            for c in &mut desc.children {
                walk(c, cx);
            }
            desc.has_view = desc.slot.is_some() || desc.children.iter().any(|c| c.has_view);
        }
        let mut cx = Cx {
            doc: &self.doc,
            views: Some(&*registry),
            kept: &[],
            mapping: None,
            work: &mut self.node_view_work,
            slots: &mut self.slots,
        };
        for c in &mut self.root.children {
            walk(c, &mut cx);
        }
        self.root.has_view = self.root.children.iter().any(|c| c.has_view);
        let selected = self.selected_slot.clone();
        if let Some(slot) = selected {
            self.node_view_work.push(Work::Sync(slot));
        }
    }

    /// Queue a refresh of every view's read-only and dark-mode memos.
    pub(crate) fn node_view_flags_changed(&mut self) {
        if !self.slots.is_empty() {
            self.node_view_work.push(Work::Flags(self.slots.clone()));
        }
    }

    /// The editor is unmounting: hand back every slot for disposal, and make no
    /// more.
    pub(crate) fn dispose_node_views(&mut self) -> Vec<Rc<Slot>> {
        self.node_views_off = true;
        self.selected_slot = None;
        self.node_view_work.clear();
        std::mem::take(&mut self.slots)
    }

    /// The model position just before the node `slot` is for.
    pub(crate) fn slot_pos(&self, slot: &Rc<Slot>) -> Option<usize> {
        fn walk(desc: &ViewDesc, slot: &Rc<Slot>, content_start: usize) -> Option<usize> {
            let mut pos = content_start;
            for child in &desc.children {
                if child.slot.as_ref().is_some_and(|s| Rc::ptr_eq(s, slot)) {
                    return Some(pos);
                }
                if child.has_view
                    && let Some(found) = walk(child, slot, pos + 1)
                {
                    return Some(found);
                }
                pos += child.node.node_size();
            }
            None
        }
        walk(&self.root, slot, 0)
    }

    /// Swap the host element of the inline leaf `slot` is for: for an inline-block
    /// `<span>` a node view is mounted in (`view_host`), or back for the node's own
    /// element (an `<img>`). The descriptor, any mark wrapper around it and the
    /// slot all name the new element; the old one is discarded. `false` when the
    /// slot is not in this view or the document is gone.
    pub(crate) fn swap_inline_host(&mut self, slot: &Rc<Slot>, view_host: bool) -> bool {
        fn find<'a>(desc: &'a mut ViewDesc, slot: &Rc<Slot>) -> Option<&'a mut ViewDesc> {
            if desc.slot.as_ref().is_some_and(|s| Rc::ptr_eq(s, slot)) {
                return Some(desc);
            }
            if !desc.has_view {
                return None;
            }
            desc.children.iter_mut().find_map(|c| find(c, slot))
        }
        let doc = self.doc.clone();
        let Some(desc) = find(&mut self.root, slot) else {
            return false;
        };
        let tag = if view_host {
            "span".to_string()
        } else {
            node_dom_tag(&desc.node)
        };
        let Some(el) = create_element(&doc, &tag) else {
            return false;
        };
        if view_host {
            apply_view_host_attrs(&el, &desc.node, None);
        } else {
            apply_element_attrs(&el, &desc.node, None);
        }
        let old = desc.dom.clone();
        // In place: its mark wrappers (`<a>`) stay around it.
        old.replace_with(&el);
        desc.dom = el.clone();
        old.discard();
        slot.set_host(el, view_host);
        // The box the caret and the node outline are drawn from moved.
        self.view_boxes.clear();
        true
    }

    /// The slot of the node starting at `pos`, if it has one.
    pub(crate) fn slot_at(&self, doc: &Node, pos: Pos) -> Option<Rc<Slot>> {
        let r = doc.resolve(pos).ok()?;
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        desc.children.get(r.index(r.depth()))?.slot.clone()
    }

    /// The slot whose host is the element `host_id`.
    pub(crate) fn slot_by_host(&self, host_id: usize) -> Option<Rc<Slot>> {
        self.slots
            .iter()
            .find(|s| s.host().node_id().0 == host_id)
            .cloned()
    }

    /// Whether `pos` names a node with a mounted view.
    pub(crate) fn has_mounted_view_at(&self, doc: &Node, pos: Pos) -> bool {
        self.slot_at(doc, pos).is_some_and(|s| s.is_mounted())
    }

    /// Follow the selection onto the node-view slot it node-selects.
    fn sync_selected_slot(&mut self, state: &EditorState) {
        if self.slots.is_empty() && self.selected_slot.is_none() {
            return;
        }
        let now = match &state.selection {
            Selection::Node(n) => self.slot_at(&state.doc, n.anchor),
            _ => None,
        };
        let same = match (&now, &self.selected_slot) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if let Some(old) = self.selected_slot.take()
            && old.set_selected(false)
        {
            self.node_view_work.push(Work::Flags(vec![old]));
        }
        if let Some(new) = &now
            && new.set_selected(true)
        {
            self.node_view_work.push(Work::Flags(vec![new.clone()]));
        }
        self.selected_slot = now;
    }

    /// Whether a mounted view's host box changed since the last call.
    fn node_view_hosts_moved(&mut self) -> bool {
        if self.view_boxes.is_empty() && !self.slots.iter().any(|s| s.is_mounted()) {
            return false;
        }
        let Some(doc) = self.doc.upgrade() else {
            return false;
        };
        let boxes: Vec<(usize, (i32, i32, i32, i32))> = {
            let d = doc.borrow();
            self.slots
                .iter()
                .filter(|s| s.is_mounted())
                .filter_map(|s| {
                    let id = s.host().node_id().0;
                    let (x, y, w, h) = d.query_node_layout(id as u64)?;
                    Some((
                        id,
                        (
                            x.round() as i32,
                            y.round() as i32,
                            w.round() as i32,
                            h.round() as i32,
                        ),
                    ))
                })
                .collect()
        };
        let moved = boxes != self.view_boxes;
        self.view_boxes = boxes;
        moved
    }

    /// The host id of the editor container (the `doc` node's element).
    pub(crate) fn container_id(&self) -> usize {
        self.root.dom.node_id().0
    }

    /// The host document's [`doc_key`](rinch_core::dom::DomDocument::doc_key),
    /// or 0 if the document is gone. Scopes the blink target — container ids
    /// collide across documents on one thread (issue #134).
    pub(crate) fn doc_key(&self) -> u64 {
        self.doc
            .upgrade()
            .map(|d| d.borrow().doc_key())
            .unwrap_or(0)
    }

    /// Switch the editor's color scheme by setting `data-pm-theme` on the container,
    /// which the default stylesheet's dark rules key off of (see
    /// [`styles`](super::styles)).
    pub(crate) fn set_dark_mode(&self, dark: bool) {
        self.root
            .dom
            .set_attribute("data-pm-theme", if dark { "dark" } else { "light" });
    }

    /// Mark the container read-only (`data-pm-readonly="true"`) or clear the mark.
    /// The attribute is a styling hook and nothing more — the default stylesheet
    /// hides the placeholder under it — and refusing edits is
    /// [`EditorHandle::set_read_only`](super::EditorHandle::set_read_only)'s job,
    /// done on the model. Present-or-absent rather than `"true"`/`"false"`, like
    /// `readonly` on an `<input>`, so `[data-pm-readonly]` is the whole selector.
    pub(crate) fn set_read_only(&self, read_only: bool) {
        if read_only {
            self.root.dom.set_attribute("data-pm-readonly", "true");
        } else {
            self.root.dom.remove_attribute("data-pm-readonly");
        }
    }

    /// Resolve a model [`Pos`] to its host caret address `(textblock element id,
    /// flat UTF-8 byte offset)` — the address app-side geometry (caret point,
    /// vertical movement) queries Parley with. `None` if `pos` isn't in a textblock.
    pub(crate) fn caret_address(&self, doc: &Node, pos: Pos) -> Option<(usize, usize)> {
        self.caret_target(doc, pos)
            .map(|(dom, byte)| (dom.node_id().0, byte))
    }

    /// Set the side of a soft wrap the next caret pass draws the caret on.
    pub(crate) fn set_caret_affinity(&mut self, affinity: CaretAffinity) {
        self.caret_affinity = affinity;
    }

    /// The on-screen caret for `pos` as `(x, y, height)`, in the host's popup
    /// frame ([`DomDocument::query_caret_rect_with_affinity`]), drawn on
    /// `affinity`'s side of a soft wrap. `None` when `pos` is not in a
    /// textblock, the host document is gone or busy, or the block has no box.
    pub(crate) fn caret_rect(
        &self,
        doc: &Node,
        pos: Pos,
        affinity: CaretAffinity,
        stored: Option<&[Mark]>,
    ) -> Option<(f32, f32, f32)> {
        let (block, byte) = self.caret_target(doc, pos)?;
        let host = self.doc.upgrade()?;
        // A soft borrow: an app may ask from a callback while a runtime holds
        // the document; that answers "no geometry" rather than panicking.
        let host = host.try_borrow().ok()?;
        let block_id = block.node_id().0 as u64;
        for (leaf, after) in self
            .inline_boxes_beside(doc, pos, affinity)
            .into_iter()
            .flatten()
        {
            if let Some(rect) = host.query_inline_box_caret_rect(block_id, leaf as u64, after) {
                return Some(rect);
            }
        }
        let shift = self.shared_exit_width(&*host, doc, pos, block_id, stored);
        let shifted = |(x, y, h): (f32, f32, f32)| (x + shift, y, h);
        for (element, end, after) in self.mark_edges_beside(doc, pos, stored, affinity) {
            if let Some(rect) =
                host.query_inline_edge_caret_rect(block_id, element as u64, end, after)
            {
                return Some(shifted(rect));
            }
        }
        host.query_caret_rect_with_affinity(block_id, byte, affinity)
            .map(shifted)
    }

    /// The inline leaves whose box a caret at `pos` can be drawn against, in
    /// the order to try them, as `(their host element id, after)`, when the
    /// flat byte offset cannot place it (#1104): for a `Downstream` caret the
    /// zero-byte leaf (an image) just after `pos` (the caret is before it),
    /// then the one just before `pos` (the caret after it); for an `Upstream`
    /// caret only the one before. Downstream tries the leaf after first
    /// because between two images a soft wrap separates, the caret before the
    /// second is the lower line's start (#1116); on one line the two edges
    /// are one x. The caller takes the first whose box the host answers for:
    /// the web answers a node view's host span and no `<img>`, so between a
    /// claimed image and an unclaimed one the caret is the span's right edge.
    ///
    /// The byte's own caret is the leading edge of whatever follows the
    /// image, so it is drawn after the image at a block's start, right after
    /// a hard break, and where the image starts a soft-wrapped line; on one
    /// line after text the image's left edge is the byte's caret anyway. An
    /// `Upstream` caret before an image at a wrap is the upper line's end,
    /// which the byte query draws. Both `None` anywhere else.
    ///
    /// Two images in a row share one byte with all four of their sides, which
    /// is why this names the leaf rather than a side of a byte.
    pub(crate) fn inline_boxes_beside(
        &self,
        doc: &Node,
        pos: Pos,
        affinity: CaretAffinity,
    ) -> [Option<(usize, bool)>; 2] {
        let none = [None, None];
        let Ok(r) = doc.resolve(pos) else {
            return none;
        };
        if !r.parent().is_textblock() {
            return none;
        }
        let mut desc = &self.root;
        for d in 0..r.depth() {
            match desc.children.get(r.index(d)) {
                Some(c) => desc = c,
                None => return none,
            }
        }
        let off = r.parent_offset();
        let block = &desc.node;
        let zero_byte_leaf = |n: &Node| n.text().is_none() && leaf_flat_bytes(n, self.flat) == 0;
        let leaf = |i: usize, after: bool| {
            zero_byte_leaf(block.child(i))
                .then(|| desc.children.get(i).map(|c| (c.dom.node_id().0, after)))
                .flatten()
        };
        // The child index at `off`, and the one before it.
        let mut at = 0usize;
        let mut before: Option<usize> = None;
        let mut next: Option<usize> = None;
        for i in 0..block.child_count() {
            if at == off {
                next = Some(i);
                break;
            }
            if at > off {
                return none; // inside a text run
            }
            at += block.child(i).node_size();
            before = Some(i);
        }
        if next.is_none() && at != off {
            return none;
        }
        let after_leaf = next
            .filter(|_| affinity == CaretAffinity::Downstream)
            .and_then(|i| leaf(i, false));
        let before_leaf = before.and_then(|i| leaf(i, true));
        match after_leaf {
            Some(a) => [Some(a), before_leaf],
            None => [before_leaf, None],
        }
    }

    /// The mark wrapper edges a caret at `pos` is drawn against, as `(the
    /// wrapper's host element id, end, after)` in the order to try them
    /// (#1426): the caret goes on the `after` (trailing) or leading side of
    /// that wrapper's start or end edge. Empty when the byte's caret stands.
    ///
    /// A mark wrapper's horizontal padding, border and margin take room on
    /// the line but hold no flat bytes, so at a run boundary one byte names
    /// every side of them. The caret is drawn **where the next typed
    /// character lands**, which is decided by the marks it takes — `stored`
    /// when the caret has stored marks, else [`ResolvedPos::marks`]:
    ///
    /// - the same marks as the text run before `pos`: it joins that run, so
    ///   the caret is inside its wrappers — before their end edges (the
    ///   innermost first) — or, with none, before the run after's start
    ///   edges;
    /// - else the same marks as the text run after `pos`: it joins that one,
    ///   so the caret is past the run after's start edges (the innermost
    ///   last), or with none past the run before's end edges;
    /// - else it makes a run of its own between them: past the run before's
    ///   end edges, before the run after's start edges.
    ///
    /// A wrapper with no padding has no edge and the host answers `None` for
    /// it, so each side lists every wrapper, the one to use first.
    /// An `Upstream` caret (the upper line's end at a soft wrap) is never
    /// placed against the run after's start edges, which are on the lower
    /// line.
    ///
    /// [`ResolvedPos::marks`]: rinch_editor_core::ResolvedPos::marks
    pub(crate) fn mark_edges_beside(
        &self,
        doc: &Node,
        pos: Pos,
        stored: Option<&[Mark]>,
        affinity: CaretAffinity,
    ) -> Vec<(usize, bool, bool)> {
        let Some((desc, before, next, marks_here)) = self.seam(doc, pos) else {
            return Vec::new();
        };
        let block = &desc.node;
        // A child's wrapper elements, innermost first — only the ones that
        // end (`before`) or start (`next`) at this seam: a wrapper the two
        // runs share (#1833) has its edges elsewhere, around both.
        let wraps_of = |i: Option<usize>| -> &[(Mark, NodeHandle)] {
            i.and_then(|i| desc.children.get(i))
                .map_or(&[], |c| c.wraps.as_slice())
        };
        let wrappers = |i: Option<usize>, other: Option<usize>| -> Vec<usize> {
            let shared = wraps_of(other);
            wraps_of(i)
                .iter()
                .rev()
                .map(|(_, w)| w.node_id().0)
                .filter(|id| !shared.iter().any(|(_, s)| s.node_id().0 == *id))
                .collect()
        };
        // An `Upstream` caret is the upper line's end at a soft wrap: the
        // run after's start edges are on the lower line, so only the run
        // before's end edges can place it.
        let starts = if affinity == CaretAffinity::Upstream {
            Vec::new()
        } else {
            wrappers(next, before)
        };
        let typed = stored.map_or(marks_here, <[Mark]>::to_vec);
        let joins = |i: Option<usize>| {
            i.map(|i| block.child(i))
                .is_some_and(|n| n.is_text() && n.marks() == typed.as_slice())
        };
        let ends = wrappers(before, next);
        // Leading side of the run before's end edges, innermost first.
        let inside_before = ends.iter().map(|&e| (e, true, false));
        // Trailing side of its end edges, outermost first.
        let past_before = ends.iter().rev().map(|&e| (e, true, true));
        // Leading side of the run after's start edges, outermost first.
        let before_after = starts.iter().rev().map(|&e| (e, false, false));
        // Trailing side of its start edges, innermost first.
        let inside_after = starts.iter().map(|&e| (e, false, true));
        if joins(before) {
            inside_before.chain(before_after).collect()
        } else if joins(next) {
            inside_after.chain(past_before).collect()
        } else {
            past_before.chain(before_after).collect()
        }
    }

    /// The textblock descriptor holding `pos`, the children before and after
    /// it when `pos` is between two children (or at an end), and the marks a
    /// caret there carries ([`ResolvedPos::marks`]). `None` inside a text run
    /// or outside a textblock.
    ///
    /// [`ResolvedPos::marks`]: rinch_editor_core::ResolvedPos::marks
    #[allow(clippy::type_complexity)]
    fn seam(
        &self,
        doc: &Node,
        pos: Pos,
    ) -> Option<(&ViewDesc, Option<usize>, Option<usize>, Vec<Mark>)> {
        let r = doc.resolve(pos).ok()?;
        if !r.parent().is_textblock() {
            return None;
        }
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        let off = r.parent_offset();
        let block = &desc.node;
        let mut at = 0usize;
        let mut before: Option<usize> = None;
        let mut next: Option<usize> = None;
        for i in 0..block.child_count() {
            if at == off {
                next = Some(i);
                break;
            }
            if at > off {
                return None; // inside a text run
            }
            at += block.child(i).node_size();
            before = Some(i);
        }
        if next.is_none() && at != off {
            return None;
        }
        Some((desc, before, next, r.marks()))
    }

    /// The wrappers the runs on both sides of `pos` share (#1833) whose mark a
    /// character typed at `pos` would not carry (`stored`, else the marks
    /// there): the typed character makes a run of its own outside them, past
    /// their end edges, though at the seam no edge of theirs stands yet. The
    /// caret is drawn that far further on (review of #1862), so it stays where
    /// a typed character lands, as it did when each run had wrappers of its
    /// own. Inside one run the same toggle is not handled (#1426's caret
    /// stands at the byte there).
    pub(crate) fn shared_wrappers_left(
        &self,
        doc: &Node,
        pos: Pos,
        stored: Option<&[Mark]>,
    ) -> Vec<usize> {
        let Some((desc, Some(before), Some(next), marks_here)) = self.seam(doc, pos) else {
            return Vec::new();
        };
        let typed = stored.map_or(marks_here, <[Mark]>::to_vec);
        let (Some(b), Some(n)) = (desc.children.get(before), desc.children.get(next)) else {
            return Vec::new();
        };
        b.wraps
            .iter()
            .filter(|(m, w)| {
                !typed.contains(m) && n.wraps.iter().any(|(_, x)| x.node_id() == w.node_id())
            })
            .map(|(_, w)| w.node_id().0)
            .collect()
    }

    /// How far past the seam a caret at `pos` is drawn for
    /// [`Self::shared_wrappers_left`]: the width of each such wrapper's end
    /// edge (its padding, border and margin), as the host measures it.
    fn shared_exit_width(
        &self,
        d: &dyn DomDocument,
        doc: &Node,
        pos: Pos,
        block_id: u64,
        stored: Option<&[Mark]>,
    ) -> f32 {
        self.shared_wrappers_left(doc, pos, stored)
            .into_iter()
            .filter_map(|w| {
                let lead = d.query_inline_edge_caret(block_id, w as u64, true, false)?;
                let trail = d.query_inline_edge_caret(block_id, w as u64, true, true)?;
                Some(trail.0 - lead.0)
            })
            .sum()
    }

    /// The first of [`Self::inline_boxes_beside`]: the leaf a host that
    /// answers every leaf's box (rinch-dom) draws the caret against.
    pub(crate) fn inline_box_beside(
        &self,
        doc: &Node,
        pos: Pos,
        affinity: CaretAffinity,
    ) -> Option<(usize, bool)> {
        self.inline_boxes_beside(doc, pos, affinity)[0]
    }

    /// Whether an IME composition (preedit) is being shown — the input method
    /// owns the keyboard until it commits or cancels.
    pub(crate) fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// Resolve a model [`Pos`] to the host caret address `(textblock element, flat
    /// UTF-8 byte offset)` for the Parley layout query (the A15 char→byte IFC map).
    /// `None` when the position is not inside a textblock.
    fn caret_target(&self, doc: &Node, pos: Pos) -> Option<(NodeHandle, usize)> {
        let r = doc.resolve(pos).ok()?;
        if !r.parent().is_textblock() {
            return None;
        }
        // Walk the descriptor tree to the textblock that owns `pos`, mirroring the
        // resolved path's child indices.
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        let flat_byte = textblock_flat_byte(&desc.node, r.parent_offset(), self.flat);
        let generated = self.generated_ranges(desc.dom.node_id().0);
        Some((desc.dom.clone(), host_flat_byte(&generated, flat_byte)))
    }

    /// The stretches of textblock `block_id`'s host flat bytes that hold CSS
    /// generated content ([`DomDocument::generated_flat_ranges`], #891), which
    /// the model's text does not: [`host_flat_byte`] steps over them. None
    /// when the host is gone or busy (a document being written answers no
    /// geometry either).
    fn generated_ranges(&self, block_id: usize) -> Vec<GeneratedFlatRange> {
        let Some(host) = self.doc.upgrade() else {
            return Vec::new();
        };
        let Ok(d) = host.try_borrow() else {
            return Vec::new();
        };
        d.generated_flat_ranges(block_id as u64)
    }

    /// Map a host caret address — `(textblock element id, flat UTF-8 byte offset)`,
    /// as produced by a pointer hit-test — back to a model [`Pos`] (the inverse of
    /// [`Self::caret_target`]). `None` if `textblock_dom_id` isn't a known
    /// textblock in this view.
    pub(crate) fn pos_at(&self, textblock_dom_id: usize, ifc_byte: usize) -> Option<Pos> {
        let (content_start, block) = find_block(&self.root, textblock_dom_id, 0)?;
        let generated = self.generated_ranges(textblock_dom_id);
        let ifc_byte = model_flat_byte(&generated, ifc_byte);
        Some(Pos(
            content_start + ifc_byte_to_char(block, ifc_byte, self.flat)
        ))
    }

    /// The inverse of [`Self::pos_in_textblock`]: the textblock holding `pos` (its
    /// host element id) and `pos`'s offset into that block's content, a leaf
    /// counting one. `None` when `pos` is not inside a textblock.
    pub(crate) fn textblock_offset(&self, doc: &Node, pos: Pos) -> Option<(usize, usize)> {
        let r = doc.resolve(pos).ok()?;
        if !r.parent().is_textblock() {
            return None;
        }
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        Some((desc.dom.node_id().0, r.parent_offset()))
    }

    /// The model position on `after`'s side of the inline leaf whose host
    /// element is `leaf_dom_id` — before it, or after it when `after`. `None`
    /// if that element is not an inline leaf in this view. What a pointer hit
    /// on an image's half resolves to (#1104), since the flat byte offset of
    /// [`Self::pos_at`] is the same number on both of its sides.
    pub(crate) fn pos_beside_inline_leaf(&self, leaf_dom_id: usize, after: bool) -> Option<Pos> {
        let (pos, node) = self.node_pos_for_host(leaf_dom_id)?;
        (node.is_inline() && node.node_type().is_leaf()).then_some(Pos(pos + usize::from(after)))
    }

    /// The model [`Pos`] `offset` positions into the textblock whose host element
    /// is `textblock_dom_id` — a text character and an inline leaf (image, hard
    /// break) each count one, as in the model. Clamped to the block's end. `None`
    /// if `textblock_dom_id` isn't a known textblock in this view.
    ///
    /// Unlike a flat byte offset ([`Self::pos_at`]), this names a side of a leaf:
    /// a leaf is one position wide and, in the browser (and for an image on
    /// rinch-dom), zero text bytes wide, so the byte offset just before it and
    /// the one just after it are the same number (#1025).
    pub(crate) fn pos_in_textblock(&self, textblock_dom_id: usize, offset: usize) -> Option<Pos> {
        let (content_start, block) = find_block(&self.root, textblock_dom_id, 0)?;
        Some(Pos(content_start + offset.min(block.content().size())))
    }

    /// The textblock's offset in container coordinates — the sum of parent-relative
    /// layouts from the block up to (excluding) the container.
    fn block_offset_in_container(&self, d: &dyn DomDocument, block_id: usize) -> (f32, f32) {
        let container_id = self.root.dom.node_id().0;
        let (mut x, mut y) = (0.0f32, 0.0f32);
        let mut cur = Some(block_id);
        while let Some(id) = cur {
            if id == container_id {
                break;
            }
            if let Some((nx, ny, _, _)) = d.query_node_layout(id as u64) {
                x += nx;
                y += ny;
            }
            cur = d.parent_node(rinch_core::dom::NodeId(id)).map(|n| n.0);
        }
        // The overlays are absolutely-positioned children of the container, so they
        // anchor to its *padding* box; the summed offsets are border-box-relative.
        // Subtract the container's border inset once so caret/selection land on glyphs
        // (both renderers report it).
        let (ix, iy) = d.content_origin_inset(container_id as u64);
        (x - ix, y - iy)
    }

    /// The caret overlay's container-space `x`, kept inside textblock
    /// `block_id`'s border box with the caret's whole [`CARET_WIDTH`] (#1041).
    ///
    /// Preserved spaces that overflow a line hang past its end (#1018), and
    /// the caret after them is at their true x, past the block: clipped away
    /// under the editor's `overflow: auto`, painted outside the editor
    /// without it. Chrome 155 draws its caret there at the block's border-box
    /// edge less the caret's width (padding and border included), whatever
    /// the block's `overflow` (measured with headless screenshots; a block
    /// that clips at its padding box hides a caret clamped into its right
    /// border, which the overlay, a child of the editor and not of the block,
    /// does not). Only the overlay is kept in:
    /// [`EditorHandle::caret_rect`](crate::EditorHandle::caret_rect), the
    /// reveal probes and the vertical motions keep the text's own x, as a
    /// collapsed `Range`'s rect does in the browser, so Up then Down from a
    /// line's end comes back to it. The selection highlight is not clamped
    /// either, in Chrome or here.
    ///
    /// [`CARET_WIDTH`]: rinch_core::dom::CARET_WIDTH
    fn keep_caret_in_block(&self, d: &dyn DomDocument, block_id: usize, x: f32) -> f32 {
        let Some((_, _, w, _)) = d.query_node_layout(block_id as u64) else {
            return x;
        };
        if w <= 0.0 {
            return x;
        }
        let (left, _) = self.block_offset_in_container(d, block_id);
        let right = left + w - rinch_core::dom::CARET_WIDTH;
        if (left..=right).contains(&x) {
            return x;
        }
        // The left arm is for right-to-left text hanging its spaces past the
        // start edge (Chrome 155 keeps that caret at the border-box left). Not
        // reachable yet: rinch-dom reads no CSS `direction`, so no editor line
        // hangs to the left, and nothing tests it.
        if x > right { right.max(left) } else { left }
    }

    /// The caret for `pos` — in textblock `block_id` at `flat_byte`, as
    /// [`Self::caret_target`] resolved it — as `(x, y, height)` in the
    /// container's coordinate space: Parley's caret, or an empty block's content
    /// origin. `None` when the block has no layout yet (desktop: edited since the
    /// last layout, or collapsed by virtualization). The caret overlay and the
    /// reveal probes ([`Self::position_reveal`]) are both placed from this.
    #[allow(clippy::too_many_arguments)]
    fn caret_geometry(
        &self,
        d: &dyn DomDocument,
        doc: &Node,
        pos: Pos,
        block_id: usize,
        flat_byte: usize,
        affinity: CaretAffinity,
        stored: Option<&[Mark]>,
    ) -> Option<(f32, f32, f32)> {
        for (leaf, after) in self
            .inline_boxes_beside(doc, pos, affinity)
            .into_iter()
            .flatten()
        {
            if let Some((x, y, height)) =
                d.query_inline_box_caret(block_id as u64, leaf as u64, after)
            {
                let (ox, oy) = self.block_offset_in_container(d, block_id);
                return Some((ox + x, oy + y, height));
            }
        }
        // #1833: past the end edges of the shared wrappers a typed
        // character would leave.
        let shift = self.shared_exit_width(d, doc, pos, block_id as u64, stored);
        // #1426: past a mark wrapper's padding, on the side its marks say.
        for (element, end, after) in self.mark_edges_beside(doc, pos, stored, affinity) {
            if let Some((x, y, height)) =
                d.query_inline_edge_caret(block_id as u64, element as u64, end, after)
            {
                let (ox, oy) = self.block_offset_in_container(d, block_id);
                return Some((ox + x + shift, oy + y, height));
            }
        }
        let from_parley = d
            .query_caret_position_with_affinity(block_id as u64, flat_byte, affinity)
            .map(|(local_x, local_y)| {
                let height = d
                    .query_glyph_bounds(block_id as u64, flat_byte)
                    .map(|g| g.height)
                    .unwrap_or(18.0);
                let (ox, oy) = self.block_offset_in_container(d, block_id);
                (ox + local_x + shift, oy + local_y, height)
            });
        from_parley.or_else(|| self.empty_block_caret(d, doc, pos, block_id))
    }

    /// Place the reveal probes over `from..to` and answer them in the order to
    /// scroll them into view: the end first, then the start. Each is a hidden,
    /// 1px-wide box over the caret line at that position, grown by
    /// [`REVEAL_MARGIN`] above and below.
    ///
    /// Two "nearest" scrolls in that order leave the start in view and as much
    /// of the range after it as fits: the first brings the end in, and the
    /// second moves only if that pushed the start out, in which case the start
    /// lands at the top edge. One box around the whole range would not do it —
    /// a "nearest" scroll to a box taller than the scroller aligns its *bottom*
    /// when it lies above, which hides the start.
    ///
    /// A position between blocks is revealed at the nearest text position. The
    /// margin is clamped to the document's own extent (the container's top and
    /// the last block's bottom), so a probe never makes the scroller's content
    /// taller than the document.
    ///
    /// With [`ScrollAlign::Fraction`] there is one probe, over the start's
    /// caret line and not grown: the scroll places that box itself
    /// ([`NodeHandle::scroll_to_fraction`]), margin included.
    ///
    /// `None`, leaving the probes where they were, when either end has no
    /// geometry yet — a block edited since the last layout, or never laid out.
    pub(crate) fn position_reveal(
        &mut self,
        doc: &Node,
        from: Pos,
        to: Pos,
        align: ScrollAlign,
    ) -> Option<Vec<NodeHandle>> {
        let size = doc.content_size();
        let (from, to) = (Pos(from.0.min(size)), Pos(to.0.min(size)));
        let placed = matches!(align, ScrollAlign::Fraction(_));
        let ends: &[(Pos, i32)] = if from == to || placed {
            &[(from, 1)]
        } else {
            &[(to, -1), (from, 1)]
        };
        let margin = if placed { 0.0 } else { REVEAL_MARGIN };
        let host = self.doc.upgrade()?;
        let boxes: Vec<(f32, f32, f32)> = {
            let d = host.try_borrow().ok()?;
            let bottom = self
                .root
                .children
                .last()
                .and_then(|last| {
                    let id = last.dom.node_id().0;
                    let (_, _, _, h) = d.query_node_layout(id as u64)?;
                    let (_, y) = self.block_offset_in_container(&*d, id);
                    Some(y + h)
                })
                .unwrap_or(f32::INFINITY);
            let mut out = Vec::with_capacity(ends.len());
            for (i, &(pos, bias)) in ends.iter().enumerate() {
                let geometry = (|| {
                    let (block, byte, pos) = match self.caret_target(doc, pos) {
                        Some((block, byte)) => (block, byte, pos),
                        None => {
                            let near = Selection::near_text(doc, pos, bias)?.head();
                            let (block, byte) = self.caret_target(doc, near)?;
                            (block, byte, near)
                        }
                    };
                    self.caret_geometry(
                        &*d,
                        doc,
                        pos,
                        block.node_id().0,
                        byte,
                        CaretAffinity::Downstream,
                        None,
                    )
                })();
                // The start (always last) is required. The end of a range is
                // not: in a virtualized editor it can sit in a block that is
                // still collapsed to its estimated height, with no caret to
                // measure, and waiting for it would leave the reveal pending
                // for good (#922's review, D3). The start alone is revealed
                // then — the start is what a reveal promises first anyway.
                let Some((x, y, h)) = geometry else {
                    if i + 1 < ends.len() {
                        continue;
                    }
                    return None;
                };
                let top = (y - margin).max(0.0).min(y);
                let end = (y + h + margin).min(bottom).max(y + h);
                out.push((x, top, end - top));
            }
            out
        };
        while self.reveal_probes.len() < boxes.len() {
            let probe = create_element(&self.doc, "div")?;
            probe.set_attribute("data-pm-reveal", "true");
            probe.set_attribute("aria-hidden", "true");
            probe.set_styles(&[
                ("position", "absolute"),
                ("width", "1px"),
                ("visibility", "hidden"),
                ("pointer-events", "none"),
            ]);
            self.root.dom.append_child(&probe);
            self.reveal_probes.push(probe);
        }
        for (probe, (x, top, height)) in self.reveal_probes.iter().zip(&boxes) {
            let left = format!("{x}px");
            let top = format!("{top}px");
            let height = format!("{height}px");
            probe.set_styles(&[("left", &left), ("top", &top), ("height", &height)]);
        }
        Some(self.reveal_probes[..boxes.len()].to_vec())
    }

    /// Whether the host document has changes its geometry does not show yet
    /// ([`DomDocument::layout_pending`]); `None` when it is gone or busy.
    pub(crate) fn layout_pending(&self) -> Option<bool> {
        let host = self.doc.upgrade()?;
        let d = host.try_borrow().ok()?;
        Some(d.layout_pending())
    }

    /// What an anchor at `pos` is measured on: a caret in a textblock (the
    /// block's host id, the caret's flat byte, and the position the caret
    /// target resolved for), or a leaf block's host (`pos` right before a
    /// rule, an embed, a node view). Anywhere else between blocks, the
    /// nearest text position after it. `None` when nothing there.
    fn margin_target(&self, doc: &Node, pos: Pos) -> Option<MarginTarget> {
        let pos = Pos(pos.0.min(doc.content_size()));
        if let Some((block, byte)) = self.caret_target(doc, pos) {
            return Some(MarginTarget::Text(block.node_id().0, byte, pos));
        }
        let r = doc.resolve(pos).ok()?;
        if r.node_after().is_some_and(|n| n.is_leaf() && n.is_block()) {
            return self.node_host_at(doc, pos).map(MarginTarget::Leaf);
        }
        let near = Selection::near_text(doc, pos, 1)?.head();
        let (block, byte) = self.caret_target(doc, near)?;
        Some(MarginTarget::Text(block.node_id().0, byte, near))
    }

    /// What the margin pass's lines for `positions` are read from, hashed so
    /// a later pass can compare: for every block an anchor is measured on, its
    /// offset in the container, its box, its padding and border, and — where
    /// the host keeps one ([`DomDocument::text_layout_generation`]) — the
    /// stamp of its text layout; and the first top-level block's box (the
    /// column edge). Equal keys for the same document mean no anchor's line
    /// can have moved, so the pass measures nothing: a layout of anything
    /// else (a spinner, a hover, a label's text, another editor) costs this
    /// key, `O(anchored blocks × depth)`, not a measure per anchor. A line in
    /// a nested block (a table cell's paragraph) is covered: its offset is
    /// summed up the chain. On a host with no stamps (the browser) a rewrap
    /// that moves no anchored block's box is not seen until the next change
    /// that does. `None` when the host is gone or busy.
    pub(crate) fn margin_key(&self, doc: &Node, positions: &[Pos]) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let host = self.doc.upgrade()?;
        let d = host.try_borrow().ok()?;
        let d: &dyn DomDocument = &*d;
        let mut ids: Vec<usize> = positions
            .iter()
            .filter_map(|&p| match self.margin_target(doc, p)? {
                MarginTarget::Text(id, _, _) | MarginTarget::Leaf(id) => Some(id),
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if let Some(first) = self.root.children.first() {
            ids.push(first.dom.node_id().0);
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for id in ids {
            id.hash(&mut h);
            let (x, y) = self.block_offset_in_container(d, id);
            let (_, _, w, bh) = d.query_node_layout(id as u64).unwrap_or_default();
            let (l, t, r, b) = d.content_box_inset(id as u64);
            for v in [x, y, w, bh, l, t, r, b] {
                v.to_bits().hash(&mut h);
            }
            d.text_layout_generation(id as u64).hash(&mut h);
        }
        Some(h.finish())
    }

    /// The line box `(top, height)` each of `positions` sits in, in the frame
    /// the overlays are placed in (the container's padding box, unscrolled),
    /// and the text column's right edge in the same frame: the right edge of
    /// the first laid-out top-level block. What
    /// [`EditorHandle::set_margin_anchors`](super::EditorHandle::set_margin_anchors)
    /// reports, in one pass over the anchors.
    ///
    /// A position in a textblock is measured on the line a caret there is
    /// drawn on (its downstream side at a soft wrap): the block's carets go to
    /// [`DomDocument::query_line_boxes`] together, so a backend reads each
    /// block's lines once per pass; a caret it cannot answer gets the caret's
    /// rect grown to the block's [`line_box_height`](DomDocument::line_box_height)
    /// about its middle. A position right before a leaf block (a horizontal
    /// rule, an embed, a node view's host) is that block's box; anywhere else
    /// between blocks, the nearest text position after it. Positions in one
    /// textblock whose boxes overlap are one line and get one box (their
    /// union). `None` for a position with no geometry: its block is not laid
    /// out (virtualized), or the host is gone or busy.
    pub(crate) fn measure_lines(
        &self,
        doc: &Node,
        positions: &[Pos],
    ) -> (Vec<Option<(f32, f32)>>, Option<f32>) {
        let mut out: Vec<Option<(f32, f32)>> = vec![None; positions.len()];
        let Some(host) = self.doc.upgrade() else {
            return (out, None);
        };
        let Ok(d) = host.try_borrow() else {
            return (out, None);
        };
        let d: &dyn DomDocument = &*d;
        // (index, block id, top, height), merged per block below.
        let mut found: Vec<(usize, usize, f32, f32)> = Vec::with_capacity(positions.len());
        // The carets, grouped by their textblock.
        let mut by_block: HashMap<usize, Vec<(usize, usize, Pos)>> = HashMap::new();
        for (i, &pos) in positions.iter().enumerate() {
            match self.margin_target(doc, pos) {
                Some(MarginTarget::Text(block, byte, at)) => {
                    by_block.entry(block).or_default().push((i, byte, at));
                }
                Some(MarginTarget::Leaf(id)) => {
                    if let Some((_, _, w, h)) = d.query_node_layout(id as u64)
                        && (w != 0.0 || h != 0.0)
                    {
                        let (_, oy) = self.block_offset_in_container(d, id);
                        found.push((i, id, oy, h));
                    }
                }
                None => {}
            }
        }
        for (block, carets) in by_block {
            let (_, oy) = self.block_offset_in_container(d, block);
            let asked: Vec<(usize, CaretAffinity)> = carets
                .iter()
                .map(|&(_, byte, _)| (byte, CaretAffinity::Downstream))
                .collect();
            let boxes = d.query_line_boxes(block as u64, &asked);
            let mut line_height: Option<Option<f32>> = None;
            for (&(i, byte, at), line) in carets.iter().zip(boxes) {
                let geometry = match line {
                    Some((top, h)) => Some((oy + top, h)),
                    None => self
                        .caret_geometry(d, doc, at, block, byte, CaretAffinity::Downstream, None)
                        .map(|(_, y, h)| {
                            let lh =
                                *line_height.get_or_insert_with(|| d.line_box_height(block as u64));
                            match lh {
                                Some(lh) if lh > h => (y - (lh - h) / 2.0, lh),
                                _ => (y, h),
                            }
                        }),
                };
                if let Some((top, h)) = geometry {
                    found.push((i, block, top, h));
                }
            }
        }
        // One line, one box: in each block, boxes that overlap are merged.
        found.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)));
        let mut start = 0;
        while start < found.len() {
            let (block, mut top, h) = (found[start].1, found[start].2, found[start].3);
            let mut bottom = top + h;
            let mut end = start + 1;
            while end < found.len() && found[end].1 == block && found[end].2 < bottom - 0.5 {
                top = top.min(found[end].2);
                bottom = bottom.max(found[end].2 + found[end].3);
                end += 1;
            }
            for f in &found[start..end] {
                out[f.0] = Some((top, bottom - top));
            }
            start = end;
        }
        let right = self.root.children.iter().find_map(|c| {
            let id = c.dom.node_id().0;
            let (_, _, w, h) = d.query_node_layout(id as u64)?;
            if w == 0.0 && h == 0.0 {
                return None;
            }
            let (x, _) = self.block_offset_in_container(d, id);
            Some(x + w)
        });
        (out, right)
    }

    /// Show the drop caret at `at` — a model position and the side of a soft
    /// wrap to draw it on — or hide it (`None`). It is the caret bar drawn at
    /// that position (the geometry the caret overlay uses), in its own
    /// element (`data-pm-drop-caret`): the selection, the caret and the
    /// caret's blink are untouched. A position with no geometry yet (a block
    /// edited since the last layout) hides it.
    ///
    /// Answers whether anything was written, which the caller turns into a
    /// layout and a repaint; it does not set [`Self::overlay_dirty`], which
    /// the caret pass reads as "the caret moved".
    pub(crate) fn show_drop_caret(&mut self, doc: &Node, at: Option<(Pos, CaretAffinity)>) -> bool {
        let geometry = at.and_then(|(pos, affinity)| {
            let pos = Pos(pos.0.min(doc.content_size()));
            let host = self.doc.upgrade()?;
            let d = host.try_borrow().ok()?;
            let (block, byte) = self.caret_target(doc, pos)?;
            self.caret_geometry(&*d, doc, pos, block.node_id().0, byte, affinity, None)
        });
        let Some((x, y, height)) = geometry else {
            if self.last_drop_caret.take().is_none() {
                return false;
            }
            if let Some(bar) = &self.drop_caret {
                bar.set_style("visibility", "hidden");
            }
            return true;
        };
        let key = (x.round() as i32, y.round() as i32, height.round() as i32);
        if self.last_drop_caret == Some(key) {
            return false;
        }
        self.last_drop_caret = Some(key);
        if self.drop_caret.is_none() {
            let Some(bar) = create_element(&self.doc, "div") else {
                return false;
            };
            bar.set_attribute("data-pm-drop-caret", "true");
            bar.set_attribute("aria-hidden", "true");
            bar.set_styles(&[
                ("position", "absolute"),
                // Placed by `transform`, as the caret is (`overlay_translate`).
                ("left", "0"),
                ("top", "0"),
                // The width a caret query keeps inside its block (#1041).
                ("width", &format!("{}px", rinch_core::dom::CARET_WIDTH)),
                ("background-color", "#1a73e8"),
                ("pointer-events", "none"),
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&bar);
            self.drop_caret = Some(bar);
        }
        if let Some(bar) = &self.drop_caret {
            let transform = overlay_translate(x, y);
            let h = format!("{}px", overlay_size(x, y, 0.0, height).1);
            bar.set_styles(&[
                ("transform", &transform),
                ("height", &h),
                ("visibility", "visible"),
            ]);
        }
        true
    }

    /// Caret geometry for an *empty* textblock (which has no Parley layout, so
    /// [`DomDocument::query_caret_position`] returns `None`): the block's content
    /// origin in container space ([`DomDocument::content_box_inset`]), one
    /// line ([`DomDocument::line_box_height`]) tall. `None` if the block at `pos` is *not* actually empty (a non-empty
    /// block that merely failed to measure — leave the caret hidden rather than
    /// misplace it) or has no host box yet.
    ///
    /// A box that is 0 wide **and** 0 tall has not been laid out: the empty
    /// paragraph Enter just made is a fresh element whose box is the default
    /// until the next layout, and its "origin" is the top of the editor. A
    /// caret placed there was the scroll anchor the input path's caret pass
    /// fulfils the caret scroll with, so Enter at the end of a paragraph in a
    /// scrolled pane jumped the pane to the editor's top (desktop). `None`
    /// keeps the scroll owed until a pass after the layout has the real box,
    /// as for a textblock whose text was edited since the last layout.
    fn empty_block_caret(
        &self,
        d: &dyn DomDocument,
        doc: &Node,
        pos: Pos,
        block_id: usize,
    ) -> Option<(f32, f32, f32)> {
        let r = doc.resolve(pos).ok()?;
        if r.parent().content().size() != 0 {
            return None;
        }
        let (_, _, bw, bh) = d.query_node_layout(block_id as u64)?;
        if bw == 0.0 && bh == 0.0 {
            return None;
        }
        let (ox, oy) = self.block_offset_in_container(d, block_id);
        // Its content box, not its border box: an empty code block is padded,
        // and its caret goes where its text would start (#1667).
        let (l, t, _, b) = d.content_box_inset(block_id as u64);
        // One line box tall: the block's line height, else its content height
        // (its box need not be one line plus its padding: desktop sizes
        // `min-height: 1lh` as a border box, #1278); clamp to a sane range and
        // fall back to a default when it has collapsed to zero.
        let ch = d.line_box_height(block_id as u64).unwrap_or(bh - t - b);
        let h = if (4.0..=80.0).contains(&ch) { ch } else { 18.0 };
        Some((ox + l, oy + t, h))
    }

    /// Take and clear the "an overlay moved" flag. The runtime re-resolves layout
    /// and schedules a repaint when set, so the caret / selection / node-outline
    /// overlays are drawn at their new boxes (see [`Self::overlay_dirty`]).
    pub(crate) fn take_overlay_dirty(&mut self) -> bool {
        std::mem::take(&mut self.overlay_dirty)
    }

    /// The host element a [`ViewRequest::ScrollSelectionIntoView`] should bring into
    /// view: the overlay the *current* selection is drawn with. (Unrelated to
    /// [`SelectionAnchor`](super::SelectionAnchor), which captures a *position* for
    /// a later async edit.) Which element to scroll to is the view's knowledge, not
    /// the runtime's — the runtime only calls
    /// [`NodeHandle::scroll_into_view`] on whatever comes back (see
    /// [`EditorHandle::update_caret`](super::EditorHandle::update_caret)).
    ///
    /// - collapsed cursor → the caret bar,
    /// - [`Selection::Node`] → the node outline,
    /// - [`Selection::Cell`] → the wash rectangle over the *head* cell.
    ///
    /// `None` when nothing is drawn (no overlay yet, or the selection has no
    /// geometry). The overlays are container children positioned in container
    /// space, so scrolling one is scrolling the selection.
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    /// [`Selection::Cell`]: rinch_editor_core::Selection::Cell
    pub(crate) fn scroll_anchor(&self) -> Option<&NodeHandle> {
        if self.last_caret.is_some() {
            return self.caret.as_ref();
        }
        if self.last_node_outline.is_some() {
            return self.node_outline.as_ref();
        }
        let idx = self.cell_anchor_rect?;
        self.selection_rects.get(idx)
    }

    /// Render (or clear) the text-selection highlight from `state.selection`.
    fn render_selection(&mut self, state: &EditorState) {
        let sel = &state.selection;
        if sel.is_empty() {
            self.clear_selection_rects();
            return;
        }
        let key = (0u8, sel.from().0, sel.to().0);
        if self.last_selection == Some(key) {
            return;
        }
        self.last_selection = Some(key);
        let Some(doc) = self.doc.upgrade() else {
            return;
        };
        let targets = self.selection_targets(&state.doc, sel.from(), sel.to());
        let rects: Vec<(f32, f32, f32, f32)> = {
            let d = doc.borrow();
            let mut out = Vec::new();
            for (block_id, byte_a, byte_b, boxes) in targets {
                let (ox, oy) = self.block_offset_in_container(&*d, block_id);
                for (rx, ry, rw, rh) in
                    d.query_selection_rects_with_boxes(block_id as u64, byte_a, byte_b, &boxes)
                {
                    out.push((ox + rx, oy + ry, rw, rh));
                }
            }
            out
        };
        self.set_selection_rects(&rects);
    }

    /// For each textblock overlapping `[from, to)`, the host element id, the
    /// flat UTF-8 byte range of the selection within that block, and the host
    /// element ids of the zero-byte inline leaves (images) the selection holds
    /// — which the byte range cannot say: an image has no bytes, so the bytes
    /// on its two sides are one number (#1120).
    #[allow(clippy::type_complexity)]
    fn selection_targets(
        &self,
        doc: &Node,
        from: Pos,
        to: Pos,
    ) -> Vec<(usize, usize, usize, Vec<u64>)> {
        let mut targets = Vec::new();
        doc.nodes_between(from.0, to.0, &mut |node, pos, _parent| {
            if node.is_textblock() {
                let content_start = pos + 1;
                let content_end = content_start + node.content().size();
                let a = from.0.max(content_start);
                let b = to.0.min(content_end);
                if a < b
                    && let Some((block_id, _)) = self.caret_address(doc, Pos(content_start))
                {
                    // The host's bytes, past its generated content (#891),
                    // which the host leaves out of the highlight.
                    let generated = self.generated_ranges(block_id);
                    let host = |off: usize| {
                        host_flat_byte(&generated, textblock_flat_byte(node, off, self.flat))
                    };
                    targets.push((
                        block_id,
                        host(a - content_start),
                        host(b - content_start),
                        self.selected_zero_byte_leaves(
                            doc,
                            content_start,
                            a - content_start,
                            b - content_start,
                        ),
                    ));
                }
                false // don't descend into the textblock's inline content
            } else {
                true
            }
        });
        targets
    }

    /// The host element ids of the zero-byte inline leaves (images) whose
    /// offsets in the textblock whose content starts at `content_start` are in
    /// `[a, b)` — the leaves a selection over those offsets holds (#1120).
    fn selected_zero_byte_leaves(
        &self,
        doc: &Node,
        content_start: usize,
        a: usize,
        b: usize,
    ) -> Vec<u64> {
        let mut out = Vec::new();
        let Ok(r) = doc.resolve(Pos(content_start)) else {
            return out;
        };
        let mut desc = &self.root;
        for d in 0..r.depth() {
            match desc.children.get(r.index(d)) {
                Some(c) => desc = c,
                None => return out,
            }
        }
        let block = &desc.node;
        let mut at = 0usize;
        for i in 0..block.child_count() {
            if at >= b {
                break;
            }
            let child = block.child(i);
            if at >= a
                && child.text().is_none()
                && leaf_flat_bytes(child, self.flat) == 0
                && let Some(c) = desc.children.get(i)
            {
                out.push(c.dom.node_id().0 as u64);
            }
            at += child.node_size();
        }
        out
    }

    /// Reconcile the selection-highlight divs (container children) to `rects`
    /// (container space).
    fn set_selection_rects(&mut self, rects: &[(f32, f32, f32, f32)]) {
        self.overlay_dirty = true;
        while self.selection_rects.len() < rects.len() {
            let Some(div) = create_element(&self.doc, "div") else {
                break;
            };
            div.set_attribute("data-pm-selection", "true");
            div.set_styles(&[
                ("position", "absolute"),
                // Anchored at the container's padding-box origin; the rect is
                // placed by `transform` (`overlay_translate`), which is paint-only.
                ("left", "0"),
                ("top", "0"),
                ("background-color", "rgba(26,115,232,0.3)"),
                ("pointer-events", "none"),
                // Behind the in-flow text (the container is a stacking context).
                ("z-index", "-1"),
            ]);
            self.root.dom.append_child(&div);
            self.selection_rects.push(div);
        }
        while self.selection_rects.len() > rects.len() {
            if let Some(div) = self.selection_rects.pop() {
                // Out of the pool for good — discard (issue #719). The grow loop
                // above builds fresh divs, so no later pass wants this one.
                div.discard();
            }
        }
        for (div, (x, y, w, h)) in self.selection_rects.iter().zip(rects) {
            let transform = overlay_translate(*x, *y);
            let (w, h) = overlay_size(*x, *y, *w, *h);
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("transform", &transform),
                ("width", &width),
                ("height", &height),
                ("display", "block"),
            ]);
        }
    }

    /// Remove all selection-highlight divs.
    fn clear_selection_rects(&mut self) {
        if self.last_selection.is_none() && self.selection_rects.is_empty() {
            return;
        }
        self.overlay_dirty = true;
        self.last_selection = None;
        self.cell_anchor_rect = None;
        for div in self.selection_rects.drain(..) {
            div.discard();
        }
    }

    /// Render the wash for a [`Selection::Cell`] — one translucent rectangle over
    /// each selected cell's box (merged cells span via their grid placement, so
    /// their host box already covers the merged area). Reuses the text-selection
    /// rect pool, so the wash sits behind the cell content just like a text
    /// highlight. Clears the wash if the table can't be resolved.
    ///
    /// Returns whether the wash actually changed (the
    /// ["overlay moved" hint](Self::position_caret) for the cell-selection arm) and records
    /// the head cell's rectangle in [`Self::cell_anchor_rect`] as the scroll anchor.
    ///
    /// [`Selection::Cell`]: rinch_editor_core::Selection::Cell
    fn render_cell_selection(&mut self, state: &EditorState) -> bool {
        use rinch_editor_core::Pos;
        let rinch_editor_core::Selection::Cell(cell) = &state.selection else {
            return false;
        };
        let key = (1u8, cell.anchor_cell.0, cell.head_cell.0);
        if self.last_selection == Some(key) {
            return false;
        }
        let Some((map, _table, _start)) =
            rinch_editor_core::tables::map_around(&state.doc, cell.head_cell)
        else {
            self.clear_selection_rects();
            self.cell_anchor_rect = None;
            return false;
        };
        let Some(rect) = map.rect_between(cell.anchor_cell.0, cell.head_cell.0) else {
            self.clear_selection_rects();
            self.cell_anchor_rect = None;
            return false;
        };
        let Some(doc) = self.doc.upgrade() else {
            return false;
        };
        let head_cell = cell.head_cell.0;
        // `(is_head, rect)` per cell: the wash rectangles are built by a filter_map,
        // so the head cell's *index* among them can only be learnt as they are made.
        let measured: Vec<(bool, (f32, f32, f32, f32))> = {
            let d = doc.borrow();
            map.cells_in_rect(rect)
                .into_iter()
                .filter_map(|cell_pos| {
                    let node_id = self.node_host_at(&state.doc, Pos(cell_pos))?;
                    let (_, _, w, h) = d.query_node_layout(node_id as u64)?;
                    let (ox, oy) = self.block_offset_in_container(&*d, node_id);
                    Some((cell_pos == head_cell, (ox, oy, w, h)))
                })
                .collect()
        };
        if measured.is_empty() {
            self.clear_selection_rects();
            self.cell_anchor_rect = None;
            return false;
        }
        self.cell_anchor_rect = measured
            .iter()
            .position(|(is_head, _)| *is_head)
            .or(Some(0));
        let rects: Vec<(f32, f32, f32, f32)> = measured.into_iter().map(|(_, r)| r).collect();
        self.last_selection = Some(key);
        self.set_selection_rects(&rects);
        true
    }

    /// Outline the node a [`Selection::Node`] selects — resolve the node's host
    /// element, compute its box in container space, and position a translucent
    /// overlay over it. Clears the outline if the node can't be located or has no
    /// geometry yet (e.g. an off-screen / virtualized block).
    ///
    /// Returns whether the outline actually moved (the
    /// ["overlay moved" hint](Self::position_caret) for the node-selection arm).
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    fn render_node_selection(&mut self, state: &EditorState) -> bool {
        let Some(node_id) = self.node_host_at(&state.doc, state.selection.from()) else {
            self.clear_node_selection();
            return false;
        };
        let Some(doc) = self.doc.upgrade() else {
            return false;
        };
        let geometry = {
            let d = doc.borrow();
            d.query_node_layout(node_id as u64).map(|(_, _, w, h)| {
                let (ox, oy) = self.block_offset_in_container(&*d, node_id);
                (ox, oy, w, h)
            })
        };
        match geometry {
            Some((x, y, w, h)) => self.position_node_outline(x, y, w, h),
            None => {
                self.clear_node_selection();
                false
            }
        }
    }

    /// Position the node-selection outline at the container-space box `(x, y, w, h)`.
    /// Reuses one overlay div (created lazily) and skips the writes when the box is
    /// unchanged, so a re-run doesn't re-dirty the tree (mirrors
    /// [`Self::position_caret`]). Returns whether the box actually moved.
    fn position_node_outline(&mut self, x: f32, y: f32, w: f32, h: f32) -> bool {
        let key = (
            x.round() as i32,
            y.round() as i32,
            w.round() as i32,
            h.round() as i32,
        );
        if self.last_node_outline == Some(key) {
            return false;
        }
        self.overlay_dirty = true;
        self.last_node_outline = Some(key);
        if self.node_outline.is_none() {
            let Some(div) = create_element(&self.doc, "div") else {
                return false;
            };
            div.set_attribute("data-pm-selected", "true");
            div.set_styles(&[
                ("position", "absolute"),
                // Placed by `transform`, like the caret (`overlay_translate`).
                ("left", "0"),
                ("top", "0"),
                // Trace the node's box exactly (border drawn inside the box).
                ("box-sizing", "border-box"),
                ("border", "2px solid #1a73e8"),
                ("background-color", "rgba(26,115,232,0.15)"),
                ("pointer-events", "none"),
                // In front of the node content, like the caret.
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&div);
            self.node_outline = Some(div);
        }
        if let Some(div) = &self.node_outline {
            let transform = overlay_translate(x, y);
            let (w, h) = overlay_size(x, y, w, h);
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("transform", &transform),
                ("width", &width),
                ("height", &height),
                ("visibility", "visible"),
            ]);
        }
        true
    }

    /// Hide the node-selection outline (the selection is no longer a node selection,
    /// or its node went off-screen).
    fn clear_node_selection(&mut self) {
        if self.last_node_outline.is_none() {
            return;
        }
        self.overlay_dirty = true;
        self.last_node_outline = None;
        // `visibility`, not `display`: see `set_caret_blink_visible`.
        if let Some(div) = &self.node_outline {
            div.set_style("visibility", "hidden");
        }
    }

    /// The host element id of the node *after* `pos` — the node a [`Selection::Node`]
    /// anchored at `pos` selects — navigating the descriptor tree by the resolved
    /// path. `None` if `pos` doesn't sit immediately before a child node.
    ///
    /// [`Selection::Node`]: rinch_editor_core::Selection::Node
    fn node_host_at(&self, doc: &Node, pos: Pos) -> Option<usize> {
        let r = doc.resolve(pos).ok()?;
        let mut desc = &self.root;
        for d in 0..r.depth() {
            desc = desc.children.get(r.index(d))?;
        }
        let child = desc.children.get(r.index(r.depth()))?;
        // The *inner* leaf element (`dom`), not a mark wrapper: a node selection
        // outlines the image / hr box itself, even when the leaf carries marks
        // (e.g. a linked image, inside an `<a>`).
        Some(child.dom.node_id().0)
    }

    /// The model position immediately **before** the node whose placed host element
    /// is `target`, plus that node — the inverse of [`Self::node_host_at`], used to
    /// node-select a leaf (image / horizontal rule) the user clicks. Matches either
    /// the node's inner element (`dom`) or one of its mark wrappers (`wraps`), so a
    /// hit on a marked inline image still resolves. `None` if `target` isn't a placed
    /// node in this view.
    pub(crate) fn node_pos_for_host(&self, target: usize) -> Option<(usize, Node)> {
        find_node_by_host(&self.root, target, 0).map(|(pos, node)| (pos, node.clone()))
    }

    /// Position the caret overlay at `(x, y)` in **container space** with `height`.
    /// The caret is a child of the container (created lazily), so it never disrupts
    /// a textblock's inline layout.
    ///
    /// Returns whether the caret actually **moved** (i.e. the geometry key changed
    /// and the writes below ran), which [`EditorView::update_caret`] reports as a
    /// `ScrollSelectionIntoView` hint. It is also what keeps a repeat pass from
    /// re-dirtying the tree.
    fn position_caret(&mut self, x: f32, y: f32, height: f32) -> bool {
        let key = (x.round() as i32, y.round() as i32, height.round() as i32);
        // Nothing changed since last frame — skip the writes so the caret doesn't
        // re-dirty itself and spin the repaint loop.
        if self.last_caret == Some(key) {
            return false;
        }
        self.overlay_dirty = true;
        self.last_caret = Some(key);
        // The caret moved (edit/cursor move) — restart the blink phase so the
        // caret is solid immediately after the interaction. Scope the reset to
        // the *focused* (blink-target) editor: with several editors mounted,
        // `update_all_carets` sweeps every one, and a programmatic caret move in
        // an unfocused editor must not stomp the focused editor's phase. The
        // clock is per document (#1149), so another document's caret is never
        // reached from here. (A focus change resets the phase separately, in
        // `caret_blink_tick`.)
        if super::blink::target(self.doc_key()) == Some(self.container_id()) {
            super::blink::reset(self.doc_key());
        }
        // The `visibility: visible` written below always puts this caret in the
        // "shown" phase, so a moved caret is never left mid-blink (hidden).
        self.blink_shown = Some(true);
        if self.caret.is_none() {
            let Some(caret) = create_element(&self.doc, "div") else {
                return false;
            };
            caret.set_attribute("data-pm-caret", "true");
            caret.set_styles(&[
                ("position", "absolute"),
                // Anchored at the container's padding-box origin and moved by
                // `transform` (`overlay_translate`): see there for why.
                ("left", "0"),
                ("top", "0"),
                // The width a caret query keeps inside its block (#1041).
                ("width", &format!("{}px", rinch_core::dom::CARET_WIDTH)),
                ("background-color", "#1a73e8"),
                ("pointer-events", "none"),
                // In front of the in-flow text and the selection highlight.
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&caret);
            self.caret = Some(caret);
        }
        if let Some(caret) = &self.caret {
            let transform = overlay_translate(x, y);
            let h = format!("{}px", overlay_size(x, y, 0.0, height).1);
            caret.set_styles(&[
                ("transform", &transform),
                ("height", &h),
                ("visibility", "visible"),
            ]);
        }
        true
    }

    /// Hide all overlays — the caret and the selection highlight. Used when the
    /// editor loses focus, so a blurred editor shows neither (the runtime's
    /// focus-aware caret pass calls this for every non-focused editor).
    pub(crate) fn hide_overlays(&mut self) {
        self.hide_caret();
        self.clear_selection_rects();
        self.clear_node_selection();
        self.hide_preedit();
    }

    /// Hide the caret (no collapsed cursor in a textblock).
    fn hide_caret(&mut self) {
        if self.last_caret.is_none() {
            return;
        }
        self.overlay_dirty = true;
        self.last_caret = None;
        self.blink_shown = None;
        // `visibility`, not `display`: see `set_caret_blink_visible`.
        if let Some(caret) = &self.caret {
            caret.set_style("visibility", "hidden");
        }
    }

    /// Apply a blink phase to the caret: show it (`visible == true`) or hide it.
    /// Returns `None` when there is no active caret to blink (no collapsed cursor),
    /// `Some(true)` when the caret's `visibility` actually toggled (the caller should
    /// repaint), and `Some(false)` when the phase was already applied (no write —
    /// so a blink tick that doesn't change the phase never re-dirties the tree).
    ///
    /// Toggles only `visibility`; the caret's *position* is owned by
    /// [`Self::position_caret`], which always leaves it in the shown phase.
    ///
    /// The caret and the node outline are hidden with
    /// `visibility`, never `display`. On the desktop a `display` change is a
    /// structural change to rinch-dom: it sets `ifc_dirty`, which drops every
    /// cached text measure and reshapes every inline formatting context in the
    /// window, and a caret hidden and shown within one update runs that twice.
    /// Measured in an app window with a 9,500-word document: 300-400 ms per pass,
    /// two passes per keystroke; a blink tick made the same change. `visibility`
    /// is paint-only. (The
    /// IME composition span still uses `display`: it is toggled once per
    /// composition rather than per keystroke, and a `visibility: hidden` span
    /// would keep its inline box in the line; #874.)
    pub(crate) fn set_caret_blink_visible(&mut self, visible: bool) -> Option<bool> {
        // No collapsed-cursor caret present → nothing to blink.
        self.last_caret?;
        if self.blink_shown == Some(visible) {
            return Some(false);
        }
        self.blink_shown = Some(visible);
        if let Some(caret) = &self.caret {
            caret.set_style("visibility", if visible { "visible" } else { "hidden" });
        }
        Some(true)
    }

    // ── IME composition (preedit) overlay ────────────────────────────────────

    /// Set the IME composition (preedit) string, shown as a transient overlay at
    /// the caret on the next [`EditorView::update_caret`] pass. The composition is
    /// **never** part of the document (design A5) — it is discarded on commit or
    /// clear. An empty `text` clears it.
    pub(crate) fn set_preedit(&mut self, text: &str) {
        self.preedit = if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        };
    }

    /// Position the IME composition overlay at `(x, y)` in container space — a span
    /// carrying the composing `text`, underlined, with an opaque background so it
    /// reads cleanly over any in-flow text it overlaps (the composition is an
    /// overlay, not part of the document). `height` is the caret line height, used
    /// to size the overlay box. Reuses one span (created lazily) and skips the
    /// writes when the composition and caret are unchanged (mirrors
    /// [`Self::position_caret`]).
    fn position_preedit(
        &mut self,
        x: f32,
        y: f32,
        height: f32,
        text: &str,
        font: Option<&NodeFont>,
    ) {
        let key = (x.round() as i32, y.round() as i32, text.to_string());
        if self.last_preedit.as_ref() == Some(&key) {
            return;
        }
        self.overlay_dirty = true;
        self.last_preedit = Some(key);
        if self.preedit_node.is_none() {
            let Some(span) = create_element(&self.doc, "span") else {
                return;
            };
            span.set_attribute("data-pm-preedit", "true");
            span.set_styles(&[
                ("position", "absolute"),
                // Preserve composition spacing; size to the content.
                ("white-space", "pre"),
                // Opaque so the composition reads cleanly over any text underneath.
                ("background-color", "#ffffff"),
                // Underline marks it as an in-progress composition.
                ("border-bottom", "1px solid #1a73e8"),
                ("pointer-events", "none"),
                // Above the in-flow text, the selection highlight, and the caret.
                ("z-index", "2"),
            ]);
            let Some(t) = create_text(&self.doc, text) else {
                return;
            };
            span.append_child(&t);
            self.root.dom.append_child(&span);
            self.preedit_text = Some(t);
            self.preedit_node = Some(span);
        }
        if let Some(t) = &self.preedit_text {
            t.set_text(text);
        }
        if let Some(span) = &self.preedit_node {
            let left = format!("{x}px");
            let top = format!("{y}px");
            let lh = format!("{height}px");
            span.set_styles(&[
                ("left", &left),
                ("top", &top),
                ("height", &lh),
                ("line-height", &lh),
                ("display", "inline-block"),
            ]);
            // Match the composing block's font so the preedit reads as the text being
            // typed (a heading composes large, body text composes at body size) rather
            // than the container's default. Backends without computed-style access
            // return `None` and the overlay keeps its inherited font.
            if let Some(f) = font {
                span.set_styles(&[
                    ("font-family", &f.family),
                    ("font-size", &f.size),
                    ("font-weight", &f.weight),
                    ("font-style", &f.style),
                ]);
            }
        }
    }

    /// Hide the IME composition overlay (composition committed or cleared).
    fn hide_preedit(&mut self) {
        if self.last_preedit.is_none() {
            return;
        }
        self.overlay_dirty = true;
        self.last_preedit = None;
        if let Some(span) = &self.preedit_node {
            span.set_style("display", "none");
        }
    }
}

/// The `transform` that places an overlay (caret, selection rect, node
/// outline) at `(x, y)` in container space. Each overlay is `position:
/// absolute` at `left: 0; top: 0` — the container's padding-box origin — and
/// is moved by this translation alone (#906).
///
/// A `transform` is paint-only. `left`/`top` are Taffy insets, so moving an
/// overlay through them was a Taffy style change, and on the desktop every
/// caret move — each arrow key, and the post-layout caret pass after each
/// keystroke — ran a root Taffy compute over the whole document. The
/// transform reaches no layout on either backend; the desktop damage still
/// names the old rect and the new one (`Node::painted` records the transform
/// the box was last painted with), `scroll_into_view` measures the painted
/// box (`painted_border_box`) and a browser's `scrollIntoView` the
/// transformed one, and every overlay already forms a stacking context of its
/// own (`position: absolute` with a `z-index`), so the transform changes
/// nothing about how it stacks or which clip it is under.
///
/// Rounded to whole pixels, because the desktop rounds a laid-out box but not
/// a transform, and a 2px caret translated by a fraction is drawn across
/// three columns, lighter. This lands **within a pixel** of where Taffy laid
/// the old insets out, not exactly on it: Taffy rounds a box's *absolute*
/// edges, and this rounds *container-relative* ones, so the two part only
/// where the container itself sits at a fractional offset. It is shared, so
/// the browser's caret is snapped to whole CSS px too, where a fractional
/// `left` used to place it exactly (up to half a CSS px off the text).
fn overlay_translate(x: f32, y: f32) -> String {
    format!("translate({}px, {}px)", x.round(), y.round())
}

/// The size an overlay at `(x, y)` sized `w` x `h` covers, in whole pixels:
/// the distance between its rounded edges, after the rule Taffy applied to
/// the old `left`/`top` plus `width`/`height` — it rounds a box's two edges,
/// not its size, so a caret at y = 10.6 with height 20.8 covered rows 11..31,
/// 20 rows and not `round(20.8)` = 21. The edges here are container-relative
/// where Taffy's were absolute, so the result is within a pixel of the old
/// size rather than equal to it (see [`overlay_translate`]).
fn overlay_size(x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
    ((x + w).round() - x.round(), (y + h).round() - y.round())
}

/// Walk the descriptor tree for the block whose host node is `target`, returning
/// its model **content-start** position and its model node. `content_start` is the
/// position passed in for `desc`'s own content (0 for the root/doc).
fn find_block(desc: &ViewDesc, target: usize, content_start: usize) -> Option<(usize, &Node)> {
    let mut pos = content_start; // the position just before the current child
    for child in &desc.children {
        if child.dom.node_id().0 == target {
            // The child's content begins just inside its opening boundary token.
            return Some((pos + 1, &child.node));
        }
        if let Some(found) = find_block(child, target, pos + 1) {
            return Some(found);
        }
        pos += child.node.node_size();
    }
    None
}

/// Walk the descriptor tree for the node whose placed host element is `target`
/// (matching its inner `dom`, a decoration segment, or a mark wrapper it sits
/// in — a wrapper shared by several runs resolves to the first, #1833),
/// returning its model **position-before** and the node. `content_start` is the
/// position just inside `desc`'s content (0 for the root/doc). The successor lookup
/// to [`find_block`] for *node* (not text-cursor) addressing.
fn find_node_by_host(
    desc: &ViewDesc,
    target: usize,
    content_start: usize,
) -> Option<(usize, &Node)> {
    let mut pos = content_start; // the position just before the current child
    for child in &desc.children {
        // A decorated run is placed as a wrapper span holding `<span data-pm-deco>`
        // segments, so a hit that lands on a segment (or on any of its own mark
        // wrappers) has to resolve to the run just as a hit on the text node does.
        if child.dom.node_id().0 == target
            || child.wraps.iter().any(|(_, w)| w.node_id().0 == target)
            || child.segments.iter().any(|s| s.node_id().0 == target)
        {
            return Some((pos, &child.node));
        }
        if let Some(found) = find_node_by_host(child, target, pos + 1) {
            return Some(found);
        }
        pos += child.node.node_size();
    }
    None
}

#[cfg(test)]
thread_local! {
    /// Test-only (#848): how many (decoration, child) range comparisons the
    /// decoration pass made — the work that used to be blocks × decorations.
    pub(crate) static DECO_RANGE_TESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn count_deco_range_tests(n: usize) {
    DECO_RANGE_TESTS.with(|c| c.set(c.get() + n));
}

#[cfg(not(test))]
#[inline(always)]
fn count_deco_range_tests(_: usize) {}

/// Re-project the inline decorations over `desc`'s subtree. `content_start` is the
/// model position just inside `desc`'s content (0 for the root/doc). Returns
/// whether any decoration segment is projected in this subtree — the caller's
/// [`ViewDesc::has_deco`].
///
/// Writes nothing for a run whose decorations are unchanged, so this is safe to
/// run on every transaction: the per-run comparison is against
/// [`ViewDesc::decos`], and a subtree that neither holds a segment nor overlaps a
/// decoration is not even descended into.
///
/// The list is sorted by `from` once per pass and walked with a cursor beside
/// the children, which are in position order, so each level costs one check per
/// child plus one per decoration that reaches it — not children × decorations
/// (#848: a spellchecker's thousand squiggles on a long document made every
/// keystroke pay blocks × decorations range tests).
fn sync_inline_decos(
    desc: &mut ViewDesc,
    decos: &[InlineDeco],
    content_start: usize,
    doc: &DocRef,
) -> bool {
    // Stable, so equal starts keep draw order; each entry also keeps its
    // draw-order index, which is the order a run's class list is written in.
    let mut sorted: Vec<(usize, &InlineDeco)> = decos.iter().enumerate().collect();
    sorted.sort_by_key(|(_, d)| d.from);
    sync_sorted_inline_decos(desc, &sorted, content_start, doc)
}

/// [`sync_inline_decos`] over `decos` sorted by `from`, each with its draw-order
/// index. Only the decorations that can reach `desc`'s content are passed down.
fn sync_sorted_inline_decos(
    desc: &mut ViewDesc,
    decos: &[(usize, &InlineDeco)],
    content_start: usize,
    doc: &DocRef,
) -> bool {
    let mut any = false;
    let mut pos = content_start; // the position just before the current child
    // `decos[next..]` start at or after the end of every child visited so far;
    // `live` holds, in `from` order, those started earlier that still reach
    // `pos` — together, exactly the decorations overlapping the current child.
    let mut next = 0;
    let mut live: Vec<(usize, &InlineDeco)> = Vec::new();
    for child in &mut desc.children {
        let size = child.node.node_size();
        let end = pos + size;
        count_deco_range_tests(live.len());
        live.retain(|(_, d)| d.to > pos);
        while let Some(&(i, d)) = decos.get(next) {
            count_deco_range_tests(1);
            if d.from >= end {
                break;
            }
            if d.to > pos {
                live.push((i, d));
            }
            next += 1;
        }
        // `has_deco` is what makes the skip sound in the other direction: a subtree
        // no decoration reaches any more still has to be visited to *take its
        // segments off*.
        if !live.is_empty() || child.has_deco {
            if child.is_text {
                let len = child.node.text().map_or(0, |t| t.chars().count());
                let run = if live.len() < 2 {
                    run_decos(live.iter().map(|(_, d)| *d), pos, len)
                } else {
                    let mut drawn = live.clone();
                    drawn.sort_unstable_by_key(|(i, _)| *i);
                    run_decos(drawn.into_iter().map(|(_, d)| d), pos, len)
                };
                apply_run_decos(child, run, doc);
                child.has_deco = !child.decos.is_empty();
            } else {
                child.has_deco = sync_sorted_inline_decos(child, &live, pos + 1, doc);
            }
        }
        any |= child.has_deco;
        pos = end;
    }
    any
}

/// Project `next` over the text run `desc`, replacing the run's inner host node
/// with the shape [`build_run_host`] builds for it. A no-op when the run's
/// decorations are unchanged — the one place the decoration pass writes to the
/// host.
fn apply_run_decos(desc: &mut ViewDesc, next: Vec<RunDeco>, doc: &DocRef) {
    if desc.decos == next {
        return;
    }
    let Some((inner, segments)) = build_run_host(desc.node.text().unwrap_or(""), &next, doc) else {
        return;
    };
    // In place, inside the run's mark wrappers, which keep their identity
    // while its content is swapped underneath.
    desc.dom.replace_with(&inner);
    // `replace_with` detaches the node it displaces and nothing can show it again
    // (issue #719) — and this is a per-keystroke path on a decorated run.
    desc.dom.discard();
    desc.dom = inner;
    desc.segments = segments;
    desc.decos = next;
}

/// What the host's flat byte offsets make of the two things in a textblock
/// whose width there is not their UTF-8 length in the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FlatWidths {
    /// The bytes a hard break's `<br>` occupies
    /// ([`DomDocument::line_break_flat_bytes`]): `1` on rinch-dom, whose inline
    /// layout pushes `"\n"` for it, `0` in the browser.
    line_break: usize,
    /// The bytes a tab occupies ([`DomDocument::tab_flat_bytes`]): `1` on
    /// rinch-dom (a space widened to its tab stop, #1122) and in the browser.
    /// rinch-dom answered `4` while it laid a tab out as four spaces (#1109).
    tab: usize,
    /// The chars the host lays out as something of another length, and its
    /// flat bytes for each ([`DomDocument::substituted_char_flat_bytes`]):
    /// rinch-dom's U+2028, U+2029, U+0085 and U+000C (#1181), none in the
    /// browser.
    substituted: &'static [(char, usize)],
}

impl FlatWidths {
    /// Plain UTF-8 text with no bytes for a `<br>`: the browser, and a host
    /// that cannot be asked.
    const UTF8: FlatWidths = FlatWidths {
        line_break: 0,
        tab: 1,
        substituted: &[],
    };

    fn of(doc: &dyn DomDocument) -> FlatWidths {
        FlatWidths {
            line_break: doc.line_break_flat_bytes(),
            tab: doc.tab_flat_bytes(),
            substituted: doc.substituted_char_flat_bytes(),
        }
    }

    /// The flat bytes of one model char of text.
    fn char_bytes(&self, ch: char) -> usize {
        if ch == '\t' {
            return self.tab;
        }
        self.substituted
            .iter()
            .find(|&&(c, _)| c == ch)
            .map_or(ch.len_utf8(), |&(_, n)| n)
    }

    /// The flat bytes of a run of model text.
    fn text_bytes(&self, text: &str) -> usize {
        if self.tab == 1 && self.substituted.is_empty() {
            text.len()
        } else {
            text.chars().map(|ch| self.char_bytes(ch)).sum()
        }
    }
}

/// The model **char** offset within a textblock for a flat UTF-8 `ifc_byte` offset
/// — the inverse of [`textblock_flat_byte`], used to turn a pointer hit into a
/// cursor position. Leaves count as one char (matching `textblock_flat_byte`);
/// a byte inside a leaf's own flat bytes (a `<br>`'s `"\n"`) is the position
/// before it. A byte inside a tab's flat bytes (a host whose tab is several,
/// as rinch-dom's four spaces were until #1122) picks a side of the tab
/// (#1109): the first of them is before it, the rest after. With four, a press
/// landed before the tab on its first 3/8 and after it from 3/8 on, where a
/// browser splits at the middle; rinch-dom's tab is one byte now, one cluster
/// that a pointer hit splits at its middle.
/// A byte inside a substituted char's flat bytes (rinch-dom's NBSP + ZWSP for
/// U+2028, #1181) is after it: the first of them is the visible half.
fn ifc_byte_to_char(block: &Node, ifc_byte: usize, flat: FlatWidths) -> usize {
    let mut bytes = 0usize;
    let mut chars = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            let run_bytes = flat.text_bytes(text);
            if ifc_byte <= bytes + run_bytes {
                let into_byte = ifc_byte - bytes;
                let mut b = 0usize;
                for ch in text.chars() {
                    if b >= into_byte {
                        return chars;
                    }
                    let width = flat.char_bytes(ch);
                    if ch == '\t' && 2 * (into_byte - b) < width {
                        return chars;
                    }
                    b += width;
                    chars += 1;
                }
                return chars;
            }
            bytes += run_bytes;
            chars += text.chars().count();
        } else {
            let width = leaf_flat_bytes(child, flat);
            if ifc_byte < bytes + width {
                return chars;
            }
            bytes += width;
            chars += 1;
        }
    }
    chars
}

/// The host's flat byte for the model's flat byte `model` of a textblock whose
/// host lays out CSS generated content in `generated` (#891): the model's
/// bytes with every generated stretch before them added. A stretch that starts
/// exactly there is passed when it is a `::before`'s, which belongs to the
/// content after it, and not when it is an `::after`'s, which belongs to the
/// content before it — so a caret at the start of a paragraph with a
/// `p::before` is drawn after that, and one at a link's end before the link's
/// `::after`. (Which side of an element's own generated text a caret at its
/// edge is on is the marks' call: [`RinchDomEditorView::mark_edges_beside`]
/// asks the host's edge carets, which count it.)
fn host_flat_byte(generated: &[GeneratedFlatRange], model: usize) -> usize {
    let mut host = model;
    for g in generated {
        if g.start < host || (g.start == host && !g.after) {
            host += g.end - g.start;
        } else {
            break;
        }
    }
    host
}

/// The inverse of [`host_flat_byte`]: the model's flat byte for the host's
/// `host` — less every generated stretch before it; a byte inside one is the
/// byte where it starts, the position its text stands at in the model.
fn model_flat_byte(generated: &[GeneratedFlatRange], host: usize) -> usize {
    let mut skipped = 0;
    for g in generated {
        if g.start >= host {
            break;
        }
        skipped += g.end.min(host) - g.start;
    }
    host - skipped
}

/// The flat bytes an inline leaf occupies in the host's offsets: the host's
/// line-break bytes for one rendered as `<br>`, zero for any other (an `<img>`
/// has no text).
fn leaf_flat_bytes(leaf: &Node, flat: FlatWidths) -> usize {
    if flat.line_break > 0 && node_dom_tag(leaf) == "br" {
        flat.line_break
    } else {
        0
    }
}

/// The flat UTF-8 byte offset, within a textblock's concatenated inline text,
/// corresponding to the model **char** offset `char_off` — the A15 char→byte half
/// of the caret map. Walks the inline runs accumulating char and byte counts.
///
/// An inline leaf is one model char. Its flat-byte width is [`leaf_flat_bytes`]:
/// the host's line-break bytes for a hard break (`<br>`), which is what the host
/// gives it ([`DomDocument::line_break_flat_bytes`]), and zero for anything else
/// (an image is an inline box with no text). A zero-byte leaf shares its byte
/// with the position beside it, so a byte offset cannot say which side of it it
/// means: the caret beside one is drawn from its box instead
/// ([`RinchDomEditorView::inline_box_beside`], #1104). A tab in the text is the
/// host's tab bytes ([`DomDocument::tab_flat_bytes`]) rather than its one UTF-8
/// byte (#1109), and a char the host substitutes its bytes for that
/// ([`DomDocument::substituted_char_flat_bytes`], #1181).
fn textblock_flat_byte(block: &Node, char_off: usize, flat: FlatWidths) -> usize {
    let mut chars_seen = 0usize;
    let mut bytes = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            let run_chars = text.chars().count();
            if chars_seen + run_chars >= char_off {
                let into = char_off - chars_seen;
                let byte_in_run: usize =
                    text.chars().take(into).map(|ch| flat.char_bytes(ch)).sum();
                return bytes + byte_in_run;
            }
            chars_seen += run_chars;
            bytes += flat.text_bytes(text);
        } else {
            if chars_seen >= char_off {
                return bytes;
            }
            chars_seen += 1; // leaf occupies one model char
            bytes += leaf_flat_bytes(child, flat);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_editor_core::model::Fragment;
    use rinch_editor_core::plugins::PlaceholderPlugin;
    use rinch_editor_core::{EditorState, Schema, Selection, default_plugins};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A host document plus a container node to mount the editor into.
    struct Harness {
        doc: Rc<RefCell<dyn DomDocument>>,
        /// The *same* allocation as `doc`, typed — the mock's test-only helpers
        /// (`__set_node_layout`, `__node_count`) are inherent methods, not trait
        /// ones, and are unreachable through the `dyn DomDocument` the view holds.
        mock: Rc<RefCell<MockDomDocument>>,
        container: NodeHandle,
        container_id: NodeId,
    }

    fn harness() -> Harness {
        let mock = Rc::new(RefCell::new(MockDomDocument::new()));
        let doc: Rc<RefCell<dyn DomDocument>> = mock.clone();
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        Harness {
            doc,
            mock,
            container,
            container_id,
        }
    }

    fn doc_ref(h: &Harness) -> DocRef {
        Rc::downgrade(&h.doc)
    }

    fn schema() -> Rc<Schema> {
        Rc::new(Schema::starter_kit())
    }

    fn para(s: &Schema, t: &str) -> Node {
        s.branch("paragraph", Fragment::from_node(s.text(t).unwrap()))
            .unwrap()
    }

    fn mk(s: &Schema, name: &str, attrs: rinch_editor_core::Attrs) -> Mark {
        Mark::new(s.mark_type(name).unwrap().clone(), attrs)
    }

    fn marked_para(s: &Schema, t: &str, marks: Vec<Mark>) -> Node {
        s.branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks(t, marks).unwrap()),
        )
        .unwrap()
    }

    fn doc_node(s: &Schema, blocks: Vec<Node>) -> Node {
        s.branch("doc", Fragment::from_children(blocks)).unwrap()
    }

    fn state(s: Rc<Schema>, doc: Node) -> EditorState {
        EditorState::create(s, doc, default_plugins())
    }

    // Read helpers over the host document.
    fn children(h: &Harness, id: NodeId) -> Vec<NodeId> {
        h.doc.borrow().get_children(id)
    }
    fn tag(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().tag_name(id)
    }
    fn pm_type(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().get_attribute(id, "data-pm-type")
    }
    fn text(h: &Harness, id: NodeId) -> Option<String> {
        h.doc.borrow().text_content(id)
    }

    #[test]
    fn builds_initial_dom_from_doc() {
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(&s, vec![para(&s, "hello"), para(&s, "world")]),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));
        assert_eq!(pm_type(&h, blocks[0]).as_deref(), Some("paragraph"));
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("hello"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("world"));
        assert_eq!(pm_type(&h, h.container_id).as_deref(), Some("doc"));
    }

    #[test]
    fn blink_toggles_caret_visibility_and_guards_redundant_writes() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "hi")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // No caret present yet → nothing to blink.
        assert_eq!(view.set_caret_blink_visible(false), None);
        assert_eq!(view.set_caret_blink_visible(true), None);

        // Placing the caret leaves it in the shown phase (and resets the clock).
        view.position_caret(1.0, 2.0, 18.0);
        assert_eq!(view.blink_shown, Some(true));

        // Hiding it for the off-phase writes once...
        assert_eq!(view.set_caret_blink_visible(false), Some(true));
        // ...and a repeated off-phase is a no-op (so a tick never re-dirties).
        assert_eq!(view.set_caret_blink_visible(false), Some(false));
        // Back to the on-phase writes again, then is idempotent.
        assert_eq!(view.set_caret_blink_visible(true), Some(true));
        assert_eq!(view.set_caret_blink_visible(true), Some(false));

        // A re-position always returns the caret to the shown phase, so a moved
        // caret is never left mid-blink (hidden).
        view.set_caret_blink_visible(false);
        view.position_caret(5.0, 2.0, 18.0);
        assert_eq!(view.blink_shown, Some(true));

        // Removing the caret (selection/blur) clears the blink state.
        view.hide_caret();
        assert_eq!(view.blink_shown, None);
        assert_eq!(view.set_caret_blink_visible(true), None);
    }

    /// The inline `style` attribute of an overlay node.
    fn style_of(h: &Harness, node: &NodeHandle) -> String {
        h.doc
            .borrow()
            .get_attribute(node.node_id(), "style")
            .unwrap_or_default()
    }

    /// Showing, blinking and hiding the caret and the node outline never write
    /// `display`: on the desktop that is a structural change that reshapes every
    /// text block in the window (see `set_caret_blink_visible`).
    #[test]
    fn overlays_hide_with_visibility_never_display() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "hi")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        view.position_caret(1.0, 2.0, 18.0);
        let caret = view.caret.clone().expect("positioning creates the caret");
        assert!(style_of(&h, &caret).contains("visibility: visible"));

        view.set_caret_blink_visible(false);
        assert!(style_of(&h, &caret).contains("visibility: hidden"));
        view.set_caret_blink_visible(true);
        assert!(style_of(&h, &caret).contains("visibility: visible"));

        view.hide_caret();
        let caret_style = style_of(&h, &caret);
        assert!(caret_style.contains("visibility: hidden"));
        assert!(
            !caret_style.contains("display"),
            "caret style: {caret_style}"
        );

        view.position_node_outline(1.0, 2.0, 30.0, 20.0);
        let outline = view.node_outline.clone().expect("the outline is created");
        view.clear_node_selection();
        let outline_style = style_of(&h, &outline);
        assert!(outline_style.contains("visibility: hidden"));
        assert!(
            !outline_style.contains("display"),
            "outline style: {outline_style}"
        );

        // A cleared outline comes back when a node is selected
        // again (and so does a hidden caret). Kills: `position_node_outline`
        // not writing `visibility: visible`, which left every node selection
        // after the first one invisible.
        view.position_node_outline(40.0, 50.0, 30.0, 20.0);
        assert!(style_of(&h, &outline).contains("visibility: visible"));
        view.position_caret(9.0, 2.0, 18.0);
        assert!(style_of(&h, &caret).contains("visibility: visible"));
    }

    #[test]
    fn blink_reset_is_scoped_to_the_focused_editor() {
        // Two editors share one host doc so they get distinct container ids.
        let h = harness();
        let s = schema();
        let container_b_id = h.doc.borrow_mut().create_element("div");
        let container_b = NodeHandle::new(container_b_id, Rc::downgrade(&h.doc));

        let st_a = state(s.clone(), doc_node(&s, vec![para(&s, "aaa")]));
        let st_b = state(s.clone(), doc_node(&s, vec![para(&s, "bbb")]));
        let mut view_a = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st_a);
        let mut view_b = RinchDomEditorView::new(container_b, doc_ref(&h), &st_b);

        // Editor A is the focused / blink-target editor.
        let dk = view_a.doc_key();
        crate::blink::set_target(dk, Some(view_a.container_id()));
        crate::blink::reset(dk);
        let anchor0 = crate::blink::anchor_for_test(dk);

        // A caret move in the UNFOCUSED editor B must NOT re-anchor the clock.
        view_b.position_caret(1.0, 2.0, 18.0);
        assert_eq!(
            crate::blink::anchor_for_test(dk),
            anchor0,
            "an unfocused editor's caret move stomped the focused editor's blink phase",
        );

        // A caret move in the FOCUSED editor A re-anchors it (caret back to solid).
        view_a.position_caret(1.0, 2.0, 18.0);
        assert_ne!(
            crate::blink::anchor_for_test(dk),
            anchor0,
            "the focused editor's caret move failed to reset its own blink phase",
        );

        crate::blink::set_target(dk, None);
    }

    #[test]
    fn heading_uses_level_tag() {
        let h = harness();
        let s = schema();
        let heading = s
            .create_node(
                "heading",
                rinch_editor_core::Attrs::from_iter([(
                    "level",
                    rinch_editor_core::AttrValue::from(3_i64),
                )]),
                Fragment::from_node(s.text("Title").unwrap()),
            )
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![heading]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let blocks = children(&h, h.container_id);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("h3"));
    }

    #[test]
    fn insert_text_patches_text_node_in_place() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "ab")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p_id = children(&h, h.container_id)[0];
        let text_id_before = children(&h, p_id)[0];

        // Type "X" at the end of "ab".
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(3)));
        tr.insert_text("X").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        let p_id_after = children(&h, h.container_id)[0];
        let text_id_after = children(&h, p_id_after)[0];
        assert_eq!(p_id, p_id_after, "the paragraph element is reused");
        assert_eq!(
            text_id_before, text_id_after,
            "the text node is patched in place"
        );
        assert_eq!(text(&h, p_id_after).as_deref(), Some("abX"));
    }

    /// A block the `ViewDesc` diff drops is **discarded**, not merely removed
    /// (issue #719).
    ///
    /// Every removal in this file is a discard, and the reason is churn rate:
    /// this diff runs on every keystroke and every selection change, and on the
    /// browser backend a merely-removed node keeps a strong `web_sys::Node` in
    /// two page-global maps for the life of the module (issue #184). `remove`
    /// here would leak one entry per block the user ever deletes.
    ///
    /// The mock retires a discarded subtree and keeps a removed one, so the two
    /// verbs are distinguishable on the host: `tag_name` answers `None` only for
    /// the discard.
    #[test]
    fn a_dropped_block_is_discarded_not_merely_removed() {
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(&s, vec![para(&s, "keep"), para(&s, "drop")]),
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2, "precondition: two blocks are mounted");
        let doomed = blocks[1];
        let doomed_text = children(&h, doomed)[0];

        // Delete the whole second block, so `diff_children` pops its ViewDesc.
        let mut tr = st.tr();
        tr.delete(6, 12).unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        assert_eq!(
            children(&h, h.container_id).len(),
            1,
            "precondition: one block is left"
        );
        assert_eq!(
            tag(&h, doomed),
            None,
            "#719: the dropped block must be discarded, so the backend can release it"
        );
        assert_eq!(
            tag(&h, doomed_text),
            None,
            "#184: and its whole subtree with it"
        );
    }

    /// A block whose **kind** changes goes through `replace_with`, and the
    /// `ViewDesc` it displaces must be discarded (issue #719).
    ///
    /// This is the other editor removal route, and the one with no `remove()`
    /// in it at all: `replace_with` *detaches* the node it displaces, so
    /// without a following `discard` the old block and its whole subtree stay
    /// in the browser backend's two maps forever. It fires once per
    /// kind-changed block per keystroke.
    #[test]
    fn a_block_whose_kind_changes_discards_the_one_it_replaced() {
        let h = harness();
        let s = schema();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "title")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let before = children(&h, h.container_id)[0];
        assert_eq!(tag(&h, before).as_deref(), Some("p"), "precondition");

        st.selection = Selection::cursor(rinch_editor_core::Pos(2));
        let next = st.run("setHeading1").expect("setHeading1 applies");
        view.update_dom(&st, &next);

        let after = children(&h, h.container_id)[0];
        assert_eq!(
            tag(&h, after).as_deref(),
            Some("h1"),
            "precondition: the block is a heading now"
        );
        assert_ne!(before, after, "precondition: it was rebuilt, not patched");
        assert_eq!(
            tag(&h, before),
            None,
            "#719: the displaced block must be discarded, not left detached"
        );
    }

    #[test]
    fn split_block_adds_a_paragraph_and_keeps_the_first() {
        let h = harness();
        let s = schema();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "ab")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let first_p_before = children(&h, h.container_id)[0];

        // Split between "a" and "b".
        st.selection = Selection::cursor(rinch_editor_core::Pos(2));
        let next = st.run("splitBlock").expect("split applies");
        view.update_dom(&st, &next);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0], first_p_before,
            "the first paragraph is reused, not rebuilt"
        );
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("a"));
        assert_eq!(text(&h, blocks[1]).as_deref(), Some("b"));
    }

    /// The text of each block the container holds, in order (overlays have
    /// no `data-pm-type` and are skipped).
    fn block_texts(h: &Harness) -> Vec<(NodeId, String)> {
        children(h, h.container_id)
            .into_iter()
            .filter(|&id| pm_type(h, id).is_some())
            .map(|id| (id, text(h, id).unwrap_or_default()))
            .collect()
    }

    /// The host element currently showing `t`.
    fn host_of(h: &Harness, t: &str) -> NodeId {
        block_texts(h)
            .into_iter()
            .find(|(_, s)| s == t)
            .unwrap_or_else(|| panic!("no block shows {t:?}"))
            .0
    }

    /// Splitting a block in the **middle** of the document keeps every block
    /// after it on its own host node (issue #905).
    ///
    /// The diff used to be purely positional, so inserting one block shifted
    /// every later model block onto its predecessor's host: each one's text
    /// was rewritten (`set_text`) and every paragraph below the caret was
    /// re-shaped on the desktop — 31 shapes for one Enter in 30 paragraphs.
    /// Matching the unchanged (`same_ref`) prefix and suffix first leaves them
    /// alone. Off the fixed point: the split is neither the first nor the last
    /// block, and two blocks follow it, so a positional diff moves both.
    #[test]
    fn a_split_in_the_middle_keeps_the_blocks_after_it_on_their_hosts() {
        let h = harness();
        let s = schema();
        let mut st = state(
            s.clone(),
            doc_node(
                &s,
                vec![para(&s, "a"), para(&s, "bc"), para(&s, "d"), para(&s, "e")],
            ),
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let (a, bc, d, e) = (
            host_of(&h, "a"),
            host_of(&h, "bc"),
            host_of(&h, "d"),
            host_of(&h, "e"),
        );
        let (d_text, e_text) = (children(&h, d)[0], children(&h, e)[0]);

        // 0[p 1 a 2]3[p 4 b 5 c 6]7 — split between "b" and "c".
        st.selection = Selection::cursor(rinch_editor_core::Pos(5));
        let next = st.run("splitBlock").expect("split applies");
        view.update_dom(&st, &next);

        let after: Vec<String> = block_texts(&h).into_iter().map(|(_, t)| t).collect();
        assert_eq!(
            after,
            ["a", "b", "c", "d", "e"],
            "the host reads as the model"
        );
        assert_eq!(host_of(&h, "a"), a, "the block before is untouched");
        assert_eq!(host_of(&h, "b"), bc, "the split block is patched in place");
        assert_eq!(host_of(&h, "d"), d, "#905: `d` stays on its own host");
        assert_eq!(host_of(&h, "e"), e, "#905: `e` stays on its own host");
        assert_eq!(children(&h, d)[0], d_text, "and on its own text node");
        assert_eq!(children(&h, e)[0], e_text, "and on its own text node");
    }

    /// Joining two blocks in the middle of the document keeps the blocks after
    /// the join on their hosts, and discards the joined-away block's host — not
    /// the last one (issue #905).
    #[test]
    fn a_join_in_the_middle_discards_the_joined_block_not_the_last() {
        let h = harness();
        let s = schema();
        let mut st = state(
            s.clone(),
            doc_node(
                &s,
                vec![
                    para(&s, "a"),
                    para(&s, "b"),
                    para(&s, "c"),
                    para(&s, "d"),
                    para(&s, "e"),
                ],
            ),
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let (b, c, d, e) = (
            host_of(&h, "b"),
            host_of(&h, "c"),
            host_of(&h, "d"),
            host_of(&h, "e"),
        );

        // 0[p1 a 2]3[p4 b 5]6[p7 c 8]9 — Backspace at the start of "c".
        st.selection = Selection::cursor(rinch_editor_core::Pos(7));
        let next = st.run("deleteCharBackward").expect("join applies");
        view.update_dom(&st, &next);

        let after: Vec<String> = block_texts(&h).into_iter().map(|(_, t)| t).collect();
        assert_eq!(after, ["a", "bc", "d", "e"], "the host reads as the model");
        assert_eq!(host_of(&h, "bc"), b, "the joined block is patched in place");
        assert_eq!(host_of(&h, "d"), d, "#905: `d` stays on its own host");
        assert_eq!(host_of(&h, "e"), e, "#905: `e` stays on its own host");
        assert_eq!(tag(&h, c), None, "#719: the joined-away block is discarded");
    }

    /// Whatever the edit, the host's blocks read as the model's, in order —
    /// inserted and removed at the start, the middle and the end, several at
    /// once, and with a changed block between two unchanged ones.
    #[test]
    fn the_host_reads_as_the_model_after_inserts_and_removals_anywhere() {
        let s = schema();
        let texts = |v: &[&str]| -> Node { doc_node(&s, v.iter().map(|t| para(&s, t)).collect()) };
        let cases: &[(&[&str], &[&str])] = &[
            (&["a", "b", "c"], &["x", "a", "b", "c"]),
            (&["a", "b", "c"], &["a", "b", "c", "x"]),
            (&["a", "b", "c"], &["a", "x", "y", "b", "c"]),
            (&["a", "b", "c", "d"], &["a", "d"]),
            (&["a", "b", "c", "d"], &["b", "c", "d"]),
            (&["a", "b", "c", "d"], &["a", "b", "c"]),
            (&["a", "b", "c"], &["a", "x", "c"]),
            (&["a", "b", "c"], &["a", "x", "y", "z", "c"]),
            (&["a", "b", "c", "d"], &["a", "x", "d"]),
            (&["a"], &["x", "y"]),
        ];
        for (before, after) in cases {
            let h = harness();
            let old = texts(before);
            let st = state(s.clone(), old.clone());
            let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
            // Keep the unchanged blocks as the SAME `Rc`s, as a real
            // transaction does, so the diff can match them.
            let blocks: Vec<Node> = after
                .iter()
                .map(|t| {
                    (0..old.child_count())
                        .map(|i| old.child(i))
                        .find(|c| c.child(0).text() == Some(*t))
                        .cloned()
                        .unwrap_or_else(|| para(&s, t))
                })
                .collect();
            let mut next = st.clone();
            next.doc = doc_node(&s, blocks);
            view.update_dom(&st, &next);
            let got: Vec<String> = block_texts(&h).into_iter().map(|(_, t)| t).collect();
            assert_eq!(&got, after, "{before:?} -> {after:?}");
        }
    }

    /// A run inserted in front of an unchanged **mark-wrapped** run goes in
    /// before the run's outermost wrapper, not inside it (issue #905's review).
    ///
    /// For a bold run the `<strong>` is a wrapper the textblock places, while
    /// its `dom` is the text node inside it. Inserting before `dom` would put
    /// the plain text inside the `<strong>`; `regroup_inline` places each run
    /// by its own marks (#1833). No block-level fixture can tell the two apart.
    #[test]
    fn a_run_typed_in_front_of_a_bold_run_lands_outside_its_wrapper() {
        let h = harness();
        let s = schema();
        let bold = mk(&s, "bold", rinch_editor_core::Attrs::default());
        let old = doc_node(&s, vec![marked_para(&s, "b", vec![bold])]);
        let st = state(s.clone(), old.clone());
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"), "precondition");

        // The same bold run (`same_ref`), with a plain run in front of it.
        let bold_run = old.child(0).child(0).clone();
        let para = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![s.text("a").unwrap(), bold_run]),
            )
            .unwrap();
        let mut next = st.clone();
        next.doc = doc_node(&s, vec![para]);
        view.update_dom(&st, &next);

        let kids = children(&h, p);
        assert_eq!(kids.len(), 2, "a plain run and the bold run: {kids:?}");
        assert_eq!(tag(&h, kids[0]), None, "the new run is a bare text node");
        assert_eq!(text(&h, kids[0]).as_deref(), Some("a"));
        assert_eq!(kids[1], strong, "the bold run keeps its wrapper");
        assert_eq!(
            text(&h, strong).as_deref(),
            Some("b"),
            "and nothing joins it"
        );
    }

    #[test]
    fn wrap_in_blockquote_restructures_host() {
        let h = harness();
        let s = schema();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "quote me")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let next = st.run("wrapInBlockquote").expect("wrap applies");
        view.update_dom(&st, &next);

        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 1);
        assert_eq!(tag(&h, blocks[0]).as_deref(), Some("blockquote"));
        let inner = children(&h, blocks[0]);
        assert_eq!(tag(&h, inner[0]).as_deref(), Some("p"));
        assert_eq!(text(&h, blocks[0]).as_deref(), Some("quote me"));
    }

    #[test]
    fn unchanged_sibling_is_not_rebuilt() {
        // Editing the 2nd paragraph must not touch the 1st paragraph's host node
        // (the `same_ref` fast skip, A12).
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(&s, vec![para(&s, "keep"), para(&s, "edit")]),
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let first_before = children(&h, h.container_id)[0];
        let first_text_before = children(&h, first_before)[0];

        // Type into the second paragraph (positions: 0[p 1 keep 5]6[p 7 edit 11]12).
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(11)));
        tr.insert_text("!").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        let first_after = children(&h, h.container_id)[0];
        let first_text_after = children(&h, first_after)[0];
        assert_eq!(first_before, first_after, "1st paragraph element untouched");
        assert_eq!(
            first_text_before, first_text_after,
            "1st paragraph text node untouched"
        );
        assert_eq!(
            text(&h, children(&h, h.container_id)[1]).as_deref(),
            Some("edit!")
        );
    }

    #[test]
    fn placeholder_shows_for_empty_doc_and_hides_after_typing() {
        let h = harness();
        let s = schema();
        let empty = doc_node(&s, vec![s.branch("paragraph", Fragment::empty()).unwrap()]);
        let st = EditorState::create(
            s.clone(),
            empty,
            vec![Rc::new(PlaceholderPlugin::new("Write something…"))],
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // The placeholder overlay exists.
        let has_placeholder = |h: &Harness| {
            children(h, h.container_id).iter().any(|&id| {
                h.doc
                    .borrow()
                    .get_attribute(id, "data-pm-placeholder")
                    .is_some()
            })
        };
        assert!(has_placeholder(&h), "placeholder shown while empty");

        // Type a character — the doc is no longer empty, placeholder disappears.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(1)));
        tr.insert_text("x").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert!(
            !has_placeholder(&h),
            "placeholder removed once content exists"
        );
    }

    /// The placeholder is **discarded** when it goes away, not merely removed
    /// (issue #719): `self.placeholder.take()` drops the handle in the same
    /// breath, and a fresh one is built the next time the doc goes empty.
    #[test]
    fn a_dismissed_placeholder_is_discarded() {
        let h = harness();
        let s = schema();
        let empty = doc_node(&s, vec![s.branch("paragraph", Fragment::empty()).unwrap()]);
        let st = EditorState::create(
            s.clone(),
            empty,
            vec![Rc::new(PlaceholderPlugin::new("Write something…"))],
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let placeholder = *children(&h, h.container_id)
            .iter()
            .find(|&&id| {
                h.doc
                    .borrow()
                    .get_attribute(id, "data-pm-placeholder")
                    .is_some()
            })
            .expect("precondition: the placeholder is mounted");

        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(1)));
        tr.insert_text("x").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);

        assert_eq!(
            tag(&h, placeholder),
            None,
            "#719: the dismissed placeholder must be discarded, so the backend can release it"
        );
    }

    // ── inline mark wrappers (M5.5a) ─────────────────────────────────────────

    fn attr_of(h: &Harness, id: NodeId, name: &str) -> Option<String> {
        h.doc.borrow().get_attribute(id, name)
    }

    #[test]
    fn bold_run_wraps_text_in_strong() {
        let h = harness();
        let s = schema();
        let doc = doc_node(
            &s,
            vec![marked_para(
                &s,
                "hi",
                vec![mk(&s, "bold", Default::default())],
            )],
        );
        let st = state(s.clone(), doc);
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
        assert_eq!(attr_of(&h, strong, "data-pm-mark").as_deref(), Some("bold"));
        assert_eq!(text(&h, strong).as_deref(), Some("hi"));
    }

    #[test]
    fn nested_marks_nest_wrappers_innermost_first() {
        // marks = [bold, italic] → bold innermost, italic outermost (matches
        // copy-out HTML `<em><strong>x</strong></em>`).
        let h = harness();
        let s = schema();
        let marks = vec![
            mk(&s, "bold", Default::default()),
            mk(&s, "italic", Default::default()),
        ];
        let st = state(s.clone(), doc_node(&s, vec![marked_para(&s, "x", marks)]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let outer = children(&h, p)[0];
        assert_eq!(tag(&h, outer).as_deref(), Some("em"), "italic is outermost");
        let inner = children(&h, outer)[0];
        assert_eq!(
            tag(&h, inner).as_deref(),
            Some("strong"),
            "bold is innermost"
        );
        assert_eq!(text(&h, inner).as_deref(), Some("x"));
    }

    #[test]
    fn list_items_with_combined_marks_project_wrappers() {
        // bullet_list(li(p[bold "a"]), li(p[italic "b"]), li(p[bold+italic "c"]))
        // → <ul><li><p><strong>a</strong></p>…<li><p><em><strong>c</strong></em>…
        let h = harness();
        let s = schema();
        let li = |t: &str, marks: Vec<Mark>| {
            s.branch("list_item", Fragment::from_node(marked_para(&s, t, marks)))
                .unwrap()
        };
        let list = s
            .branch(
                "bullet_list",
                Fragment::from_children(vec![
                    li("a", vec![mk(&s, "bold", Default::default())]),
                    li("b", vec![mk(&s, "italic", Default::default())]),
                    li(
                        "c",
                        vec![
                            mk(&s, "bold", Default::default()),
                            mk(&s, "italic", Default::default()),
                        ],
                    ),
                ]),
            )
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![list]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let ul = children(&h, h.container_id)[0];
        assert_eq!(tag(&h, ul).as_deref(), Some("ul"));
        let items = children(&h, ul);
        assert_eq!(items.len(), 3, "three list items");

        // The first inline child of an item's <p> (its outermost mark wrapper).
        let item_wrapper = |idx: usize| {
            let p = children(&h, items[idx])[0];
            assert_eq!(
                tag(&h, p).as_deref(),
                Some("p"),
                "list item holds a paragraph"
            );
            children(&h, p)[0]
        };

        let bold = item_wrapper(0);
        assert_eq!(tag(&h, bold).as_deref(), Some("strong"));
        assert_eq!(text(&h, bold).as_deref(), Some("a"));

        let ital = item_wrapper(1);
        assert_eq!(tag(&h, ital).as_deref(), Some("em"));
        assert_eq!(text(&h, ital).as_deref(), Some("b"));

        let outer = item_wrapper(2);
        assert_eq!(tag(&h, outer).as_deref(), Some("em"), "italic outermost");
        let inner = children(&h, outer)[0];
        assert_eq!(tag(&h, inner).as_deref(), Some("strong"), "bold innermost");
        assert_eq!(text(&h, inner).as_deref(), Some("c"));
    }

    #[test]
    fn link_mark_sets_href_on_anchor() {
        let h = harness();
        let s = schema();
        let href = rinch_editor_core::Attrs::from_iter([(
            "href",
            rinch_editor_core::AttrValue::from("https://example.test"),
        )]);
        let st = state(
            s.clone(),
            doc_node(
                &s,
                vec![marked_para(&s, "click", vec![mk(&s, "link", href)])],
            ),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let a = children(&h, p)[0];
        assert_eq!(tag(&h, a).as_deref(), Some("a"));
        assert_eq!(
            attr_of(&h, a, "href").as_deref(),
            Some("https://example.test")
        );
        assert_eq!(text(&h, a).as_deref(), Some("click"));
    }

    #[test]
    fn toggling_bold_over_a_range_rebuilds_the_run_with_a_wrapper() {
        // A mark-set change can't be patched in place — the run rebuilds, wrapping
        // the text in <strong>.
        let h = harness();
        let mut st = state(schema(), doc_node(&schema(), vec![para(&schema(), "abcd")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        assert_eq!(
            tag(&h, children(&h, p)[0]),
            None,
            "plain text node, no wrapper"
        );

        // Select the whole word and bold it.
        st.selection = Selection::text(rinch_editor_core::Pos(1), rinch_editor_core::Pos(5));
        let next = st.run("toggleBold").expect("toggleBold applies");
        view.update_dom(&st, &next);

        let p_after = children(&h, h.container_id)[0];
        let child = children(&h, p_after)[0];
        assert_eq!(
            tag(&h, child).as_deref(),
            Some("strong"),
            "run now wrapped in <strong>"
        );
        assert_eq!(text(&h, child).as_deref(), Some("abcd"));
    }

    // ── A15: char→flat-byte caret map (the off-by-one-prone part) ─────────────

    // ── node-views: NodeSelection of a leaf (image / horizontal rule) ─────────

    #[test]
    fn node_selection_host_round_trips_for_block_atom() {
        let h = harness();
        let s = schema();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let st = state(s.clone(), doc_node(&s, vec![para(&s, "ab"), hr]));
        let view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // The hr is the 2nd container child; it renders as <hr>.
        let blocks = children(&h, h.container_id);
        assert_eq!(blocks.len(), 2);
        assert_eq!(tag(&h, blocks[1]).as_deref(), Some("hr"));
        let hr_id = blocks[1].0;

        // Forward: the position just before the hr (model pos 4) → the hr host id.
        assert_eq!(
            view.node_host_at(&st.doc, rinch_editor_core::Pos(4)),
            Some(hr_id)
        );
        // Inverse: the hr host id → (pos-before, node).
        let (pos, node) = view.node_pos_for_host(hr_id).expect("hr is a placed node");
        assert_eq!(pos, 4, "position immediately before the hr");
        assert_eq!(node.type_name(), "horizontal_rule");

        // A textblock host is not a node-selectable leaf.
        assert!(view.node_pos_for_host(blocks[0].0).is_some()); // the paragraph IS a node…
        // …but its position-before is 0 and it isn't a leaf — selectability is the
        // core's call (`Selection::node_at`), exercised in the handle tests.
    }

    #[test]
    fn marked_inline_leaf_node_host_is_the_inner_element() {
        // A node selection of a *marked* inline leaf (a linked image) must outline
        // the inner <img>, not the <a> mark wrapper — so node_host_at returns the
        // img host id (the inner `dom`), not the `<a>` wrapper id.
        let h = harness();
        let s = schema();
        let slice = rinch_editor_core::serialize::slice_from_html(
            &s,
            "<p>a<a href=\"https://example.com\"><img src=\"y.png\"></a></p>",
        )
        .expect("parse linked image");
        let st = state(s.clone(), s.branch("doc", slice.content).unwrap());
        let view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // Host tree: the paragraph's children are the text "a" and the <a> mark
        // wrapper, whose only child is the <img>. (Also asserts the fixture really
        // wraps the image — i.e. the parser applied the link mark to the leaf.)
        let para = children(&h, h.container_id)[0];
        let a_id = *children(&h, para)
            .iter()
            .find(|&&c| tag(&h, c).as_deref() == Some("a"))
            .expect("the image is wrapped in an <a> mark wrapper");
        let img_id = children(&h, a_id)[0];
        assert_eq!(tag(&h, img_id).as_deref(), Some("img"));

        // doc(p(text "a", image)) → positions 0[p 1 a 2 (img) 3]4 ; the image is at 2.
        let host = view.node_host_at(&st.doc, rinch_editor_core::Pos(2));
        assert_eq!(host, Some(img_id.0), "outline traces the inner <img>");
        assert_ne!(host, Some(a_id.0), "not the <a> mark wrapper");
    }

    // ── ScrollSelectionIntoView: emitted only when the selection actually moved ──

    /// `doc(p(), p())` mounted into `h`, with both paragraphs given stacked 200x20
    /// boxes. Two empty paragraphs because
    /// [`empty_block_caret`](RinchDomEditorView::empty_block_caret) is the one caret
    /// path the mock can drive — it has no text layout, so `query_caret_position`
    /// always declines — and stacked because a cursor in the first vs. the second is
    /// then a genuine caret *move*.
    fn measured_empty_paragraphs(h: &Harness, s: &Rc<Schema>) -> (EditorState, RinchDomEditorView) {
        let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
        let st = state(s.clone(), doc_node(s, vec![empty(), empty()]));
        let view = RinchDomEditorView::new(h.container.clone(), doc_ref(h), &st);
        let blocks = children(h, h.container_id);
        let mut m = h.mock.borrow_mut();
        for (i, b) in blocks.iter().enumerate() {
            m.__set_node_layout(*b, 0.0, i as f32 * 20.0, 200.0, 20.0);
        }
        (st, view)
    }

    fn cursor_at(st: &mut EditorState, pos: usize) {
        st.selection = Selection::cursor(rinch_editor_core::Pos(pos));
    }

    /// The attribute marking which overlay div a handle points at.
    fn overlay_kind(h: &Harness, node: &NodeHandle) -> Option<String> {
        let d = h.doc.borrow();
        ["data-pm-caret", "data-pm-selected", "data-pm-selection"]
            .into_iter()
            .find(|a| d.get_attribute(node.node_id(), a).is_some())
            .map(str::to_string)
    }

    #[test]
    fn caret_placement_and_moves_request_a_scroll_into_view() {
        let h = harness();
        let s = schema();
        let (mut st, mut view) = measured_empty_paragraphs(&h, &s);

        // doc(p(), p()) → 0[p 1]2[p 3]4.
        cursor_at(&mut st, 1);
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView],
            "the caret's first placement is a move — bring it into view"
        );
        assert!(view.last_caret.is_some(), "a caret was placed");

        cursor_at(&mut st, 3);
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView],
            "a caret that moved to another block is brought into view"
        );
    }

    #[test]
    fn a_repeat_caret_pass_on_an_unchanged_selection_requests_no_scroll() {
        let h = harness();
        let s = schema();
        let (mut st, mut view) = measured_empty_paragraphs(&h, &s);

        cursor_at(&mut st, 1);
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView]
        );

        // `update_all_carets` sweeps every mounted editor on every layout pass, so
        // this runs constantly with nothing changed. Scrolling here would yank a
        // user who has wheel-scrolled away from the caret straight back to it.
        assert_eq!(
            view.update_caret(&st),
            Vec::new(),
            "an unchanged caret must not re-request a scroll"
        );
        assert_eq!(view.update_caret(&st), Vec::new());
    }

    #[test]
    fn a_blink_toggle_requests_no_scroll() {
        let h = harness();
        let s = schema();
        let (mut st, mut view) = measured_empty_paragraphs(&h, &s);

        cursor_at(&mut st, 1);
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView]
        );

        // The blink driver never goes through `update_caret` at all, and a caret
        // pass landing between the two phases finds the same geometry — so neither
        // half of a blink scrolls anything.
        assert_eq!(view.set_caret_blink_visible(false), Some(true));
        assert_eq!(
            view.update_caret(&st),
            Vec::new(),
            "a caret in its hidden blink phase is still in the same place"
        );
        assert_eq!(view.set_caret_blink_visible(true), Some(true));
        assert_eq!(view.update_caret(&st), Vec::new());
    }

    #[test]
    fn a_caret_that_cannot_be_placed_requests_no_scroll() {
        let h = harness();
        let s = schema();
        // The same document, *unmeasured*: the mock reports no box for either
        // paragraph, so there is no caret geometry to scroll to.
        let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
        let mut st = state(s.clone(), doc_node(&s, vec![empty(), empty()]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        cursor_at(&mut st, 1);
        assert_eq!(view.update_caret(&st), Vec::new());
        assert!(view.last_caret.is_none(), "no caret was placed");
        assert!(
            view.scroll_anchor().is_none(),
            "and so there is nothing to anchor a scroll to"
        );
    }

    #[test]
    fn the_caret_arms_scroll_anchor_is_the_caret_element() {
        let h = harness();
        let s = schema();
        let (mut st, mut view) = measured_empty_paragraphs(&h, &s);

        cursor_at(&mut st, 1);
        view.update_caret(&st);

        let anchor = view.scroll_anchor().expect("a caret to scroll to");
        assert_eq!(
            overlay_kind(&h, anchor).as_deref(),
            Some("data-pm-caret"),
            "the view points the runtime at the caret bar itself"
        );
    }

    #[test]
    fn a_node_selection_requests_a_scroll_once_per_move() {
        let h = harness();
        let s = schema();
        let hr = || s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let mut st = state(s.clone(), doc_node(&s, vec![hr(), hr()]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let blocks = children(&h, h.container_id);
        {
            let mut m = h.mock.borrow_mut();
            m.__set_node_layout(blocks[0], 0.0, 0.0, 200.0, 2.0);
            m.__set_node_layout(blocks[1], 0.0, 40.0, 200.0, 2.0);
        }

        // doc(hr, hr) → the rules occupy one position each, at 0 and 1.
        st.selection = Selection::node_at(&st.doc, rinch_editor_core::Pos(0)).unwrap();
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView],
            "selecting a node brings its outline into view"
        );
        assert_eq!(
            view.update_caret(&st),
            Vec::new(),
            "re-rendering the same node selection must not scroll again"
        );
        let anchor = view.scroll_anchor().expect("a node outline to scroll to");
        assert_eq!(
            overlay_kind(&h, anchor).as_deref(),
            Some("data-pm-selected"),
            "the node-selection arm anchors on the outline, not the caret"
        );

        st.selection = Selection::node_at(&st.doc, rinch_editor_core::Pos(1)).unwrap();
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView],
            "moving the node selection down the document scrolls to the new node"
        );
    }

    // ── review #837: the arms the PR's own fixtures do not reach ──

    /// A node selection made *after* a text cursor existed must anchor on the
    /// outline, not on the (now hidden) caret div, which still exists.
    #[test]
    fn r837_a_node_selection_after_a_cursor_anchors_on_the_outline() {
        let h = harness();
        let s = schema();
        let empty = || s.branch("paragraph", Fragment::empty()).unwrap();
        let hr = || s.branch("horizontal_rule", Fragment::empty()).unwrap();
        // doc(p(), hr) → 0[p 1]2 (hr at 2)
        let mut st = state(s.clone(), doc_node(&s, vec![empty(), hr()]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let blocks = children(&h, h.container_id);
        {
            let mut m = h.mock.borrow_mut();
            m.__set_node_layout(blocks[0], 0.0, 0.0, 200.0, 20.0);
            m.__set_node_layout(blocks[1], 0.0, 40.0, 200.0, 2.0);
        }
        cursor_at(&mut st, 1);
        view.update_caret(&st);
        assert!(view.caret.is_some(), "positive control: a caret div exists");
        st.selection = Selection::node_at(&st.doc, rinch_editor_core::Pos(2)).unwrap();
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView]
        );
        let anchor = view.scroll_anchor().expect("an outline to scroll to");
        assert_eq!(
            overlay_kind(&h, anchor).as_deref(),
            Some("data-pm-selected")
        );
    }

    /// The cell arm: a request once per head move, none on a repeat pass, and
    /// the anchor is the HEAD cell's wash (not the fixed corner).
    #[test]
    fn r837_a_cell_selection_scrolls_once_per_head_move_and_anchors_on_the_head() {
        let h = harness();
        let s = schema();
        let table = rinch_editor_core::commands::build_table(&s, 3, 2).unwrap();
        let mut st = state(s.clone(), doc_node(&s, vec![table.clone()]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        // Give every node a box, stacked by id, so each cell has a distinct offset.
        {
            let mut m = h.mock.borrow_mut();
            let n = m.__node_count();
            for id in 1..n {
                m.__set_node_layout(NodeId(id), 3.0, 7.0 + id as f32, 100.0, 20.0);
            }
        }
        let map = rinch_editor_core::tables::TableMap::compute(&table, 1);
        let c00 = map.cell_at(0, 0).unwrap();
        let c11 = map.cell_at(1, 1).unwrap();
        let c21 = map.cell_at(2, 1).unwrap();
        st.selection = Selection::cell(rinch_editor_core::Pos(c00), rinch_editor_core::Pos(c11));
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView]
        );
        assert_eq!(
            view.update_caret(&st),
            Vec::new(),
            "a repeat pass scrolls nothing"
        );
        let head_style = |view: &RinchDomEditorView, head: usize| {
            let d = h.doc.borrow();
            let host = view
                .node_host_at(&st.doc, rinch_editor_core::Pos(head))
                .unwrap();
            let (ox, oy) = view.block_offset_in_container(&*d, host);
            (ox.round() as i32, oy.round() as i32)
        };
        let anchor_pos = |view: &RinchDomEditorView| {
            let a = view.scroll_anchor().expect("a wash to scroll to");
            let style = h
                .doc
                .borrow()
                .get_attribute(a.node_id(), "style")
                .unwrap_or_default();
            // Placed by `translate(x, y)`, not by insets (#906).
            let (x, y) = style
                .split(';')
                .find_map(|d| {
                    let (n, v) = d.split_once(':')?;
                    let args = v.trim().strip_prefix("translate(")?.strip_suffix(')')?;
                    let (x, y) = args.split_once(',')?;
                    let px = |s: &str| s.trim().trim_end_matches("px").parse::<f32>().ok();
                    (n.trim() == "transform")
                        .then(|| Some((px(x)?, px(y)?)))
                        .flatten()
                })
                .expect("a transform");
            (x.round() as i32, y.round() as i32)
        };
        assert_eq!(
            anchor_pos(&view),
            head_style(&view, c11),
            "anchored on the head cell"
        );
        st.selection = Selection::cell(rinch_editor_core::Pos(c00), rinch_editor_core::Pos(c21));
        assert_eq!(
            view.update_caret(&st),
            vec![ViewRequest::ScrollSelectionIntoView]
        );
        assert_eq!(
            anchor_pos(&view),
            head_style(&view, c21),
            "follows the moving head"
        );
    }

    #[test]
    fn node_selection_hides_caret_and_text_highlight() {
        let h = harness();
        let s = schema();
        let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "ab"), hr]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        // Seed a caret so there is something for the node-selection pass to clear.
        view.position_caret(1.0, 2.0, 18.0);
        assert!(view.last_caret.is_some());

        // Select the hr and run the post-layout pass.
        st.selection =
            rinch_editor_core::Selection::node_at(&st.doc, rinch_editor_core::Pos(4)).unwrap();
        view.update_caret(&st);

        // A node selection shows neither a caret nor a text highlight (the mock has
        // no geometry, so the outline itself can't be positioned here — but the
        // branch runs without panicking and leaves no stale overlays).
        assert!(
            view.last_caret.is_none(),
            "caret hidden for a node selection"
        );
        assert!(
            view.selection_rects.is_empty(),
            "no text highlight for a node selection"
        );
    }

    // ── inline decorations ───────────────────────────────────────────────────

    /// A stand-in for a consumer's spellcheck plugin: it reports whatever ranges
    /// the test puts in it, so a test can change the decorations without touching
    /// the document (and vice versa).
    struct SpellPlugin {
        ranges: RefCell<Vec<(usize, usize, &'static str)>>,
    }

    impl SpellPlugin {
        fn new() -> Rc<SpellPlugin> {
            Rc::new(SpellPlugin {
                ranges: RefCell::new(Vec::new()),
            })
        }
        fn set(&self, ranges: &[(usize, usize, &'static str)]) {
            *self.ranges.borrow_mut() = ranges.to_vec();
        }
    }

    impl rinch_editor_core::Plugin for SpellPlugin {
        fn key(&self) -> rinch_editor_core::PluginKey {
            rinch_editor_core::PluginKey("test.spell")
        }
        fn decorations(&self, _state: &EditorState) -> DecorationSet {
            DecorationSet::new(
                self.ranges
                    .borrow()
                    .iter()
                    .map(|(from, to, class)| {
                        rinch_editor_core::decoration::Decoration::inline(
                            rinch_editor_core::Pos(*from),
                            rinch_editor_core::Pos(*to),
                            rinch_editor_core::Attrs::new().with("class", *class),
                        )
                    })
                    .collect(),
            )
        }
    }

    /// A state over `doc` whose only plugin is `spell`.
    fn spell_state(s: Rc<Schema>, doc: Node, spell: &Rc<SpellPlugin>) -> EditorState {
        EditorState::create(s, doc, vec![spell.clone()])
    }

    /// `(element id, class)` for every `[data-pm-deco]` segment under `id`, in
    /// document order — the squiggles as the host actually carries them.
    fn deco_spans(h: &Harness, id: NodeId) -> Vec<(NodeId, String, String)> {
        let mut out = Vec::new();
        if attr_of(h, id, "data-pm-deco").is_some() {
            out.push((
                id,
                attr_of(h, id, "class").unwrap_or_default(),
                text(h, id).unwrap_or_default(),
            ));
        }
        for child in children(h, id) {
            out.extend(deco_spans(h, child));
        }
        out
    }

    #[test]
    fn inline_decoration_wraps_its_range_in_a_deco_span() {
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        // "hello world": the paragraph's content starts at 1, so "world" is 7..12.
        spell.set(&[(7, 12, "pm-spell-error")]);
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![para(&s, "hello world")]),
            &spell,
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        // The run is split, but the block still reads back as the model text — the
        // caret map and both backends' hit-tests depend on that.
        assert_eq!(text(&h, p).as_deref(), Some("hello world"));
        let spans = deco_spans(&h, p);
        assert_eq!(spans.len(), 1, "expected exactly one decorated segment");
        assert_eq!(spans[0].1, "pm-spell-error");
        assert_eq!(spans[0].2, "world");
        // The segment is a real element inside a wrapper, not the block itself.
        assert_eq!(tag(&h, spans[0].0).as_deref(), Some("span"));
        let wrapper = children(&h, p)[0];
        assert_eq!(
            attr_of(&h, wrapper, "data-pm-deco-run").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn an_undecorated_run_is_still_a_bare_text_node() {
        // The whole point of building the wrapper lazily: a document with no
        // decorations projects exactly the DOM it did before they existed.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        let st = spell_state(s.clone(), doc_node(&s, vec![para(&s, "hello")]), &spell);
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let kid = children(&h, p)[0];
        assert_eq!(tag(&h, kid), None, "an undecorated run stays a text node");
        assert!(deco_spans(&h, p).is_empty());
    }

    #[test]
    fn dropping_the_decoration_restores_a_plain_text_node() {
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        spell.set(&[(1, 6, "pm-spell-error")]);
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![para(&s, "helo world")]),
            &spell,
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let p = children(&h, h.container_id)[0];
        let wrapper = children(&h, p)[0];
        assert_eq!(deco_spans(&h, p).len(), 1, "precondition: squiggle mounted");

        // The user accepted the correction, the plugin drops the range — a
        // decoration-only change, with no transaction touching the document.
        spell.set(&[]);
        view.update_dom(&st, &st);

        assert!(deco_spans(&h, p).is_empty());
        let kid = children(&h, p)[0];
        assert_eq!(tag(&h, kid), None, "the run is a bare text node again");
        assert_eq!(text(&h, p).as_deref(), Some("helo world"));
        assert_eq!(
            tag(&h, wrapper),
            None,
            "#719: the displaced wrapper must be discarded, not merely detached"
        );
    }

    #[test]
    fn a_decoration_only_change_leaves_the_document_nodes_alone() {
        // Design A4: decorations diff independently of the document. The block
        // elements must keep their identity, or every squiggle would re-create the
        // paragraph it lands in (and with it the caret's host).
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![para(&s, "helo"), para(&s, "there")]),
            &spell,
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let blocks_before = children(&h, h.container_id);
        let untouched_text = children(&h, blocks_before[1])[0];

        spell.set(&[(1, 5, "pm-spell-error")]);
        view.update_dom(&st, &st);

        assert_eq!(children(&h, h.container_id), blocks_before);
        assert_eq!(
            children(&h, blocks_before[1])[0],
            untouched_text,
            "an undecorated block's text node was rebuilt by a decoration change"
        );
        assert_eq!(deco_spans(&h, blocks_before[0]).len(), 1);
    }

    #[test]
    fn a_squiggle_survives_an_edit_that_leaves_the_decoration_set_equal() {
        // The trap this guards: the document diff rebuilds the decorated run (a
        // decorated run cannot be patched in place), and the decoration set is
        // *unchanged*, so a pass that ran only on a decoration diff would leave the
        // fresh text node bare.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        spell.set(&[(1, 5, "pm-spell-error")]); // "helo" at the start of the block
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![para(&s, "helo world")]),
            &spell,
        );
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let p = children(&h, h.container_id)[0];

        // Type at the END of the block: the run's text changes, the misspelling at
        // the start does not move, so the plugin reports the very same range.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(11)));
        tr.insert_text("!").unwrap();
        let next = st.apply(tr);
        assert_eq!(
            st.decorations(),
            next.decorations(),
            "precondition: this edit must leave the decoration set equal"
        );
        view.update_dom(&st, &next);

        assert_eq!(text(&h, p).as_deref(), Some("helo world!"));
        let spans = deco_spans(&h, p);
        assert_eq!(
            spans.len(),
            1,
            "the squiggle was lost when the run was rebuilt"
        );
        assert_eq!(spans[0].2, "helo");
    }

    #[test]
    fn the_caret_map_is_blind_to_the_deco_wrapper() {
        // An inline element contributes nothing to the flat IFC text, so splitting
        // a run must not move a single caret address — in either direction.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        let doc = doc_node(&s, vec![para(&s, "héllo wörld")]);
        let st = spell_state(s.clone(), doc.clone(), &spell);
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let addresses: Vec<Option<(usize, usize)>> = (1..=12)
            .map(|i| view.caret_address(&doc, rinch_editor_core::Pos(i)))
            .collect();

        spell.set(&[(7, 12, "pm-spell-error")]);
        view.update_dom(&st, &st);
        assert_eq!(deco_spans(&h, children(&h, h.container_id)[0]).len(), 1);

        for (i, before) in addresses.iter().enumerate() {
            let pos = rinch_editor_core::Pos(i + 1);
            assert_eq!(
                &view.caret_address(&doc, pos),
                before,
                "decorating the run moved the caret address for {pos:?}"
            );
            // And the hit-test inverse still round-trips through the wrapper.
            if let Some((block, byte)) = before {
                assert_eq!(view.pos_at(*block, *byte), Some(pos));
            }
        }
    }

    #[test]
    fn a_hit_on_a_deco_segment_resolves_to_its_run() {
        // A pointer lands on the `<span data-pm-deco>`, not on the run's own host —
        // it must still resolve to the text run, like a hit on a mark wrapper does.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        spell.set(&[(7, 12, "pm-spell-error")]);
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![para(&s, "hello world")]),
            &spell,
        );
        let view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let segment = deco_spans(&h, p)[0].0;
        let (pos, node) = view
            .node_pos_for_host(segment.0)
            .expect("a hit on a decoration segment must resolve");
        assert_eq!(pos, 1, "the run starts just inside the paragraph");
        assert_eq!(node.text(), Some("hello world"));
    }

    #[test]
    fn a_decorated_marked_run_keeps_its_mark_wrappers() {
        // Decoration segments live *inside* the mark chain, so a bold misspelling
        // is still bold and `<strong>` is still what the paragraph holds.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        spell.set(&[(1, 5, "pm-spell-error")]);
        let bold = mk(&s, "bold", rinch_editor_core::Attrs::new());
        let st = spell_state(
            s.clone(),
            doc_node(&s, vec![marked_para(&s, "helo", vec![bold])]),
            &spell,
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let p = children(&h, h.container_id)[0];
        let strong = children(&h, p)[0];
        assert_eq!(tag(&h, strong).as_deref(), Some("strong"));
        assert_eq!(attr_of(&h, strong, "data-pm-mark").as_deref(), Some("bold"));
        let spans = deco_spans(&h, p);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].2, "helo");
        assert_eq!(text(&h, strong).as_deref(), Some("helo"));
    }

    #[test]
    fn decorations_spanning_runs_and_overlapping_each_other_flatten_cleanly() {
        // `run_decos` is the whole of the clipping/overlap policy, so test it
        // directly: a range is clipped to the run, an overlap yields one segment
        // per distinct stretch carrying both classes, and abutting stretches with
        // the same class merge back into one.
        let d = |from: usize, to: usize, class: &str| InlineDeco {
            from,
            to,
            class: Rc::from(class),
        };
        // Run occupying model chars 10..20.
        let decos = vec![d(5, 13, "a"), d(11, 16, "b"), d(30, 40, "a")];
        let out = run_decos(&decos, 10, 10);
        let seen: Vec<(usize, usize, &str)> =
            out.iter().map(|r| (r.start, r.end, &*r.class)).collect();
        assert_eq!(seen, vec![(0, 1, "a"), (1, 3, "a b"), (3, 6, "b")]);

        // Two decorations that abut and agree are one segment, not two.
        let merged = run_decos(&[d(10, 13, "a"), d(13, 16, "a")], 10, 10);
        let seen: Vec<(usize, usize, &str)> =
            merged.iter().map(|r| (r.start, r.end, &*r.class)).collect();
        assert_eq!(seen, vec![(0, 6, "a")]);

        // A range that misses the run entirely decorates nothing.
        assert!(run_decos(&[d(30, 40, "a")], 10, 10).is_empty());
    }

    #[test]
    fn a_decoration_reaching_across_a_mark_boundary_decorates_both_runs() {
        // Known limitation, pinned here: the model's runs are the unit of
        // decorating (a mark wrapper is shared across runs since #1833, a
        // decoration segment is not), so a decoration crossing a mark boundary
        // becomes one segment per run rather than one continuous element. Visually identical for an
        // underline; worth knowing for anything that draws a box.
        let h = harness();
        let s = schema();
        let spell = SpellPlugin::new();
        let bold = mk(&s, "bold", rinch_editor_core::Attrs::new());
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text("he").unwrap(),
                    s.text_with_marks("lo", vec![bold]).unwrap(),
                ]),
            )
            .unwrap();
        spell.set(&[(1, 5, "pm-spell-error")]);
        let st = spell_state(s.clone(), doc_node(&s, vec![p]), &spell);
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);

        let block = children(&h, h.container_id)[0];
        let spans = deco_spans(&h, block);
        assert_eq!(spans.len(), 2, "one segment per model run");
        assert_eq!(spans[0].2, "he");
        assert_eq!(spans[1].2, "lo");
        assert_eq!(text(&h, block).as_deref(), Some("helo"));
    }

    /// #891: the host's generated text is stepped over both ways. `ab` + an
    /// `::after`'s "XYZ" (host 2..5, at model 2) + `cd` + a `::before`'s "Q"
    /// (host 7..8, at model 4) + `ef`, laid out `abXYZcdQef`.
    #[test]
    fn generated_flat_ranges_are_stepped_over_both_ways() {
        let g = [
            GeneratedFlatRange {
                start: 2,
                end: 5,
                after: true,
            },
            GeneratedFlatRange {
                start: 7,
                end: 8,
                after: false,
            },
        ];
        // Model 2 is at the `::after`'s start: before it. Model 4 is at the
        // `::before`'s start: past it.
        let host: Vec<usize> = (0..=6).map(|m| host_flat_byte(&g, m)).collect();
        assert_eq!(host, vec![0, 1, 2, 6, 8, 9, 10]);
        // Every host byte inside generated text is the model byte it stands at.
        let model: Vec<usize> = (0..=10).map(|h| model_flat_byte(&g, h)).collect();
        assert_eq!(model, vec![0, 1, 2, 2, 2, 2, 3, 4, 4, 5, 6]);
        for m in 0..=6 {
            assert_eq!(model_flat_byte(&g, host_flat_byte(&g, m)), m);
        }
        // None: the identity.
        assert_eq!(host_flat_byte(&[], 7), 7);
        assert_eq!(model_flat_byte(&[], 7), 7);
    }

    /// rinch-dom's flat offsets as they were until #1122: a `<br>` is
    /// `"\n"`, a tab four spaces. Kept as the case of a host whose tab is
    /// several bytes.
    const DESKTOP: FlatWidths = FlatWidths {
        line_break: 1,
        tab: 4,
        substituted: &[
            ('\u{2028}', 5),
            ('\u{2029}', 5),
            ('\u{85}', 3),
            ('\u{c}', 0),
        ],
    };
    /// The browser's: a `<br>` has no text, a tab is its one character.
    const WEB: FlatWidths = FlatWidths::UTF8;

    #[test]
    fn textblock_flat_byte_spans_runs_and_multibyte() {
        // paragraph(text("ab",[bold]), text("é"), text("cd")) — flat text "abécd",
        // where 'é' is one char but two UTF-8 bytes.
        let s = schema();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text_with_marks("ab", vec![mk(&s, "bold", Default::default())])
                        .unwrap(),
                    s.text("é").unwrap(),
                    s.text("cd").unwrap(),
                ]),
            )
            .unwrap();
        assert_eq!(textblock_flat_byte(&p, 0, WEB), 0); // start
        assert_eq!(textblock_flat_byte(&p, 2, WEB), 2); // after "ab"
        assert_eq!(textblock_flat_byte(&p, 3, WEB), 4); // after "é" — past its 2 bytes
        assert_eq!(textblock_flat_byte(&p, 4, WEB), 5); // after "c"
        assert_eq!(textblock_flat_byte(&p, 5, WEB), 6); // end
    }

    /// A hard break is worth the host's `line_break_flat_bytes` in the caret map
    /// (#1099), an image nothing, and the two halves of the map invert each
    /// other around both. `ab<br>cd<img>ef`: model chars a0 b1 br2 c3 d4 img5 e6 f7.
    #[test]
    fn the_caret_map_gives_a_break_the_hosts_bytes_and_an_image_none() {
        let s = schema();
        use rinch_editor_core::Attrs;
        let leaf =
            |name: &str, attrs: Attrs| s.create_node(name, attrs, Fragment::empty()).unwrap();
        let img = Attrs::new().with("src", "x.png");
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text("ab").unwrap(),
                    leaf("hard_break", Attrs::new()),
                    s.text("cd").unwrap(),
                    leaf("image", img),
                    s.text("ef").unwrap(),
                ]),
            )
            .unwrap();
        // rinch-dom: the `<br>` is one byte, `"ab\ncdef"`.
        let desktop: Vec<usize> = (0..=8)
            .map(|c| textblock_flat_byte(&p, c, DESKTOP))
            .collect();
        assert_eq!(desktop, [0, 1, 2, 3, 4, 5, 5, 6, 7]);
        // The browser: no byte for either leaf, `"abcdef"`.
        let web: Vec<usize> = (0..=8).map(|c| textblock_flat_byte(&p, c, WEB)).collect();
        assert_eq!(web, [0, 1, 2, 2, 3, 4, 4, 5, 6]);
        // The inverse: the `"\n"` byte (2) is before the break, the next after
        // it; an image's shared byte (5 / 4) reads as the side before it.
        let back: Vec<usize> = (0..=7).map(|b| ifc_byte_to_char(&p, b, DESKTOP)).collect();
        assert_eq!(back, [0, 1, 2, 3, 4, 5, 7, 8]);
        let back_web: Vec<usize> = (0..=6).map(|b| ifc_byte_to_char(&p, b, WEB)).collect();
        assert_eq!(back_web, [0, 1, 2, 4, 5, 7, 8]);
    }

    /// A tab is worth the host's `tab_flat_bytes` in the caret map (#1109),
    /// across runs and beside a multibyte char, and a byte inside a tab's four
    /// is before it for the first, after it for the rest. `a\té` + bold `\tb`: chars a0 tab1 é2 tab3 b4.
    #[test]
    fn the_caret_map_gives_a_tab_the_hosts_bytes() {
        let s = schema();
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![
                    s.text("a\té").unwrap(),
                    s.text_with_marks("\tb", vec![mk(&s, "bold", Default::default())])
                        .unwrap(),
                ]),
            )
            .unwrap();
        // rinch-dom: `"a    é    b"`, é two bytes.
        let desktop: Vec<usize> = (0..=5)
            .map(|c| textblock_flat_byte(&p, c, DESKTOP))
            .collect();
        assert_eq!(desktop, [0, 1, 5, 7, 11, 12]);
        let web: Vec<usize> = (0..=5).map(|c| textblock_flat_byte(&p, c, WEB)).collect();
        assert_eq!(web, [0, 1, 2, 4, 5, 6]);
        // Inverse: bytes 2 inside the first tab (1..5) is before it (1), 3 and
        // 4 after it (2); 6 inside é reads after it; 8 before the second tab.
        let back: Vec<usize> = (0..=12).map(|b| ifc_byte_to_char(&p, b, DESKTOP)).collect();
        assert_eq!(back, [0, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4, 4, 5]);
        let back_web: Vec<usize> = (0..=6).map(|b| ifc_byte_to_char(&p, b, WEB)).collect();
        assert_eq!(back_web, [0, 1, 2, 3, 3, 4, 5]);
    }

    // ── The trailing-break placeholder (#1172) ─────────────────────────────

    fn hard_break(s: &Schema) -> Node {
        s.create_node(
            "hard_break",
            rinch_editor_core::Attrs::new(),
            Fragment::empty(),
        )
        .unwrap()
    }

    fn para_of(s: &Schema, kids: Vec<Node>) -> Node {
        s.branch("paragraph", Fragment::from_children(kids))
            .unwrap()
    }

    /// `(tag, data-pm-type, is the placeholder)` for each child of `id`.
    fn shape(h: &Harness, id: NodeId) -> Vec<(String, Option<String>, bool)> {
        children(h, id)
            .into_iter()
            .map(|c| {
                let d = h.doc.borrow();
                (
                    d.tag_name(c).unwrap_or_else(|| "#text".into()),
                    d.get_attribute(c, "data-pm-type"),
                    d.get_attribute(c, TRAILING_BREAK_ATTR).is_some(),
                )
            })
            .collect()
    }

    fn placeholders(h: &Harness, id: NodeId) -> usize {
        shape(h, id).iter().filter(|c| c.2).count()
    }

    fn is_last_child_placeholder(h: &Harness, id: NodeId) -> bool {
        shape(h, id).last().is_some_and(|c| c.2)
    }

    /// A paragraph ending in a hard break gets a second, unmodelled `<br>`
    /// after it; one with anything after its break does not.
    #[test]
    fn a_trailing_hard_break_gets_a_placeholder_after_it() {
        let h = harness();
        let s = schema();
        let st = state(
            s.clone(),
            doc_node(
                &s,
                vec![
                    para_of(&s, vec![s.text("ab").unwrap(), hard_break(&s)]),
                    para_of(
                        &s,
                        vec![s.text("ab").unwrap(), hard_break(&s), s.text("cd").unwrap()],
                    ),
                    para_of(&s, vec![hard_break(&s)]),
                ],
            ),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let blocks = children(&h, h.container_id);
        let br = |ph: bool| {
            (
                "br".to_string(),
                (!ph).then(|| "hard_break".to_string()),
                ph,
            )
        };
        let t = ("#text".to_string(), None, false);
        assert_eq!(shape(&h, blocks[0]), vec![t.clone(), br(false), br(true)]);
        assert_eq!(shape(&h, blocks[1]), vec![t.clone(), br(false), t.clone()]);
        assert_eq!(shape(&h, blocks[2]), vec![br(false), br(true)]);
    }

    /// Every edit that makes or unmakes a trailing break shows or drops the
    /// placeholder, which is always the last child; a dropped one is
    /// discarded (#719), not merely removed.
    #[test]
    fn the_placeholder_follows_edits_and_stays_last() {
        let h = harness();
        let s = schema();
        let mut st = state(s.clone(), doc_node(&s, vec![para(&s, "ab")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let p = children(&h, h.container_id)[0];
        assert_eq!(placeholders(&h, p), 0, "no break, no placeholder");

        let mut step = |st: &mut EditorState, f: &dyn Fn(&mut rinch_editor_core::Transaction)| {
            let mut tr = st.tr();
            f(&mut tr);
            let next = st.apply(tr);
            view.update_dom(st, &next);
            *st = next;
        };

        // Shift+Enter at the end: a trailing break.
        step(&mut st, &|tr| {
            tr.set_selection(Selection::cursor(rinch_editor_core::Pos(3)));
            tr.replace_selection_with(hard_break(&s)).unwrap();
        });
        assert_eq!(placeholders(&h, p), 1);
        assert!(is_last_child_placeholder(&h, p));
        let first = *children(&h, p).last().unwrap();

        // Typing before the break keeps it trailing: the same placeholder.
        step(&mut st, &|tr| {
            tr.set_selection(Selection::cursor(rinch_editor_core::Pos(3)));
            tr.insert_text("X").unwrap();
        });
        assert_eq!(placeholders(&h, p), 1);
        assert_eq!(*children(&h, p).last().unwrap(), first, "kept, not rebuilt");

        // A second break at the end: still one placeholder, still last — the
        // new break goes in before it.
        step(&mut st, &|tr| {
            tr.set_selection(Selection::cursor(rinch_editor_core::Pos(5)));
            tr.replace_selection_with(hard_break(&s)).unwrap();
        });
        assert_eq!(placeholders(&h, p), 1);
        assert!(is_last_child_placeholder(&h, p));
        assert_eq!(children(&h, p).len(), 4, "abX, br, br, placeholder");

        // Typing after the last break: it is no longer trailing.
        step(&mut st, &|tr| {
            tr.set_selection(Selection::cursor(rinch_editor_core::Pos(6)));
            tr.insert_text("y").unwrap();
        });
        assert_eq!(placeholders(&h, p), 0);
        assert_eq!(
            tag(&h, NodeId(first.0)),
            None,
            "#719: the dropped placeholder is discarded"
        );

        // Deleting the text after it makes it trailing again.
        step(&mut st, &|tr| {
            tr.delete(6, 7).unwrap();
        });
        assert_eq!(placeholders(&h, p), 1);
        assert!(is_last_child_placeholder(&h, p));

        // Deleting the trailing break: the block ends in a break still (the
        // first one), so one placeholder, last.
        step(&mut st, &|tr| {
            tr.delete(5, 6).unwrap();
        });
        assert_eq!(placeholders(&h, p), 1);
        assert!(is_last_child_placeholder(&h, p));
        // And the last break: none.
        step(&mut st, &|tr| {
            tr.delete(4, 5).unwrap();
        });
        assert_eq!(placeholders(&h, p), 0);
        assert_eq!(text(&h, p).as_deref(), Some("abX"));
    }

    /// A marked hard break is wrapped in its mark; the placeholder is the
    /// block's own child, after the wrapper.
    #[test]
    fn a_marked_trailing_break_puts_the_placeholder_after_its_wrapper() {
        let h = harness();
        let s = schema();
        let bold = mk(&s, "bold", Default::default());
        let br = s
            .create_node(
                "hard_break",
                rinch_editor_core::Attrs::new(),
                Fragment::empty(),
            )
            .unwrap()
            .with_marks(vec![bold.clone()]);
        let st = state(
            s.clone(),
            doc_node(
                &s,
                vec![para_of(
                    &s,
                    vec![s.text_with_marks("ab", vec![bold]).unwrap(), br],
                )],
            ),
        );
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let p = children(&h, h.container_id)[0];
        assert!(is_last_child_placeholder(&h, p), "{:?}", shape(&h, p));
        assert_eq!(placeholders(&h, p), 1);
    }

    /// A code block whose text ends in a newline gets the placeholder too
    /// (ProseMirror's rule), after the text; text that stops ending in one
    /// drops it.
    #[test]
    fn a_code_block_ending_in_a_newline_gets_a_placeholder() {
        let h = harness();
        let s = schema();
        let code = |t: &str| {
            s.branch("code_block", Fragment::from_node(s.text(t).unwrap()))
                .unwrap()
        };
        let mut st = state(s.clone(), doc_node(&s, vec![code("ab\n"), code("cd")]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let blocks = children(&h, h.container_id);
        assert_eq!(placeholders(&h, blocks[0]), 1);
        assert!(is_last_child_placeholder(&h, blocks[0]));
        assert_eq!(placeholders(&h, blocks[1]), 0, "no final newline");
        // Typing after the newline patches the text in place; it no longer
        // ends in one.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(4)));
        tr.insert_text("x").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        st = next;
        assert_eq!(placeholders(&h, blocks[0]), 0);
        // And a newline at the end of the second block's text.
        let mut tr = st.tr();
        tr.set_selection(Selection::cursor(rinch_editor_core::Pos(9)));
        tr.insert_text("\n").unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert_eq!(placeholders(&h, blocks[1]), 1, "{:?}", shape(&h, blocks[1]));
        assert!(is_last_child_placeholder(&h, blocks[1]));
    }

    /// The `<img>` of an image asks for the source the app chose from the
    /// node's attrs (`set_image_source`), and asks again when they change:
    /// two images with one `src` can be two pictures.
    #[test]
    fn an_images_host_asks_for_the_source_the_app_chose_from_its_attrs() {
        use rinch_editor_core::{AttrValue, Attrs, SetNodeAttrStep};
        crate::images::set_image_source(|attrs| {
            let title = attrs.get_str("title").filter(|t| !t.is_empty())?;
            Some(format!("{}#{title}", attrs.get_str("src")?))
        });
        let h = harness();
        let s = schema();
        let img = |title: &str| {
            let mut attrs = Attrs::new().with("src", AttrValue::from("blob:1"));
            if !title.is_empty() {
                attrs = attrs.with("title", AttrValue::from(title));
            }
            s.create_node("image", attrs, Fragment::empty()).unwrap()
        };
        let p = s
            .branch(
                "paragraph",
                Fragment::from_children(vec![img("a"), img(""), img("b")]),
            )
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![p]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let srcs = |h: &Harness| -> Vec<String> {
            children(h, children(h, h.container_id)[0])
                .into_iter()
                .map(|id| h.doc.borrow().get_attribute(id, "src").unwrap_or_default())
                .collect()
        };
        assert_eq!(srcs(&h), ["blob:1#a", "blob:1", "blob:1#b"]);

        // An attribute edit asks for the new source, on the same `<img>`.
        let host = children(&h, children(&h, h.container_id)[0])[1];
        let mut tr = st.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            2,
            "title",
            AttrValue::from("c"),
        )))
        .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert_eq!(srcs(&h), ["blob:1#a", "blob:1#c", "blob:1#b"]);
        assert_eq!(children(&h, children(&h, h.container_id)[0])[1], host);
        crate::images::clear_image_source();
    }

    /// An image's app data attributes and `width` reach its host element as
    /// themselves and the `width` hint, and follow an attribute edit in place:
    /// the same `<img>`, so a picture on screen is not torn down to change
    /// them. A data attribute rinch reserves on the node is never written over
    /// the view's own (`data-pm-type`).
    #[test]
    fn an_images_data_attrs_and_width_reach_its_host_and_follow_an_edit_in_place() {
        use rinch_editor_core::{AttrValue, Attrs, SetNodeAttrStep};
        let h = harness();
        let s = schema();
        let img = s
            .create_node(
                "image",
                Attrs::new().with("src", AttrValue::from("x.png")),
                Fragment::empty(),
            )
            .unwrap();
        let p = s
            .branch("paragraph", Fragment::from_children(vec![img]))
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![p]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let host = children(&h, children(&h, h.container_id)[0])[0];
        let attr = |name: &str| h.doc.borrow().get_attribute(host, name);
        assert_eq!(tag(&h, host).as_deref(), Some("img"));
        assert_eq!((attr("data-ref"), attr("width")), (None, None));

        let mut tr = st.tr();
        for (name, value) in [
            ("data-ref", "r1"),
            ("data-annotation-id", "a1"),
            ("data-pm-type", "paragraph"),
            ("data-rid", "7"),
        ] {
            tr.step(Box::new(SetNodeAttrStep::new(
                1,
                name,
                AttrValue::from(value),
            )))
            .unwrap();
        }
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "width",
            AttrValue::Int(320),
        )))
        .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert_eq!(
            children(&h, children(&h, h.container_id)[0])[0],
            host,
            "the same <img>"
        );
        assert_eq!(attr("data-ref").as_deref(), Some("r1"));
        assert_eq!(attr("data-annotation-id").as_deref(), Some("a1"));
        assert_eq!(attr("data-pm-type").as_deref(), Some("image"));
        assert_eq!(attr("data-rid"), None);
        assert_eq!(attr("width").as_deref(), Some("320"));
        assert_eq!(attr("src").as_deref(), Some("x.png"));

        // A width past the bound is shown at the bound.
        let mut tr = next.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "width",
            AttrValue::Int(99_999_999_999_999),
        )))
        .unwrap();
        let huge = next.apply(tr);
        view.update_dom(&next, &huge);
        assert_eq!(attr("width").as_deref(), Some("65535"));
        view.update_dom(&huge, &next);
        assert_eq!(attr("width").as_deref(), Some("320"));

        // One changed, one taken away; and a width that is not positive is no
        // hint.
        let mut tr = next.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "data-ref",
            AttrValue::from("r2"),
        )))
        .unwrap();
        tr.step(Box::new(SetNodeAttrStep {
            pos: 1,
            attr: "data-annotation-id".into(),
            value: None,
        }))
        .unwrap();
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "width",
            AttrValue::Int(0),
        )))
        .unwrap();
        let last = next.apply(tr);
        view.update_dom(&next, &last);
        assert_eq!(attr("data-ref").as_deref(), Some("r2"));
        assert_eq!((attr("data-annotation-id"), attr("width")), (None, None));
        assert_eq!(attr("data-pm-type").as_deref(), Some("image"));
    }

    /// Review of PR #1518 (mutant M17): a reserved name the model carried
    /// (a step can store any attr) and then drops is never removed from the
    /// host: `data-pm-type` stays the view's own.
    #[test]
    fn review_1518_dropping_a_reserved_model_attr_keeps_the_hosts_own() {
        use rinch_editor_core::{AttrValue, Attrs, SetNodeAttrStep};
        let h = harness();
        let s = schema();
        let img = s
            .create_node(
                "image",
                Attrs::new().with("src", AttrValue::from("x.png")),
                Fragment::empty(),
            )
            .unwrap();
        let p = s
            .branch("paragraph", Fragment::from_children(vec![img]))
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![p]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let host = children(&h, children(&h, h.container_id)[0])[0];
        let mut tr = st.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "data-pm-type",
            AttrValue::from("paragraph"),
        )))
        .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        let mut tr = next.tr();
        tr.step(Box::new(SetNodeAttrStep {
            pos: 1,
            attr: "data-pm-type".into(),
            value: None,
        }))
        .unwrap();
        let last = next.apply(tr);
        view.update_dom(&next, &last);
        assert_eq!(
            h.doc
                .borrow()
                .get_attribute(host, "data-pm-type")
                .as_deref(),
            Some("image")
        );
    }
    /// The host shows only the data attributes the node keeps (the first
    /// `MAX_DATA_ATTRS` by name): one pushed past the bound by an edit is taken
    /// off the host, not left stale, and a value past `MAX_DATA_ATTR_VALUE` is
    /// never stamped.
    #[test]
    fn the_host_shows_only_the_data_attrs_the_node_keeps() {
        use rinch_editor_core::schema::{MAX_DATA_ATTR_VALUE, MAX_DATA_ATTRS};
        use rinch_editor_core::{AttrValue, Attrs, SetNodeAttrStep};
        let h = harness();
        let s = schema();
        let mut attrs: Vec<(String, AttrValue)> = vec![("src".into(), AttrValue::from("x.png"))];
        attrs.extend((0..MAX_DATA_ATTRS).map(|i| (format!("data-k{i:02}"), AttrValue::from("v"))));
        attrs.push((
            "data-big".into(),
            AttrValue::from("x".repeat(MAX_DATA_ATTR_VALUE + 1)),
        ));
        let img = s
            .create_node("image", Attrs::from_iter(attrs), Fragment::empty())
            .unwrap();
        let p = s
            .branch("paragraph", Fragment::from_children(vec![img]))
            .unwrap();
        let st = state(s.clone(), doc_node(&s, vec![p]));
        let mut view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let host = children(&h, children(&h, h.container_id)[0])[0];
        let attr = |name: &str| h.doc.borrow().get_attribute(host, name);
        let last = format!("data-k{:02}", MAX_DATA_ATTRS - 1);
        assert_eq!(attr(&last).as_deref(), Some("v"));
        assert_eq!(attr("data-big"), None);
        // `data-a` sorts first and pushes the last one past the bound.
        let mut tr = st.tr();
        tr.step(Box::new(SetNodeAttrStep::new(
            1,
            "data-a",
            AttrValue::from("new"),
        )))
        .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert_eq!(attr("data-a").as_deref(), Some("new"));
        assert_eq!(
            attr(&last),
            None,
            "pushed past the bound, taken off the host"
        );
    }

    /// Review of PR #1518: names rinch's desktop runtime reads on any element
    /// (`data-tcm-item` by the text menu's hit walk, `data-block-index` by the
    /// click text-hit walk, the input caret/selection state paint reads) must
    /// not reach an editor `<img>` from a paste.
    #[test]
    fn review_1518_a_pasted_runtime_name_never_reaches_the_host() {
        let h = harness();
        let s = schema();
        let slice = rinch_editor_core::serialize::slice_from_html(
            &s,
            r#"<p><img src="x.png" data-tcm-item="3" data-block-index="0" data-cursor-pos="1" data-selection-start="0" data-cursor-visible="true" data-text-sel-start="0" data-text-sel-end="1" data-video-player="v" data-user-rid="1" data-ref="ok"></p>"#,
        )
        .unwrap();
        let p = slice.content.child(0).clone();
        let st = state(s.clone(), doc_node(&s, vec![p]));
        let _view = RinchDomEditorView::new(h.container.clone(), doc_ref(&h), &st);
        let host = children(&h, children(&h, h.container_id)[0])[0];
        let attr = |name: &str| h.doc.borrow().get_attribute(host, name);
        assert_eq!(tag(&h, host).as_deref(), Some("img"));
        assert_eq!(attr("data-ref").as_deref(), Some("ok"), "positive control");
        let leaked: Vec<&str> = [
            "data-tcm-item",
            "data-block-index",
            "data-cursor-pos",
            "data-selection-start",
            "data-cursor-visible",
            "data-text-sel-start",
            "data-text-sel-end",
            "data-video-player",
            "data-user-rid",
        ]
        .into_iter()
        .filter(|n| attr(n).is_some())
        .collect();
        assert!(leaked.is_empty(), "stamped on the editor <img>: {leaked:?}");
    }
}

/// Issue #1182: a cell's grid span is its [`TableMap`] rectangle, not its raw
/// `colspan` / `rowspan`. A table that reaches the model by an app's own
/// transaction can say `colspan = 3_000_000` or `rowspan = i64::MAX` (a
/// load caps `colspan` at 1000 since #1214, and `rowspan` at 65534 since #1244); written raw, that asked the host's CSS grid for that many
/// implicit tracks (and on the desktop, four stacked cells of `span 10000` —
/// Stylo's clamp — made more grid lines than Taffy's `i16` lines hold, and it
/// panicked). The map cuts a span at the grid's edge, and a cell past the
/// grid's capped width is in no slot at all.
#[cfg(test)]
mod table_span_tests {
    use super::*;

    thread_local! {
        /// How many times a table computed its map and placed its cells.
        pub(super) static SYNCS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    use rinch_core::dom::NodeId;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_editor_core::model::{Attrs, Fragment};
    use rinch_editor_core::{AttrValue, EditorState, Schema, default_plugins};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Rig {
        doc: Rc<RefCell<dyn DomDocument>>,
        container: NodeHandle,
        container_id: NodeId,
    }

    fn rig() -> Rig {
        let mock = Rc::new(RefCell::new(MockDomDocument::new()));
        let doc: Rc<RefCell<dyn DomDocument>> = mock;
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        Rig {
            doc,
            container,
            container_id,
        }
    }

    fn cell_of(s: &Schema, ty: &str, colspan: i64, rowspan: i64, text: &str) -> Node {
        let p = s
            .create_node(
                "paragraph",
                Attrs::new(),
                Fragment::from_node(s.text(text).unwrap()),
            )
            .unwrap();
        s.create_node(
            ty,
            Attrs::from_iter([
                ("colspan", AttrValue::Int(colspan)),
                ("rowspan", AttrValue::Int(rowspan)),
            ]),
            Fragment::from_node(p),
        )
        .unwrap()
    }

    fn row(s: &Schema, cells: &[(i64, i64)]) -> Node {
        let cells = cells
            .iter()
            .enumerate()
            .map(|(i, &(c, r))| cell_of(s, "table_cell", c, r, &format!("c{i}")))
            .collect();
        s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
            .unwrap()
    }

    fn table(s: &Schema, rows: &[Vec<(i64, i64)>]) -> Node {
        let rows = rows.iter().map(|r| row(s, r)).collect();
        s.create_node("table", Attrs::new(), Fragment::from_children(rows))
            .unwrap()
    }

    fn state_with(s: &Rc<Schema>, t: Node) -> EditorState {
        let doc = s.branch("doc", Fragment::from_node(t)).unwrap();
        EditorState::create(s.clone(), doc, default_plugins())
    }

    /// Every cell host in document order.
    fn cells(r: &Rig) -> Vec<NodeId> {
        fn walk(d: &dyn DomDocument, id: NodeId, out: &mut Vec<NodeId>) {
            if matches!(
                d.get_attribute(id, "data-pm-type").as_deref(),
                Some("table_cell" | "table_header_cell")
            ) {
                out.push(id);
            }
            for c in d.get_children(id) {
                walk(d, c, out);
            }
        }
        let mut out = Vec::new();
        walk(&*r.doc.borrow(), r.container_id, &mut out);
        out
    }

    /// `(grid-column, grid-row)` as the view wrote them inline.
    fn placement(r: &Rig, id: NodeId) -> (String, String) {
        let style = r
            .doc
            .borrow()
            .get_attribute(id, "style")
            .unwrap_or_default();
        let decls = rinch_core::dom::split_declarations(&style);
        let get = |p: &str| {
            decls
                .iter()
                .find(|(k, _)| k == p)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| format!("<no {p}>"))
        };
        (get("grid-column"), get("grid-row"))
    }

    fn p(c: &str, r: &str) -> (String, String) {
        (c.to_string(), r.to_string())
    }

    /// An ordinary table's cells are written at their map rectangles' column
    /// lines (#1209) and as row spans (#1226): auto-placement puts each in its
    /// map row, so a row line would only make the cell's style depend on its
    /// row index.
    #[test]
    fn an_ordinary_tables_cells_are_written_at_their_column_lines_and_row_spans() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let st = state_with(&s, table(&s, &[vec![(2, 1), (1, 2)], vec![(1, 1), (1, 1)]]));
        let _view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let got: Vec<_> = cells(&r).into_iter().map(|c| placement(&r, c)).collect();
        assert_eq!(
            got,
            vec![
                p("1 / 3", "auto"),
                p("3 / 4", "span 2"),
                p("1 / 2", "auto"),
                p("2 / 3", "auto"),
            ]
        );
    }

    /// A rowspan past the last row is cut at it, as the map cuts it: a
    /// two-row table's `rowspan = i64::MAX` cell spans two grid rows. Off the
    /// fixed point: the colspan is 2, so a writer that swapped the axes, or
    /// wrote the raw colspan and the rect's rowspan, is caught too.
    #[test]
    fn an_overlong_rowspan_is_written_cut_at_the_last_row() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let st = state_with(&s, table(&s, &[vec![(2, i64::MAX), (1, 1)], vec![(1, 1)]]));
        let _view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let got: Vec<_> = cells(&r).into_iter().map(|c| placement(&r, c)).collect();
        assert_eq!(
            got,
            vec![p("1 / 3", "span 2"), p("3 / 4", "auto"), p("3 / 4", "auto")]
        );
    }

    /// A colspan past the capped grid is cut at the grid's right edge, and a
    /// cell the capped grid has no slot for is written as a band across the
    /// whole grid rather than as one more track. Two rows under a 3,000,000-
    /// column cell whose rowspan carries it down: the map's width is capped at
    /// `2^22 / 2` columns, the big cell spans all of them and both rows, and
    /// the second row's own cell finds its row full.
    #[test]
    fn a_colspan_past_the_capped_grid_is_cut_and_a_cell_outside_it_is_a_band() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let t = table(&s, &[vec![(3_000_000, i64::MAX)], vec![(1, 1)]]);
        let width = rinch_editor_core::tables::column_count(&t);
        assert_eq!(width, (1 << 22) / 2, "control: the grid is capped");
        let st = state_with(&s, t);
        let _view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let got: Vec<_> = cells(&r).into_iter().map(|c| placement(&r, c)).collect();
        assert_eq!(
            got,
            vec![p(&format!("span {width}"), "span 2"), p("1 / -1", "auto")]
        );
    }

    /// The rectangle depends on the whole table, not on the cell: a row added
    /// below a `rowspan = i64::MAX` cell lengthens it by one though its own
    /// attributes did not change, and the view re-writes it.
    #[test]
    fn a_row_added_under_an_overlong_rowspan_lengthens_it() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let t = table(&s, &[vec![(1, i64::MAX)], vec![(1, 1)]]);
        let st = state_with(&s, t.clone());
        let mut view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        assert_eq!(placement(&r, cells(&r)[0]), p("1 / 2", "1 / 3"), "control");
        // The table's inner end: after its open token and its content.
        let end = 1 + t.content_size();
        let mut tr = st.tr();
        tr.replace_with(end, end, Fragment::from_node(row(&s, &[(1, 1)])))
            .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        assert_eq!(placement(&r, cells(&r)[0]), p("1 / 2", "1 / 4"));
    }

    /// A row removed from under an overlong rowspan shortens it, and a cell's
    /// own colspan or rowspan changed in place re-places it: none of them builds
    /// a cell host, so only the table's span comparison can see them.
    #[test]
    fn a_removed_row_and_a_changed_colspan_re_place_cells_patched_in_place() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let t = table(
            &s,
            &[vec![(1, i64::MAX), (1, 1)], vec![(1, 1)], vec![(1, 1)]],
        );
        let st = state_with(&s, t.clone());
        let mut view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let c = cells(&r);
        assert_eq!(placement(&r, c[0]), p("1 / 2", "span 3"), "control");
        assert_eq!(placement(&r, c[1]), p("2 / 3", "auto"), "control");
        // Remove the last row: it ends one before the table's inner end.
        let last = t.child(2).node_size();
        let end = 1 + t.content_size();
        let mut tr = st.tr();
        tr.delete(end - last, end).unwrap();
        let st2 = st.apply(tr);
        view.update_dom(&st, &st2);
        let c2 = cells(&r);
        assert_eq!(&c2[..3], &c[..3], "control: no cell host rebuilt");
        assert_eq!(placement(&r, c[0]), p("1 / 2", "span 2"));
        // The second cell of the first row: after the row open and cell one.
        let second = 2 + t.child(0).child(0).node_size();
        let mut tr = st2.tr();
        tr.set_node_attr(second, "colspan", AttrValue::Int(2))
            .unwrap();
        let st3 = st2.apply(tr);
        view.update_dom(&st2, &st3);
        assert_eq!(cells(&r), c2, "control: no cell host rebuilt");
        assert_eq!(placement(&r, c[1]), p("2 / 4", "auto"));
        // And its rowspan: the second row's one cell then sits beside the two,
        // in row 1 with a hole above it that auto-placement would lift it into,
        // so the table is placed by row lines from here on (#1226).
        let mut tr = st3.tr();
        tr.set_node_attr(second, "rowspan", AttrValue::Int(2))
            .unwrap();
        let st4 = st3.apply(tr);
        view.update_dom(&st3, &st4);
        assert_eq!(cells(&r), c2, "control: no cell host rebuilt");
        assert_eq!(placement(&r, c[1]), p("2 / 4", "1 / 3"));
        assert_eq!(placement(&r, c[0]), p("1 / 2", "1 / 3"));
    }

    /// Typing in a cell computes no map and places no cell: only a change to
    /// what the map is computed from does. (A table computing its whole map on
    /// every keystroke would give each placement the same answer, so no
    /// placement assertion can see it; the count does.)
    #[test]
    fn typing_in_a_cell_places_no_cell() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let t = table(&s, &[vec![(2, 1)], vec![(1, 1), (1, 1)]]);
        let st = state_with(&s, t.clone());
        SYNCS.with(|n| n.set(0));
        let mut view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        assert_eq!(
            SYNCS.with(|n| n.get()),
            1,
            "control: the build placed the cells"
        );
        // Inside the second row's first cell's paragraph: table open 0, row 1,
        // the first row, then row open, cell open, paragraph open.
        let inside = 1 + t.child(0).node_size() + 3;
        let mut tr = st.tr();
        tr.set_selection(rinch_editor_core::Selection::cursor(
            rinch_editor_core::Pos(inside),
        ));
        tr.insert_text("x").unwrap();
        let st2 = st.apply(tr);
        view.update_dom(&st, &st2);
        assert_eq!(
            st2.doc.child(0).child(1).child(0).child(0).child(0).text(),
            Some("xc0"),
            "control: the text landed in the cell"
        );
        assert_eq!(
            SYNCS.with(|n| n.get()),
            1,
            "a keystroke re-placed the cells"
        );
        // Control: a span change does place them.
        let mut tr = st2.tr();
        tr.set_node_attr(2, "colspan", AttrValue::Int(1)).unwrap();
        let st3 = st2.apply(tr);
        view.update_dom(&st2, &st3);
        assert_eq!(
            SYNCS.with(|n| n.get()),
            2,
            "control: a span change re-placed them"
        );
    }

    /// A cell whose host is rebuilt — `td` to `th`, the same spans — gets its
    /// placement written on the new host, though nothing about the table's
    /// spans changed.
    #[test]
    fn a_rebuilt_cell_host_gets_its_span() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let t = table(&s, &[vec![(2, 1)], vec![(1, 1), (1, 1)]]);
        let st = state_with(&s, t.clone());
        let mut view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let before = cells(&r)[0];
        assert_eq!(placement(&r, before), p("1 / 3", "auto"), "control");
        // The first cell spans positions 2 .. 2 + its size.
        let first = t.child(0).child(0).clone();
        let th = cell_of(&s, "table_header_cell", 2, 1, "c0");
        let mut tr = st.tr();
        tr.replace_with(2, 2 + first.node_size(), Fragment::from_node(th))
            .unwrap();
        let next = st.apply(tr);
        view.update_dom(&st, &next);
        let after = cells(&r)[0];
        assert_ne!(after, before, "control: the host was rebuilt");
        assert_eq!(placement(&r, after), p("1 / 3", "auto"));
    }

    /// #1209: a row a rowspan leaves short keeps its cells — `[[A rowspan=2],
    /// [B]]` writes B at row line 2, where auto-placement packed it into row 0.
    #[test]
    fn a_cell_in_a_short_row_is_written_at_its_own_row() {
        let r = rig();
        let s = Rc::new(Schema::starter_kit());
        let st = state_with(&s, table(&s, &[vec![(1, 2)], vec![(1, 1)]]));
        let _view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        let got: Vec<_> = cells(&r).into_iter().map(|c| placement(&r, c)).collect();
        assert_eq!(got, vec![p("1 / 2", "1 / 3"), p("2 / 3", "2 / 3")]);
    }

    /// #1226: which tables auto-placement lays out in their map rows, so that
    /// their cells are written with row spans. Each `false` is a shape
    /// auto-placement lifts a cell out of its row in.
    #[test]
    fn row_spans_are_used_exactly_where_auto_placement_keeps_the_rows() {
        let s = Rc::new(Schema::starter_kit());
        let matches = |rows: &[Vec<(i64, i64)>]| {
            auto_rows_match_map(&rinch_editor_core::tables::cell_rects(&table(&s, rows)))
        };
        // Plain, merged and rowspanned rectangular tables.
        assert!(matches(&[vec![(1, 1), (1, 1)], vec![(1, 1), (1, 1)]]));
        assert!(matches(&[vec![(2, 1), (1, 2)], vec![(1, 1), (1, 1)]]));
        assert!(matches(&[vec![(1, 2), (1, 1)], vec![(1, 1)]]));
        assert!(matches(&[vec![(1, 1), (1, 2)], vec![(1, 1)]]));
        assert!(matches(&[
            vec![(1, 3), (1, 3)],
            vec![],
            vec![],
            vec![(1, 1), (1, 1)]
        ]));
        // A ragged row only short at its end.
        assert!(matches(&[vec![(2, 1)], vec![(1, 1)]]));
        // #1209's shape: auto-placed in both axes, B rose into the slot beside
        // X; locked to its column (line 2), it can only go below X.
        assert!(matches(&[vec![(1, 2), (1, 1)], vec![(1, 1), (1, 1)]]));
        // A short row whose hole the next row's cell rises into.
        assert!(!matches(&[vec![(1, 2)], vec![(1, 1)]]));
        // A cell placed past a hole in the row above it.
        assert!(!matches(&[vec![(1, 2), (2, 2)], vec![(1, 1)]]));
        // A cell outside the grid.
        assert!(!matches(&[vec![(1, i64::MAX)], vec![(1, 1)]]));
    }

    /// Definite lines stop where Stylo's grid lines do: a grid of
    /// [`MAX_DEFINITE_GRID_TRACKS`] columns ends on line 10000, Stylo's
    /// `MAX_GRID_LINE`, and is placed by lines; one column more and its cells
    /// are auto-placed by their spans in both axes, as before #1209, rather
    /// than piled on line 10000 (rows too: see [`GridLines`]). One row more
    /// than the cap loses only the row lines: the columns keep theirs.
    #[test]
    fn a_grid_past_the_line_cap_falls_back_to_spans() {
        let s = Rc::new(Schema::starter_kit());
        let placed = |rows: &[Vec<(i64, i64)>]| {
            let r = rig();
            let st = state_with(&s, table(&s, rows));
            let _view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
            cells(&r)
                .into_iter()
                .map(|c| placement(&r, c))
                .collect::<Vec<_>>()
        };
        let w = MAX_DEFINITE_GRID_TRACKS as i64;
        assert_eq!(
            placed(&[vec![(w, 1)], vec![(1, 1)]]),
            vec![p("1 / 10000", "auto"), p("1 / 2", "auto")]
        );
        assert_eq!(
            placed(&[vec![(w + 1, 1)], vec![(1, 1)]]),
            vec![p("span 10000", "auto"), p("auto", "auto")]
        );
        let tall = |n: usize| vec![vec![(1, 2)]; n];
        let at = placed(&tall(MAX_DEFINITE_GRID_TRACKS));
        assert_eq!(at[0], p("1 / 2", "1 / 3"));
        assert_eq!(at.last(), Some(&p("1 / 2", "9999 / 10000")));
        let past = placed(&tall(MAX_DEFINITE_GRID_TRACKS + 1));
        assert_eq!(past[0], p("1 / 2", "span 2"));
        assert_eq!(past[1], p("2 / 3", "span 2"));
        assert_eq!(past.last(), Some(&p("2 / 3", "auto")));
    }
}

/// #1182, from its review: every cell's placement after an incremental update
/// equals a fresh render of the same state, after random table commands,
/// undo/redo, in-place span edits, row replacements and whole-document swaps;
/// and a text edit inside a cell rewrites no placement. A row replaced by one
/// of another width changes a row's cell count while the row count stays put,
/// which `same_table_spans` must see and no named fixture covers.
#[cfg(test)]
mod table_span_differential {
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_core::dom::mock::MockDomDocument;
    use rinch_editor_core::model::{Attrs, Fragment};
    use rinch_editor_core::{AttrValue, EditorState, Schema, Selection, default_plugins};
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Rig {
        doc: Rc<RefCell<dyn DomDocument>>,
        container: NodeHandle,
        container_id: NodeId,
    }
    fn rig() -> Rig {
        let mock = Rc::new(RefCell::new(MockDomDocument::new()));
        let doc: Rc<RefCell<dyn DomDocument>> = mock;
        let container_id = doc.borrow_mut().create_element("div");
        let container = NodeHandle::new(container_id, Rc::downgrade(&doc));
        Rig {
            doc,
            container,
            container_id,
        }
    }
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }
    fn span(rng: &mut Rng, big: bool) -> i64 {
        match rng.below(10) {
            0..=5 => 1,
            6 => 2,
            7 => 3,
            8 if big => i64::MAX,
            8 => 2,
            _ if big => 3_000_000,
            _ => 1,
        }
    }
    fn cell(s: &Schema, c: i64, r: i64, text: &str) -> Node {
        let p = s
            .create_node(
                "paragraph",
                Attrs::new(),
                Fragment::from_node(s.text(text).unwrap()),
            )
            .unwrap();
        s.create_node(
            "table_cell",
            Attrs::from_iter([
                ("colspan", AttrValue::Int(c)),
                ("rowspan", AttrValue::Int(r)),
            ]),
            Fragment::from_node(p),
        )
        .unwrap()
    }
    fn random_table(s: &Schema, rng: &mut Rng, big: bool) -> Node {
        let nrows = 1 + rng.below(4) as usize;
        let rows = (0..nrows)
            .map(|ri| {
                let n = 1 + rng.below(4) as usize;
                let cells = (0..n)
                    .map(|ci| cell(s, span(rng, big), span(rng, big), &format!("r{ri}c{ci}")))
                    .collect();
                s.create_node("table_row", Attrs::new(), Fragment::from_children(cells))
                    .unwrap()
            })
            .collect();
        s.create_node("table", Attrs::new(), Fragment::from_children(rows))
            .unwrap()
    }
    fn state_with(s: &Rc<Schema>, t: Node) -> EditorState {
        let para = s
            .create_node(
                "paragraph",
                Attrs::new(),
                Fragment::from_node(s.text("after").unwrap()),
            )
            .unwrap();
        let doc = s
            .branch("doc", Fragment::from_children(vec![t, para]))
            .unwrap();
        EditorState::create(s.clone(), doc, default_plugins())
    }
    fn cells(r: &Rig) -> Vec<NodeId> {
        fn walk(d: &dyn DomDocument, id: NodeId, out: &mut Vec<NodeId>) {
            if matches!(
                d.get_attribute(id, "data-pm-type").as_deref(),
                Some("table_cell" | "table_header_cell")
            ) {
                out.push(id);
            }
            for c in d.get_children(id) {
                walk(d, c, out);
            }
        }
        let mut out = Vec::new();
        walk(&*r.doc.borrow(), r.container_id, &mut out);
        out
    }
    fn placements(r: &Rig) -> Vec<(String, String)> {
        cells(r)
            .into_iter()
            .map(|id| {
                let style = r
                    .doc
                    .borrow()
                    .get_attribute(id, "style")
                    .unwrap_or_default();
                let decls = rinch_core::dom::split_declarations(&style);
                let get = |p: &str| {
                    decls
                        .iter()
                        .find(|(k, _)| k == p)
                        .map(|(_, v)| v.clone())
                        .unwrap_or_else(|| format!("<no {p}>"))
                };
                (get("grid-column"), get("grid-row"))
            })
            .collect()
    }
    /// Positions of every cell (the position before it).
    fn cell_positions(doc: &Node) -> Vec<usize> {
        let mut out = Vec::new();
        doc.nodes_between(0, doc.content_size(), &mut |n, pos, _| {
            if n.type_name() == "table_cell" || n.type_name() == "table_header_cell" {
                out.push(pos);
            }
            true
        });
        out
    }
    fn fresh(st: &EditorState) -> Vec<(String, String)> {
        let r = rig();
        let _v = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), st);
        placements(&r)
    }
    const CMDS: &[&str] = &[
        "addRowAfter",
        "addRowBefore",
        "addColumnAfter",
        "addColumnBefore",
        "deleteRow",
        "deleteColumn",
        "mergeCells",
        "splitCell",
        "undo",
        "redo",
    ];

    fn run(seed: u64, big: bool, steps: usize) -> (usize, usize) {
        let mut rng = Rng(seed * 2654435761 + 7);
        let s = Rc::new(Schema::starter_kit());
        let r = rig();
        let mut st = state_with(&s, random_table(&s, &mut rng, big));
        let mut view = RinchDomEditorView::new(r.container.clone(), Rc::downgrade(&r.doc), &st);
        assert_eq!(placements(&r), fresh(&st), "seed {seed} initial");
        let (mut applied, mut textprobe) = (0, 0);
        for step in 0..steps {
            let cps = cell_positions(&st.doc);
            if cps.is_empty() {
                break;
            }
            let kind = rng.below(10);
            let next: Option<EditorState> = if kind <= 5 {
                let a = cps[rng.below(cps.len() as u64) as usize];
                let sel = if rng.below(2) == 0 {
                    Selection::text(rinch_editor_core::Pos(a + 2), rinch_editor_core::Pos(a + 2))
                } else {
                    let b = cps[rng.below(cps.len() as u64) as usize];
                    Selection::cell(rinch_editor_core::Pos(a), rinch_editor_core::Pos(b))
                };
                let mut tr = st.tr();
                tr.set_selection(sel);
                let st1 = st.apply(tr);
                let name = CMDS[rng.below(CMDS.len() as u64) as usize];
                let cmd = st1.command(name).unwrap_or_else(|| panic!("no {name}"));
                if big
                    && (name.starts_with("add") || name == "splitCell")
                    && rinch_editor_core::tables::column_count(st1.doc.child(0)) > 4096
                {
                    st = st1;
                    continue;
                }
                let out = st1.run_command(&cmd);
                // Also feed the selection-only step through the view.
                view.update_dom(&st, &st1);
                st = st1;
                out
            } else if kind == 6 {
                // Attr edit in place.
                let a = cps[rng.below(cps.len() as u64) as usize];
                let mut tr = st.tr();
                let attr = if rng.below(2) == 0 {
                    "colspan"
                } else {
                    "rowspan"
                };
                {
                    let ok = tr
                        .set_node_attr(a, attr, AttrValue::Int(span(&mut rng, big)))
                        .is_ok();
                    if ok { Some(st.apply(tr)) } else { None }
                }
            } else if kind == 7 {
                // Text edit inside a cell: must not rewrite placements.
                let a = cps[rng.below(cps.len() as u64) as usize];
                // Tamper every cell's grid-column with a sentinel; a text edit must leave it.
                let ids = cells(&r);
                let saved = placements(&r);
                for id in &ids {
                    let h = NodeHandle::new(*id, Rc::downgrade(&r.doc));
                    h.set_style("grid-column", "7 / 8");
                }
                let mut tr = st.tr();
                tr.set_selection(Selection::text(
                    rinch_editor_core::Pos(a + 2),
                    rinch_editor_core::Pos(a + 2),
                ));
                tr.insert_text("x").unwrap();
                let st2 = st.apply(tr);
                view.update_dom(&st, &st2);
                let after = placements(&r);
                assert!(
                    after.iter().all(|(c, _)| c == "7 / 8"),
                    "seed {seed} step {step}: a text edit rewrote placements: {after:?}"
                );
                for (id, (c, _)) in ids.iter().zip(&saved) {
                    NodeHandle::new(*id, Rc::downgrade(&r.doc)).set_style("grid-column", c);
                }
                textprobe += 1;
                st = st2;
                None
            } else if kind == 8 {
                // Collab-like whole-doc replacement with a fresh random table.
                let t = random_table(&s, &mut rng, big);
                let size = st.doc.content_size();
                let para = s
                    .create_node(
                        "paragraph",
                        Attrs::new(),
                        Fragment::from_node(s.text("after").unwrap()),
                    )
                    .unwrap();
                let mut tr = st.tr();
                tr.replace_with(0, size, Fragment::from_children(vec![t, para]))
                    .unwrap();
                Some(st.apply(tr))
            } else {
                // Replace one row with a random row (same or different cell count).
                let t = &st.doc.child(0);
                let ri = rng.below(t.child_count() as u64) as usize;
                let mut pos = 1;
                for i in 0..ri {
                    pos += t.child(i).node_size();
                }
                let old = t.child(ri).node_size();
                let n = 1 + rng.below(3) as usize;
                let cellsv = (0..n)
                    .map(|ci| {
                        cell(
                            &s,
                            span(&mut rng, big),
                            span(&mut rng, big),
                            &format!("n{ci}"),
                        )
                    })
                    .collect();
                let row = s
                    .create_node("table_row", Attrs::new(), Fragment::from_children(cellsv))
                    .unwrap();
                let mut tr = st.tr();
                {
                    let ok = tr
                        .replace_with(pos, pos + old, Fragment::from_node(row))
                        .is_ok();
                    if ok { Some(st.apply(tr)) } else { None }
                }
            };
            if let Some(n) = next {
                if !rinch_editor_core::tables::is_table(n.doc.child(0)) {
                    break;
                }
                view.update_dom(&st, &n);
                st = n;
                applied += 1;
            }
            assert_eq!(
                placements(&r),
                fresh(&st),
                "seed {seed} step {step} kind {kind}"
            );
        }
        (applied, textprobe)
    }

    #[test]
    fn incremental_placements_match_a_fresh_render() {
        let (mut a, mut t) = (0, 0);
        for seed in 1..=300 {
            let (x, y) = run(seed, false, 40);
            a += x;
            t += y;
        }
        assert!(a > 1000 && t > 500);
    }
}

#[cfg(test)]
#[path = "shared_wraps_1833_tests.rs"]
mod shared_wraps_1833_tests;

#[cfg(test)]
#[path = "incremental_fresh_1833_prop_tests.rs"]
mod incremental_fresh_1833_prop_tests;

#[cfg(test)]
#[path = "inline_doc_1833_tests.rs"]
mod inline_doc_1833_tests;

#[cfg(test)]
#[path = "inline_deco_848_tests.rs"]
mod inline_deco_848_tests;
