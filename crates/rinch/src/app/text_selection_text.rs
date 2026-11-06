//! What a read-only selection holds, as Chrome has it: the selectable
//! roots of a document in order ([`RootsCache`]), which of them have text
//! ([`has_content`]), and the parts of each a selection keeps
//! ([`kept_ranges`]: not hidden text, not `user-select: none` text, not a
//! list marker). What a copy writes is `text_selection_copy`.

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
    /// Whether each root has something a selection takes ([`has_content`]).
    pub(crate) content: Vec<bool>,
}

impl RootsCache {
    pub(crate) fn build(tree: &rinch_dom::NodeTree) -> Self {
        let roots = selectable_roots(tree, tree.root_id);
        let index = roots.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        let boxes = roots
            .iter()
            .map(|&r| super::hit_testing::painted_element_box(tree, r))
            .collect();
        let content = roots.iter().map(|&r| has_content(tree, r)).collect();
        Self {
            generation: tree.hit_cache.generation(),
            roots,
            index,
            boxes,
            content,
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
            .zip(&self.content)
            .filter(|(_, has)| **has)
            .map(|((&r, &(bx, by, bw, bh)), _)| (gap(by, bh, y), gap(bx, bw, x), r))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|(.., r)| r)
    }
}

/// Whether IFC root `root` has text a selection takes: something drawn,
/// not only the white space its lines drop (a `<br>` and a no-break space
/// are text). Mail is full of roots that hold nothing but the white space
/// between its tables and columns, or only a picture; Chrome passes them, so
/// a selection does too: they are not highlighted, and a drag is not drawn
/// to them.
pub(crate) fn has_content(tree: &rinch_dom::NodeTree, root: u64) -> bool {
    super::text_selection_copy::rendered_span(tree, root).is_some()
}

/// Whether the text of `text_node` is left out of a selection: generated
/// content, drawn hidden (`visibility` inherits, so the text's own element
/// says; a visible child of a hidden span is kept), or not selectable: the
/// nearest `user-select` that is not `auto`, at or above its element, says
/// `none`, or none says anything (rinch reads that as `none`). Read past the
/// text's root: `user-select: text` inside a `user-select: none`
/// inline-block is selectable (review of #2249).
fn left_out(tree: &rinch_dom::NodeTree, text_node: u64) -> bool {
    let parent = tree.get(text_node).and_then(|n| n.parent);
    if let Some(p) = parent.and_then(|p| tree.get(p))
        && p.computed_style.visibility != VisibilityValue::Visible
    {
        return true;
    }
    let mut id = parent;
    while let Some(i) = id {
        let Some(n) = tree.get(i) else {
            return true;
        };
        // Generated content (a list marker, `::before`) is no document text.
        if n.is_pseudo_element {
            return true;
        }
        match n.computed_style.user_select {
            UserSelectValue::Auto => {}
            UserSelectValue::None => return true,
            _ => return false,
        }
        id = n.parent;
    }
    true
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
        super::text_selection_copy::work(1);
        let (a, b) = (r.flat_start.max(start), r.flat_end.min(end));
        if a >= b || left_out(tree, r.node_id) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.1 == a => last.1 = b,
            _ => out.push((a, b)),
        }
    }
    out
}
