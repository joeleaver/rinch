//! Keyboard extension of a read-only text selection (desktop, #2953), as
//! Chrome 158 extends one in text that cannot be edited:
//!
//! - Only a selection that selects something: a collapsed one (a click) is
//!   no caret in read-only text, and the keys leave it.
//! - Shift+Left / Shift+Right move the focus a character (a grapheme), Left
//!   backward in a left-to-right root and forward in a right-to-left one
//!   (the root's direction, as Blink's `ModifyExtendingLeft` reads the
//!   enclosing block's). Passing a root's edge to the next root costs one
//!   step (the paragraph break); entering or leaving an `inline-block`
//!   costs none, its text is stepped through.
//! - The word chord (Ctrl on Windows and Linux, Option on macOS: the text
//!   keymap's) moves to the end of the next word or the start of the one
//!   before, in the root's text (`rinch_words::text_word_boundary`); at a
//!   root's edge it steps to the next root as a character does.
//! - Shift+Up / Shift+Down move to the line above or below at the focus's x
//!   (kept across moves), into the next or previous root's nearest line, and
//!   from the first or last line of all to the text's start or end.
//! - Shift+Home / Shift+End (macOS: Cmd+Shift+Left / Right) go to the focus's
//!   visual line's edges, a wrapped line's end before the space hung there;
//!   Ctrl+Shift+Home / End (macOS: Shift+Home / End, Cmd+Shift+Up / Down) to
//!   the document's.
//! - The anchor stays; a double or triple click's unit is dropped, so the
//!   keys move by characters after one, as Chrome's do.
//!
//! Not offered while a text field holds the keyboard: the field's own keys
//! (`handle_text_key`). The editor's keys never reach here.

use super::text_selection::{outer_roots, root_len, root_text};
use super::text_selection_text::RootsCache;
use super::*;

/// One end of a selection while it moves: a root, an offset in its text and
/// its side of the boxes standing there.
type Pos = (u64, usize, EndSide);

/// After the in-flow boxes at an offset (a backward step lands there).
const AFTER: EndSide = EndSide {
    after_inline: true,
    takes_floats: false,
};

impl RinchApp {
    /// Extend the read-only selection by `motion` (a Shift chord of the text
    /// keymap). `false` when there is no such selection to extend, or the
    /// motion is not one a read-only selection takes.
    pub(super) fn extend_text_selection_by_key(&mut self, motion: TextMotion) -> bool {
        let Some(doc) = self.doc.clone() else {
            return false;
        };
        let Some(sel) = self.text_selection.as_mut().filter(|s| !s.is_empty()) else {
            return false;
        };
        if matches!(motion, TextMotion::PageUp | TextMotion::PageDown) {
            return false;
        }
        let d = doc.borrow();
        let tree = &d.tree;
        let focus: Pos = (sel.focus_node, sel.focus_offset, sel.focus_side);
        let rtl = tree
            .get(focus.0)
            .is_some_and(|n| n.computed_style.direction_rtl);
        let goal = sel.goal_x.take();
        let mut keep_goal = None;
        let cache = sel.cache(tree);
        let moved = match motion {
            TextMotion::CharLeft => step_char(tree, cache, focus, rtl),
            TextMotion::CharRight => step_char(tree, cache, focus, !rtl),
            TextMotion::WordLeft => step_word(tree, cache, focus, rtl),
            TextMotion::WordRight => step_word(tree, cache, focus, !rtl),
            TextMotion::LineStart => line_edge(tree, focus, false),
            TextMotion::LineEnd => line_edge(tree, focus, true),
            TextMotion::DocStart => doc_edge(tree, cache, false).unwrap_or(focus),
            TextMotion::DocEnd => doc_edge(tree, cache, true).unwrap_or(focus),
            TextMotion::LineUp | TextMotion::LineDown => {
                let x = goal.or_else(|| {
                    d.text_caret_window_rect(focus.0, focus.1)
                        .map(|(x, _, _)| x)
                });
                keep_goal = x;
                match x {
                    Some(x) => vertical(&d, cache, focus, x, motion == TextMotion::LineDown),
                    None => focus,
                }
            }
            TextMotion::PageUp | TextMotion::PageDown => focus,
        };
        drop(d);
        let Some(sel) = self.text_selection.as_mut() else {
            return false;
        };
        (sel.focus_node, sel.focus_offset, sel.focus_side) = moved;
        sel.goal_x = keep_goal;
        sel.unit = None;
        sel.dragged = true;
        self.paint_text_selection();
        true
    }
}

/// The roots standing in `root`'s lines (its nested roots, which follow it
/// in root order), with where each stands and whether it has text.
fn nested(tree: &rinch_dom::NodeTree, cache: &RootsCache, root: u64) -> Vec<(u64, usize)> {
    let Some(&i) = cache.index.get(&root) else {
        return Vec::new();
    };
    let lines = super::text_selection::line_offsets(tree, root);
    let mut out = Vec::new();
    for k in i + 1..cache.roots.len() {
        let r = cache.roots[k];
        let Some(&at) = lines.get(&super::text_selection_copy::root_node(tree, r)) else {
            break;
        };
        // Only a root directly in these lines, with text, on a line (not a
        // float's: Chrome does not step into one).
        let direct = outer_roots(tree, r).first() == Some(&root);
        if direct && cache.content[k] && !super::text_selection::floats_in(tree, root, r) {
            out.push((r, at));
        }
    }
    out
}

/// One grapheme from `pos`, forward or back, across boxes and roots.
fn step_char(tree: &rinch_dom::NodeTree, cache: &RootsCache, pos: Pos, forward: bool) -> Pos {
    let (root, off, side) = pos;
    let text = root_text(tree, root).unwrap_or_default();
    let len = text.len();
    let off = off.min(len);
    let boxes = nested(tree, cache, root);
    if forward {
        // A box at the offset, the focus before it: into its text.
        if !side.after_inline
            && let Some(&(n, _)) = boxes.iter().find(|&&(_, at)| at == off)
        {
            return step_char(tree, cache, (n, 0, EndSide::default()), true);
        }
        if off < len {
            let next = rinch_words::next_grapheme_boundary(&text, off);
            return (root, next, EndSide::default());
        }
        // Out of a box: just past it in its root.
        if let Some(&o) = outer_roots(tree, root).first()
            && cache.index.contains_key(&o)
        {
            let at = super::text_selection_copy::box_offset(tree, o, root);
            return step_char(tree, cache, (o, at, AFTER), true);
        }
        // To the next root: the paragraph break is a step.
        match next_root(tree, cache, root) {
            Some(n) => (n, 0, EndSide::default()),
            None => pos,
        }
    } else {
        if side.after_inline
            && let Some(&(n, _)) = boxes.iter().rev().find(|&&(_, at)| at == off)
        {
            let nl = root_len(tree, n);
            return step_char(tree, cache, (n, nl, AFTER), false);
        }
        if off > 0 {
            let prev = rinch_words::prev_grapheme_boundary(&text, off);
            return (root, prev, AFTER);
        }
        if let Some(&o) = outer_roots(tree, root).first()
            && cache.index.contains_key(&o)
        {
            let at = super::text_selection_copy::box_offset(tree, o, root);
            return step_char(tree, cache, (o, at, EndSide::default()), false);
        }
        match prev_root(tree, cache, root) {
            Some(p) => (p, root_len(tree, p), AFTER),
            None => pos,
        }
    }
}

/// The word chord from `pos`: to the next word's end or the previous one's
/// start in the root's text, or at the root's edge a step to the next root.
fn step_word(tree: &rinch_dom::NodeTree, cache: &RootsCache, pos: Pos, forward: bool) -> Pos {
    let (root, off, _) = pos;
    let text = root_text(tree, root).unwrap_or_default();
    let off = off.min(text.len());
    let at_edge = if forward { off == text.len() } else { off == 0 };
    if at_edge {
        return step_char(tree, cache, pos, forward);
    }
    let to = rinch_words::text_word_boundary(&text, off, forward);
    (root, to, if forward { EndSide::default() } else { AFTER })
}

/// The first root after `root` in the document with text, not in its lines.
fn next_root(tree: &rinch_dom::NodeTree, cache: &RootsCache, root: u64) -> Option<u64> {
    let i = *cache.index.get(&root)?;
    (i + 1..cache.roots.len())
        .find(|&k| {
            cache.content[k] && !super::text_selection_copy::holds(tree, root, cache.roots[k])
        })
        .map(|k| cache.roots[k])
}

/// The last root before `root` in the document with text, outermost (the
/// paragraph, not an `inline-block` at its end), not one holding `root`.
fn prev_root(tree: &rinch_dom::NodeTree, cache: &RootsCache, root: u64) -> Option<u64> {
    let i = *cache.index.get(&root)?;
    let outers = outer_roots(tree, root);
    (0..i)
        .rev()
        .find(|&k| cache.content[k] && !outers.contains(&cache.roots[k]))
        .map(|k| super::text_selection_copy::outermost_root(tree, cache.roots[k], tree.root_id))
}

/// The document's first or last selectable text: its start or end.
fn doc_edge(tree: &rinch_dom::NodeTree, cache: &RootsCache, end: bool) -> Option<Pos> {
    let mut with_text = cache
        .roots
        .iter()
        .zip(&cache.content)
        .filter(|(_, has)| **has)
        .map(|(&r, _)| r);
    if end {
        let last = with_text.next_back()?;
        let last = super::text_selection_copy::outermost_root(tree, last, tree.root_id);
        Some((last, root_len(tree, last), AFTER))
    } else {
        Some((with_text.next()?, 0, EndSide::default()))
    }
}

/// The start or end of the visual line the focus is on. A wrapped line ends
/// before the white space hung at its wrap (Chrome 158).
fn line_edge(tree: &rinch_dom::NodeTree, pos: Pos, end: bool) -> Pos {
    let (root, off, _) = pos;
    let text = root_text(tree, root).unwrap_or_default();
    let Some(layout) = tree.get(root).and_then(|n| n.text_layout.as_ref()) else {
        // A text leaf: one line.
        return if end {
            (root, text.len(), AFTER)
        } else {
            (root, 0, EndSide::default())
        };
    };
    let affinity = rinch_core::dom::CaretAffinity::Downstream;
    let Some((at, _)) = RinchApp::visual_line_edge(&layout.layout, off, affinity, end) else {
        return pos;
    };
    if !end {
        return (root, at, EndSide::default());
    }
    let trimmed = text[..at.min(text.len())]
        .trim_end_matches([' ', '\t'])
        .len();
    let at = if at < text.len() {
        trimmed.max(off.min(at))
    } else {
        at
    };
    (root, at, AFTER)
}

/// The line above or below the focus at window x `x`: in its root, else the
/// nearest line of the next or previous root, else the text's start or end.
fn vertical(d: &RinchDocument, cache: &RootsCache, pos: Pos, x: f32, down: bool) -> Pos {
    let tree = &d.tree;
    let (root, off, _) = pos;
    let lines_of = |r: u64| {
        tree.get(r)
            .and_then(|n| n.text_layout.as_ref())
            .map(|l| RinchApp::input_visual_lines(&l.layout))
            .unwrap_or_default()
    };
    // Where on line `i` of root `r` the x falls.
    let on_line = |r: u64, lines: &[(f32, f32, std::ops::Range<usize>)], i: usize| -> Option<Pos> {
        let start = lines.get(i)?.2.start;
        let (_, cy, ch) = d.text_caret_window_rect(r, start)?;
        let o = RinchApp::compute_ifc_offset_from_click(tree, r, x, cy + ch / 2.0);
        Some((r, o, EndSide::default()))
    };
    let lines = lines_of(root);
    let affinity = rinch_core::dom::CaretAffinity::Downstream;
    let current = tree
        .get(root)
        .and_then(|n| n.text_layout.as_ref())
        .and_then(|l| RinchApp::input_caret_line(&l.layout, &lines, off, affinity).0);
    if let Some(c) = current {
        let target = if down {
            c.checked_add(1)
        } else {
            c.checked_sub(1)
        };
        if let Some(t) = target.filter(|&t| t < lines.len())
            && let Some(p) = on_line(root, &lines, t)
        {
            return p;
        }
    }
    let other = if down {
        next_root(tree, cache, root)
    } else {
        prev_root(tree, cache, root)
    };
    if let Some(r) = other {
        let l = lines_of(r);
        let i = if down { 0 } else { l.len().saturating_sub(1) };
        if let Some(p) = on_line(r, &l, i) {
            return p;
        }
        return if down {
            (r, 0, EndSide::default())
        } else {
            (r, root_len(tree, r), AFTER)
        };
    }
    if down {
        (root, root_len(tree, root), AFTER)
    } else {
        (root, 0, EndSide::default())
    }
}
