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
    // (node, leaving it)
    let mut stack = vec![(tree.root_id, false)];
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
        for &c in n.children.iter().rev() {
            // A root's own text is written with the root; its children are
            // walked for the boxes and pictures inside it.
            let child_is_text = tree
                .get(c)
                .is_some_and(|c| c.tag().is_none() && c.run_box.is_none());
            if !child_is_text {
                stack.push((c, false));
            }
        }
    }
    out.text
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
        if first_cell(tree, id) || !out.emitted || out.ends_in_newline() {
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
    let ws = n.computed_style.white_space;
    let breaks: Vec<(usize, usize)> = layout
        .text_ranges
        .iter()
        .filter(|r| r.is_br)
        .map(|r| (r.flat_start, r.flat_end))
        .collect();
    let drops = |i: usize, c: char| -> bool {
        if breaks.iter().any(|&(a, b)| a <= i && i < b) {
            return false;
        }
        match ws {
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
