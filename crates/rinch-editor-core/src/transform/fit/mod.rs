//! Fitting a [`Slice`] into a document range it does not fit as it is.
//!
//! [`ReplaceStep`] is strict: the slice's open depths must line up with the
//! range's, and every node it rebuilds must be valid. Content that comes from
//! somewhere else — a paste — rarely lines up: a list has no place in the
//! paragraph the caret is in. This module finds the step that puts as much of
//! the slice as possible into the range **with its structure**, closing or
//! splitting the nodes around the range where the content needs it.
//!
//! It is ProseMirror's algorithm, in two layers (`prosemirror-transform`,
//! `replace.ts`):
//!
//! - [`fit_step`] is the `Fitter`. It walks the slice from its start and
//!   places each piece in the deepest open node around the range's start that
//!   takes it — the *frontier* — closing frontier nodes that do not. Content
//!   that is open at the slice's start therefore continues the textblock the
//!   range starts in; a closed block closes that textblock and goes in beside
//!   it. At the end it joins the frontier back to what follows the range, and
//!   when the placed content ends inside a textblock, the inline content after
//!   the range moves into that textblock.
//! - `replace_range` is `replaceRange`: before fitting, it looks at whether
//!   the range covers whole nodes (a caret in an empty textblock does) and
//!   whether the slice starts in a [defining](crate::schema::NodeSpec::defining)
//!   node. If so, the defining node is kept and replaces the covered node,
//!   rather than its content being poured into it: a list pasted on an empty
//!   line is a list, a heading pasted on an empty line is a heading.
//!
//! Two things ProseMirror does are not done here, because the content
//! matcher (`ContentMatch`) has no way to ask
//! for them: nodes are never *created* to make content valid (`fillBefore`),
//! and content is never *wrapped* in a node it was not in (`findWrapping`).
//! A fit that would need either fails. And one thing is added: the fit never
//! leaves an [isolating](crate::schema::NodeSpec::isolating) node (a table
//! cell) the range starts in.
//!
//! Nothing here can produce an invalid document: the result is a step, and
//! applying it validates every node it rebuilds
//! ([`replace::close`](super::replace)).

use crate::model::{Fragment, Mark, Node, NodeType, Slice};
use crate::pos::{Pos, ResolvedPos};
use crate::schema::content_match::MatchState;
use crate::schema::data_attrs::node_app_data;
use crate::transform::step::Step;
use crate::transform::steps::{ReplaceAroundStep, ReplaceStep};

/// A fitted replacement: the step, and the position right after the content
/// it inserted (where a caret goes after a paste), in the document the step
/// produces.
pub(crate) struct Fit {
    pub step: Box<dyn Step>,
    pub end: usize,
}

thread_local! {
    /// Content-match steps taken by fits on this thread: the fitter's unit of
    /// work, counted so that a test can hold its growth to the size of the
    /// content without reading a clock.
    static MATCH_STEPS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The number of content-match steps fits have taken on this thread. For
/// tests: a fit's cost is linear in what it places, and this is what says so.
#[doc(hidden)]
pub fn match_steps() -> u64 {
    MATCH_STEPS.with(std::cell::Cell::get)
}

/// The marks of `marks` that `typ` allows on its content.
fn allowed_marks(typ: &NodeType, marks: &[Mark]) -> Vec<Mark> {
    marks
        .iter()
        .filter(|m| typ.spec().marks.allows(m.type_name()))
        .cloned()
        .collect()
}

/// Whether `node`'s content is a whole valid content for its type.
fn content_valid(node: &Node) -> bool {
    let names: Vec<&str> = node
        .content()
        .children()
        .iter()
        .map(Node::type_name)
        .collect();
    node.node_type().content_match().matches(&names)
}

/// Whether `node` and its first `levels - 1` first-descendants are valid as
/// they stand: what closing a node that was open at its start requires.
/// (A node cut at its very end is open and empty, and an empty list item is
/// not a list item. ProseMirror fills such a node; here it is not closed.)
fn start_closes(node: &Node, levels: usize) -> bool {
    let mut node = Some(node);
    for _ in 0..levels {
        let Some(n) = node else { break };
        if !content_valid(n) {
            return false;
        }
        node = n.content().children().first();
    }
    true
}

/// Whether every node of `slice` is valid, the nodes open at its edges
/// aside (their content is a part of a valid content, which the fit checks
/// where it closes or joins them).
pub(crate) fn slice_is_sound(slice: &Slice) -> bool {
    fn walk(nodes: &[Node], open_start: usize, open_end: usize) -> bool {
        let last = nodes.len().saturating_sub(1);
        nodes.iter().enumerate().all(|(i, node)| {
            let (s, e) = (
                if i == 0 { open_start } else { 0 },
                if i == last { open_end } else { 0 },
            );
            (s > 0 || e > 0 || content_valid(node))
                && walk(
                    node.content().children(),
                    s.saturating_sub(1),
                    e.saturating_sub(1),
                )
        })
    }
    walk(slice.content.children(), slice.open_start, slice.open_end)
}

/// The content `depth` first-children down.
fn content_at(fragment: &Fragment, depth: usize) -> Fragment {
    let mut fragment = fragment.clone();
    for _ in 0..depth {
        fragment = fragment.child(0).content().clone();
    }
    fragment
}

/// `fragment` without the first `count` children of the node `depth`
/// first-children down.
fn drop_from_fragment(fragment: &Fragment, depth: usize, count: usize) -> Fragment {
    if depth == 0 {
        return Fragment::from_children(fragment.children()[count..].to_vec());
    }
    let first = fragment.child(0);
    fragment.replace_child(
        0,
        first.copy_with_content(drop_from_fragment(first.content(), depth - 1, count)),
    )
}

/// `fragment` with `content` appended to the node `depth` last-children down.
fn add_to_fragment(fragment: &Fragment, depth: usize, content: &Fragment) -> Fragment {
    if depth == 0 {
        return fragment.append(content);
    }
    let last = fragment.child_count() - 1;
    let node = fragment.child(last);
    fragment.replace_child(
        last,
        node.copy_with_content(add_to_fragment(node.content(), depth - 1, content)),
    )
}

/// `slice` with the code block open at its start turned into a line of text
/// for each of its lines, when `from` is in a textblock that is not code and
/// `from..to` is not exactly that textblock's whole content. `None` when it
/// is not, or when the lines are not valid where the block was.
///
/// Code text keeps its line ends in the text; a paragraph that took that text
/// as it is would hold them too, as one line that only looks like several,
/// and a line whose own text was taken into the code block (the fit at a
/// line's start) would be code. So the first line continues the textblock at
/// `from`, each further line is a textblock of its own (a `paragraph`, or
/// the type of the one at `from` in a schema with no paragraph), and what
/// follows the range joins the last. Decided once for the range, before any
/// candidate: a range that is exactly the whole content of one textblock (a
/// caret in an empty one) takes the code block as a block, and so does a
/// slice that is closed at its start; a target that is code takes the text.
/// A range that starts at a textblock's start and ends in a later one is
/// unfolded like any other (what it keeps of the later textblock joins the
/// last line), and a block of one line is unfolded too: its text continues
/// the textblock at `from`.
pub(crate) fn code_lines_as_blocks(
    schema: &crate::schema::Schema,
    doc: &Node,
    from: usize,
    to: usize,
    slice: &Slice,
) -> Option<Slice> {
    if slice.open_start == 0 {
        return None;
    }
    let r = doc.resolve(Pos(from)).ok()?;
    let target = r.parent();
    if !target.is_textblock() || target.node_type().spec().code {
        return None;
    }
    if r.parent_offset() == 0 && to == from + target.content_size() {
        return None;
    }
    let line = schema
        .node_type("paragraph")
        .filter(|t| t.is_textblock() && !t.spec().code)
        .map_or(target.type_name(), NodeType::name);
    let content = unfold_first(schema, &slice.content, slice.open_start, line)?;
    Some(Slice::new(content, slice.open_start, slice.open_end))
}

/// [`code_lines_as_blocks`] for the node `levels` deep along the first
/// children of `content`.
fn unfold_first(
    schema: &crate::schema::Schema,
    content: &Fragment,
    levels: usize,
    line: &str,
) -> Option<Fragment> {
    let (first, rest) = content.children().split_first()?;
    let mut nodes = Vec::new();
    if levels > 1 {
        let inner = unfold_first(schema, first.content(), levels - 1, line)?;
        let node = first.copy_with_content(inner);
        if !content_valid(&node) {
            return None;
        }
        nodes.push(node);
    } else {
        if !first.is_textblock() || !first.node_type().spec().code {
            return None;
        }
        let mut text = String::new();
        for child in first.content().children() {
            // A code block holds text; anything else is not unfolded.
            text.push_str(child.text()?);
        }
        for l in text.split('\n') {
            let l = l.strip_suffix('\r').unwrap_or(l);
            let inline = if l.is_empty() {
                Fragment::empty()
            } else {
                Fragment::from_node(schema.text(l).ok()?)
            };
            nodes.push(schema.branch(line, inline).ok()?);
        }
    }
    nodes.extend(rest.iter().cloned());
    Some(Fragment::from_children(nodes))
}

/// The step that fits `slice` into `from..to` of `doc`, when there is one.
/// Port of ProseMirror's `replaceStep`.
pub(crate) fn fit_step(doc: &Node, from: usize, to: usize, slice: &Slice) -> Option<Fit> {
    let r_from = doc.resolve(Pos(from)).ok()?;
    let r_to = doc.resolve(Pos(to)).ok()?;
    if fits_trivially(&r_from, &r_to, slice) {
        return Some(Fit {
            end: from + slice.size(),
            step: Box::new(ReplaceStep::new(from, to, slice.clone())),
        });
    }
    Fitter::new(&r_from, &r_to, slice).fit()
}

/// A closed slice into one parent that takes it as it is.
fn fits_trivially(from: &ResolvedPos, to: &ResolvedPos, slice: &Slice) -> bool {
    if slice.open_start != 0
        || slice.open_end != 0
        || from.depth() != to.depth()
        || from.start(from.depth()) != to.start(to.depth())
    {
        return false;
    }
    let parent = from.parent();
    let children = parent.content().children();
    // What stays on each side; a text node the range cuts stays on both.
    let before = &children[..from.index_after(from.depth()).min(children.len())];
    let after = &children[to.index(to.depth()).min(children.len())..];
    let names: Vec<&str> = before
        .iter()
        .chain(slice.content.children())
        .chain(after)
        .map(Node::type_name)
        .collect();
    let marks = &parent.node_type().spec().marks;
    parent.node_type().content_match().matches(&names)
        && slice
            .content
            .children()
            .iter()
            .all(|n| n.marks().iter().all(|m| marks.allows(m.type_name())))
}

mod fitter;
mod range;

use fitter::Fitter;
pub(crate) use range::range_candidates;
