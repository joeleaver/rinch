//! Stylesheets that change after the document has been styled (#884).
//!
//! A sheet added, replaced or removed — `load_css`, `set_theme_css`, a
//! `<style>` element entering or leaving the document or having its text
//! edited — is recorded on the Stylist, which collects **stylesheet
//! invalidations** for it (`StylesheetInvalidationSet`, the set Gecko and
//! Servo use): every selector's rightmost id, class or tag (or the outermost
//! one to the left of a descendant combinator, which invalidates the subtree)
//! names the elements whose style the rule can reach. At the next flush
//! ([`RinchDocument::flush_stylist`]) those are matched against the styled
//! document, starting at `<html>`, and marked with restyle hints, which the
//! style walk follows like any other. A rule that matches no element restyles
//! nothing, and nothing is laid out again.
//!
//! What Stylo cannot bucket — a selector with no id/class/tag on either side
//! (`*`, `:root`, a bare pseudo-class), a referenced `@keyframes`, `@property`,
//! an `@layer a, b;` statement — marks the whole document. (An `@layer x { … }`
//! block is bucketed by the rules inside it, like any other.) That is a whole-document restyle, the
//! one a `<style>` append always paid before (`full_restyle_stylesheet`).
//!
//! **`<style>` elements.** Their sheets apply while the element is connected,
//! in tree order, after every `load_css` sheet: on the web those are `<head>`
//! sheets (`WebDocument::inject_style`), so a `<style>` in the body cascades
//! after them. [`RinchDocument::sync_style_element_sheets`] brings the
//! installed sheets in line with the document at the next style resolve, so
//! an element built (and filled) while detached applies when it is attached,
//! and a removed one stops applying.

use style::invalidation::element::restyle_hints::RestyleHint;
use style::stylesheets::DocumentStyleSheet;

use crate::RinchDocument;
use crate::node::RawNodeId;

/// A connected `<style>` element's sheet, with the CSS text it was parsed
/// from. `sheet` is `None` for an element with no text.
pub(crate) struct StyleElementSheet {
    pub(crate) node: RawNodeId,
    pub(crate) css: String,
    pub(crate) sheet: Option<DocumentStyleSheet>,
}

impl RinchDocument {
    /// Whether `id` is a live `<style>` element. A checked lookup:
    /// `style_elements` keeps ids that may since have been freed or reused
    /// (#723), and a stale id's generation no longer matches.
    fn is_style_element(&self, id: RawNodeId) -> bool {
        self.tree
            .nodes
            .get(id)
            .is_some_and(|n| n.tag() == Some("style"))
    }

    /// A `<style>` element's text changed: its sheet is brought up to date at
    /// the next style resolve.
    pub(crate) fn note_style_text_changed(&mut self, node_id: RawNodeId) {
        if self.is_style_element(node_id) {
            self.style_elements_stale = true;
            self.tree.styles_dirty = true;
        }
    }

    /// `parent`'s child list changed. If that moved a `<style>` element into
    /// or out of the document, or changed a `<style>`'s text children, its
    /// sheet is brought up to date at the next style resolve. Costs nothing in
    /// a document with no `<style>` element.
    pub(crate) fn note_style_elements_moved(&mut self, parent: RawNodeId) {
        if self.style_elements.is_empty() || self.style_elements_stale {
            return;
        }
        let changed = self.is_style_element(parent)
            || self.style_elements.iter().any(|&id| {
                let connected = self.is_style_element(id) && self.depth_if_connected(id).is_some();
                let installed = self.style_element_sheets.iter().any(|s| s.node == id);
                connected != installed
            });
        if changed {
            self.style_elements_stale = true;
            self.tree.styles_dirty = true;
        }
    }

    /// Install the sheets of the connected `<style>` elements, in tree order,
    /// and remove the rest. The common prefix of what is installed is kept;
    /// from the first difference on, the installed sheets are removed and the
    /// wanted ones appended (re-using a parsed sheet whose element and text
    /// are unchanged), since they are the last author sheets.
    pub(crate) fn sync_style_element_sheets(&mut self) {
        if !std::mem::take(&mut self.style_elements_stale) {
            return;
        }
        // Freed ids, and ids since reused by another element, drop out; a
        // reused id that is a `<style>` again was registered twice.
        let mut ids = std::mem::take(&mut self.style_elements);
        ids.retain(|&id| self.is_style_element(id));
        ids.sort_unstable();
        ids.dedup();
        self.style_elements = ids;

        let mut wanted: Vec<(Vec<usize>, RawNodeId, String)> = Vec::new();
        for &id in &self.style_elements {
            let Some(path) = self.tree_order_path(id) else {
                continue;
            };
            let mut css = String::new();
            for &child in &self.tree.nodes[id].children {
                if let Some(text) = self
                    .tree
                    .nodes
                    .get_live(child)
                    .and_then(|n| n.text_content())
                {
                    css.push_str(text);
                }
            }
            wanted.push((path, id, css));
        }
        wanted.sort_by(|a, b| a.0.cmp(&b.0));

        let keep = self
            .style_element_sheets
            .iter()
            .zip(&wanted)
            .take_while(|(have, (_, id, css))| have.node == *id && have.css == *css)
            .count();
        if keep == self.style_element_sheets.len() && keep == wanted.len() {
            return;
        }

        let mut removed = self.style_element_sheets.split_off(keep);
        {
            let guard = self.tree.guard.read();
            for entry in &removed {
                if let Some(sheet) = &entry.sheet {
                    self.stylist.remove_stylesheet(sheet.clone(), &guard);
                }
            }
        }
        for (_, node, css) in wanted.into_iter().skip(keep) {
            let reused = removed
                .iter()
                .position(|e| e.node == node && e.css == css)
                .map(|i| removed.swap_remove(i).sheet);
            let sheet = match reused {
                Some(sheet) => sheet,
                None if css.is_empty() => None,
                None => Some(self.parse_author_stylesheet(&css)),
            };
            if let Some(sheet) = &sheet {
                let guard = self.tree.guard.read();
                self.stylist.append_stylesheet(sheet.clone(), &guard);
            }
            self.style_element_sheets
                .push(StyleElementSheet { node, css, sheet });
        }
        self.sheet_invalidations_pending = true;
        self.recompute_bare_focus_rules();
    }

    /// The child indices from the document node down to `id`, which order
    /// nodes in tree order; `None` when `id` is not in the document.
    fn tree_order_path(&self, id: RawNodeId) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        let mut current = id;
        while current != self.tree.root_id {
            let parent = self.tree.nodes.get(current)?.parent?;
            let index = self.tree.nodes[parent]
                .children
                .iter()
                .position(|&c| c == current)?;
            path.push(index);
            current = parent;
        }
        path.reverse();
        Some(path)
    }

    /// The sheet a new `load_css` sheet is inserted before: the first
    /// `<style>` element's, so it cascades after the earlier `load_css` sheets
    /// and before every `<style>` element (see the module doc).
    pub(crate) fn first_style_element_sheet(&self) -> Option<DocumentStyleSheet> {
        self.style_element_sheets
            .iter()
            .find_map(|e| e.sheet.clone())
    }

    /// Flush the Stylist's pending sheet changes, applying their
    /// invalidations to the styled document (see the module doc). Answers
    /// whether that queued a restyle: a whole-document walk, or `<html>` as a
    /// style root from which the walk follows Stylo's hints.
    pub(crate) fn flush_stylist(&mut self) -> bool {
        use crate::stylo_impl::RinchNode;
        use style::shared_lock::StylesheetGuards;

        self.sync_style_element_sheets();
        let invalidate = std::mem::take(&mut self.sheet_invalidations_pending);
        let html_id = self.tree.html_id;
        let html_hint = |doc: &Self| {
            doc.tree.nodes[html_id]
                .stylo_element_data
                .borrow()
                .map_or(RestyleHint::empty(), |d| d.hint)
        };
        let whole_before = html_hint(self).contains(RestyleHint::RESTYLE_DESCENDANTS);
        let invalidated = {
            let guard = self.tree.guard.read();
            let guards = StylesheetGuards::same(&guard);
            let invalidations = self.stylist.flush(&guards);
            // 0.22 returns the set after a forced-dirty flush (viewport, DPR,
            // theme) too; only a sheet change here is ours to apply.
            let root = invalidate.then(|| RinchNode::new(html_id, &self.tree));
            root.is_some_and(|r| invalidations.process_style(r, None))
        };
        if !invalidated {
            return false;
        }
        if !whole_before && html_hint(self).contains(RestyleHint::RESTYLE_DESCENDANTS) {
            // The whole document: the same restyle a `<style>` append always
            // made.
            self.tree
                .note_full_restyle(crate::perf::FullRestyleReason::Stylesheet);
            for (nid, _) in self.tree.nodes.iter() {
                self.tree.nodes[nid].stylo_element_data.clear();
            }
            self.tree.style_roots.clear();
            self.tree.full_style_walk = true;
        } else {
            self.tree.style_roots.push(html_id);
        }
        self.tree.styles_dirty = true;
        true
    }
}
