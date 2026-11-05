//! The text a read-only selection copies, as Chrome 156's
//! `getSelection().toString()` gives it: Blink's text iterator, walking the
//! document in order from the selection's first root to its last.
//!
//! - Text is each root's selected part ([`rendered_ranges`]: its kept text,
//!   without the white space a line drops at its start and end); a `<br>` is
//!   its line break.
//! - Entering a block (a block container, a table or a row; not a cell, an
//!   inline-level box, a float, an absolutely positioned box or a picture)
//!   writes a line break, and leaving one writes one, unless the last thing
//!   written is a line break. Leaving a `<p>` with height writes one more.
//! - Entering a table cell other than its row's first writes a tab, unless
//!   the last thing written is a line break.
//! - An `<img>` or `<iframe>` is written as nothing, but is something
//!   written: the block around it then ends with a line break.
//! - Nothing is written before the first text, and what is not displayed is
//!   passed whole.

use std::collections::{HashMap, HashSet};

use rinch_dom::computed_style::WhiteSpaceValue;
use rinch_dom::computed_style::{DisplayValue, PositionValue, TablePart, VisibilityValue};

use super::text_selection_text::kept_ranges;

/// What the walk has written.
struct Out {
    text: String,
    /// Anything written, a picture included (Blink's `HasEmitted`).
    emitted: bool,
    /// The last thing written was a picture, so no line break.
    after_picture: bool,
}

impl Out {
    fn ends_in_newline(&self) -> bool {
        !self.after_picture && self.text.ends_with('\n')
    }

    fn push(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.text.push_str(s);
        self.emitted = true;
        self.after_picture = false;
    }

    /// A line break, unless there is nothing yet or one was just written.
    fn line_break(&mut self) {
        if self.emitted && !self.ends_in_newline() {
            self.push("\n");
        }
    }
}

/// The copy of `runs` (each selected root with its part, in document order,
/// as `TextSelection::runs` gives them).
pub(crate) fn copy_text(tree: &rinch_dom::NodeTree, runs: &[(u64, usize, usize)]) -> String {
    let Some(&(last_root, ..)) = runs.last() else {
        return String::new();
    };
    let parts: HashMap<u64, (usize, usize)> = runs.iter().map(|&(r, a, b)| (r, (a, b))).collect();
    let mut out = Out {
        text: String::new(),
        emitted: false,
        after_picture: false,
    };
    let mut active = false;
    let mut seen_boxes = HashSet::new();
    let mut stack = walk_from(tree, runs[0].0);
    while let Some((id, leaving)) = stack.pop() {
        let Some(n) = tree.get(id) else {
            continue;
        };
        if leaving {
            if active {
                leave(tree, id, &mut out);
            }
            continue;
        }
        let element = n.tag().is_some();
        if element && n.computed_style.display == DisplayValue::None {
            continue;
        }
        // Loose text beside blocks: its anonymous box stands in at its
        // first member, as `selectable_roots` orders them.
        let root = match n.run_box {
            Some(b) if !seen_boxes.insert(b) => continue,
            Some(b) => Some(b),
            None => n.text_layout.is_some().then_some(id),
        };
        if active && element {
            enter(tree, id, &mut out);
        }
        if let Some(r) = root
            && let Some(&(start, end)) = parts.get(&r)
        {
            active = true;
            for (a, b) in rendered_ranges(tree, r, start, end) {
                if let Some(t) = tree.get(r).and_then(|n| n.text_layout.as_ref()) {
                    out.push(t.text_content.get(a..b).unwrap_or(""));
                }
            }
            if r == last_root {
                break;
            }
        }
        if n.run_box.is_some() {
            continue;
        }
        if active && element && is_picture(n) {
            if n.computed_style.visibility == VisibilityValue::Visible {
                out.emitted = true;
                out.after_picture = true;
            }
            continue;
        }
        stack.push((id, true));
        push_children(tree, &n.children, &mut stack);
    }
    out.text
}

/// Push `children` to walk next, in order. A root's own text is written
/// with the root; its children are walked for the boxes and pictures inside
/// it, so its text nodes are left out.
fn push_children(tree: &rinch_dom::NodeTree, children: &[u64], stack: &mut Vec<(u64, bool)>) {
    for &c in children.iter().rev() {
        let text = tree
            .get(c)
            .is_some_and(|c| c.tag().is_none() && c.run_box.is_none());
        if !text {
            stack.push((c, false));
        }
    }
}

/// The walk's stack (node, leaving it) as it stands on reaching `first`, the
/// selection's first root: the node there next, then the rest of each of
/// its ancestors after it and the ancestor itself to leave, innermost first.
/// Nothing before it in the document is walked (it could write nothing:
/// nothing is written before the first text).
fn walk_from(tree: &rinch_dom::NodeTree, first: u64) -> Vec<(u64, bool)> {
    // An anonymous box is met at its first member.
    let start = match tree.get(first) {
        Some(n) if n.is_anonymous_block_box => n
            .parent
            .and_then(|p| tree.get(p))
            .and_then(|p| {
                p.children
                    .iter()
                    .copied()
                    .find(|&c| tree.get(c).is_some_and(|c| c.run_box == Some(first)))
            })
            .unwrap_or(first),
        _ => first,
    };
    let mut path = vec![start];
    while let Some(p) = path
        .last()
        .and_then(|&c| tree.get(c))
        .and_then(|n| n.parent)
    {
        path.push(p);
    }
    let mut stack = Vec::new();
    for pair in path.windows(2).rev() {
        let (child, parent) = (pair[0], pair[1]);
        let Some(p) = tree.get(parent) else {
            continue;
        };
        stack.push((parent, true));
        let at = p
            .children
            .iter()
            .position(|&c| c == child)
            .map_or(0, |i| i + 1);
        push_children(tree, &p.children[at..], &mut stack);
    }
    stack.push((start, false));
    stack
}

/// An `<img>` or `<iframe>`: the pictures Blink's iterator writes as a
/// replaced element (not `<svg>`, `<canvas>` or `<video>`).
fn is_picture(n: &rinch_dom::Node) -> bool {
    matches!(n.tag(), Some("img" | "iframe"))
}

/// Whether Blink writes line breaks before and after element `n`.
fn breaks_around(n: &rinch_dom::Node) -> bool {
    let s = &n.computed_style;
    if s.table_part == TablePart::Row {
        return true;
    }
    if s.table_part == TablePart::Cell
        || is_picture(n)
        || n.tag() == Some("body")
        || s.float != Default::default()
        || matches!(s.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return false;
    }
    matches!(
        s.display,
        DisplayValue::Block | DisplayValue::ListItem | DisplayValue::Flex | DisplayValue::Grid
    )
}

fn enter(tree: &rinch_dom::NodeTree, id: u64, out: &mut Out) {
    let Some(n) = tree.get(id) else {
        return;
    };
    if n.computed_style.table_part == TablePart::Cell {
        // Only a cell that can be selected has its tab.
        let unselectable = super::text_selection::resolved_user_select(tree, id)
            == rinch_dom::computed_style::UserSelectValue::None;
        if unselectable || first_cell(tree, id) || !out.emitted || out.ends_in_newline() {
            return;
        }
        out.push("\t");
    } else if breaks_around(n) {
        out.line_break();
    }
}

fn leave(tree: &rinch_dom::NodeTree, id: u64, out: &mut Out) {
    let Some(n) = tree.get(id) else {
        return;
    };
    if !breaks_around(n) || !out.emitted {
        return;
    }
    let paragraph = n.tag() == Some("p") && n.layout.height > 0.0;
    if !out.ends_in_newline() {
        out.push("\n");
        if paragraph {
            out.push("\n");
        }
    } else if paragraph {
        out.push("\n");
    }
}

/// Whether table cell `id` is the first displayed cell of its row.
fn first_cell(tree: &rinch_dom::NodeTree, id: u64) -> bool {
    let Some(parent) = tree
        .get(id)
        .and_then(|n| n.parent)
        .and_then(|p| tree.get(p))
    else {
        return true;
    };
    !parent.children.iter().take_while(|&&c| c != id).any(|&c| {
        tree.get(c).is_some_and(|c| {
            c.computed_style.table_part == TablePart::Cell
                && c.computed_style.display != DisplayValue::None
        })
    })
}

/// The ranges of `root`'s text in `start..end` a copy writes: its kept text
/// ([`kept_ranges`]) without the white space a line drops at the root's start
/// and end (a `<br>`'s line break, and a `pre-line` one, stay).
pub(crate) fn rendered_ranges(
    tree: &rinch_dom::NodeTree,
    root: u64,
    start: usize,
    end: usize,
) -> Vec<(usize, usize)> {
    let Some((lo, hi)) = rendered_span(tree, root) else {
        return Vec::new();
    };
    kept_ranges(tree, root, start.max(lo), end.min(hi))
}

/// The part of `root`'s text that is drawn: from its first character that
/// is not white space its lines drop to just after its last. `None` when it
/// has none.
pub(crate) fn rendered_span(tree: &rinch_dom::NodeTree, root: u64) -> Option<(usize, usize)> {
    let n = tree.get(root)?;
    let layout = n.text_layout.as_ref()?;
    let text = &layout.text_content;
    // Each text's own `white-space` (its element's), and whether it is a
    // `<br>`'s line break.
    let ranges: Vec<(usize, usize, bool, WhiteSpaceValue)> = layout
        .text_ranges
        .iter()
        .map(|r| {
            let ws = tree
                .get(r.node_id)
                .and_then(|t| t.parent)
                .and_then(|p| tree.get(p))
                .map_or(n.computed_style.white_space, |e| {
                    e.computed_style.white_space
                });
            (r.flat_start, r.flat_end, r.is_br, ws)
        })
        .collect();
    let drops = |i: usize, c: char| -> bool {
        let at = ranges.iter().find(|&&(a, b, ..)| a <= i && i < b);
        if at.is_some_and(|r| r.2) {
            return false;
        }
        match at.map_or(n.computed_style.white_space, |r| r.3) {
            WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap | WhiteSpaceValue::BreakSpaces => false,
            WhiteSpaceValue::PreLine => matches!(c, ' ' | '\t'),
            WhiteSpaceValue::Normal | WhiteSpaceValue::NoWrap => {
                matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
            }
        }
    };
    let first = text.char_indices().find(|&(i, c)| !drops(i, c))?.0;
    let (last, c) = text.char_indices().rev().find(|&(i, c)| !drops(i, c))?;
    Some((first, last + c.len_utf8()))
}
