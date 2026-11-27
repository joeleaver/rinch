//! Whitespace after a line's last `<br>` (#3096 review).
//!
//! A block's last `<br>` makes no line ([`super::html_lines`]), but
//! pretty-printed HTML puts whitespace after it: `<p>a<br>\n</p>`,
//! `<p>\n  a<br>\n</p>`, `<li>a<br>\n  </li>`. Chrome draws each as one line,
//! as it draws `<p>a<br></p>`: whitespace that collapses is not content at a
//! line's end. The reader keeps whitespace inside a textblock as it is
//! (#1406), so without this pass the `<br>` would not be the last thing in the
//! block and would be read as a hard break, with the whitespace as a line of
//! its own after it.
//!
//! So before the forest is read, [`trim_line_end_space`] takes out the
//! whitespace that follows a line's last `<br>`, when that whitespace
//! collapses and nothing else follows it on the line. A line ends where a
//! block element starts or ends, and at the end of the markup when that end
//! is closed ([`super::html_lines::LineEnd::Closed`]). Comments make no node,
//! and dropped elements (`<style>`, `<script>`, …) and empty inline elements
//! show nothing, so they are looked through.
//!
//! Whether whitespace collapses is CSS `white-space`, read from the `style`
//! of the elements the text is in (and `<pre>`, `<textarea>`, `<listing>`,
//! `<xmp>`, `<plaintext>`, which preserve it), measured in Chrome:
//! - `normal`, `nowrap`: all ASCII whitespace collapses. `<p>a<br>\n</p>` and
//!   `<p>a<br> </p>` are one line.
//! - `pre-line`: spaces and tabs collapse, a newline does not.
//!   `<p style="white-space:pre-line">a<br>  </p>` is one line, with `\n`
//!   two.
//! - `pre`, `pre-wrap`, `break-spaces`: nothing collapses.
//!   `<p style="white-space:pre-wrap">a<br> </p>` is two lines, and so is
//!   `<p><span style="white-space:pre-wrap">a<br>\n</span></p>`.
//!
//! A non-breaking space is not ASCII whitespace and never collapses:
//! `<p>a<br>&nbsp;</p>` is two lines. Like the rest of the reader this reads
//! no stylesheet, so `white-space` set by a class or on an element outside
//! the markup read is not seen.

use super::html::is_dropped;
use super::html_tree::ParsedNode;
use super::style_scan::style_declarations;
use crate::model::Node;

/// Whether the writer ends `node` with one more `<br>`, the placeholder a
/// browser draws no line for: when it is a textblock whose last hard break
/// is followed by nothing but ASCII whitespace text. A textblock that ends
/// in a hard break (`<p>a<br><br></p>`) is the common case; one with a
/// space or a newline after its last break is written `<p>a<br>\n<br></p>`,
/// since without the placeholder [`trim_line_end_space`] would read that
/// whitespace, and the break before it, as nothing. A code block is written
/// as `<pre>`, whose whitespace is content, so whitespace there needs none.
pub(super) fn needs_line_end_placeholder(node: &Node) -> bool {
    if !node.is_textblock() {
        return false;
    }
    let code = node.node_type().spec().code;
    for child in node.content().children().iter().rev() {
        if child.type_name() == "hard_break" {
            return true;
        }
        match child.text() {
            Some(t) if !code && t.chars().all(|c| c.is_ascii_whitespace()) => {}
            _ => return false,
        }
    }
    false
}

/// How an element treats whitespace, as far as this pass needs to know.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WhiteSpace {
    /// `normal`, `nowrap`: ASCII whitespace collapses.
    Collapse,
    /// `pre-line`: spaces and tabs collapse, segment breaks do not.
    PreserveBreaks,
    /// `pre`, `pre-wrap`, `break-spaces`: all whitespace is content.
    Preserve,
}

impl WhiteSpace {
    /// The `white-space` of an element whose parent's is `self`.
    fn of(self, tag: &str, attributes: &[(String, String)]) -> WhiteSpace {
        if matches!(tag, "pre" | "textarea" | "listing" | "xmp" | "plaintext") {
            return WhiteSpace::Preserve;
        }
        let declared = attributes
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case("style"))
            .flat_map(|(_, v)| style_declarations(v))
            // The last declaration wins, as in CSS.
            .rfind(|(name, _)| name.eq_ignore_ascii_case("white-space"));
        let Some((_, value)) = declared else {
            return self;
        };
        match value.trim().to_ascii_lowercase().as_str() {
            "normal" | "nowrap" => WhiteSpace::Collapse,
            "pre-line" => WhiteSpace::PreserveBreaks,
            "pre" | "pre-wrap" | "break-spaces" => WhiteSpace::Preserve,
            // `inherit` and anything not understood: the parent's.
            _ => self,
        }
    }

    /// Whether all of `t` collapses away at a line's end.
    fn collapses(self, t: &str) -> bool {
        match self {
            WhiteSpace::Collapse => t.chars().all(|c| c.is_ascii_whitespace()),
            WhiteSpace::PreserveBreaks => t.chars().all(|c| matches!(c, ' ' | '\t')),
            WhiteSpace::Preserve => t.is_empty(),
        }
    }
}

/// Take the collapsible whitespace after each line's last `<br>` out of
/// `forest` (see the module docs). `is_block` says which tags are blocks;
/// `closed` whether the markup's own end ends its last line. Run after
/// `holds_block` is marked: an inline element that holds a block is not
/// looked into for a line's end.
pub(super) fn trim_line_end_space(
    forest: &mut [ParsedNode],
    is_block: &dyn Fn(&str) -> bool,
    closed: bool,
) {
    trim_children(forest, WhiteSpace::Collapse, is_block, closed);
}

/// Trim the lines in `nodes`, the content of an element whose
/// `white-space` is `ws`. `closed` says whether the element's end ends the
/// line its last inline content is on.
fn trim_children(
    nodes: &mut [ParsedNode],
    ws: WhiteSpace,
    is_block: &dyn Fn(&str) -> bool,
    closed: bool,
) {
    // The start of the run of inline siblings since the last block.
    let mut run = 0;
    for i in 0..nodes.len() {
        let ParsedNode::Element {
            tag,
            attributes,
            children,
            holds_block,
        } = &mut nodes[i]
        else {
            continue;
        };
        if is_dropped(tag) {
            continue;
        }
        let inner = ws.of(tag, attributes);
        if is_block(tag) {
            // A block ends the line before it, and its own end its last.
            trim_children(children, inner, is_block, true);
            trim_run(&mut nodes[run..i], ws);
            run = i + 1;
        } else if *holds_block {
            // Inline around blocks: its lines are its blocks', and the
            // content after its last block continues past its end.
            trim_children(children, inner, is_block, false);
        }
    }
    if closed {
        trim_run(&mut nodes[run..], ws);
    }
}

/// When the line `run` ends with a `<br>` and collapsible whitespace, take
/// that whitespace out.
fn trim_run(run: &mut [ParsedNode], ws: WhiteSpace) {
    if ends_in_break(run, ws) {
        strip_end(run, ws);
    }
}

/// Whether the last thing `nodes` show, looking through collapsible
/// whitespace, dropped elements and empty inline elements, is a `<br>`.
/// `None` when they show nothing at all.
fn last_shown_is_break(nodes: &[ParsedNode], ws: WhiteSpace) -> Option<bool> {
    for n in nodes.iter().rev() {
        match n {
            ParsedNode::Text(t) if ws.collapses(t) => {}
            ParsedNode::Text(_) => return Some(false),
            ParsedNode::Element {
                tag,
                attributes,
                children,
                holds_block,
            } => {
                if is_dropped(tag) {
                    continue;
                }
                if tag == "br" {
                    return Some(true);
                }
                if *holds_block {
                    return Some(false);
                }
                if let Some(shown) = last_shown_is_break(children, ws.of(tag, attributes)) {
                    return Some(shown);
                }
                // An inline element with nothing in it shows nothing, but a
                // leaf (`<img>`, `<input>`, …) is content.
                if is_leaf(tag) {
                    return Some(false);
                }
            }
        }
    }
    None
}

fn ends_in_break(nodes: &[ParsedNode], ws: WhiteSpace) -> bool {
    last_shown_is_break(nodes, ws) == Some(true)
}

/// Empty the collapsible whitespace text after the last `<br>` in `nodes`,
/// which [`ends_in_break`] said is there. Returns whether the `<br>` was
/// reached.
fn strip_end(nodes: &mut [ParsedNode], ws: WhiteSpace) -> bool {
    for n in nodes.iter_mut().rev() {
        match n {
            ParsedNode::Text(t) => {
                // `ends_in_break` saw only collapsible text after the `<br>`.
                t.clear();
            }
            ParsedNode::Element {
                tag,
                attributes,
                children,
                ..
            } => {
                if is_dropped(tag) {
                    continue;
                }
                if tag == "br" {
                    return true;
                }
                let inner = ws.of(tag, attributes);
                if strip_end(children, inner) {
                    return true;
                }
            }
        }
    }
    false
}

/// HTML elements that are content with no children: a replaced or void
/// element other than `<br>` (and `<wbr>`, which shows nothing).
fn is_leaf(tag: &str) -> bool {
    matches!(
        tag,
        "img"
            | "input"
            | "embed"
            | "object"
            | "iframe"
            | "video"
            | "audio"
            | "canvas"
            | "svg"
            | "math"
            | "select"
            | "button"
            | "textarea"
            | "meter"
            | "progress"
            | "hr"
    )
}
