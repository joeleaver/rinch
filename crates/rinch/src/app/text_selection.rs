//! Read-only text selection for `user-select: text` content.
//!
//! - **Which text is selectable.** `user-select` does not inherit, but its
//!   initial `auto` takes its parent's value (CSS UI 4 §6.1), so an IFC root
//!   is selectable when its nearest ancestor-or-self whose value is not
//!   `auto` says `text`, `all` or `contain`. With none at all it is not:
//!   rinch's default, for an app's own controls. Worked out where it is
//!   asked ([`RinchApp::find_selectable_ifc`], [`selectable_roots`]), not
//!   stored, so a container's `user-select` changing needs no restyle below it.
//! - **Roots.** A selection's ends and parts are in *roots*: IFC roots, and
//!   the text leaves of flex and grid containers (an anonymous item's text,
//!   laid out on its own in no IFC root, #1692), whose text is
//!   [`root_text`]'s.
//! - **One selection, many paragraphs.** A press starts it in one IFC root (the
//!   anchor) and a drag moves its focus into whatever selectable root is under
//!   the pointer. Every selectable root between the two in document order is
//!   part of it ([`TextSelection::runs`]): the first from its offset to its
//!   end, the last from its start to its offset, the ones between whole. Each
//!   carries its own part as the `data-text-sel*` attributes paint reads,
//!   less what a selection leaves out (`text_selection_text::kept_ranges`),
//!   and a move writes only the roots whose part changed.
//! - **Copy** gives what Chrome's `getSelection().toString()` gives
//!   (`text_selection_text`).
//! - **Gestures**: double and triple click, Shift+click, Select All, a
//!   `user-select: all` element taken whole, and a link in selectable text
//!   followed on release (a drag across it selects it).

use std::borrow::Cow;
use std::collections::HashMap;

use super::text_selection_text::{RootsCache, kept_ranges};
use super::*;
use rinch_dom::computed_style::UserSelectValue;

/// An end after the in-flow boxes at its offset, taking no floats.
const AFTER_INLINE: EndSide = EndSide {
    after_inline: true,
    takes_floats: false,
};

impl TextSelection {
    /// A selection collapsed at `offset` in `ifc_node_id`.
    pub(crate) fn at(ifc_node_id: u64, offset: usize) -> Self {
        Self {
            ifc_node_id,
            anchor_offset: offset,
            focus_node: ifc_node_id,
            focus_offset: offset,
            painted: Vec::new(),
            cache: None,
            painted_generation: u64::MAX,
            press: (0.0, 0.0),
            dragged: false,
            anchor_side: EndSide::default(),
            focus_side: EndSide::default(),
            unit: None,
            goal_x: None,
        }
    }

    /// Whether it selects anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.ifc_node_id == self.focus_node && self.anchor_offset == self.focus_offset
    }

    /// The document's selectable roots for this layout, built when there
    /// are none or they are stale.
    pub(crate) fn cache(&mut self, tree: &rinch_dom::NodeTree) -> &RootsCache {
        if !self.cache.as_ref().is_some_and(|c| c.current(tree)) {
            self.cache = Some(Box::new(RootsCache::build(tree)));
        }
        self.cache.as_deref().expect("built just now")
    }

    /// Each root the selection covers, in document order, with the part of
    /// its text (byte offsets, clamped to it) it covers. Its two ends are put
    /// in order where they stand (`text_selection_copy::before`): a root in
    /// another's lines (an `inline-block` in a paragraph) stands where its
    /// box stands in that root's text, not after it as root order has it.
    /// So a root holding an end is covered from (or to) where the root
    /// holding that end stands in it, and a root in the lines of a root
    /// holding an end is covered when the selection reaches past where it
    /// stands (#2245), as the copy takes it.
    pub(crate) fn runs(&mut self, tree: &rinch_dom::NodeTree) -> Vec<(u64, usize, usize)> {
        use super::text_selection_copy::root_node;
        let (a, f, ao, fo) = (
            self.ifc_node_id,
            self.focus_node,
            self.anchor_offset,
            self.focus_offset,
        );
        let (a_side, f_side) = (self.anchor_side, self.focus_side);
        let len = |id: u64| root_len(tree, id);
        if a == f {
            let (lo, hi) = order(ao.min(len(a)), fo.min(len(a)));
            return vec![(a, lo, hi)];
        }
        let cache = self.cache(tree);
        let (Some(&ia), Some(&if_)) = (cache.index.get(&a), cache.index.get(&f)) else {
            return vec![(a, ao, ao)];
        };
        // Where each node in a root's lines stands in its text, one pass per
        // root asked about (a paragraph of many inline-blocks is walked once,
        // not once per box: review of #2949).
        let mut lines = Lines::default();
        // Whether root `inner` stands in root `outer`'s lines, and where.
        let mut stands = |outer: u64, inner: u64| -> Option<usize> {
            if outer == inner {
                return None;
            }
            lines.of(tree, outer).get(&root_node(tree, inner)).copied()
        };
        // The ends in order where they stand (as `text_selection_copy::before`).
        let (ao_, fo_) = (ao.min(len(a)), fo.min(len(f)));
        let f_first = match (stands(f, a), stands(a, f)) {
            // The anchor's root stands in the focus's lines: the focus is
            // first when it is at or before where that root stands.
            (Some(at), _) => fo_ <= at,
            // The focus's root stands in the anchor's: before the anchor.
            (None, Some(at)) => at < ao_,
            (None, None) => if_ < ia,
        };
        let ((first, start, s_side), (last, end, e_side)) = if f_first {
            ((f, fo, f_side), (a, ao, a_side))
        } else {
            ((a, ao, a_side), (f, fo, f_side))
        };
        let (i0, i1) = if f_first { (if_, ia) } else { (ia, if_) };
        // The roots that need a look at where they stand: those holding an
        // end, and those in the lines of a root holding an end (a root's
        // nested roots follow it in root order). Every other root between
        // the two ends in root order is covered whole.
        let mut looked: Vec<usize> = Vec::new();
        for (end_root, i) in [(first, i0), (last, i1)] {
            looked.extend(
                outer_roots(tree, end_root)
                    .into_iter()
                    .filter_map(|r| cache.index.get(&r).copied()),
            );
            looked.extend(
                (i + 1..cache.roots.len())
                    .take_while(|&k| stands(end_root, cache.roots[k]).is_some()),
            );
        }
        let checked: std::collections::HashSet<usize> = looked.iter().copied().collect();
        let mut picked: Vec<usize> = looked;
        picked.extend((i0.min(i1)..=i0.max(i1)).filter(|k| !checked.contains(k)));
        picked.sort_unstable();
        picked.dedup();
        let mut out = Vec::new();
        for k in picked {
            super::text_selection_copy::work(1);
            let r = cache.roots[k];
            let has = cache.content[k];
            if !(has || r == a || r == f) {
                continue;
            }
            let (lo, hi) = if checked.contains(&k) || r == first || r == last {
                // Where the root holding an end stands in `r`, when `r`
                // holds it, and where `r` stands in an end's root.
                let (holds_first, holds_last) = (stands(r, first), stands(r, last));
                let after_start = r == first
                    || holds_first.is_some()
                    || match stands(first, r) {
                        Some(at) => {
                            let takes = if floats_in(tree, first, r) {
                                s_side.takes_floats
                            } else {
                                !s_side.after_inline
                            };
                            at > start || (at == start && takes)
                        }
                        None => i0 < k,
                    };
                let before_end = r == last
                    || holds_last.is_some()
                    || match stands(last, r) {
                        Some(at) => {
                            let takes = if floats_in(tree, last, r) {
                                e_side.takes_floats
                            } else {
                                e_side.after_inline
                            };
                            at < end || (at == end && takes)
                        }
                        None => k < i1,
                    };
                if !(after_start && before_end) {
                    continue;
                }
                let lo = if r == first {
                    start
                } else {
                    holds_first.unwrap_or(0)
                };
                let hi = if r == last {
                    end
                } else {
                    holds_last.unwrap_or_else(|| len(r))
                };
                (lo.min(len(r)), hi.min(len(r)))
            } else {
                (0, len(r))
            };
            out.push((r, lo, hi.max(lo)));
        }
        out
    }

    /// Whether `(node, offset)` comes before the anchor in the document.
    fn before_anchor(&mut self, tree: &rinch_dom::NodeTree, node: u64, offset: usize) -> bool {
        super::text_selection_copy::before(
            tree,
            (node, offset),
            (self.ifc_node_id, self.anchor_offset),
        )
    }
}

/// Where each node in a root's lines stands in the root's text (after the
/// last of its text before the node, as `text_selection_copy::box_offset`
/// has it), worked out once per root in one walk of its lines.
#[derive(Default)]
struct Lines {
    of: HashMap<u64, HashMap<u64, usize>>,
}

impl Lines {
    fn of(&mut self, tree: &rinch_dom::NodeTree, root: u64) -> &HashMap<u64, usize> {
        self.of
            .entry(root)
            .or_insert_with(|| line_offsets(tree, root))
    }
}

/// Every node in root `root`'s lines (its children's subtrees, an
/// anonymous box's run members'), each with where it stands in the root's
/// text. Empty for a text leaf, which holds nothing.
pub(crate) fn line_offsets(tree: &rinch_dom::NodeTree, root: u64) -> HashMap<u64, usize> {
    let mut out = HashMap::new();
    let Some(n) = tree.get(root) else {
        return out;
    };
    let Some(layout) = n.text_layout.as_ref() else {
        return out;
    };
    let mut ends: HashMap<u64, usize> = HashMap::new();
    for r in &layout.text_ranges {
        super::text_selection_copy::work(1);
        let e = ends.entry(r.node_id).or_insert(r.flat_end);
        *e = (*e).max(r.flat_end);
    }
    let mut stack: Vec<u64> = if n.is_anonymous_block_box {
        n.run_members.iter().rev().copied().collect()
    } else {
        n.children.iter().rev().copied().collect()
    };
    let mut at = 0;
    while let Some(id) = stack.pop() {
        super::text_selection_copy::work(1);
        out.insert(id, at);
        if let Some(&e) = ends.get(&id) {
            at = at.max(e);
        }
        if let Some(c) = tree.get(id) {
            stack.extend(c.children.iter().rev().copied());
        }
    }
    out
}

/// The roots whose lines hold root `id` (the paragraph an `inline-block`
/// stands in, and so on out), innermost first.
pub(crate) fn outer_roots(tree: &rinch_dom::NodeTree, id: u64) -> Vec<u64> {
    let node = super::text_selection_copy::root_node(tree, id);
    let mut out = Vec::new();
    let mut at = Some(node);
    while let Some(i) = at {
        let Some(n) = tree.get(i) else {
            break;
        };
        if let Some(b) = n.run_box
            && b != id
        {
            out.push(b);
        }
        at = n.parent;
        if let Some(p) = at
            && tree.get(p).is_some_and(|p| p.text_layout.is_some())
        {
            out.push(p);
        }
    }
    out
}

/// Whether root `inner` stands in root `outer`'s lines inside a float or
/// positioned box (on no line: an end takes it only when it says so).
pub(crate) fn floats_in(tree: &rinch_dom::NodeTree, outer: u64, inner: u64) -> bool {
    let mut at = Some(super::text_selection_copy::root_node(tree, inner));
    while let Some(i) = at {
        if i == outer {
            return false;
        }
        let Some(n) = tree.get(i) else {
            return false;
        };
        if is_float(n) {
            return true;
        }
        at = n.parent;
    }
    false
}

/// A float or an absolutely positioned box.
fn is_float(n: &rinch_dom::Node) -> bool {
    use rinch_dom::computed_style::PositionValue;
    n.computed_style.float != Default::default()
        || matches!(
            n.computed_style.position,
            PositionValue::Absolute | PositionValue::Fixed
        )
}

fn order(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// The text of selection root `id`, which its offsets index: an IFC root's
/// laid-out text, or a text leaf's ([`rinch_dom::text_query::text_leaf_text`]).
pub(crate) fn root_text(tree: &rinch_dom::NodeTree, id: u64) -> Option<Cow<'_, str>> {
    let n = tree.get(id)?;
    match &n.text_layout {
        Some(l) => Some(Cow::Borrowed(l.text_content.as_str())),
        None => rinch_dom::text_query::text_leaf_text(tree, id).map(Cow::Owned),
    }
}

/// The length of selection root `id`'s text ([`root_text`]).
pub(crate) fn root_len(tree: &rinch_dom::NodeTree, id: u64) -> usize {
    match tree.get(id).and_then(|n| n.text_layout.as_ref()) {
        Some(l) => l.text_content.len(),
        None => root_text(tree, id).map_or(0, |t| t.len()),
    }
}

/// The text leaf under, or else nearest, the window point `(x, y)` among
/// `hit`'s children, when `hit` is a flex or grid container holding text
/// of its own: a press on the container beside its text is in it, as a
/// press beside a paragraph's last line is in the paragraph. `None` when
/// `hit` holds no text leaf.
pub(crate) fn leaf_at(tree: &rinch_dom::NodeTree, hit: u64, x: f32, y: f32) -> Option<u64> {
    let n = tree.get(hit)?;
    if n.text_layout.is_some() {
        return None;
    }
    let gap = |lo: f32, len: f32, v: f32| (lo - v).max(v - (lo + len)).max(0.0);
    n.children
        .iter()
        .filter(|&&c| root_text_leaf(tree, c))
        .map(|&c| {
            let (bx, by, bw, bh) = super::hit_testing::painted_element_box(tree, c);
            ((gap(by, bh, y), gap(bx, bw, x)), c)
        })
        .min_by(|a, b| a.0.0.total_cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)))
        .map(|(_, c)| c)
}

/// A text leaf with text ([`rinch_dom::text_query::is_text_leaf`]).
fn root_text_leaf(tree: &rinch_dom::NodeTree, id: u64) -> bool {
    rinch_dom::text_query::is_text_leaf(tree, id)
}

/// `node`'s `user-select` as `auto` resolves it: its own value, else its
/// nearest ancestor's that is not `auto` (see the module docs).
pub(crate) fn resolved_user_select(tree: &rinch_dom::NodeTree, node: u64) -> UserSelectValue {
    let mut id = Some(node);
    while let Some(n) = id.and_then(|i| tree.get(i)) {
        if n.computed_style.user_select != UserSelectValue::Auto {
            return n.computed_style.user_select;
        }
        id = n.parent;
    }
    UserSelectValue::Auto
}

/// The selectable roots (IFC roots and text leaves) at or under `top`, in
/// document order. A run of
/// inline content beside blocks lays out in an anonymous box, which stands
/// in the order where its first member is. A root in another's lines comes
/// after the root holding it.
pub(crate) fn selectable_roots(tree: &rinch_dom::NodeTree, top: u64) -> Vec<u64> {
    let above = tree
        .get(top)
        .and_then(|n| n.parent)
        .map_or(UserSelectValue::Auto, |p| resolved_user_select(tree, p));
    let mut out = Vec::new();
    let mut seen_boxes = std::collections::HashSet::new();
    let mut stack = vec![(top, above)];
    while let Some((id, inherited)) = stack.pop() {
        let Some(n) = tree.get(id) else {
            continue;
        };
        let own = match n.computed_style.user_select {
            UserSelectValue::Auto => inherited,
            v => v,
        };
        // A run's box stands where its first member is; the roots inside
        // its members (an `inline-block` in a span of the run) follow it
        // (#2245).
        if let Some(b) = n.run_box
            && seen_boxes.insert(b)
            && let Some(anon) = tree.get(b)
            && anon.text_layout.is_some()
            && own.is_selectable()
        {
            out.push(b);
        }
        if (n.text_layout.is_some() || root_text_leaf(tree, id)) && own.is_selectable() {
            out.push(id);
        }
        stack.extend(n.children.iter().rev().map(|&c| (c, own)));
    }
    out
}

/// The element that makes `node` selectable: its nearest ancestor-or-self
/// with a `user-select` of its own.
fn selection_host(tree: &rinch_dom::NodeTree, node: u64) -> u64 {
    let mut host = node;
    while let Some(n) = tree.get(host) {
        if n.computed_style.user_select != UserSelectValue::Auto {
            break;
        }
        match n.parent {
            Some(p) => host = p,
            None => break,
        }
    }
    host
}

/// Where a `user-select: all` element under the point `(x, y)` in `root`
/// starts and ends, `((root, offset), (root, offset))`: it is selected
/// whole. `None` when the point is in no such element.
fn all_extent(
    tree: &rinch_dom::NodeTree,
    root: u64,
    x: f32,
    y: f32,
) -> Option<((u64, usize), (u64, usize))> {
    let text = super::hit_testing::text_node_at(tree, root, x, y).unwrap_or(root);
    let mut id = Some(text);
    let mut element = None;
    while let Some(i) = id {
        let n = tree.get(i)?;
        if n.is_element() && n.computed_style.user_select != UserSelectValue::Auto {
            if n.computed_style.user_select == UserSelectValue::All {
                element = Some(i);
            }
            break;
        }
        id = n.parent;
    }
    let element = element?;
    let inside = |mut i: u64| loop {
        if i == element {
            return true;
        }
        match tree.get(i).and_then(|n| n.parent) {
            Some(p) => i = p,
            None => return false,
        }
    };
    if inside(root) {
        // A block: every root in it.
        let roots = selectable_roots(tree, element);
        let (&first, &last) = (roots.first()?, roots.last()?);
        return Some(((first, 0), (last, root_len(tree, last))));
    }
    let layout = tree.get(root)?.text_layout.as_ref()?;
    let mut spans = layout.text_ranges.iter().filter(|r| inside(r.node_id));
    let first = spans.next()?;
    let (lo, hi) = spans.fold((first.flat_start, first.flat_end), |(lo, hi), r| {
        (lo.min(r.flat_start), hi.max(r.flat_end))
    });
    Some(((root, lo), (root, hi)))
}

/// What stands in root `p`'s lines that is not text, in document order:
/// each box element (an atomic inline, a picture, a float or positioned
/// box) and where it stands in `p`'s text.
fn line_boxes(tree: &rinch_dom::NodeTree, p: u64) -> Vec<(u64, usize)> {
    use rinch_dom::computed_style::DisplayValue;
    let Some(n) = tree.get(p) else {
        return Vec::new();
    };
    let mut stack: Vec<u64> = if n.is_anonymous_block_box {
        n.run_members.iter().rev().copied().collect()
    } else {
        n.children.iter().rev().copied().collect()
    };
    let mut out = Vec::new();
    while let Some(id) = stack.pop() {
        let Some(c) = tree.get(id) else {
            continue;
        };
        if c.tag().is_none() || c.computed_style.display == DisplayValue::None {
            continue;
        }
        let in_line = matches!(
            c.computed_style.display,
            DisplayValue::Inline | DisplayValue::Contents
        ) && !matches!(c.tag(), Some("img" | "iframe"))
            && !is_float(c);
        if in_line {
            stack.extend(c.children.iter().rev().copied());
        } else {
            out.push((id, super::text_selection_copy::box_offset(tree, p, id)));
        }
    }
    out
}

/// A word double-clicked at `offset` in `root`: its two ends. It runs on
/// through an `inline-block`, `inline-flex` or float touching it with no
/// break between, into that box's text up to a break there; a picture or
/// an inline table ends it (Chrome 158, #2259). A start leaves out the
/// boxes at its offset it did not run through, an end the same.
fn word_unit(tree: &rinch_dom::NodeTree, root: u64, offset: usize) -> (SelectionEnd, SelectionEnd) {
    let text = root_text(tree, root).unwrap_or_default();
    let (s, e) = super::word_around(&text, offset).unwrap_or((offset, offset));
    let mut start = (root, s, AFTER_INLINE);
    let mut end = (root, e, EndSide::default());
    if s == e {
        return (start, end);
    }
    let boxes = line_boxes(tree, root);
    let takes_all = |after_inline| EndSide {
        after_inline,
        takes_floats: true,
    };
    // The box touching the word at its start: the last of those there.
    if let Some(&(b, _)) = boxes.iter().rfind(|&&(_, at)| at == s)
        && let Some((r, lo, hi)) = word_box_root(tree, b, false)
    {
        let t = root_text(tree, r).unwrap_or_default();
        if let Some((ws, we)) = super::word_around(&t, hi)
            && we == hi
        {
            start = if ws > lo {
                (r, ws, EndSide::default())
            } else {
                (root, s, takes_all(false))
            };
        }
    }
    // And at its end: the first of those there.
    if let Some(&(b, _)) = boxes.iter().find(|&&(_, at)| at == e)
        && let Some((r, lo, hi)) = word_box_root(tree, b, true)
    {
        let t = root_text(tree, r).unwrap_or_default();
        if let Some((ws, we)) = super::word_around(&t, lo)
            && ws == lo
        {
            end = if we < hi {
                (r, we, EndSide::default())
            } else {
                (root, e, takes_all(true))
            };
        }
    }
    (start, end)
}

/// The root inside box `b` a word touching it runs into, with the drawn
/// part of its text: the box's last root (`first`: its first, for a box
/// after the word). `None` for a picture, an inline table, a button after
/// the word, or a box with no text drawn.
fn word_box_root(tree: &rinch_dom::NodeTree, b: u64, first: bool) -> Option<(u64, usize, usize)> {
    let n = tree.get(b)?;
    // A button after a word ends it; one before it does not (Chrome 158:
    // `word<button>OK</button>` selects "word", `<button>OK</button>word`
    // "OKword").
    if matches!(n.tag(), Some("img" | "iframe"))
        || n.computed_style.table_part == rinch_dom::computed_style::TablePart::Table
        || (first && n.tag() == Some("button"))
    {
        return None;
    }
    let roots = selectable_roots(tree, b);
    let r = if first { roots.first() } else { roots.last() }?;
    let (lo, hi) = super::text_selection_copy::rendered_span(tree, *r)?;
    Some((*r, lo, hi))
}

/// A paragraph triple-clicked at `offset` in `root`: its two ends, as
/// Chrome 158 has them (#2256). The paragraph is in the outermost root
/// holding `root` (the one an `inline-block` stands in), not above `host`.
/// It starts after a `<br>` or a float before the offset (a float is a
/// block: a paragraph's edge), else at the root's start, and ends after
/// the next `<br>`, at the next float, or else at the start of the next
/// root with text, so its copy ends with its line breaks; the last root
/// of all ends at its own end. The start takes the in-flow boxes there, an
/// end at the root's end or a float those before it.
fn paragraph_unit(
    tree: &rinch_dom::NodeTree,
    cache: &RootsCache,
    root: u64,
    offset: usize,
    host: u64,
) -> (SelectionEnd, SelectionEnd) {
    use super::text_selection_copy::{box_offset, holds, outermost_root, rendered_span};
    let p = outermost_root(tree, root, host);
    let at = if p == root {
        offset
    } else {
        box_offset(tree, p, root)
    };
    let len = root_len(tree, p);
    let drawn_end = rendered_span(tree, p).map_or(len, |(_, hi)| hi);
    let mut start = 0;
    // The nearest edge after the offset: (offset, after a `<br>`).
    let mut edge: Option<(usize, bool)> = None;
    let mut nearer = |o: usize, br: bool| {
        if edge.is_none_or(|(e, _)| o < e) {
            edge = Some((o, br));
        }
    };
    if let Some(l) = tree.get(p).and_then(|n| n.text_layout.as_ref()) {
        for r in l.text_ranges.iter().filter(|r| r.is_br) {
            if r.flat_end <= at {
                start = start.max(r.flat_end);
            } else {
                nearer(r.flat_end, true);
            }
        }
    }
    for (b, o) in line_boxes(tree, p) {
        if !tree.get(b).is_some_and(is_float) {
            continue;
        }
        if o <= at && o < drawn_end {
            start = start.max(o);
        } else if o > at || o >= drawn_end {
            nearer(o, false);
        }
    }
    let first = (p, start, EndSide::default());
    let last = match edge {
        Some((o, true)) => (p, o, EndSide::default()),
        Some((o, false)) => (p, o, AFTER_INLINE),
        None => cache
            .index
            .get(&p)
            .and_then(|&i| {
                (i + 1..cache.roots.len())
                    .find(|&k| cache.content[k] && !holds(tree, p, cache.roots[k]))
            })
            .map_or((p, len, AFTER_INLINE), |k| {
                (cache.roots[k], 0, EndSide::default())
            }),
    };
    (first, last)
}

impl RinchApp {
    /// The IFC root (a block with an inline layout) or text leaf at or above
    /// `hit_id` whose resolved `user-select` makes it selectable (see the
    /// module docs). A root that is not selectable gives way to one further
    /// up.
    pub(super) fn find_selectable_ifc(tree: &rinch_dom::NodeTree, hit_id: u64) -> Option<u64> {
        let mut candidate: Option<u64> = None;
        let mut check = Some(hit_id);
        while let Some(nid) = check {
            let node = tree.get(nid)?;
            if candidate.is_none() && (node.text_layout.is_some() || root_text_leaf(tree, nid)) {
                candidate = Some(nid);
            }
            if let Some(c) = candidate
                && node.computed_style.user_select != UserSelectValue::Auto
            {
                if node.computed_style.user_select.is_selectable() {
                    return Some(c);
                }
                candidate = None;
            }
            check = node.parent;
        }
        None
    }

    /// [`Self::find_selectable_ifc`] for a hit at the window point `(x, y)`:
    /// over a flex or grid container that holds text of its own, from its
    /// text leaf there ([`leaf_at`]).
    pub(super) fn selectable_at(
        tree: &rinch_dom::NodeTree,
        hit_id: u64,
        x: f32,
        y: f32,
    ) -> Option<u64> {
        Self::find_selectable_ifc(tree, leaf_at(tree, hit_id, x, y).unwrap_or(hit_id))
    }

    /// Which side of the in-flow atomic inlines standing at `offset` in
    /// `root` the window point `(x, y)` is on: after one when it is past the
    /// box's middle toward its after edge (`inline_box_caret`: the right
    /// edge left to right, the left on a right-to-left line, #2259). Never
    /// after a float (it is on no line).
    pub(super) fn side_at(
        tree: &rinch_dom::NodeTree,
        root: u64,
        x: f32,
        y: f32,
        offset: usize,
    ) -> EndSide {
        let Some(layout) = tree.get(root).and_then(|n| n.text_layout.as_ref()) else {
            return EndSide::default();
        };
        let (rel_x, _) = Self::layout_point(tree, root, x, y);
        let after_inline =
            layout
                .atomic_inlines()
                .filter(|&(_, at)| at == offset)
                .any(|(id, _)| {
                    let edge = |after| {
                        rinch_dom::text_query::inline_box_caret(&layout.layout, id, after)
                            .map(|c| c.0)
                    };
                    edge(false)
                        .zip(edge(true))
                        .is_some_and(|(b, a)| (rel_x - (a + b) / 2.0) * (a - b) >= 0.0)
                });
        EndSide {
            after_inline,
            takes_floats: false,
        }
    }

    /// The window point `(x, y)` in IFC root `ifc_node_id`'s lines' own space.
    fn layout_point(tree: &rinch_dom::NodeTree, ifc_node_id: u64, x: f32, y: f32) -> (f32, f32) {
        let Some(node) = tree.get(ifc_node_id) else {
            return (x, y);
        };
        // The point in the IFC root's own space — where Parley laid the text
        // out — rather than a window point minus a transform-blind
        // parent-chain sum (#203).
        let (local_x, local_y) = pointer_in_node(tree, ifc_node_id, x, y);
        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();
        let border_left = node.computed_style.border_left_width.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        (
            local_x - padding_left - border_left + node.scroll_offset.0 as f32,
            local_y - padding_top - border_top + node.scroll_offset.1 as f32,
        )
    }

    /// Compute a byte offset into an IFC layout from a click position.
    pub(super) fn compute_ifc_offset_from_click(
        tree: &rinch_dom::NodeTree,
        ifc_node_id: u64,
        click_x: f32,
        click_y: f32,
    ) -> usize {
        let Some(node) = tree.get(ifc_node_id) else {
            return 0;
        };
        let Some(ref inline_layout) = node.text_layout else {
            // A text leaf: its layout is at its box's origin.
            let (Some(layout), Some(text)) = (
                node.cached_text_parley.as_deref(),
                root_text(tree, ifc_node_id),
            ) else {
                return 0;
            };
            let (lx, ly) = pointer_in_node(tree, ifc_node_id, click_x, click_y);
            return rinch_dom::text_query::grapheme_offset_from_position(layout, &text, lx, ly);
        };
        let (rel_x, rel_y) = Self::layout_point(tree, ifc_node_id, click_x, click_y);
        // Never inside a grapheme cluster (#1684).
        // At the text drawn under the point (#2404).
        rinch_dom::text_query::grapheme_offset_in_inline(inline_layout, rel_x, rel_y)
    }

    /// Write `node`'s highlight (`RinchDocument::set_read_only_selection`).
    fn write_selection_part(d: &mut RinchDocument, node: u64, ranges: &[(usize, usize)]) {
        d.set_read_only_selection(node, ranges);
    }

    /// Paint the current selection: each root it covers carries the ranges
    /// of its part a selection keeps ([`kept_ranges`]). Only the roots whose
    /// ranges changed are written, and a root it no longer covers loses its
    /// highlight.
    pub(super) fn paint_text_selection(&mut self) {
        let (Some(doc), Some(sel)) = (&self.doc, &mut self.text_selection) else {
            return;
        };
        let mut d = doc.borrow_mut();
        let runs = sel.runs(&d.tree);
        let now: Vec<(u64, Vec<(usize, usize)>)> = runs
            .iter()
            .map(|&(node, start, end)| (node, kept_ranges(&d.tree, node, start, end)))
            .collect();
        let before: HashMap<u64, &Vec<(usize, usize)>> =
            sel.painted.iter().map(|(n, r)| (*n, r)).collect();
        let kept: std::collections::HashSet<u64> = now.iter().map(|(n, _)| *n).collect();
        for (old, _) in &sel.painted {
            if !kept.contains(old) {
                Self::write_selection_part(&mut d, *old, &[]);
            }
        }
        for (node, ranges) in &now {
            if before.get(node).is_none_or(|old| *old != ranges) {
                Self::write_selection_part(&mut d, *node, ranges);
            }
        }
        sel.painted = now;
        sel.painted_generation = d.tree.hit_cache.generation();
        self.scene_dirty = true;
    }

    /// After a layout: a selection whose text changed under it is painted
    /// again (its parts clamped to the text there is now).
    pub(super) fn refresh_text_selection(&mut self) {
        let stale = match (&self.doc, &self.text_selection) {
            (Some(doc), Some(sel)) => {
                sel.painted_generation != doc.borrow().tree.hit_cache.generation()
            }
            _ => false,
        };
        if stale {
            self.paint_text_selection();
        }
    }

    /// Move the focus of the selection being dragged to the window point
    /// `(x, y)`: into the selectable root under it, or, over anything else,
    /// the selectable root nearest it in the whole document
    /// ([`RootsCache::nearest`]), as a browser does. A `user-select: all`
    /// element it reaches is taken whole.
    pub(super) fn extend_text_selection_to(&mut self, x: f32, y: f32) {
        let Some(doc) = self.doc.clone() else {
            return;
        };
        {
            let d = doc.borrow();
            let hit = self
                .shared_hit(&d, x, y)
                .and_then(|hit| Self::selectable_at(&d.tree, hit, x, y));
            let Some(sel) = &mut self.text_selection else {
                return;
            };
            let node = hit
                .or_else(|| sel.cache(&d.tree).nearest(x, y))
                .unwrap_or(sel.focus_node);
            let mut offset = Self::compute_ifc_offset_from_click(&d.tree, node, x, y);
            let mut side = Self::side_at(&d.tree, node, x, y, offset);
            // Below a root's box is its end, above it its start.
            if hit.is_none() {
                let (_, top, _, height) = super::hit_testing::painted_element_box(&d.tree, node);
                if y >= top + height {
                    offset = root_len(&d.tree, node);
                    side = AFTER_INLINE;
                } else if y < top {
                    offset = 0;
                    side = EndSide::default();
                }
            }
            let mut node = node;
            if let Some((start, end)) = all_extent(&d.tree, node, x, y) {
                ((node, offset), side) = if sel.before_anchor(&d.tree, node, offset) {
                    (start, EndSide::default())
                } else {
                    (end, AFTER_INLINE)
                };
            }
            sel.focus_side = side;
            sel.focus_node = node;
            sel.focus_offset = offset;
            sel.goal_x = None;
        }
        self.paint_text_selection();
    }

    /// Start a selection at `offset` in `root` for a press `click_count`
    /// times over at `press` (the word around it on a double click, its
    /// paragraph on a triple, a `user-select: all` element whole), or, with
    /// Shift held and a selection in place, move that selection's focus
    /// there: by the word or paragraph a double or triple click selected,
    /// from that unit's far end (Chrome 158, #2256).
    pub(super) fn start_text_selection(
        &mut self,
        root: u64,
        offset: usize,
        click_count: u8,
        shift: bool,
        press: (f32, f32),
    ) {
        if shift && let Some(sel) = &mut self.text_selection {
            sel.press = press;
            sel.goal_x = None;
            sel.dragged = true;
            if let Some(doc) = &self.doc {
                let d = doc.borrow();
                if let Some(unit) = sel.unit {
                    let (start, end) = if unit.paragraph {
                        let host = selection_host(&d.tree, root);
                        paragraph_unit(&d.tree, sel.cache(&d.tree), root, offset, host)
                    } else {
                        word_unit(&d.tree, root, offset)
                    };
                    let back = super::text_selection_copy::before(
                        &d.tree,
                        (root, offset),
                        (unit.start.0, unit.start.1),
                    );
                    let (anchor, focus) = if back {
                        (unit.end, start)
                    } else {
                        (unit.start, end)
                    };
                    (sel.ifc_node_id, sel.anchor_offset, sel.anchor_side) = anchor;
                    (sel.focus_node, sel.focus_offset, sel.focus_side) = focus;
                } else {
                    sel.focus_node = root;
                    sel.focus_offset = offset;
                    sel.focus_side = Self::side_at(&d.tree, root, press.0, press.1, offset);
                }
            } else {
                sel.focus_node = root;
                sel.focus_offset = offset;
            }
        } else {
            self.clear_text_selection();
            let mut sel = TextSelection::at(root, offset);
            sel.press = press;
            if let Some(doc) = &self.doc {
                let d = doc.borrow();
                sel.anchor_side = Self::side_at(&d.tree, root, press.0, press.1, offset);
                sel.focus_side = sel.anchor_side;
                if let Some(((a, ao), (f, fo))) = all_extent(&d.tree, root, press.0, press.1) {
                    (sel.ifc_node_id, sel.anchor_offset) = (a, ao);
                    (sel.focus_node, sel.focus_offset) = (f, fo);
                    (sel.anchor_side, sel.focus_side) = (EndSide::default(), AFTER_INLINE);
                } else if click_count >= 2 {
                    let paragraph = click_count > 2;
                    let (start, end) = if paragraph {
                        let host = selection_host(&d.tree, root);
                        paragraph_unit(&d.tree, sel.cache(&d.tree), root, offset, host)
                    } else {
                        word_unit(&d.tree, root, offset)
                    };
                    (sel.ifc_node_id, sel.anchor_offset, sel.anchor_side) = start;
                    (sel.focus_node, sel.focus_offset, sel.focus_side) = end;
                    sel.unit = Some(SelectionUnit {
                        paragraph,
                        start,
                        end,
                    });
                }
            }
            self.text_selection = Some(sel);
        }
        self.text_selecting = true;
        self.paint_text_selection();
    }

    /// Select all the read-only text the current selection is in: every
    /// selectable root under the element that makes it selectable. `false`
    /// when there is no selection to go by.
    pub(super) fn select_all_text(&mut self) -> bool {
        let Some(anchor) = self.text_selection.as_ref().map(|s| s.ifc_node_id) else {
            return false;
        };
        let Some(doc) = &self.doc else {
            return false;
        };
        let ends = {
            let d = doc.borrow();
            let host = selection_host(&d.tree, anchor);
            let roots = selectable_roots(&d.tree, host);
            let len = |id: u64| root_len(&d.tree, id);
            roots.first().zip(roots.last()).map(|(&first, &last)| {
                // The last root may be an inline-block inside the last
                // paragraph: the selection ends at the paragraph's end.
                let last = super::text_selection_copy::outermost_root(&d.tree, last, host);
                (first, last, len(last))
            })
        };
        let Some((first, last, end)) = ends else {
            return false;
        };
        if let Some(sel) = &mut self.text_selection {
            sel.ifc_node_id = first;
            sel.anchor_offset = 0;
            sel.focus_node = last;
            sel.focus_offset = end;
            sel.dragged = true;
            // A Shift+click after it extends by characters again (Chrome 158).
            sel.unit = None;
            sel.goal_x = None;
            // Its ends are outside the paragraphs: every box is in.
            sel.anchor_side = EndSide {
                after_inline: false,
                takes_floats: true,
            };
            sel.focus_side = EndSide {
                after_inline: true,
                takes_floats: true,
            };
        }
        self.paint_text_selection();
        true
    }

    /// The release of the press that started a selection: a link click held
    /// for it ([`HeldLinkClick`]) runs now, if nothing was selected.
    pub(super) fn release_held_link_click(&mut self) {
        let Some(held) = self.held_link_click.take() else {
            return;
        };
        if self.text_selection.as_ref().is_some_and(|s| !s.is_empty()) {
            return;
        }
        let live = self.doc.as_ref().is_some_and(|doc| {
            doc.borrow()
                .tree
                .get(held.node_id)
                .and_then(|n| n.attributes.get("data-rid"))
                .and_then(|s| s.parse::<usize>().ok())
                .is_some_and(|h| {
                    h == held.handler_id && events::has_click_handler(events::EventHandlerId(h))
                })
        });
        if live {
            self.dispatch_rid_click(
                held.node_id,
                held.handler_id,
                held.text_hit,
                held.at,
                MouseButton::Left,
                held.viewport,
                held.link,
            );
        }
    }

    /// Clear the current text selection state and DOM attributes.
    pub(super) fn clear_text_selection(&mut self) {
        if let Some(sel) = self.text_selection.take() {
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                let painted = sel.painted.iter().map(|(n, _)| *n);
                for node in painted.chain([sel.ifc_node_id]) {
                    Self::write_selection_part(&mut d, node, &[]);
                }
            }
            self.scene_dirty = true;
        }
        self.text_selecting = false;
    }

    /// The selected text, as Chrome's `getSelection().toString()` has it
    /// (`text_selection_copy`).
    pub(super) fn text_selection_text(&mut self) -> Option<String> {
        let doc = self.doc.clone()?;
        let sel = self.text_selection.as_mut().filter(|s| !s.is_empty())?;
        let d = doc.borrow();
        let out = super::text_selection_copy::copy_text(
            &d.tree,
            (sel.ifc_node_id, sel.anchor_offset),
            (sel.focus_node, sel.focus_offset),
            sel.anchor_side,
            sel.focus_side,
        );
        (!out.is_empty()).then_some(out)
    }

    /// Copy the currently selected text to the clipboard.
    pub(super) fn copy_text_selection(&mut self) {
        let Some(selected) = self.text_selection_text() else {
            return;
        };
        #[cfg(feature = "clipboard")]
        {
            let _ = crate::clipboard::copy_text(&selected);
        }
        #[cfg(not(feature = "clipboard"))]
        let _ = selected;
    }
}

#[cfg(test)]
mod tests {
    use super::RinchApp;
    use rinch_core::dom::{DomDocument, NodeId};
    use rinch_dom::RinchDocument;

    fn child_of(doc: &mut RinchDocument, parent: NodeId, tag: &str, style: &str) -> NodeId {
        let el = doc.create_element(tag);
        doc.set_attribute(el, "style", style);
        doc.append_child(parent, el);
        el
    }

    fn text_in(doc: &mut RinchDocument, parent: NodeId, text: &str) -> NodeId {
        let t = doc.create_text(text);
        doc.append_child(parent, t);
        t
    }

    /// #505 — `user_select` is on `ComputedStyle::for_anonymous_box`'s
    /// inherited copy list, justified in prose only: dropping it from the
    /// literal (falling through to `..Self::default()`) passed the entire
    /// workspace. `find_selectable_ifc` is its only consumer, and it requires
    /// **both** `user_select.is_selectable()` **and** `text_layout.is_some()`
    /// on the *same* node — which for an anonymous IFC root is exactly the
    /// struct `for_anonymous_box` builds.
    ///
    /// Sampling the drop at `user-select: none` would sit on a fixed point
    /// (CLAUDE.md "fixed-point blindness"): `UserSelectValue::default()` is
    /// `Auto`, and `Auto.is_selectable()` is `false` — the same answer `None`
    /// gives — so a dropped copy and a correct copy of `none` agree. `text`
    /// is the value that discriminates: copied, the anonymous root answers
    /// selectable; dropped, it silently falls back to `Auto` and does not,
    /// however the real parent declared it.
    ///
    /// Mixed content (a trailing block sibling) forces the text into an
    /// anonymous block box rather than the container's own IFC — the shape
    /// `for_anonymous_box` exists for.
    #[test]
    fn an_anonymous_roots_selectability_comes_from_its_parents_user_select() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = child_of(
            &mut doc,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: text",
        );
        let text = text_in(&mut doc, container, "select me");
        child_of(&mut doc, container, "div", "height: 20px");
        doc.resolve_layout(800.0, 600.0);

        let anon = doc
            .tree
            .get(text.0)
            .unwrap()
            .ifc_root
            .expect("mixed content must put the inline run in an anonymous block box");
        assert!(
            doc.tree.get(anon).unwrap().is_anonymous_block_box,
            "the IFC root must be the synthetic anonymous box, or this test \
             is not exercising the shape it claims to"
        );
        assert!(
            doc.tree.get(anon).unwrap().text_layout.is_some(),
            "the anonymous root must carry the IFC's inline layout, or \
             find_selectable_ifc's second condition is vacuous here"
        );

        // Hitting the anonymous root directly is what a real hit test
        // resolves to for text painted inside one — same node `paint` and
        // `ifc_content_box_offset` treat as the drawn box
        // (`ifc_anonymous_box_tests.rs`).
        assert_eq!(
            RinchApp::find_selectable_ifc(&doc.tree, anon),
            Some(anon),
            "the anonymous root must inherit `user-select: text` from its \
             real parent and answer selectable at itself — `None` here means \
             the copy list dropped `user_select` (it falls back to `Auto`, \
             not selectable), and no ancestor of an anonymous root both \
             carries the inline layout and a `user_select` to fall back on \
             (#505)"
        );
    }

    /// The fixed-point control named above: `none` does not discriminate,
    /// and both the real and the hypothetically-dropped copy must agree it
    /// is not selectable — matching what a real (non-anonymous) root with
    /// the same declaration does.
    #[test]
    fn user_select_none_is_not_selectable_inside_or_outside_an_anonymous_box() {
        let mut mixed = RinchDocument::new();
        let body = mixed.body();
        let container = child_of(
            &mut mixed,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: none",
        );
        let text = text_in(&mut mixed, container, "not selectable");
        child_of(&mut mixed, container, "div", "height: 20px");
        mixed.resolve_layout(800.0, 600.0);
        let anon = mixed.tree.get(text.0).unwrap().ifc_root.unwrap();
        assert_eq!(RinchApp::find_selectable_ifc(&mixed.tree, anon), None);

        let mut plain = RinchDocument::new();
        let body = plain.body();
        let container = child_of(
            &mut plain,
            body,
            "div",
            "width: 400px; font-size: 16px; line-height: 20px; user-select: none",
        );
        text_in(&mut plain, container, "not selectable either");
        plain.resolve_layout(800.0, 600.0);
        assert_eq!(
            RinchApp::find_selectable_ifc(&plain.tree, container.0),
            None,
            "a real root with the same declaration must agree"
        );
    }
}
