//! Which `<br>` makes a line: the HTML reader's rule for the inline content
//! of a block (#3096).
//!
//! In a browser a `<br>` ends the line it is on, and the line after it is
//! drawn only when something follows it there. So a block's **last** `<br>`
//! makes no line of its own: `<div>a<br></div>` and `<p>a<br></p>` are one
//! line, `<div><br></div>` and `<p><br></p>` one empty line, and a bare
//! `<br>` between two `<p>`s (the way Google Docs writes an empty paragraph)
//! is one empty line, `<br><br>` two. rinch's editor draws every `hard_break`
//! as a line end, the one at the end of a paragraph included (the view's
//! trailing-break placeholder, rinch-dom laying out `<p>a<br></p>` as one
//! line as Chrome does), so a block keeps every `<br>` but its last.
//!
//! The rule covers a textblock element's content (`<p>`, `<h1>`, …) and
//! *loose* inline content: what sits between blocks, or directly in a
//! container (`<div>`, `<li>`, a cell) with no textblock element around it.
//! rinch's own writer writes a textblock that ends in a hard break with the
//! placeholder `<br>` after it, as a browser's editor does (`<p>a<br><br></p>`),
//! so its HTML reads back the same and shows the same lines anywhere.
//!
//! A line that the end of the pasted content leaves open is not ended when
//! text follows the caret: that text continues it, so a `<br>` there still
//! breaks the line before it, as Chrome pastes `a<br>` into `x|y` as
//! `xa<br>y`. When nothing follows the caret the line is ended like any other
//! ([`PasteEnd`]). A copy that ends in `<br class="Apple-interchange-newline">`
//! ends in a paragraph end, not a line ([`take_interchange_newline`]).

use super::html_tree::ParsedNode;
use crate::model::{Node, NodeType};
use crate::pos::Pos;

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

/// Whether anything follows a paste in the textblock it ends in, which says
/// how the pasted markup's last loose line ends (#3096). Chrome, measured
/// with `insertHTML`: `<p>alpha</p><br>` pasted at the end of `x` is two
/// lines (`xalpha` and one empty line), and four `<br>`s are five; the same
/// at the end of a paragraph with another after it and on an empty line.
/// Pasted into `x|y` the `<br>` breaks the line before `y`: three lines.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PasteEnd {
    /// Text follows the caret in its textblock and continues the pasted
    /// markup's last line, so a `<br>` that ends it still breaks the line
    /// before that text. What a read with no caret assumes.
    #[default]
    Continued,
    /// Nothing follows the caret in its textblock (an empty line, the end of
    /// a paragraph or of the document): a `<br>` that ends pasted markup
    /// holding blocks makes no line, as a block's last `<br>` makes none.
    /// Markup of inline content alone (one line, as a selection inside a
    /// line copies) keeps its `<br>`s: it continues the caret's line.
    Ended,
}

impl PasteEnd {
    /// How a paste that ends at `to` in `doc` ends: [`PasteEnd::Continued`]
    /// when `to` is in a textblock with content after it, else
    /// [`PasteEnd::Ended`].
    pub fn at(doc: &Node, to: Pos) -> PasteEnd {
        match doc.resolve(to) {
            Ok(pos)
                if pos.parent().is_textblock()
                    && pos.parent_offset() < pos.parent().content_size() =>
            {
                PasteEnd::Continued
            }
            _ => PasteEnd::Ended,
        }
    }
}

/// The class Chrome and WebKit put on a `<br>` that ends a copy whose
/// selection ended at a paragraph's end.
const INTERCHANGE_NEWLINE: &str = "Apple-interchange-newline";

/// Remove the `<br class="Apple-interchange-newline">` that ends `forest`, if
/// one does, and say whether it did. It is not a line but a paragraph end:
/// Chrome pastes `<p>a</p><p>b</p>` followed by one into `x|y` as `xa`, `b`,
/// `y` (three lines, the `y` in a paragraph of its own). Only the last node
/// counts (whitespace, and the `<html>`/`<body>` a platform's clipboard
/// format wraps the markup in, are looked through); one anywhere else is
/// read as the `<br>` it is.
pub(super) fn take_interchange_newline(forest: &mut Vec<ParsedNode>) -> bool {
    let mut nodes = forest;
    loop {
        let Some(i) = nodes
            .iter()
            .rposition(|n| !matches!(n, ParsedNode::Text(t) if t.trim().is_empty()))
        else {
            return false;
        };
        let ParsedNode::Element {
            tag, attributes, ..
        } = &nodes[i]
        else {
            return false;
        };
        if tag == "br" {
            let interchange = attributes.iter().any(|(k, v)| {
                k == "class" && v.split_ascii_whitespace().any(|c| c == INTERCHANGE_NEWLINE)
            });
            if interchange {
                nodes.remove(i);
            }
            return interchange;
        }
        if !matches!(tag.as_str(), "html" | "body") {
            return false;
        }
        let ParsedNode::Element { children, .. } = &mut nodes[i] else {
            return false;
        };
        nodes = children;
    }
}
