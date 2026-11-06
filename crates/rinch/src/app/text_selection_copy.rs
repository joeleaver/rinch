//! The text a read-only selection copies, as Chrome 156's
//! `getSelection().toString()` gives it: Blink's text iterator, walking the
//! document in order from the selection's first root to its last.
//!
//! - Text is written where its node stands: each text node (and `<br>`) of
//!   an IFC root writes the root's selected part up to its end
//!   ([`rendered_ranges`]: its kept text, without the white space a line
//!   drops at its start and end), so an atomic inline, float or positioned
//!   box inside a root is written between the text before it and the text
//!   after it (#2242). The rest of a root's part is written on leaving it.
//! - What stands in a root's lines is in the selection when the root's part
//!   reaches where it stands: an `inline-block` before a selection's start,
//!   or after its end, in the same root is not written, and one in a root the
//!   selection covers is written whole, whatever the selection's runs say.
//! - The text of a flex or grid container (an anonymous item, laid out as a
//!   text leaf, with no root) is written as its `white-space` draws it
//!   (#2241).
//! - Entering a block (a block container, a table or a row; not a cell, an
//!   inline-level box, a float, an absolutely positioned box or a picture)
//!   writes a line break, and leaving one writes one, unless the last thing
//!   written is a line break. Leaving a `<p>` with height writes one more.
//! - An inline table writes a space before it (unless at a line's start) and
//!   one after it.
//! - Entering a table cell other than its row's first writes a tab, unless
//!   the last thing written is a line break.
//! - An `<img>` or `<iframe>` is written as nothing, but is something
//!   written: the block around it then ends with a line break.
//! - Nothing is written before the first text, and what is not displayed is
//!   passed whole.

use std::collections::HashMap;

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

/// One step of the walk.
#[derive(Clone, Copy)]
enum Step {
    Enter(u64),
    Leave(u64),
    /// The first member of an anonymous box's run: the box's root opens.
    EnterRun(u64),
    /// Past the last member of an anonymous box's run.
    LeaveRun(u64),
}

/// An IFC root the walk is inside, and how far its text is written.
struct Root {
    id: u64,
    /// The part of its text the selection takes, `None` when it takes none
    /// of it (a root that cannot be selected, or one beside the selection).
    part: Option<(usize, usize)>,
    len: usize,
    /// Its text before this is written (or passed).
    cursor: usize,
    /// Whether the walk has met its text: not yet for a root the walk starts
    /// inside, whose text before the start is no part of the selection.
    started: bool,
    /// Where each of its text nodes and `<br>`s stands in its text.
    texts: HashMap<u64, (usize, usize)>,
    /// The ranges of its text a copy writes ([`rendered_ranges`]), worked out
    /// once: each text node writes a slice of them.
    drawn: Vec<(usize, usize)>,
    /// The first of `drawn` not yet written past.
    next: usize,
}

impl Root {
    fn new(
        tree: &rinch_dom::NodeTree,
        id: u64,
        part: Option<(usize, usize)>,
        started: bool,
    ) -> Self {
        let mut texts: HashMap<u64, (usize, usize)> = HashMap::new();
        let mut len = 0;
        let mut drawn = Vec::new();
        if let Some(layout) = tree.get(id).and_then(|n| n.text_layout.as_ref()) {
            len = layout.text_content.len();
            if part.is_some() {
                drawn = rendered_ranges(tree, id, 0, len);
            }
            for r in &layout.text_ranges {
                work(1);
                let e = texts.entry(r.node_id).or_insert((r.flat_start, r.flat_end));
                *e = (e.0.min(r.flat_start), e.1.max(r.flat_end));
            }
        }
        Self {
            id,
            part,
            len,
            cursor: 0,
            started,
            texts,
            drawn,
            next: 0,
        }
    }

    /// Whether what stands at the walk's place in this root's lines (an
    /// atomic inline, a float, a picture) is in the selection: its part
    /// reaches past it. A part that starts or ends where a box stands has the
    /// box only at the root's own start or end: an offset there cannot say
    /// which side of the box it is on, and a selection that starts or ends
    /// in the text beside a box (Chrome 156) is on the text's side. A root
    /// the selection takes none of leaves that to the roots inside.
    fn takes_here(&self) -> bool {
        match self.part {
            None => true,
            Some((s, e)) => {
                !self.started || ((s < self.cursor || s == 0) && (self.cursor < e || e == self.len))
            }
        }
    }

    /// Write its part up to `upto`.
    fn write_to(&mut self, tree: &rinch_dom::NodeTree, upto: usize, out: &mut Out) {
        if let Some((s, e)) = self.part {
            let (a, b) = (self.cursor.max(s), upto.min(e));
            if a < b
                && let Some(t) = tree.get(self.id).and_then(|n| n.text_layout.as_ref())
            {
                while let Some(&(x, y)) = self.drawn.get(self.next) {
                    work(1);
                    if x >= b {
                        break;
                    }
                    let (from, to) = (x.max(a), y.min(b));
                    if from < to {
                        out.push(t.text_content.get(from..to).unwrap_or(""));
                    }
                    if y > b {
                        break;
                    }
                    self.next += 1;
                }
            }
        }
        self.cursor = self.cursor.max(upto);
    }
}

/// Whether the root `id` (an anonymous box: its run's) can be selected.
fn selectable(tree: &rinch_dom::NodeTree, id: u64) -> bool {
    let at = tree
        .get(id)
        .filter(|n| n.is_anonymous_block_box)
        .and_then(|n| n.run_members.first().copied())
        .unwrap_or(id);
    super::text_selection::resolved_user_select(tree, at).is_selectable()
}

/// The length of root `id`'s text.
fn text_len(tree: &rinch_dom::NodeTree, id: u64) -> usize {
    tree.get(id)
        .and_then(|n| n.text_layout.as_ref())
        .map_or(0, |l| l.text_content.len())
}

/// Open root `id`, met by the walk: its part is the selection's at an end
/// of it, and all of it between (when it can be selected).
fn open(
    tree: &rinch_dom::NodeTree,
    parts: &HashMap<u64, (usize, usize)>,
    roots: &mut Vec<Root>,
    id: u64,
) {
    let part = parts
        .get(&id)
        .copied()
        .or_else(|| selectable(tree, id).then_some((0, text_len(tree, id))));
    roots.push(Root::new(tree, id, part, true));
}

/// Close root `id` if it is the innermost open one, writing the rest of its
/// part.
fn close(tree: &rinch_dom::NodeTree, roots: &mut Vec<Root>, id: u64, out: &mut Out) {
    if roots.last().is_some_and(|r| r.id == id)
        && let Some(mut r) = roots.pop()
        && r.started
    {
        let len = r.len;
        r.write_to(tree, len, out);
    }
}

/// The copy of the selection between `a` and `b` (each a root and an offset
/// in its text), whichever comes first in the document.
pub(crate) fn copy_text(tree: &rinch_dom::NodeTree, a: (u64, usize), b: (u64, usize)) -> String {
    let (a, b) = (
        (a.0, a.1.min(text_len(tree, a.0))),
        (b.0, b.1.min(text_len(tree, b.0))),
    );
    let ((first, start), (last_root, end)) = if before(tree, b, a) { (b, a) } else { (a, b) };
    let mut parts: HashMap<u64, (usize, usize)> = HashMap::new();
    if first == last_root {
        parts.insert(first, (start, end));
    } else {
        parts.insert(first, (start, text_len(tree, first)));
        parts.insert(last_root, (0, end));
    }
    // What holds the last root: never passed, and leaving it ends the walk
    // (the last root is then behind it).
    let holds_last = holders(tree, last_root);
    let mut out = Out {
        text: String::new(),
        emitted: false,
        after_picture: false,
    };
    let (mut stack, mut roots) = walk_from(tree, &parts, first);
    while let Some(step) = stack.pop() {
        let id = match step {
            Step::EnterRun(b) => {
                open(tree, &parts, &mut roots, b);
                continue;
            }
            Step::LeaveRun(b) => {
                close(tree, &mut roots, b, &mut out);
                if b == last_root || holds_last.contains(&b) {
                    break;
                }
                continue;
            }
            Step::Leave(id) => {
                close(tree, &mut roots, id, &mut out);
                if id == last_root || holds_last.contains(&id) {
                    break;
                }
                leave(tree, id, &mut out);
                continue;
            }
            Step::Enter(id) => id,
        };
        let Some(n) = tree.get(id) else {
            continue;
        };
        // A text node or `<br>` of the root it is in: the root's text up to
        // its end.
        if let Some(r) = roots.last_mut()
            && let Some(&(a, b)) = r.texts.get(&id)
        {
            if !r.started {
                r.cursor = a;
                r.started = true;
            }
            r.write_to(tree, b, &mut out);
            continue;
        }
        if n.tag().is_none() {
            if n.cached_text_parley.is_some() && roots.last().is_none_or(Root::takes_here) {
                write_leaf(tree, id, &mut out);
            }
            continue;
        }
        if n.computed_style.display == DisplayValue::None {
            continue;
        }
        // An atomic inline, float, positioned box or picture is there only
        // when the part of the root it stands in reaches it (what holds the
        // last root always does).
        let in_line = matches!(
            n.computed_style.display,
            DisplayValue::Inline | DisplayValue::Contents
        ) && !is_picture(n);
        if !in_line && !holds_last.contains(&id) && roots.last().is_some_and(|r| !r.takes_here()) {
            continue;
        }
        enter(tree, id, &mut out);
        if n.text_layout.is_some() {
            open(tree, &parts, &mut roots, id);
        }
        if is_picture(n) {
            if n.computed_style.visibility == VisibilityValue::Visible {
                out.emitted = true;
                out.after_picture = true;
            }
            continue;
        }
        stack.push(Step::Leave(id));
        push_children(tree, id, 0, None, &mut stack);
    }
    out.text
}

/// Write text leaf `id`, the text of a flex or grid container, as its
/// `white-space` draws it: an anonymous item's lines drop the spaces at
/// their start and end.
fn write_leaf(tree: &rinch_dom::NodeTree, id: u64, out: &mut Out) {
    let Some(n) = tree.get(id) else {
        return;
    };
    let Some(parent) = n.parent.and_then(|p| tree.get(p).map(|e| (p, e))) else {
        return;
    };
    if parent.1.computed_style.visibility != VisibilityValue::Visible
        || !super::text_selection::resolved_user_select(tree, parent.0).is_selectable()
    {
        return;
    }
    let Some(text) = n.text_content() else {
        return;
    };
    let collapse = |s: &str| -> String {
        s.split([' ', '\t', '\n', '\r', '\u{c}'])
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let drawn = match parent.1.computed_style.white_space {
        WhiteSpaceValue::Pre | WhiteSpaceValue::PreWrap | WhiteSpaceValue::BreakSpaces => {
            text.to_string()
        }
        WhiteSpaceValue::PreLine => text
            .split('\n')
            .map(collapse)
            .collect::<Vec<_>>()
            .join("\n"),
        WhiteSpaceValue::Normal | WhiteSpaceValue::NoWrap => collapse(text),
    };
    out.push(&drawn);
}

/// Push `parent`'s children from index `from` to walk next, in order. A run
/// of children an anonymous box lays out opens the box's root at its first
/// member (unless it is `open` already) and closes it after its last.
fn push_children(
    tree: &rinch_dom::NodeTree,
    parent: u64,
    from: usize,
    open: Option<u64>,
    stack: &mut Vec<Step>,
) {
    let Some(p) = tree.get(parent) else {
        return;
    };
    let mut steps = Vec::new();
    for &c in p.children.iter().skip(from) {
        let run = tree.get(c).and_then(|n| n.run_box);
        match run.and_then(|b| tree.get(b).map(|n| (b, &n.run_members))) {
            Some((b, members)) => {
                if members.first() == Some(&c) && open != Some(b) {
                    steps.push(Step::EnterRun(b));
                }
                steps.push(Step::Enter(c));
                if members.last() == Some(&c) {
                    steps.push(Step::LeaveRun(b));
                }
            }
            None => steps.push(Step::Enter(c)),
        }
    }
    stack.extend(steps.into_iter().rev());
}

/// The walk's stack as it stands on reaching `first`, the selection's first
/// root, and the roots it is inside then: the node there next, then the rest
/// of each of its ancestors after it and the ancestor itself to leave,
/// innermost first. Nothing before it in the document is walked (it could
/// write nothing: nothing is written before the first text). A root it
/// starts inside (an `inline-block`'s paragraph) writes its text from the
/// next of its text nodes the walk meets.
fn walk_from(
    tree: &rinch_dom::NodeTree,
    parts: &HashMap<u64, (usize, usize)>,
    first: u64,
) -> (Vec<Step>, Vec<Root>) {
    // An anonymous box is met at its first member.
    let anonymous = tree.get(first).is_some_and(|n| n.is_anonymous_block_box);
    let start = if anonymous {
        tree.get(first)
            .and_then(|n| n.run_members.first().copied())
            .unwrap_or(first)
    } else {
        first
    };
    let mut path = vec![start];
    while let Some(p) = path
        .last()
        .and_then(|&c| tree.get(c))
        .and_then(|n| n.parent)
    {
        path.push(p);
    }
    // A root the walk starts inside: its part after the start (all of it, or
    // up to the selection's end when that is in it).
    let around = |id: u64| {
        let part = parts
            .get(&id)
            .copied()
            .or_else(|| selectable(tree, id).then_some((0, text_len(tree, id))));
        Root::new(tree, id, part, false)
    };
    let mut stack = Vec::new();
    let mut roots = Vec::new();
    for pair in path.windows(2).rev() {
        let (child, parent) = (pair[0], pair[1]);
        let Some(p) = tree.get(parent) else {
            continue;
        };
        if p.text_layout.is_some() {
            roots.push(around(parent));
        }
        let run = tree.get(child).and_then(|c| c.run_box);
        if let Some(b) = run
            && !(child == start && anonymous)
        {
            roots.push(around(b));
        }
        stack.push(Step::Leave(parent));
        let at = p
            .children
            .iter()
            .position(|&c| c == child)
            .map_or(p.children.len(), |i| i + 1);
        push_children(tree, parent, at, run, &mut stack);
        if let Some(b) = run
            && tree
                .get(b)
                .is_some_and(|n| n.run_members.last() == Some(&child))
        {
            stack.push(Step::LeaveRun(b));
        }
    }
    stack.push(Step::Enter(start));
    if anonymous {
        stack.push(Step::EnterRun(first));
    }
    (stack, roots)
}

/// Whether element `n` is an inline table, which Blink writes a space before
/// and after.
fn is_inline_table(n: &rinch_dom::Node) -> bool {
    n.computed_style.table_part == TablePart::Table
        && matches!(
            n.computed_style.display,
            DisplayValue::InlineBlock | DisplayValue::InlineFlex | DisplayValue::InlineGrid
        )
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
    } else if is_inline_table(n) && out.emitted && !out.ends_in_newline() {
        out.push(" ");
    }
}

fn leave(tree: &rinch_dom::NodeTree, id: u64, out: &mut Out) {
    let Some(n) = tree.get(id) else {
        return;
    };
    if is_inline_table(n) && out.emitted {
        out.push(" ");
        return;
    }
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
/// is not white space its lines drop, or its first atomic inline, to just
/// after its last such character or its last atomic inline. `None` when it
/// is empty (white space only, or a lone picture).
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
    // Whether character `c` at `i`, in text range `at`, is white space the
    // lines drop.
    let drops = |c: char, at: Option<&(usize, usize, bool, WhiteSpaceValue)>| -> bool {
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
    // From the start, a cursor over the ranges (in text order) rather than a
    // search per character.
    let mut j = 0;
    let first = text.char_indices().find(|&(i, c)| {
        work(1);
        while ranges.get(j).is_some_and(|r| r.1 <= i) {
            work(1);
            j += 1;
        }
        !drops(c, ranges.get(j).filter(|r| r.0 <= i))
    });
    // And from the end.
    let mut j = ranges.len();
    let last = text.char_indices().rev().find(|&(i, c)| {
        work(1);
        while j > 0 && ranges[j - 1].0 > i {
            work(1);
            j -= 1;
        }
        !drops(c, j.checked_sub(1).map(|k| &ranges[k]).filter(|r| r.1 > i))
    });
    // An atomic inline is drawn too: the white space between two is kept
    // (Chrome writes the " " between two `inline-block` columns), and only
    // what is before the first and after the last thing drawn is dropped.
    let boxes = || layout.atomic_inline_offsets();
    let first = first.map(|(i, _)| i).into_iter().chain(boxes()).min()?;
    let last = last
        .map(|(i, c)| i + c.len_utf8())
        .into_iter()
        .chain(boxes())
        .max()?;
    (first < last).then_some((first, last))
}

// ── Document order ─────────────────────────────────────────────────────────

/// The node root `id` stands at in the element tree: an anonymous box at
/// its run's first member.
fn root_node(tree: &rinch_dom::NodeTree, id: u64) -> u64 {
    tree.get(id)
        .filter(|n| n.is_anonymous_block_box)
        .and_then(|n| n.run_members.first().copied())
        .unwrap_or(id)
}

/// The elements (and anonymous boxes) that hold root `id`: its ancestors,
/// and the box of each run one of them (or it) is in.
fn holders(tree: &rinch_dom::NodeTree, id: u64) -> std::collections::HashSet<u64> {
    let mut out = std::collections::HashSet::new();
    let node = root_node(tree, id);
    let mut at = Some(node);
    while let Some(i) = at {
        let Some(n) = tree.get(i) else {
            break;
        };
        if i != node {
            out.insert(i);
        }
        if let Some(b) = n.run_box
            && b != id
        {
            out.insert(b);
        }
        at = n.parent;
    }
    out
}

/// The path from the document down to node `id`.
fn path_to(tree: &rinch_dom::NodeTree, id: u64) -> Vec<u64> {
    let mut path = vec![id];
    while let Some(p) = path
        .last()
        .and_then(|&i| tree.get(i))
        .and_then(|n| n.parent)
    {
        path.push(p);
    }
    path.reverse();
    path
}

/// Whether node `a` comes before node `b` in the document (an ancestor
/// before what it holds).
fn node_before(tree: &rinch_dom::NodeTree, a: u64, b: u64) -> bool {
    let (pa, pb) = (path_to(tree, a), path_to(tree, b));
    let common = pa.iter().zip(&pb).take_while(|(x, y)| x == y).count();
    match (pa.get(common), pb.get(common)) {
        (None, Some(_)) => true,
        (_, None) => false,
        (Some(&x), Some(&y)) => {
            let siblings = common
                .checked_sub(1)
                .and_then(|i| tree.get(pa[i]))
                .map_or(&[][..], |p| &p.children[..]);
            let at = |c: u64| siblings.iter().position(|&s| s == c);
            at(x) < at(y)
        }
    }
}

/// Whether root `inner` stands inside root `outer`'s lines (an
/// `inline-block` in a paragraph, at any depth).
fn holds(tree: &rinch_dom::NodeTree, outer: u64, inner: u64) -> bool {
    outer != inner && holders(tree, inner).contains(&outer)
}

/// Where the box holding root `inner` stands in root `outer`'s text: after
/// the last of `outer`'s text before it.
fn box_offset(tree: &rinch_dom::NodeTree, outer: u64, inner: u64) -> usize {
    let at = root_node(tree, inner);
    tree.get(outer)
        .and_then(|n| n.text_layout.as_ref())
        .map_or(0, |l| {
            l.text_ranges
                .iter()
                .take_while(|r| node_before(tree, r.node_id, at))
                .last()
                .map_or(0, |r| r.flat_end)
        })
}

/// Whether selection end `a` comes before end `b` in the document. A root
/// inside another stands where its box stands in that root's text, not
/// where the root order puts it (after the root holding it); an offset where
/// the box stands is before it.
fn before(tree: &rinch_dom::NodeTree, a: (u64, usize), b: (u64, usize)) -> bool {
    if a.0 == b.0 {
        a.1 < b.1
    } else if holds(tree, a.0, b.0) {
        a.1 <= box_offset(tree, a.0, b.0)
    } else if holds(tree, b.0, a.0) {
        box_offset(tree, b.0, a.0) < b.1
    } else {
        node_before(tree, root_node(tree, a.0), root_node(tree, b.0))
    }
}

/// Count `n` steps of the copy's work over a root's text: a range or a
/// character looked at. Tests read it ([`WORK`]) to pin that a copy is
/// linear in the text it walks; it does nothing outside tests.
#[inline]
pub(crate) fn work(n: u64) {
    #[cfg(test)]
    WORK.with(|w| w.set(w.get() + n));
    #[cfg(not(test))]
    let _ = n;
}

#[cfg(test)]
thread_local! {
    /// The steps [`work`] has counted on this thread.
    pub(crate) static WORK: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
