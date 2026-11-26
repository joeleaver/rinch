//! Which `<br>` makes a line: the HTML reader's rule for the inline content
//! it gathers outside any textblock element (#3096).
//!
//! In a browser a `<br>` ends the line it is on, and the line after it is
//! drawn only when something follows it there. So a block's **last** `<br>`
//! makes no line of its own: `<div>a<br></div>` is one line, `<div><br></div>`
//! is one empty line, and a bare `<br>` between two `<p>`s (the way Google Docs
//! writes an empty paragraph) is one empty line, `<br><br>` two. rinch's
//! editor draws every `hard_break` as a line end, the one at the end of a
//! paragraph included (the trailing-break placeholder), so a paragraph made of
//! such a line keeps every `<br>` but its last.
//!
//! The rule covers *loose* inline content only: what sits between blocks, or
//! directly in a container (`<div>`, `<li>`, a cell) with no textblock element
//! around it. Inside a `<p>`, `<h1>`, … every `<br>` is a `hard_break`,
//! because that is how rinch's own writer writes one at the end of a
//! paragraph (`<p>a<br></p>`), and its HTML must read back the same. rinch
//! never writes loose inline content inside blocks.
//!
//! A line that the end of the pasted content leaves open is not ended: the
//! text after the caret continues it, so a `<br>` there still breaks the line
//! before that text, as Chrome pastes `a<br>` into `x|y` as `xa<br>y`.

use crate::model::{Node, NodeType};

/// How a loose line of inline content ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum LineEnd {
    /// By a block, or by the end of the element it is in: a final `<br>`
    /// makes no line.
    Closed,
    /// By the end of the content read: content may follow it there (a
    /// paste's open end), so a final `<br>` is kept.
    Open,
}

/// Drop the final `hard_break` of a closed loose line, the one a browser
/// shows no line for. The line stays a line (an empty paragraph) when that
/// was all it held.
pub(super) fn drop_line_end_break(
    nodes: &mut Vec<Node>,
    hard_break: Option<&NodeType>,
    end: LineEnd,
) {
    if end == LineEnd::Closed
        && let Some(hard_break) = hard_break
        && nodes.last().is_some_and(|n| n.node_type() == hard_break)
    {
        nodes.pop();
    }
}
