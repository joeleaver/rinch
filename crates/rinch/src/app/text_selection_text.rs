//! What a read-only selection holds, as Chrome has it: the selectable
//! roots of a document in order ([`RootsCache`]), the parts of each a
//! selection keeps ([`kept_ranges`]: not hidden text, not `user-select: none`
//! text, not a list marker), and the text copied between two roots
//! ([`separator`], measured against Chrome 156's `getSelection().toString()`).

use std::collections::HashMap;

use rinch_dom::computed_style::{UserSelectValue, VisibilityValue};

use super::text_selection::selectable_roots;

/// The document's selectable IFC roots in order, with their painted boxes,
/// for one generation of the hit cache (a layout or a scroll moves it): a
/// drag reads them on every move.
pub(crate) struct RootsCache {
    pub(crate) generation: u64,
    pub(crate) roots: Vec<u64>,
    pub(crate) index: HashMap<u64, usize>,
    pub(crate) boxes: Vec<(f32, f32, f32, f32)>,
}

impl RootsCache {
    pub(crate) fn build(tree: &rinch_dom::NodeTree) -> Self {
        let roots = selectable_roots(tree, tree.root_id);
        let index = roots.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        let boxes = roots
            .iter()
            .map(|&r| super::hit_testing::painted_element_box(tree, r))
            .collect();
        Self {
            generation: tree.hit_cache.generation(),
            roots,
            index,
            boxes,
        }
    }

    pub(crate) fn current(&self, tree: &rinch_dom::NodeTree) -> bool {
        self.generation == tree.hit_cache.generation()
    }

    /// The root nearest the window point `(x, y)`: nearest in `y` first (a
    /// root level with it is nearest), then in `x`. As a browser extends a
    /// selection into the space beside, between or below text.
    pub(crate) fn nearest(&self, x: f32, y: f32) -> Option<u64> {
        let gap = |lo: f32, len: f32, v: f32| (lo - v).max(v - (lo + len)).max(0.0);
        self.roots
            .iter()
            .zip(&self.boxes)
            .map(|(&r, &(bx, by, bw, bh))| (gap(by, bh, y), gap(bx, bw, x), r))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|(.., r)| r)
    }
}

/// Whether the text of `text_node` (inside IFC root `root`) is left out of
/// a selection: generated content, drawn hidden (`visibility` inherits, so
/// the text's own element says; a visible child of a hidden span is kept),
/// or `user-select: none` nearer than anything that says otherwise.
fn left_out(tree: &rinch_dom::NodeTree, root: u64, text_node: u64) -> bool {
    let parent = tree.get(text_node).and_then(|n| n.parent);
    if let Some(p) = parent.and_then(|p| tree.get(p))
        && p.computed_style.visibility != VisibilityValue::Visible
    {
        return true;
    }
    let mut id = parent;
    while let Some(i) = id {
        let Some(n) = tree.get(i) else {
            return false;
        };
        // Generated content (a list marker, `::before`) is no document text.
        if n.is_pseudo_element {
            return true;
        }
        match n.computed_style.user_select {
            UserSelectValue::Auto => {}
            UserSelectValue::None => return i != root,
            _ => return false,
        }
        if i == root {
            return false;
        }
        id = n.parent;
    }
    false
}

/// The byte ranges of `root`'s text in `start..end` that a selection keeps:
/// its text nodes' text, but not text that is hidden or `user-select: none`
/// (nor a list marker, which is no text node's). Adjacent ranges merge.
pub(crate) fn kept_ranges(
    tree: &rinch_dom::NodeTree,
    root: u64,
    start: usize,
    end: usize,
) -> Vec<(usize, usize)> {
    let Some(layout) = tree.get(root).and_then(|n| n.text_layout.as_ref()) else {
        return Vec::new();
    };
    let (start, end) = (
        start.min(end),
        end.max(start).min(layout.text_content.len()),
    );
    if start >= end {
        return Vec::new();
    }
    if layout.text_ranges.is_empty() {
        return vec![(start, end)];
    }
    let mut out: Vec<(usize, usize)> = Vec::new();
    for r in &layout.text_ranges {
        let (a, b) = (r.flat_start.max(start), r.flat_end.min(end));
        if a >= b || left_out(tree, root, r.node_id) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.1 == a => last.1 = b,
            _ => out.push((a, b)),
        }
    }
    out
}

/// The ancestors-or-self of `id`, nearest first.
fn chain(tree: &rinch_dom::NodeTree, id: u64) -> Vec<u64> {
    let mut out = vec![id];
    let mut cur = id;
    while let Some(p) = tree.get(cur).and_then(|n| n.parent) {
        out.push(p);
        cur = p;
    }
    out
}

fn tag(tree: &rinch_dom::NodeTree, id: u64) -> Option<&str> {
    tree.get(id).and_then(|n| n.tag())
}

/// The real element a root stands for: itself, or the element an anonymous
/// box's run belongs to.
fn element_of(tree: &rinch_dom::NodeTree, root: u64) -> u64 {
    match tree.get(root) {
        Some(n) if n.is_anonymous_block_box => n.parent.unwrap_or(root),
        _ => root,
    }
}

/// What a copy puts between the text of root `a` and the root after it, `b`:
/// a tab between two cells of a row; else a line break, and one more for
/// each `<p>` the selection leaves on the way (the ones `a` is in, and any it
/// passes whole), as Chrome's `getSelection().toString()` has it.
pub(crate) fn separator(tree: &rinch_dom::NodeTree, a: u64, b: u64) -> String {
    let (ea, eb) = (element_of(tree, a), element_of(tree, b));
    let above_a = chain(tree, ea);
    let above_b = chain(tree, eb);
    let b_set: std::collections::HashSet<u64> = above_b.iter().copied().collect();
    let lca = above_a.iter().copied().find(|id| b_set.contains(id));
    let exited: Vec<u64> = above_a
        .iter()
        .copied()
        .take_while(|&id| Some(id) != lca)
        .collect();
    let entered: Vec<u64> = above_b
        .iter()
        .copied()
        .take_while(|&id| Some(id) != lca)
        .collect();
    let is_cell = |id: &u64| matches!(tag(tree, *id), Some("td" | "th"));
    let is_row = |id: &u64| tag(tree, *id) == Some("tr");
    if exited.iter().any(is_cell)
        && entered.iter().any(is_cell)
        && !exited.iter().any(is_row)
        && !entered.iter().any(is_row)
    {
        return "\t".into();
    }
    // (An anonymous box stands for its parent element, which is not left
    // when the next root is in it: then it is the common ancestor.)
    let breaks = 1
        + exited
            .iter()
            .filter(|&&id| tag(tree, id) == Some("p"))
            .count()
        + paragraphs_passed(tree, ea, eb, &b_set);
    "\n".repeat(breaks)
}

/// The `<p>` elements wholly between `a` and `b` in document order (a
/// selection passing an unselectable paragraph still leaves it).
fn paragraphs_passed(
    tree: &rinch_dom::NodeTree,
    a: u64,
    b: u64,
    above_b: &std::collections::HashSet<u64>,
) -> usize {
    // The node after `id`'s subtree in document order.
    let after = |mut id: u64| -> Option<u64> {
        loop {
            let parent = tree.get(id)?.parent?;
            let siblings = &tree.get(parent)?.children;
            let at = siblings.iter().position(|&c| c == id)?;
            if let Some(&next) = siblings.get(at + 1) {
                return Some(next);
            }
            id = parent;
        }
    };
    let mut count = 0;
    let mut cur = after(a);
    let mut steps = 0;
    while let Some(id) = cur {
        steps += 1;
        if id == b || steps > 10_000 {
            break;
        }
        if above_b.contains(&id) {
            cur = tree
                .get(id)
                .and_then(|n| n.children.first().copied())
                .or_else(|| after(id));
            continue;
        }
        let shown = tree.get(id).is_some_and(|n| {
            n.computed_style.display != rinch_dom::computed_style::DisplayValue::None
        });
        if shown && tag(tree, id) == Some("p") {
            count += 1;
        }
        cur = after(id);
    }
    count
}
