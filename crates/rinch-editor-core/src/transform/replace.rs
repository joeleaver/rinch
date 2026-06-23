//! The replace algorithm — the engine underneath `ReplaceStep`.
//!
//! Replacing the document range `from..to` with a [`Slice`] is the single
//! operation that expresses text insertion, deletion, splitting, joining, and
//! paste. The algorithm is delicate because the cut endpoints and the slice's
//! open boundaries can sit at different depths: the content on the left of `from`
//! must be re-joined to the slice's open-start, the slice's open-end re-joined to
//! the content on the right of `to`, and any ancestors that the cut passes
//! through reconstructed. This is a faithful port of ProseMirror's
//! `transform/src/replace.ts`; **schema enforcement** (decision G) lives in
//! [`close`], which rejects any reconstructed node whose children violate its
//! content expression — so an invalid step never produces a document.

use crate::Slice;
use crate::model::{Fragment, Node};
use crate::pos::{Pos, ResolvedPos};
use crate::transform::step::StepError;

/// Resolve `from`/`to` against `doc` and replace that range with `slice`
/// (ProseMirror's `Node.replace` / `StepResult.fromReplace`).
pub(crate) fn replace_doc(
    doc: &Node,
    from: usize,
    to: usize,
    slice: &Slice,
) -> Result<Node, StepError> {
    let r_from = doc
        .resolve(Pos(from))
        .map_err(|e| StepError::new(e.to_string()))?;
    let r_to = doc
        .resolve(Pos(to))
        .map_err(|e| StepError::new(e.to_string()))?;
    replace(&r_from, &r_to, slice)
}

/// Replace the range between two resolved positions with `slice`.
fn replace(from: &ResolvedPos, to: &ResolvedPos, slice: &Slice) -> Result<Node, StepError> {
    if slice.open_start > from.depth() {
        return Err(StepError::new(
            "inserted content deeper than insertion position",
        ));
    }
    if from.depth() as isize - slice.open_start as isize
        != to.depth() as isize - slice.open_end as isize
    {
        return Err(StepError::new("inconsistent open depths"));
    }
    replace_outer(from, to, slice, 0)
}

fn replace_outer(
    from: &ResolvedPos,
    to: &ResolvedPos,
    slice: &Slice,
    depth: usize,
) -> Result<Node, StepError> {
    let index = from.index(depth);
    let node = from.node(depth).clone();
    if index == to.index(depth) && depth < from.depth() - slice.open_start {
        // The cut stays inside one child here — recurse, then splice the rebuilt
        // child back in (sharing every sibling).
        let inner = replace_outer(from, to, slice, depth + 1)?;
        Ok(node.copy_with_content(node.content().replace_child(index, inner)))
    } else if slice.is_empty() {
        // Pure deletion: stitch the left and right remainders together.
        let frag = replace_two_way(from, to, depth)?;
        close(&node, frag)
    } else if slice.open_start == 0
        && slice.open_end == 0
        && from.depth() == depth
        && to.depth() == depth
    {
        // The flat, common case: insert closed content within a single textblock.
        let parent = from.parent();
        let content = parent.content();
        let merged = content
            .cut(0, from.parent_offset())
            .append(&slice.content)
            .append(&content.cut(to.parent_offset(), content.size()));
        close(parent, merged)
    } else {
        // The general three-way case: left remainder + open slice + right remainder.
        let (start, end) = prepare_slice_for_replace(slice, from)?;
        let frag = replace_three_way(from, &start, &end, to, depth)?;
        close(&node, frag)
    }
}

/// Reconstruct `node`'s content as `content`, **enforcing the schema**: the child
/// sequence must satisfy `node`'s content expression and each child's marks must
/// be allowed. Returns `Err` otherwise — the schema gate for every replace.
fn close(node: &Node, content: Fragment) -> Result<Node, StepError> {
    let names: Vec<&str> = content.children().iter().map(Node::type_name).collect();
    if !node.node_type().content_match().matches(&names) {
        return Err(StepError::new(format!(
            "invalid content for <{}>: {:?}",
            node.type_name(),
            names
        )));
    }
    let mark_set = &node.node_type().spec().marks;
    for child in content.children() {
        for m in child.marks() {
            if !mark_set.allows(m.type_name()) {
                return Err(StepError::new(format!(
                    "mark <{}> not allowed in <{}>",
                    m.type_name(),
                    node.type_name()
                )));
            }
        }
    }
    Ok(node.copy_with_content(content))
}

/// Append a child to `target`, merging into the previous node if both are text
/// with identical markup (so runs never fragment). Port of `addNode`.
fn add_node(child: Node, target: &mut Vec<Node>) {
    let merge = matches!(target.last(), Some(last)
        if child.is_text() && last.is_text() && child.same_markup(last));
    if merge {
        let n = target.len() - 1;
        let merged = format!("{}{}", target[n].text().unwrap(), child.text().unwrap());
        target[n] = target[n].with_text(merged.into());
    } else {
        target.push(child);
    }
}

/// Append the children of the node at `depth` that lie between `start` and `end`
/// (either bound `None` means "this side's edge of the node"), slicing a text
/// node at a mid-text bound. Port of `addRange`.
fn add_range(
    start: Option<&ResolvedPos>,
    end: Option<&ResolvedPos>,
    depth: usize,
    target: &mut Vec<Node>,
) {
    let node = end
        .or(start)
        .expect("addRange needs a bound")
        .node(depth)
        .clone();
    let end_index = match end {
        Some(e) => e.index(depth),
        None => node.child_count(),
    };
    let mut start_index = 0;
    if let Some(s) = start {
        start_index = s.index(depth);
        if s.depth() > depth {
            start_index += 1;
        } else if s.text_offset() > 0 {
            add_node(
                s.node_after().expect("text offset implies a node after"),
                target,
            );
            start_index += 1;
        }
    }
    for i in start_index..end_index {
        add_node(node.child(i).clone(), target);
    }
    if let Some(e) = end
        && e.depth() == depth
        && e.text_offset() > 0
    {
        add_node(
            e.node_before().expect("text offset implies a node before"),
            target,
        );
    }
}

/// Check two nodes can be joined (their content types are compatible). Port of
/// `checkJoin`.
fn check_join(main: &Node, sub: &Node) -> Result<(), StepError> {
    if !sub.node_type().compatible_content(main.node_type()) {
        return Err(StepError::new(format!(
            "cannot join <{}> onto <{}>",
            sub.type_name(),
            main.type_name()
        )));
    }
    Ok(())
}

/// The node at `depth` on the `before` path, validated as joinable with the node
/// at `depth` on the `after` path. Port of `joinable`.
fn joinable(before: &ResolvedPos, after: &ResolvedPos, depth: usize) -> Result<Node, StepError> {
    let node = before.node(depth).clone();
    check_join(&node, after.node(depth))?;
    Ok(node)
}

/// Stitch the content to the left of `from` and to the right of `to` together,
/// recursing through joined ancestors. Port of `replaceTwoWay`.
fn replace_two_way(
    from: &ResolvedPos,
    to: &ResolvedPos,
    depth: usize,
) -> Result<Fragment, StepError> {
    let mut content: Vec<Node> = Vec::new();
    add_range(None, Some(from), depth, &mut content);
    if from.depth() > depth {
        let typ = joinable(from, to, depth + 1)?;
        let inner = replace_two_way(from, to, depth + 1)?;
        add_node(close(&typ, inner)?, &mut content);
    }
    add_range(Some(to), None, depth, &mut content);
    Ok(Fragment::from_children(content))
}

/// The general case: left remainder, then the (open) slice content represented by
/// `start..end`, then the right remainder, joining open boundaries where the
/// depths line up. Port of `replaceThreeWay`.
fn replace_three_way(
    from: &ResolvedPos,
    start: &ResolvedPos,
    end: &ResolvedPos,
    to: &ResolvedPos,
    depth: usize,
) -> Result<Fragment, StepError> {
    let open_start = if from.depth() > depth {
        Some(joinable(from, start, depth + 1)?)
    } else {
        None
    };
    let open_end = if to.depth() > depth {
        Some(joinable(end, to, depth + 1)?)
    } else {
        None
    };

    let mut content: Vec<Node> = Vec::new();
    add_range(None, Some(from), depth, &mut content);
    match (&open_start, &open_end) {
        (Some(os), Some(oe)) if start.index(depth) == end.index(depth) => {
            check_join(os, oe)?;
            let inner = replace_three_way(from, start, end, to, depth + 1)?;
            add_node(close(os, inner)?, &mut content);
        }
        _ => {
            if let Some(os) = &open_start {
                let inner = replace_two_way(from, start, depth + 1)?;
                add_node(close(os, inner)?, &mut content);
            }
            add_range(Some(start), Some(end), depth, &mut content);
            if let Some(oe) = &open_end {
                let inner = replace_two_way(end, to, depth + 1)?;
                add_node(close(oe, inner)?, &mut content);
            }
        }
    }
    add_range(Some(to), None, depth, &mut content);
    Ok(Fragment::from_children(content))
}

/// Wrap the slice's content in copies of `along`'s open ancestors, then resolve
/// the slice's two open edges within that synthetic node — giving the `start`/`end`
/// resolved positions `replace_three_way` consumes. Port of
/// `prepareSliceForReplace`.
fn prepare_slice_for_replace(
    slice: &Slice,
    along: &ResolvedPos,
) -> Result<(ResolvedPos, ResolvedPos), StepError> {
    let extra = along.depth() - slice.open_start;
    let parent = along.node(extra);
    let mut node = parent.copy_with_content(slice.content.clone());
    let mut i = extra as isize - 1;
    while i >= 0 {
        node = along
            .node(i as usize)
            .copy_with_content(Fragment::from_node(node));
        i -= 1;
    }
    let start_pos = slice.open_start + extra;
    let end_pos = node.content_size() - slice.open_end - extra;
    let start = node
        .resolve(Pos(start_pos))
        .map_err(|e| StepError::new(e.to_string()))?;
    let end = node
        .resolve(Pos(end_pos))
        .map_err(|e| StepError::new(e.to_string()))?;
    Ok((start, end))
}
