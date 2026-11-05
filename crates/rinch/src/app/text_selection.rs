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
//!   carries its own part as the `data-text-sel*` attributes paint reads.
//! - **Copy** joins the parts, one line each.

use super::*;
use rinch_dom::computed_style::UserSelectValue;

impl TextSelection {
    /// A selection collapsed at `offset` in `ifc_node_id`.
    pub(crate) fn at(ifc_node_id: u64, offset: usize) -> Self {
        Self {
            ifc_node_id,
            anchor_offset: offset,
            focus_node: ifc_node_id,
            focus_offset: offset,
            painted: Vec::new(),
        }
    }

    /// Whether it selects anything.
    pub(crate) fn is_empty(&self) -> bool {
        self.ifc_node_id == self.focus_node && self.anchor_offset == self.focus_offset
    }

    /// Each IFC root the selection covers, in document order, with the part
    /// of its text (byte offsets) it covers.
    pub(crate) fn runs(&self, tree: &rinch_dom::NodeTree) -> Vec<(u64, usize, usize)> {
        let (a, f) = (self.ifc_node_id, self.focus_node);
        if a == f {
            let (lo, hi) = order(self.anchor_offset, self.focus_offset);
            return vec![(a, lo, hi)];
        }
        let Some(top) = common_ancestor(tree, a, f) else {
            return vec![(a, self.anchor_offset, self.anchor_offset)];
        };
        let roots = selectable_roots(tree, top);
        let (Some(ia), Some(ifo)) = (
            roots.iter().position(|&r| r == a),
            roots.iter().position(|&r| r == f),
        ) else {
            return vec![(a, self.anchor_offset, self.anchor_offset)];
        };
        let ((first, first_off), (last, last_off), (i0, i1)) = if ia < ifo {
            ((a, self.anchor_offset), (f, self.focus_offset), (ia, ifo))
        } else {
            ((f, self.focus_offset), (a, self.anchor_offset), (ifo, ia))
        };
        let len = |id: u64| {
            tree.get(id)
                .and_then(|n| n.text_layout.as_ref())
                .map_or(0, |l| l.text_content.len())
        };
        roots[i0..=i1]
            .iter()
            .map(|&r| {
                if r == first {
                    (r, first_off, len(r))
                } else if r == last {
                    (r, 0, last_off)
                } else {
                    (r, 0, len(r))
                }
            })
            .collect()
    }
}

fn order(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// The nearest node that is an ancestor-or-self of both `a` and `b`.
fn common_ancestor(tree: &rinch_dom::NodeTree, a: u64, b: u64) -> Option<u64> {
    let chain = |mut id: u64| {
        let mut out = vec![id];
        while let Some(p) = tree.get(id).and_then(|n| n.parent) {
            out.push(p);
            id = p;
        }
        out
    };
    let above_a = chain(a);
    let above_b: std::collections::HashSet<u64> = chain(b).into_iter().collect();
    above_a.into_iter().find(|id| above_b.contains(id))
}

/// `node`'s `user-select` as `auto` resolves it: its own value, else its
/// nearest ancestor's that is not `auto` (see the module docs).
fn resolved_user_select(tree: &rinch_dom::NodeTree, node: u64) -> UserSelectValue {
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

/// The selectable root a drag over no selectable text extends to: of those
/// under the element that makes `anchor` selectable (its nearest
/// ancestor-or-self with a `user-select` of its own), the one level with `y`,
/// else the last one above it, else the first. As a browser extends a
/// selection into the space beside and below text.
pub(crate) fn nearest_selectable_root(
    tree: &rinch_dom::NodeTree,
    anchor: u64,
    y: f32,
) -> Option<u64> {
    let mut host = anchor;
    while let Some(n) = tree.get(host) {
        if n.computed_style.user_select != UserSelectValue::Auto {
            break;
        }
        host = n.parent?;
    }
    let roots = selectable_roots(tree, host);
    let mut above = None;
    for &r in &roots {
        let (_, top, _, height) = super::hit_testing::painted_element_box(tree, r);
        if y >= top && y < top + height {
            return Some(r);
        }
        if top + height <= y {
            above = Some(r);
        }
    }
    above.or_else(|| roots.first().copied())
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
        // The click in the IFC root's own space — where Parley laid the text out
        // — rather than a window point minus a transform-blind parent-chain sum
        // (#203).
        let (local_x, local_y) = pointer_in_node(tree, ifc_node_id, click_x, click_y);
        let padding_left = node.computed_style.padding_left.to_px();
        let padding_top = node.computed_style.padding_top.to_px();
        let border_left = node.computed_style.border_left_width.to_px();
        let border_top = node.computed_style.border_top_width.to_px();
        let rel_x = local_x - padding_left - border_left + node.scroll_offset.0 as f32;
        let rel_y = local_y - padding_top - border_top + node.scroll_offset.1 as f32;
        // Never inside a grapheme cluster (#1684).
        rinch_dom::text_query::grapheme_offset_from_position(
            &inline_layout.layout,
            &inline_layout.text_content,
            rel_x,
            rel_y,
        )
    }

    /// Write `node`'s part of the selection (`start..end`, empty to clear)
    /// as the attributes paint reads, and mark it for repaint.
    fn write_selection_part(d: &mut RinchDocument, node: u64, start: usize, end: usize) {
        if let Some(n) = d.tree.nodes.get_mut(node) {
            if start != end {
                n.write_attribute("data-text-sel", "true");
                n.attributes
                    .insert("data-text-sel-start".to_string(), start.to_string());
                n.attributes
                    .insert("data-text-sel-end".to_string(), end.to_string());
            } else {
                n.erase_attribute("data-text-sel");
                n.attributes.remove("data-text-sel-start");
                n.attributes.remove("data-text-sel-end");
            }
        }
        // The highlight is painted from these attributes on the IFC root,
        // so the root is the damage.
        d.tree.mark_paint_dirty(node);
    }

    /// Paint the current selection: each root it covers carries its part,
    /// and a root it no longer covers loses its highlight.
    pub(super) fn paint_text_selection(&mut self) {
        let (Some(doc), Some(sel)) = (&self.doc, &mut self.text_selection) else {
            return;
        };
        let mut d = doc.borrow_mut();
        let runs = sel.runs(&d.tree);
        for &old in &sel.painted {
            if !runs.iter().any(|r| r.0 == old) {
                Self::write_selection_part(&mut d, old, 0, 0);
            }
        }
        for &(node, start, end) in &runs {
            Self::write_selection_part(&mut d, node, start, end);
        }
        sel.painted = runs.iter().map(|r| r.0).collect();
        self.scene_dirty = true;
    }

    /// Clear the current text selection state and DOM attributes.
    pub(super) fn clear_text_selection(&mut self) {
        if let Some(sel) = self.text_selection.take() {
            if let Some(doc) = &self.doc {
                let mut d = doc.borrow_mut();
                for node in sel.painted.iter().copied().chain([sel.ifc_node_id]) {
                    Self::write_selection_part(&mut d, node, 0, 0);
                }
            }
            self.scene_dirty = true;
        }
        self.text_selecting = false;
    }

    /// The selected text: each covered root's part, one line each.
    pub(super) fn text_selection_text(&self) -> Option<String> {
        let sel = self.text_selection.as_ref().filter(|s| !s.is_empty())?;
        let d = self.doc.as_ref()?.borrow();
        let parts: Vec<&str> = sel
            .runs(&d.tree)
            .into_iter()
            .filter_map(|(node, start, end)| {
                let text = &d.tree.get(node)?.text_layout.as_ref()?.text_content;
                text.get(start.min(text.len())..end.min(text.len()))
            })
            .filter(|t| !t.is_empty())
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n"))
    }

    /// Copy the currently selected text to the clipboard.
    pub(super) fn copy_text_selection(&self) {
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
