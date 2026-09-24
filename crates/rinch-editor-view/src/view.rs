//! [`RinchDomEditorView`] and its [`ViewDesc`] tree — the desktop projection of
//! `EditorState` onto rinch-dom (design §6).

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rinch_core::dom::{DomDocument, NodeFont, NodeHandle};
use rinch_editor_core::decoration::DecorationSet;
use rinch_editor_core::serialize::{mark_dom_tag, node_dom_tag};
use rinch_editor_core::{EditorState, EditorView, Mark, Node, Pos, ViewRequest};

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
fn apply_element_attrs(dom: &NodeHandle, node: &Node) {
    dom.set_attribute("data-pm-type", node.type_name());
    match node.type_name() {
        "image" => {
            dom.set_attribute("src", node.attrs().get_str("src").unwrap_or(""));
            match node.attrs().get_str("alt") {
                Some(alt) if !alt.is_empty() => dom.set_attribute("alt", alt),
                _ => dom.remove_attribute("alt"),
            }
        }
        "ordered_list" => match node.attrs().get_int("start") {
            Some(start) if start != 1 => dom.set_attribute("start", &start.to_string()),
            _ => dom.remove_attribute("start"),
        },
        // A table is laid out as a CSS grid (the default stylesheet sets
        // `display: grid` on `<table>` and `display: contents` on `<tr>`, so the
        // cells are the grid's items). The column count is data-dependent, so the
        // view writes it inline; cells carry their colspan/rowspan as a grid span.
        "table" => {
            let cols = rinch_editor_core::tables::column_count(node).max(1);
            dom.set_style(
                "grid-template-columns",
                &format!("repeat({cols}, minmax(0, 1fr))"),
            );
        }
        "table_cell" | "table_header_cell" => {
            let cspan = node.attrs().get_int("colspan").unwrap_or(1).max(1);
            let rspan = node.attrs().get_int("rowspan").unwrap_or(1).max(1);
            // Always write both (resetting to `auto`) so a split/merge that shrinks
            // a span doesn't leave a stale `span N` behind.
            dom.set_style(
                "grid-column",
                &(if cspan > 1 {
                    format!("span {cspan}")
                } else {
                    "auto".to_string()
                }),
            );
            dom.set_style(
                "grid-row",
                &(if rspan > 1 {
                    format!("span {rspan}")
                } else {
                    "auto".to_string()
                }),
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

/// Wrap `inner` in `marks`' host elements, innermost-first (`marks[0]` closest to
/// the text, matching copy-out HTML), and return the outermost node (or `inner`
/// itself when there are no marks / no mark renders to a tag).
fn wrap_marks(inner: &NodeHandle, marks: &[Mark], doc: &DocRef) -> NodeHandle {
    let mut current = inner.clone();
    for mark in marks {
        if let Some(tag) = mark_dom_tag(mark)
            && let Some(wrapper) = create_element(doc, &tag)
        {
            apply_mark_attrs(&wrapper, mark);
            wrapper.append_child(&current);
            current = wrapper;
        }
    }
    current
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
fn run_decos(decos: &[InlineDeco], run_from: usize, len: usize) -> Vec<RunDeco> {
    let run_to = run_from + len;
    let clipped: Vec<(usize, usize, &str)> = decos
        .iter()
        .filter_map(|d| {
            let a = d.from.max(run_from);
            let b = d.to.min(run_to);
            (a < b).then(|| (a - run_from, b - run_from, &*d.class))
        })
        .collect();
    if clipped.is_empty() {
        return Vec::new();
    }
    let mut bounds: Vec<usize> = clipped.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    bounds.sort_unstable();
    bounds.dedup();
    let mut out: Vec<RunDeco> = Vec::new();
    for w in bounds.windows(2) {
        let (a, b) = (w[0], w[1]);
        let mut classes: Vec<&str> = clipped
            .iter()
            .filter(|(x, y, _)| *x <= a && *y >= b)
            .map(|(_, _, c)| *c)
            .collect();
        if classes.is_empty() {
            continue;
        }
        classes.dedup();
        let class: Rc<str> = Rc::from(classes.join(" "));
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
/// **Inline marks (M5.5a):** a marked inline run (`text("ab", [bold, italic])`)
/// projects to a *chain* of host nodes — `<strong><em>#text</em></strong>` — so
/// [`Self::dom`] (the inner text/leaf node, used for caret geometry and the A15
/// position map) is distinct from [`Self::outer`] (the outermost wrapper, the node
/// actually placed in / removed from the parent). For an unmarked node the two are
/// the same node. Wrappers are built **per run** (no cross-run sharing yet); a
/// shared-wrapper pass is a later refinement.
pub(crate) struct ViewDesc {
    /// The model node this descriptor currently projects.
    node: Node,
    /// The inner host node projecting `node` itself — a host text node for a text
    /// run, or the element for a block/leaf. Caret and the A15 map address this.
    dom: NodeHandle,
    /// The host node placed in the parent: the outermost inline mark wrapper, or
    /// (when `node` is unmarked) the same node as [`Self::dom`].
    outer: NodeHandle,
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
}

impl ViewDesc {
    /// Build a descriptor (and its host subtree, including inline mark wrappers)
    /// for `node`.
    fn build(node: &Node, doc: &DocRef) -> Option<ViewDesc> {
        let (dom, children, is_text) = if let Some(text) = node.text() {
            (create_text(doc, text)?, Vec::new(), true)
        } else {
            let el = create_element(doc, &node_dom_tag(node))?;
            apply_element_attrs(&el, node);
            let mut children = Vec::with_capacity(node.child_count());
            for i in 0..node.child_count() {
                if let Some(child) = ViewDesc::build(node.child(i), doc) {
                    el.append_child(&child.outer);
                    children.push(child);
                }
            }
            (el, children, false)
        };
        // Inline marks (on a text run or inline leaf) wrap the node's own host
        // node; blocks are unmarked, so `outer == dom` there.
        let outer = wrap_marks(&dom, node.marks(), doc);
        Some(ViewDesc {
            node: node.clone(),
            dom,
            outer,
            children,
            is_text,
            // A freshly built run carries no decorations; the decoration pass that
            // runs right after the document diff puts them back on.
            decos: Vec::new(),
            segments: Vec::new(),
            has_deco: false,
        })
    }

    /// Patch this descriptor in place to project `new`. Returns `false` if the
    /// kinds are incompatible (text vs element, a different element tag, or a
    /// changed mark set whose wrapper chain differs) and the caller must replace
    /// the whole subtree instead.
    fn update(&mut self, new: &Node, doc: &DocRef) -> bool {
        // Fast path: same persistent subtree — nothing changed (design A12).
        if self.node.same_ref(new) {
            return true;
        }
        // A changed mark set alters the wrapper chain (`outer`), which can't be
        // patched in place — force a rebuild. (Blocks are always unmarked.)
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
        // Element: the tag must match to patch in place.
        if self.is_text || node_dom_tag(&self.node) != node_dom_tag(new) {
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
            apply_element_attrs(&self.dom, new);
        }
        self.diff_children(new, doc);
        self.node = new.clone();
        true
    }

    /// Reconcile this descriptor's children against `new`'s children: a positional
    /// diff that recurses (with the `same_ref` fast skip), replaces a child whose
    /// kind changed, appends new trailing children, and removes surplus ones.
    ///
    /// Positional (not keyed) is correct and minimal for text editing, where edits
    /// are local; a keyed/LIS pass can replace this if reorder churn ever matters.
    fn diff_children(&mut self, new: &Node, doc: &DocRef) {
        let new_count = new.child_count();
        for i in 0..new_count {
            let new_child = new.child(i);
            if i < self.children.len() {
                if self.children[i].update(new_child, doc) {
                    continue;
                }
                // Kind changed — build a replacement and swap it into the host
                // (by the placed `outer` node, which differs from `dom` for a
                // mark-wrapped run).
                if let Some(replacement) = ViewDesc::build(new_child, doc) {
                    self.children[i].outer.replace_with(&replacement.outer);
                    // `replace_with` *detaches* the node it displaces (issue
                    // #719); the `ViewDesc` holding it is overwritten on the
                    // next line, so nothing can show it again. Say so, or the
                    // browser backend pins it for the life of the page — this
                    // is a per-keystroke path.
                    self.children[i].outer.discard();
                    self.children[i] = replacement;
                }
            } else if let Some(new_desc) = ViewDesc::build(new_child, doc) {
                self.dom.append_child(&new_desc.outer);
                self.children.push(new_desc);
            }
        }
        while self.children.len() > new_count {
            // `pop` keeps removal O(1) and order-independent (host removal is by id).
            if let Some(extra) = self.children.pop() {
                // Popped off the end and dropped — `discard`, not `remove`
                // (issue #719).
                extra.outer.discard();
            }
        }
    }

    /// The placed host node of the first block child, if any (the placeholder is
    /// inserted before it so it overlays the empty editor).
    fn first_child_dom(&self) -> Option<&NodeHandle> {
        self.children.first().map(|c| &c.outer)
    }
}

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
    /// Set whenever the post-layout pass writes an overlay's geometry (caret,
    /// selection rect, or node outline). The runtime reads it via
    /// [`Self::take_overlay_dirty`] to re-resolve layout and schedule a repaint,
    /// since the overlays' new boxes only exist after a layout pass. A
    /// dirty-region repaint: rinch-dom keeps each overlay's last *painted* rect
    /// until the paint consumes it, so the old caret is cleared however many
    /// resolves land first. (This used to force a full repaint per keystroke,
    /// because a second resolve overwrote that old rect.)
    overlay_dirty: bool,
}

impl RinchDomEditorView {
    /// Build a view over `state`, rendering its document into `container` (the host
    /// element standing in for the `doc` node). `container` is owned by the caller
    /// and never replaced.
    pub fn new(container: NodeHandle, doc: DocRef, state: &EditorState) -> RinchDomEditorView {
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
        for i in 0..state.doc.child_count() {
            if let Some(child) = ViewDesc::build(state.doc.child(i), &doc) {
                container.append_child(&child.outer);
                children.push(child);
            }
        }
        let root = ViewDesc {
            node: state.doc.clone(),
            outer: container.clone(),
            dom: container,
            children,
            is_text: false,
            decos: Vec::new(),
            segments: Vec::new(),
            has_deco: false,
        };
        let mut view = RinchDomEditorView {
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
            preedit_node: None,
            preedit_text: None,
            preedit: None,
            last_preedit: None,
            decorations: DecorationSet::empty(),
            overlay_dirty: false,
        };
        view.sync_decorations(state);
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
        }
        // Inline decorations are re-applied whenever any are present *or* any were
        // last time, even when the set itself compares equal. The document diff
        // runs first and may have rebuilt a decorated run — a fresh text node
        // carries no segments — so "the decorations did not change" does not imply
        // "what projects them is still there". The `has_deco` prune below keeps
        // that re-check to one range test per undecorated top-level block.
        let inline = inline_decos(&next);
        if !inline.is_empty() || self.root.has_deco {
            self.root.has_deco = sync_inline_decos(&mut self.root, &inline, 0, &self.doc);
        }
        self.decorations = next;
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
        self.root.diff_children(&next.doc, &self.doc);
        self.root.node = next.doc.clone();
        // Decoration diff, independent of the document diff (A4).
        self.sync_decorations(next);
        Vec::new()
    }

    fn update_caret(&mut self, next: &EditorState) -> Vec<ViewRequest> {
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
        // Query Parley for the caret geometry (layout-local to the textblock), then
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
            let from_parley =
                d.query_caret_position(block_id as u64, flat_byte)
                    .map(|(local_x, local_y)| {
                        let height = d
                            .query_glyph_bounds(block_id as u64, flat_byte)
                            .map(|g| g.height)
                            .unwrap_or(18.0);
                        let (ox, oy) = self.block_offset_in_container(&*d, block_id);
                        (ox + local_x, oy + local_y, height)
                    });
            from_parley
                .or_else(|| self.empty_block_caret(&*d, &next.doc, next.selection.head(), block_id))
        };
        match geometry {
            Some((x, y, height)) => {
                // While composing, the preedit overlay stands in for the caret at
                // the cursor — show it and hide the blinking caret. Otherwise place
                // the caret as usual.
                if let Some(text) = self.preedit.clone() {
                    self.position_preedit(x, y, height, &text, preedit_font.as_ref());
                    self.hide_caret();
                    Vec::new()
                } else {
                    self.hide_preedit();
                    scroll_if(self.position_caret(x, y, height))
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

    /// The on-screen caret for `pos` as `(x, y, height)`, in the host's popup
    /// frame ([`DomDocument::query_caret_rect`]). `None` when `pos` is not in a
    /// textblock, the host document is gone or busy, or the block has no box.
    pub(crate) fn caret_rect(&self, doc: &Node, pos: Pos) -> Option<(f32, f32, f32)> {
        let (block, byte) = self.caret_target(doc, pos)?;
        let host = self.doc.upgrade()?;
        // A soft borrow: an app may ask from a callback while a runtime holds
        // the document; that answers "no geometry" rather than panicking.
        let host = host.try_borrow().ok()?;
        host.query_caret_rect(block.node_id().0 as u64, byte)
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
        let flat_byte = textblock_flat_byte(&desc.node, r.parent_offset());
        Some((desc.dom.clone(), flat_byte))
    }

    /// Map a host caret address — `(textblock element id, flat UTF-8 byte offset)`,
    /// as produced by a pointer hit-test — back to a model [`Pos`] (the inverse of
    /// [`Self::caret_target`]). `None` if `textblock_dom_id` isn't a known
    /// textblock in this view.
    pub(crate) fn pos_at(&self, textblock_dom_id: usize, ifc_byte: usize) -> Option<Pos> {
        let (content_start, block) = find_block(&self.root, textblock_dom_id, 0)?;
        Some(Pos(content_start + ifc_byte_to_char(block, ifc_byte)))
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
        // (a no-op on the desktop renderer — default `(0, 0)`).
        let (ix, iy) = d.content_origin_inset(container_id as u64);
        (x - ix, y - iy)
    }

    /// Caret geometry for an *empty* textblock (which has no Parley layout, so
    /// [`DomDocument::query_caret_position`] returns `None`): the block's content
    /// origin in container space with the block's own box height as the line
    /// height. `None` if the block at `pos` is *not* actually empty (a non-empty
    /// block that merely failed to measure — leave the caret hidden rather than
    /// misplace it) or has no host box yet.
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
        let (_, _, _, bh) = d.query_node_layout(block_id as u64)?;
        let (ox, oy) = self.block_offset_in_container(d, block_id);
        // An empty block's box height is its single line box; clamp to a sane range
        // and fall back to a default when it has collapsed to zero.
        let h = if (4.0..=80.0).contains(&bh) { bh } else { 18.0 };
        Some((ox, oy, h))
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
            for (block_id, byte_a, byte_b) in targets {
                let (ox, oy) = self.block_offset_in_container(&*d, block_id);
                for (rx, ry, rw, rh) in d.query_selection_rects(block_id as u64, byte_a, byte_b) {
                    out.push((ox + rx, oy + ry, rw, rh));
                }
            }
            out
        };
        self.set_selection_rects(&rects);
    }

    /// For each textblock overlapping `[from, to)`, the host element id and the
    /// flat UTF-8 byte range of the selection within that block.
    fn selection_targets(&self, doc: &Node, from: Pos, to: Pos) -> Vec<(usize, usize, usize)> {
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
                    targets.push((
                        block_id,
                        textblock_flat_byte(node, a - content_start),
                        textblock_flat_byte(node, b - content_start),
                    ));
                }
                false // don't descend into the textblock's inline content
            } else {
                true
            }
        });
        targets
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
            let left = format!("{x}px");
            let top = format!("{y}px");
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("left", &left),
                ("top", &top),
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
            let left = format!("{x}px");
            let top = format!("{y}px");
            let width = format!("{w}px");
            let height = format!("{h}px");
            div.set_styles(&[
                ("left", &left),
                ("top", &top),
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
        // The *inner* leaf element (`dom`), not the mark wrapper (`outer`): a node
        // selection outlines the image / hr box itself, even when the leaf carries
        // marks (e.g. a linked image, where `outer` is the `<a>`). For an unmarked
        // leaf `dom == outer`, so this is unchanged for the common case.
        Some(child.dom.node_id().0)
    }

    /// The model position immediately **before** the node whose placed host element
    /// is `target`, plus that node — the inverse of [`Self::node_host_at`], used to
    /// node-select a leaf (image / horizontal rule) the user clicks. Matches either
    /// the node's inner element (`dom`) or its outermost mark wrapper (`outer`), so a
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
        // an unfocused editor must not stomp the focused editor's global phase.
        // (A focus change resets the phase separately, in `caret_blink_tick`.)
        if super::blink::target() == Some((self.doc_key(), self.container_id())) {
            super::blink::reset();
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
                ("width", "2px"),
                ("background-color", "#1a73e8"),
                ("pointer-events", "none"),
                // In front of the in-flow text and the selection highlight.
                ("z-index", "1"),
            ]);
            self.root.dom.append_child(&caret);
            self.caret = Some(caret);
        }
        if let Some(caret) = &self.caret {
            let left = format!("{x}px");
            let top = format!("{y}px");
            let h = format!("{height}px");
            caret.set_styles(&[
                ("left", &left),
                ("top", &top),
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

/// The flat UTF-8 byte offset, within a textblock's concatenated inline text,
/// corresponding to the model **char** offset `char_off` — the A15 char→byte half
/// of the caret map. Walks the inline runs accumulating char and byte counts.
///
/// Inline leaves (image / hard_break) are treated as one char of zero flat-byte
/// width for now; exact leaf byte widths in the IFC text stream are tuned against
/// the live renderer (caret-after-leaf is the open edge).
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
/// (matching either its inner `dom` or its outermost mark wrapper `outer`),
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
            || child.outer.node_id().0 == target
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

/// Re-project the inline decorations over `desc`'s subtree. `content_start` is the
/// model position just inside `desc`'s content (0 for the root/doc). Returns
/// whether any decoration segment is projected in this subtree — the caller's
/// [`ViewDesc::has_deco`].
///
/// Writes nothing for a run whose decorations are unchanged, so this is safe to
/// run on every transaction: the per-run comparison is against
/// [`ViewDesc::decos`], and a subtree that neither holds a segment nor overlaps a
/// decoration is not even descended into.
fn sync_inline_decos(
    desc: &mut ViewDesc,
    decos: &[InlineDeco],
    content_start: usize,
    doc: &DocRef,
) -> bool {
    let mut any = false;
    let mut pos = content_start; // the position just before the current child
    for child in &mut desc.children {
        let size = child.node.node_size();
        let overlaps = decos.iter().any(|d| d.from < pos + size && d.to > pos);
        // `has_deco` is what makes the skip sound in the other direction: a subtree
        // no decoration reaches any more still has to be visited to *take its
        // segments off*.
        if overlaps || child.has_deco {
            if child.is_text {
                let len = child.node.text().map_or(0, |t| t.chars().count());
                apply_run_decos(child, run_decos(decos, pos, len), doc);
                child.has_deco = !child.decos.is_empty();
            } else {
                child.has_deco = sync_inline_decos(child, decos, pos + 1, doc);
            }
        }
        any |= child.has_deco;
        pos += size;
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
    // An unmarked run is placed in the parent by its own inner node (`outer ==
    // dom`); a marked one is placed by its outermost mark wrapper, which keeps its
    // identity while its content is swapped underneath.
    let was_outer = desc.outer.node_id() == desc.dom.node_id();
    desc.dom.replace_with(&inner);
    // `replace_with` detaches the node it displaces and nothing can show it again
    // (issue #719) — and this is a per-keystroke path on a decorated run.
    desc.dom.discard();
    if was_outer {
        desc.outer = inner.clone();
    }
    desc.dom = inner;
    desc.segments = segments;
    desc.decos = next;
}

/// The model **char** offset within a textblock for a flat UTF-8 `ifc_byte` offset
/// — the inverse of [`textblock_flat_byte`], used to turn a pointer hit into a
/// cursor position. Leaves count as one char (matching `textblock_flat_byte`).
fn ifc_byte_to_char(block: &Node, ifc_byte: usize) -> usize {
    let mut bytes = 0usize;
    let mut chars = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            if ifc_byte <= bytes + text.len() {
                let into_byte = ifc_byte - bytes;
                for (b, _) in text.char_indices() {
                    if b >= into_byte {
                        return chars;
                    }
                    chars += 1;
                }
                return chars;
            }
            bytes += text.len();
            chars += text.chars().count();
        } else {
            chars += 1;
        }
    }
    chars
}

fn textblock_flat_byte(block: &Node, char_off: usize) -> usize {
    let mut chars_seen = 0usize;
    let mut bytes = 0usize;
    for i in 0..block.child_count() {
        let child = block.child(i);
        if let Some(text) = child.text() {
            let run_chars = text.chars().count();
            if chars_seen + run_chars >= char_off {
                let into = char_off - chars_seen;
                let byte_in_run = text
                    .char_indices()
                    .nth(into)
                    .map(|(b, _)| b)
                    .unwrap_or(text.len());
                return bytes + byte_in_run;
            }
            chars_seen += run_chars;
            bytes += text.len();
        } else {
            if chars_seen >= char_off {
                return bytes;
            }
            chars_seen += 1; // leaf occupies one model char
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
        crate::blink::set_target(Some((view_a.doc_key(), view_a.container_id())));
        crate::blink::reset();
        let anchor0 = crate::blink::anchor_for_test();

        // A caret move in the UNFOCUSED editor B must NOT re-anchor the clock.
        view_b.position_caret(1.0, 2.0, 18.0);
        assert_eq!(
            crate::blink::anchor_for_test(),
            anchor0,
            "an unfocused editor's caret move stomped the focused editor's blink phase",
        );

        // A caret move in the FOCUSED editor A re-anchors it (caret back to solid).
        view_a.position_caret(1.0, 2.0, 18.0);
        assert_ne!(
            crate::blink::anchor_for_test(),
            anchor0,
            "the focused editor's caret move failed to reset its own blink phase",
        );

        crate::blink::set_target(None);
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
        // img host id (the inner `dom`), not the wrapper id (`outer`).
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
            let num = |k: &str| {
                style
                    .split(';')
                    .find_map(|d| {
                        let (n, v) = d.split_once(':')?;
                        (n.trim() == k)
                            .then(|| v.trim().trim_end_matches("px").parse::<f32>().ok())
                            .flatten()
                    })
                    .map(|v| v.round() as i32)
            };
            (num("left").unwrap(), num("top").unwrap())
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
        // wrapping, so a decoration crossing a mark boundary becomes one segment
        // per run rather than one continuous element. Visually identical for an
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
        assert_eq!(textblock_flat_byte(&p, 0), 0); // start
        assert_eq!(textblock_flat_byte(&p, 2), 2); // after "ab"
        assert_eq!(textblock_flat_byte(&p, 3), 4); // after "é" — past its 2 bytes
        assert_eq!(textblock_flat_byte(&p, 4), 5); // after "c"
        assert_eq!(textblock_flat_byte(&p, 5), 6); // end
    }
}
