//! `replaceRange`'s choice of what a paste replaces: the candidate ranges and
//! how far each opens the slice.

use super::*;

/// The depths at which `from..to` covers the whole content of the node both
/// ends are in (so the node itself can be replaced), innermost first. Port of
/// `coveredDepths`.
fn covered_depths(from: &ResolvedPos, to: &ResolvedPos) -> Vec<usize> {
    let mut result = Vec::new();
    for d in (0..=from.depth().min(to.depth())).rev() {
        let start = from.start(d);
        if start + (from.depth() - d) < from.pos().0
            || to.end(d) > to.pos().0 + (to.depth() - d)
            || from.node(d).node_type().is_isolating()
            || to.node(d).node_type().is_isolating()
        {
            break;
        }
        if start == to.start(d)
            || (d == from.depth()
                && d == to.depth()
                && from.parent().is_textblock()
                && to.parent().is_textblock()
                && d > 0
                && to.start(d - 1) == start - 1)
        {
            result.push(d);
        }
    }
    result
}

/// Whether a `typ` child may be inserted in `parent` at child `index`.
fn can_insert(parent: &Node, index: usize, typ: &Node) -> bool {
    let children = parent.content().children();
    let index = index.min(children.len());
    let names: Vec<&str> = children[..index]
        .iter()
        .map(Node::type_name)
        .chain(std::iter::once(typ.type_name()))
        .chain(children[index..].iter().map(Node::type_name))
        .collect();
    parent.node_type().content_match().matches(&names)
        && typ
            .marks()
            .iter()
            .all(|m| parent.node_type().spec().marks.allows(m.type_name()))
}

/// The candidate replacements for `slice` over `from..to`, best first. Port of
/// ProseMirror's `replaceRange`; see the module doc. Each candidate is
/// `(from, to, slice)` for [`fit_step`]; the caller takes the first whose step
/// applies. (ProseMirror commits to its first candidate; here a candidate
/// that does not fit — one that would need a node created — gives way to the
/// next.)
pub(crate) fn range_candidates(
    doc: &Node,
    from: usize,
    to: usize,
    slice: &Slice,
) -> Vec<(usize, usize, Slice)> {
    let mut out = Vec::new();
    let (Ok(r_from), Ok(r_to)) = (doc.resolve(Pos(from)), doc.resolve(Pos(to))) else {
        return out;
    };
    if fits_trivially(&r_from, &r_to, slice) {
        out.push((from, to, slice.clone()));
        return out;
    }

    // Positive: replace the whole node at that depth. Negative `-d`: replace
    // from before the node at depth `d` (which `from` is at the start of) to
    // `to`.
    let mut targets: Vec<isize> = covered_depths(&r_from, &r_to)
        .into_iter()
        .map(|d| d as isize)
        .collect();
    if targets.last() == Some(&0) {
        targets.pop();
    }
    let mut preferred_target = -(r_from.depth() as isize + 1);
    targets.insert(0, preferred_target);
    let mut pos = r_from.pos().0;
    for d in (1..=r_from.depth()).rev() {
        // `pos` is where `from` would be were it at the very start of the
        // node at depth `d`, minus one: the position before that node.
        pos = pos.wrapping_sub(1);
        let spec = r_from.node(d).node_type().spec();
        if spec.defining || spec.isolating {
            break;
        }
        if targets.contains(&(d as isize)) {
            preferred_target = d as isize;
        } else if r_from.before(d) == Some(pos) {
            targets.insert(1, -(d as isize));
        }
    }
    let preferred_index = targets
        .iter()
        .position(|t| *t == preferred_target)
        .unwrap_or(0);

    // The slice's first node at each open depth.
    let mut left: Vec<Option<Node>> = Vec::new();
    let mut content = slice.content.clone();
    for i in 0..=slice.open_start {
        let node = content.maybe_child(0).cloned();
        if i < slice.open_start
            && let Some(n) = &node
        {
            content = n.content().clone();
        }
        left.push(node);
    }
    // Back the preferred depth up over defining nodes directly above it, so
    // that the defining node comes along; a non-defining textblock is skipped.
    let mut preferred_depth = slice.open_start;
    let context = r_from.node(
        (preferred_target.unsigned_abs())
            .saturating_sub(1)
            .min(r_from.depth()),
    );
    for d in (0..slice.open_start).rev() {
        let Some(node) = &left[d] else { break };
        let defines = node.node_type().spec().defining;
        if defines && !same_markup_but_data(node, context) {
            preferred_depth = d;
        } else if defines || !node.is_textblock() {
            break;
        }
    }

    let levels = slice.open_start + 1;
    for j in (0..levels).rev() {
        let open_depth = (j + preferred_depth + 1) % levels;
        let Some(insert) = &left[open_depth] else {
            continue;
        };
        for i in 0..targets.len() {
            let target = targets[(i + preferred_index) % targets.len()];
            let (depth, expand) = (target.unsigned_abs(), target > 0);
            let Some(parent_depth) = depth.checked_sub(1) else {
                continue;
            };
            if parent_depth > r_from.depth() {
                continue;
            }
            let parent = r_from.node(parent_depth);
            if !can_insert(parent, r_from.index(parent_depth), insert) {
                continue;
            }
            let start = if depth > r_from.depth() {
                Some(from)
            } else {
                r_from.before(depth)
            };
            let end = if expand { r_to.after(depth) } else { Some(to) };
            // The slice's start is closed from `open_depth` down.
            let closes = left[open_depth..slice.open_start]
                .iter()
                .all(|n| n.as_ref().is_some_and(content_valid));
            if let (Some(start), Some(end), true) = (start, end, closes) {
                out.push((
                    start,
                    end,
                    Slice::new(slice.content.clone(), open_depth, slice.open_end),
                ));
            }
        }
    }

    // Nothing takes the slice's start as it is: fit it at the range, then at
    // each covered node, outermost first.
    let (mut a, mut b) = (from, to);
    out.push((a, b, slice.clone()));
    for target in targets.iter().rev() {
        if *target < 0 {
            continue;
        }
        let d = *target as usize;
        if let (Some(before), Some(after)) = (r_from.before(d), r_to.after(d)) {
            (a, b) = (before, after);
            out.push((a, b, slice.clone()));
        }
    }
    out
}
