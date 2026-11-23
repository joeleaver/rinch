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
//! **Layer order.** A sheet change can reorder the cascade layers of sheets
//! it does not touch (each layer is ordered by where its name first appears),
//! which Stylo's invalidation does not see;
//! [`RinchDocument::invalidate_reordered_layers`] adds the rules of the layers
//! that moved (#2694).
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
        self.note_style_elements_changed(parent, None);
    }

    /// [`Self::note_style_elements_moved`] for `child`, just inserted into
    /// `parent`. A connected `<style>` in `child`'s subtree may have moved
    /// within the document (`insert_before` / `append_child` of a connected
    /// node, with no remove first): connected before and after, it would not
    /// count as entering, yet its sheet's place in the cascade may have
    /// changed (#2694). The sync works out whether it did.
    pub(crate) fn note_style_subtree_inserted(&mut self, parent: RawNodeId, child: RawNodeId) {
        self.note_style_elements_changed(parent, Some(child));
    }

    fn note_style_elements_changed(&mut self, parent: RawNodeId, inserted: Option<RawNodeId>) {
        if self.style_elements.is_empty() || self.style_elements_stale {
            return;
        }
        let changed = self.is_style_element(parent)
            || self.style_elements.iter().any(|&id| {
                let (connected, inside) = if self.is_style_element(id) {
                    self.connected_and_under(id, inserted)
                } else {
                    (false, false)
                };
                let installed = self.style_element_sheets.iter().any(|s| s.node == id);
                connected != installed || (connected && inside)
            });
        if changed {
            self.style_elements_stale = true;
            self.tree.styles_dirty = true;
        }
    }

    /// Whether `id` is in the document, and whether `ancestor` is `id` or one
    /// of its ancestors: one walk up.
    fn connected_and_under(&self, id: RawNodeId, ancestor: Option<RawNodeId>) -> (bool, bool) {
        let mut under = false;
        let mut current = Some(id);
        while let Some(node) = current {
            under |= Some(node) == ancestor;
            if node == self.tree.root_id {
                return (true, under);
            }
            current = self.tree.nodes.get_live(node).and_then(|n| n.parent);
        }
        (false, under)
    }

    /// Install the sheets of the connected `<style>` elements, in tree order,
    /// and remove the rest. An installed sheet whose element is still
    /// connected with the same text stays where it is; a sheet that left is
    /// removed and a new one inserted before the next sheet that stayed (or
    /// appended: they are the last author sheets), so only the sheets that
    /// changed reach Stylo's invalidation (#2636). If the staying sheets are
    /// out of tree order (an element moved past another), the longest run of
    /// them still in order stays and the others are re-inserted at their new
    /// places (#2694), re-using a parsed sheet whose element and text are
    /// unchanged.
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

        let wanted_at: std::collections::HashMap<RawNodeId, usize> = wanted
            .iter()
            .enumerate()
            .map(|(i, (_, id, _))| (*id, i))
            .collect();
        // The wanted index of an installed sheet whose element is still
        // connected with the same text.
        let still_wanted: Vec<Option<usize>> = self
            .style_element_sheets
            .iter()
            .map(|have| {
                wanted_at
                    .get(&have.node)
                    .copied()
                    .filter(|&i| wanted[i].2 == have.css)
            })
            .collect();
        if self.style_element_sheets.len() == wanted.len()
            && still_wanted
                .iter()
                .enumerate()
                .all(|(i, at)| *at == Some(i))
        {
            return;
        }
        // Which installed sheets stay. Only the ones that left are removed and
        // only the ones that came are added, each new one before the next
        // sheet that stayed (#2636): Stylo collects invalidations for every
        // sheet added or removed, so re-adding the later sheets made a
        // `<style>` coming or going re-invalidate all of them, and one `*` or
        // `:root` rule among them restyled the whole document. When the
        // staying sheets are no longer in tree order (a `<style>`, or an
        // ancestor of one, moved past another: #2694), the longest run of
        // them that is still in order stays and the others are removed and
        // re-inserted at their new places, so a move re-invalidates the
        // sheets that moved and no others.
        let candidates: Vec<usize> = still_wanted.iter().flatten().copied().collect();
        let mut in_run = longest_increasing_run(&candidates).into_iter();
        let stays: Vec<bool> = still_wanted
            .iter()
            .map(|at| at.is_some() && in_run.next().unwrap_or(false))
            .collect();

        let mut kept = Vec::new();
        let mut removed = Vec::new();
        {
            let guard = self.tree.guard.read();
            for (entry, stays) in std::mem::take(&mut self.style_element_sheets)
                .into_iter()
                .zip(stays)
            {
                if stays {
                    kept.push(entry);
                } else {
                    if let Some(sheet) = &entry.sheet {
                        self.stylist.remove_stylesheet(sheet.clone(), &guard);
                    }
                    removed.push(entry);
                }
            }
        }
        // `kept` is in `wanted`'s order: walk both.
        let mut next_kept = 0;
        let mut sheets = Vec::with_capacity(wanted.len());
        let mut kept = kept.into_iter().map(Some).collect::<Vec<_>>();
        for (_, node, css) in wanted {
            if let Some(entry) = kept
                .get_mut(next_kept)
                .filter(|e| e.as_ref().is_some_and(|e| e.node == node && e.css == css))
                .and_then(Option::take)
            {
                next_kept += 1;
                sheets.push(entry);
                continue;
            }
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
                let before = kept[next_kept..]
                    .iter()
                    .flatten()
                    .find_map(|e| e.sheet.clone());
                let guard = self.tree.guard.read();
                match before {
                    Some(before) => {
                        self.stylist
                            .insert_stylesheet_before(sheet.clone(), before, &guard)
                    }
                    None => self.stylist.append_stylesheet(sheet.clone(), &guard),
                }
            }
            sheets.push(StyleElementSheet { node, css, sheet });
        }
        debug_assert!(
            kept.iter().all(Option::is_none),
            "every staying sheet is still wanted"
        );
        self.style_element_sheets = sheets;
        self.forget_dead_motion_sheets();
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
        if invalidate {
            self.invalidate_reordered_layers();
        }
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
            crate::node::invalidate_sibling_marks();
            self.tree.full_style_walk = true;
        } else {
            self.tree.style_roots.push(html_id);
        }
        self.tree.styles_dirty = true;
        true
    }
}

/// One segment of a cascade layer's full name: a named layer's ident, or an
/// anonymous layer (`@layer { … }`, `@import … layer`), which is a layer of
/// its own per rule and is identified by that rule.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum LayerSegment {
    Named(style::values::AtomIdent),
    Anonymous(usize),
}

/// A cascade layer's full name, outermost first (`a.b` is `[a, b]`).
pub(crate) type LayerPath = Vec<LayerSegment>;

/// [`RinchDocument::walk_author_layers`]'s visitor: a sheet, a layer's full
/// name, and the rule holding that layer's rules (`None` for a name only
/// declared); answering false skips the rule's children.
type SheetLayerVisitor<'f> = dyn FnMut(&DocumentStyleSheet, &[LayerSegment], Option<&style::stylesheets::CssRule>) -> bool
    + 'f;

/// Walks the effective rules of a list of sheets for the rules that declare
/// cascade layers, in the order Stylo registers them
/// (`CascadeData::add_rule_list`).
struct LayerWalk<'a, 'g> {
    guard: &'a style::shared_lock::SharedRwLockReadGuard<'g>,
    device: &'a style::device::Device,
    custom_media: &'a style::stylesheets::CustomMediaMap,
}

impl LayerWalk<'_, '_> {
    /// Calls `f` with every layer each rule in `rules` declares (and, for a
    /// dotted name, each of its prefixes first), nested in `parent`. `f` gets
    /// the rule whose children belong to the layer — an `@layer` block or a
    /// layered `@import` — with the layer's full name, `None` for a name
    /// only declared (a prefix, an `@layer a, b;` statement); answering false
    /// skips that rule's children.
    fn walk(
        &self,
        rules: &[style::stylesheets::CssRule],
        parent: &[LayerSegment],
        f: &mut dyn FnMut(&[LayerSegment], Option<&style::stylesheets::CssRule>) -> bool,
    ) {
        use style::stylesheets::import_rule::ImportLayer;
        use style::stylesheets::{CssRule, EffectiveRules, RulesIterator};

        for rule in rules {
            let mut effective = true;
            let children =
                RulesIterator::<EffectiveRules, &style::stylesheets::CustomMediaMap>::children(
                    rule,
                    self.device,
                    self.device.quirks_mode(),
                    self.custom_media,
                    self.guard,
                    &mut effective,
                );
            if !effective {
                continue;
            }
            let layer: Option<(Option<&style::stylesheets::layer_rule::LayerName>, usize)> =
                match rule {
                    CssRule::LayerBlock(block) => Some((block.name.as_ref(), arc_addr(block))),
                    CssRule::Import(lock) => match &lock.read_with(self.guard).layer {
                        ImportLayer::Named(name) => Some((Some(name), 0)),
                        ImportLayer::Anonymous => Some((None, arc_addr(lock))),
                        ImportLayer::None => None,
                    },
                    CssRule::LayerStatement(statement) => {
                        for name in &statement.names {
                            let mut path = parent.to_vec();
                            for segment in name.layer_names() {
                                path.push(LayerSegment::Named(segment.clone()));
                                f(&path, None);
                            }
                        }
                        continue;
                    }
                    _ => None,
                };
            match layer {
                Some((name, anonymous)) => {
                    let mut path = parent.to_vec();
                    match name {
                        Some(name) => {
                            let segments = name.layer_names();
                            for (i, segment) in segments.iter().enumerate() {
                                path.push(LayerSegment::Named(segment.clone()));
                                if i + 1 < segments.len() {
                                    f(&path, None);
                                }
                            }
                        }
                        None => path.push(LayerSegment::Anonymous(anonymous)),
                    }
                    if f(&path, Some(rule)) {
                        self.walk(children, &path, f);
                    }
                }
                None => self.walk(children, parent, f),
            }
        }
    }
}

/// A sheet's address, which keys `layered_sheets`.
fn sheet_addr(sheet: &DocumentStyleSheet) -> usize {
    arc_addr(&sheet.0)
}

/// The address of an `Arc`-held rule, which names an anonymous layer.
fn arc_addr<T>(arc: &style::servo_arc::Arc<T>) -> usize {
    &**arc as *const T as usize
}

impl RinchDocument {
    /// The author sheets that declare a cascade layer, in cascade order: the
    /// rest cannot change the layer order.
    fn author_sheets(&self) -> Vec<DocumentStyleSheet> {
        use style::stylesheets::Origin;
        if self.layered_sheets.is_empty() {
            return Vec::new();
        }
        (0..self.stylist.sheet_count(Origin::Author))
            .filter_map(|i| self.stylist.sheet_at(Origin::Author, i))
            .filter(|sheet| self.layered_sheets.contains(&sheet_addr(sheet)))
            .cloned()
            .collect()
    }

    /// Record whether a just-parsed author sheet declares a cascade layer
    /// anywhere: an `@layer` block or statement, or a layered `@import`, at
    /// any depth and whatever its conditions.
    pub(crate) fn note_sheet_layers(&mut self, sheet: &DocumentStyleSheet) {
        use style::stylesheets::import_rule::ImportLayer;
        use style::stylesheets::{CssRule, StylesheetInDocument};
        fn declares(rules: &[CssRule], guard: &style::shared_lock::SharedRwLockReadGuard) -> bool {
            rules.iter().any(|rule| {
                match rule {
                    CssRule::LayerBlock(_) | CssRule::LayerStatement(_) => return true,
                    CssRule::Import(lock)
                        if !matches!(lock.read_with(guard).layer, ImportLayer::None) =>
                    {
                        return true;
                    }
                    _ => {}
                }
                declares(rule.children(guard), guard)
            })
        }
        let layered = {
            let guard = self.tree.guard.read();
            declares(sheet.contents(&guard).rules(&guard), &guard)
        };
        let addr = sheet_addr(sheet);
        if layered {
            self.layered_sheets.insert(addr);
        } else {
            self.layered_sheets.remove(&addr);
        }
    }

    /// Recompute the saved author layer order without invalidating anything:
    /// for a change that restyles the whole document anyway (a media flip),
    /// after which a later sheet change must compare against the order now
    /// in effect.
    pub(crate) fn refresh_author_layer_order(&mut self) {
        let sheets = self.author_sheets();
        self.author_layer_order = self.compute_author_layer_order(&sheets);
    }

    /// Walk the author sheets' layer-declaring rules (see [`LayerWalk`]).
    fn walk_author_layers(
        &self,
        sheets: &[DocumentStyleSheet],
        guard: &style::shared_lock::SharedRwLockReadGuard<'_>,
        f: &mut SheetLayerVisitor<'_>,
    ) {
        use style::stylesheets::{Origin, StylesheetInDocument};
        let empty = style::stylesheets::CustomMediaMap::default();
        let custom_media = self
            .stylist
            .iter_origins()
            .find(|(_, origin)| *origin == Origin::Author)
            .map_or(&empty, |(data, _)| data.custom_media_map());
        let walk = LayerWalk {
            guard,
            device: self.stylist.device(),
            custom_media,
        };
        for sheet in sheets {
            if !sheet.enabled() {
                continue;
            }
            walk.walk(
                sheet.contents(guard).rules(guard),
                &[],
                &mut |path, rule| f(sheet, path, rule),
            );
        }
    }

    /// The author origin's cascade layers in cascade order: as Stylo orders
    /// them (`CascadeData::compute_layer_order`), each where its name first
    /// appears, a layer's sub-layers ahead of the layer itself.
    fn compute_author_layer_order(&self, sheets: &[DocumentStyleSheet]) -> Vec<LayerPath> {
        // The registered layers, each with the indices of its sub-layers in
        // registration order; 0 is the root (the unlayered rules).
        let mut layers: Vec<(LayerPath, Vec<usize>)> = vec![(Vec::new(), Vec::new())];
        let mut index: std::collections::HashMap<LayerPath, usize> =
            std::collections::HashMap::new();
        let guard = self.tree.guard.read();
        self.walk_author_layers(sheets, &guard, &mut |_, path, _| {
            if !index.contains_key(path) {
                let parent = index.get(&path[..path.len() - 1]).copied().unwrap_or(0);
                let id = layers.len();
                layers[parent].1.push(id);
                layers.push((path.to_vec(), Vec::new()));
                index.insert(path.to_vec(), id);
            }
            true
        });
        if layers.len() == 1 {
            return Vec::new();
        }
        fn post_order(
            layers: &mut Vec<(LayerPath, Vec<usize>)>,
            id: usize,
            out: &mut Vec<LayerPath>,
        ) {
            for child in std::mem::take(&mut layers[id].1) {
                post_order(layers, child, out);
            }
            if id != 0 {
                out.push(std::mem::take(&mut layers[id].0));
            }
        }
        let mut order = Vec::with_capacity(layers.len() - 1);
        post_order(&mut layers, 0, &mut order);
        order
    }

    /// A sheet added, removed or edited can change the order of the author
    /// cascade layers: each is ordered by where its name first appears, so a
    /// sheet naming `a` added ahead of one with `@layer b { … } @layer a
    /// { … }` puts `a` before `b` (#2694). Stylo's stylesheet invalidation
    /// covers only the changed sheet's own rules (an `@layer` block is
    /// bucketed by the rules inside it), so the elements the *other* sheets'
    /// layered rules match would keep their old cascade. Here, when the order
    /// changed, the layers that moved — every one outside a longest run that
    /// kept its relative order, so of any two layers that swapped at least one
    /// — have their rules' invalidations collected, in every sheet, as if
    /// those rules had been inserted. Rules in layers that kept their place,
    /// and unlayered rules, are not touched.
    fn invalidate_reordered_layers(&mut self) {
        let sheets = self.author_sheets();
        let order = self.compute_author_layer_order(&sheets);
        let old = std::mem::replace(&mut self.author_layer_order, order);
        if old == self.author_layer_order {
            return;
        }
        // The layers in both orders: their old positions, in the new order.
        let old_at: std::collections::HashMap<&LayerPath, usize> =
            old.iter().enumerate().map(|(i, p)| (p, i)).collect();
        let common: Vec<(&LayerPath, usize)> = self
            .author_layer_order
            .iter()
            .filter_map(|p| old_at.get(p).map(|&i| (p, i)))
            .collect();
        let positions: Vec<usize> = common.iter().map(|&(_, i)| i).collect();
        let moved: std::collections::HashSet<LayerPath> = common
            .iter()
            .zip(longest_increasing_run(&positions))
            .filter(|(_, stays)| !stays)
            .map(|((p, _), _)| (*p).clone())
            .collect();
        if moved.is_empty() {
            return;
        }
        let mut changed: Vec<(DocumentStyleSheet, style::stylesheets::CssRule)> = Vec::new();
        {
            let guard = self.tree.guard.read();
            self.walk_author_layers(&sheets, &guard, &mut |sheet, path, rule| match rule {
                Some(rule) if moved.contains(path) => {
                    changed.push((sheet.clone(), rule.clone()));
                    false
                }
                _ => true,
            });
        }
        let guard = self.tree.guard.read();
        for (sheet, rule) in &changed {
            self.stylist.rule_changed(
                sheet,
                rule,
                &guard,
                style::invalidation::stylesheets::RuleChangeKind::Insertion,
                &[],
            );
        }
    }
}

/// Which entries of `seq` belong to one longest strictly increasing
/// subsequence of it: what stays in place when `seq` (old positions, in the
/// new order) is reordered with the fewest moves.
fn longest_increasing_run(seq: &[usize]) -> Vec<bool> {
    // `tails[k]`: the index of the smallest last entry of an increasing run
    // of length k + 1; `prev[i]`: the entry before `i` in its run.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; seq.len()];
    for i in 0..seq.len() {
        let k = tails.partition_point(|&t| seq[t] < seq[i]);
        prev[i] = k.checked_sub(1).map(|k| tails[k]);
        if k == tails.len() {
            tails.push(i);
        } else {
            tails[k] = i;
        }
    }
    let mut stays = vec![false; seq.len()];
    let mut at = tails.last().copied();
    while let Some(i) = at {
        stays[i] = true;
        at = prev[i];
    }
    stays
}

#[cfg(test)]
mod tests {
    use super::longest_increasing_run;

    #[test]
    fn longest_increasing_run_keeps_the_most_in_order() {
        assert_eq!(longest_increasing_run(&[]), Vec::<bool>::new());
        assert_eq!(longest_increasing_run(&[0, 1, 2]), vec![true; 3]);
        // One entry moved to the front: only it leaves the run.
        assert_eq!(longest_increasing_run(&[2, 0, 1]), vec![false, true, true]);
        // Moved to the back.
        assert_eq!(longest_increasing_run(&[1, 2, 0]), vec![true, true, false]);
        assert_eq!(
            longest_increasing_run(&[1, 0])
                .iter()
                .filter(|s| **s)
                .count(),
            1
        );
        let run = longest_increasing_run(&[3, 0, 4, 1, 2, 5]);
        assert_eq!(run, vec![false, true, false, true, true, true]);
    }
}
