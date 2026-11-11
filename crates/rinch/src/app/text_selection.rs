//! Read-only text selection for `user-select: text` content.
//!
//! - **Which text is selectable.** `user-select` does not inherit, but its
//!   initial `auto` takes its parent's value (CSS UI 4 §6.1), so an IFC root
//!   is selectable when its nearest ancestor-or-self whose value is not
//!   `auto` says `text`, `all` or `contain`. With none at all it is not:
//!   rinch's default, for an app's own controls. Worked out where it is
//!   asked ([`RinchApp::find_selectable_ifc`], [`selectable_roots`]), not
//!   stored, so a container's `user-select` changing needs no restyle below it.
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
        }
    }

    /// Whether it selects anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.ifc_node_id == self.focus_node && self.anchor_offset == self.focus_offset
    }

    /// The document's selectable roots for this layout, built when there
    /// are none or they are stale.
    fn cache(&mut self, tree: &rinch_dom::NodeTree) -> &RootsCache {
        if !self.cache.as_ref().is_some_and(|c| c.current(tree)) {
            self.cache = Some(Box::new(RootsCache::build(tree)));
        }
        self.cache.as_deref().expect("built just now")
    }

    /// Each IFC root the selection covers, in document order, with the part
    /// of its text (byte offsets, clamped to it) it covers.
    pub(crate) fn runs(&mut self, tree: &rinch_dom::NodeTree) -> Vec<(u64, usize, usize)> {
        let (a, f, ao, fo) = (
            self.ifc_node_id,
            self.focus_node,
            self.anchor_offset,
            self.focus_offset,
        );
        let len = |id: u64| {
            tree.get(id)
                .and_then(|n| n.text_layout.as_ref())
                .map_or(0, |l| l.text_content.len())
        };
        if a == f {
            let (lo, hi) = order(ao.min(len(a)), fo.min(len(a)));
            return vec![(a, lo, hi)];
        }
        let cache = self.cache(tree);
        let (Some(&ia), Some(&ifo)) = (cache.index.get(&a), cache.index.get(&f)) else {
            return vec![(a, ao, ao)];
        };
        let ((first, first_off), (last, last_off), (i0, i1)) = if ia < ifo {
            ((a, ao), (f, fo), (ia, ifo))
        } else {
            ((f, fo), (a, ao), (ifo, ia))
        };
        // Roots with nothing to take (white space between a mail's tables)
        // are passed, as Chrome passes them.
        cache.roots[i0..=i1]
            .iter()
            .zip(&cache.content[i0..=i1])
            .filter(|&(&r, &has)| has || r == a || r == f)
            .map(|(&r, _)| {
                if r == first {
                    (r, first_off.min(len(r)), len(r))
                } else if r == last {
                    (r, 0, last_off.min(len(r)))
                } else {
                    (r, 0, len(r))
                }
            })
            .collect()
    }

    /// Whether `(node, offset)` comes before the anchor in document order.
    fn before_anchor(&mut self, tree: &rinch_dom::NodeTree, node: u64, offset: usize) -> bool {
        if node == self.ifc_node_id {
            return offset < self.anchor_offset;
        }
        let anchor = self.ifc_node_id;
        let cache = self.cache(tree);
        match (cache.index.get(&node), cache.index.get(&anchor)) {
            (Some(n), Some(a)) => n < a,
            _ => false,
        }
    }
}

fn order(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
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

/// The selectable IFC roots at or under `top`, in document order. A run of
/// inline content beside blocks lays out in an anonymous box, which stands
/// in the order where its first member is.
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
        if let Some(b) = n.run_box {
            if seen_boxes.insert(b)
                && let Some(anon) = tree.get(b)
                && anon.text_layout.is_some()
                && own.is_selectable()
            {
                out.push(b);
            }
            continue;
        }
        if n.text_layout.is_some() && own.is_selectable() {
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
        let end = tree.get(last)?.text_layout.as_ref()?.text_content.len();
        return Some(((first, 0), (last, end)));
    }
    let layout = tree.get(root)?.text_layout.as_ref()?;
    let mut spans = layout.text_ranges.iter().filter(|r| inside(r.node_id));
    let first = spans.next()?;
    let (lo, hi) = spans.fold((first.flat_start, first.flat_end), |(lo, hi), r| {
        (lo.min(r.flat_start), hi.max(r.flat_end))
    });
    Some(((root, lo), (root, hi)))
}

impl RinchApp {
    /// The IFC root (a block with an inline layout) at or above `hit_id`
    /// whose resolved `user-select` makes it selectable (see the module
    /// docs). A root that is not selectable gives way to one further up.
    pub(super) fn find_selectable_ifc(tree: &rinch_dom::NodeTree, hit_id: u64) -> Option<u64> {
        let mut candidate: Option<u64> = None;
        let mut check = Some(hit_id);
        while let Some(nid) = check {
            let node = tree.get(nid)?;
            if candidate.is_none() && node.text_layout.is_some() {
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

    /// Which side of the in-flow atomic inlines standing at `offset` in
    /// `root` the window point `(x, y)` is on: after one when it is past the
    /// box's middle. Never after a float (it is on no line). Left to right:
    /// a box's after side is its right edge (`inline_box_caret`).
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
                        .is_some_and(|(l, r)| rel_x >= (l + r) / 2.0)
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
            return 0;
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
                .and_then(|hit| Self::find_selectable_ifc(&d.tree, hit));
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
                    offset = d
                        .tree
                        .get(node)
                        .and_then(|n| n.text_layout.as_ref())
                        .map_or(offset, |l| l.text_content.len());
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
        }
        self.paint_text_selection();
    }

    /// Start a selection at `offset` in `root` for a press `click_count`
    /// times over at `press` (the word around it on a double click, the whole
    /// root on a triple, a `user-select: all` element whole), or, with Shift
    /// held and a selection in place, move that selection's focus there.
    pub(super) fn start_text_selection(
        &mut self,
        root: u64,
        offset: usize,
        click_count: u8,
        shift: bool,
        press: (f32, f32),
    ) {
        if shift && let Some(sel) = &mut self.text_selection {
            sel.focus_node = root;
            sel.focus_offset = offset;
            sel.press = press;
            sel.dragged = true;
            if let Some(doc) = &self.doc {
                sel.focus_side = Self::side_at(&doc.borrow().tree, root, press.0, press.1, offset);
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
                    let text = d
                        .tree
                        .get(root)
                        .and_then(|n| n.text_layout.as_ref())
                        .map(|l| l.text_content.as_str())
                        .unwrap_or_default();
                    let (start, end) = if click_count == 2 {
                        super::word_around(text, offset).unwrap_or((offset, offset))
                    } else {
                        (0, text.len())
                    };
                    sel.anchor_offset = start;
                    sel.focus_offset = end;
                    // A word leaves the boxes beside it out; a paragraph takes
                    // the inline-blocks at its edges, not its floats (Chrome 156).
                    (sel.anchor_side, sel.focus_side) = if click_count == 2 {
                        (AFTER_INLINE, EndSide::default())
                    } else {
                        (EndSide::default(), AFTER_INLINE)
                    };
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
            let len = |id: u64| {
                d.tree
                    .get(id)
                    .and_then(|n| n.text_layout.as_ref())
                    .map_or(0, |l| l.text_content.len())
            };
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
