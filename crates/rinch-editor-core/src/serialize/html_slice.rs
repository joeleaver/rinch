//! The HTML reader's entry points: markup to a [`Slice`] for a load, an
//! import or a paste. The reader itself is [`HtmlParser`] in `html`; how a
//! line of pasted markup ends is `html_lines` (#3096).

use super::html::{DataAttrSource, HtmlParser};
use super::html_lines::{LineEnd, PasteEnd, take_interchange_newline};
use super::html_sources::drop_source_default_colours;
use super::html_tree::HtmlFragmentParser;
use crate::EditorError;
use crate::model::{Fragment, Node, Slice};
use crate::schema::Schema;

/// Parse an HTML fragment into a sanitized [`Slice`] against `schema`.
///
/// The result is structurally valid content, **open at each end down to the
/// textblock there** (`open_depth`): `<p>a</p><p>b</p>` is open through its
/// paragraphs, `<ul><li>a</li></ul>` through the list, its item and the
/// item's paragraph. Markup carries no open depths, so this is the reading
/// that suits a paste ([`Transaction::replace_selection`]): the first
/// textblock's content continues the line it is pasted on and the last one
/// takes the text after the caret, while everything between keeps its
/// structure. An edge that reaches no textblock (a rule, a table) is closed.
///
/// It does not fail on markup, and the content is always valid: what the
/// reader has no node for is read through for its text (an element that holds
/// a block is a container of blocks, any other is inline), table parts with
/// no `<table>` around them are a table, and what has no place in a table
/// goes in front of it. Unknown marks and attributes are dropped; dangerous
/// elements never materialize. The only errors are a schema with no
/// paragraph or list item type, and attributes the schema refuses.
///
/// A node whose spec keeps app data attributes ([`NodeSpec::data_attrs`])
/// keeps every one the element carries (bounded, [`kept_data_attrs`]); the
/// default paste reads with [`slice_from_pasted_html`] instead.
///
/// [`Transaction::replace_selection`]: crate::state::Transaction::replace_selection
/// [`NodeSpec::data_attrs`]: crate::schema::NodeSpec::data_attrs
/// [`kept_data_attrs`]: crate::schema::kept_data_attrs
pub fn slice_from_html(schema: &Schema, html: &str) -> Result<Slice, EditorError> {
    read_slice(schema, html, DataAttrSource::Any, PasteEnd::Continued)
}

/// [`slice_from_html`] for the default (unclaimed) paste: an element's app
/// data attributes are kept only when rinch's own copy-out wrote them
/// ([`CLIPBOARD_MARK`] on the element), so a copy between rinch editors keeps
/// them and another application's are dropped. A
/// [`Plugin::handle_paste`](crate::Plugin::handle_paste) that wants the
/// pasted markup's attributes reads them from `PasteContent::html` itself.
///
/// It reads as though text follows the caret ([`PasteEnd::Continued`]);
/// [`slice_from_pasted_html_ending`] reads for a known caret.
///
/// [`CLIPBOARD_MARK`]: super::CLIPBOARD_MARK
pub fn slice_from_pasted_html(schema: &Schema, html: &str) -> Result<Slice, EditorError> {
    slice_from_pasted_html_ending(schema, html, PasteEnd::Continued)
}

/// [`slice_from_pasted_html`] for a paste that ends as `end` says
/// ([`PasteEnd::at`] the selection's end), which the default paste reads
/// with (#3096). With [`PasteEnd::Ended`] a `<br>` that ends markup holding
/// blocks makes no line: `<p>a</p><br>` at the end of a paragraph is `a` and one empty
/// line, as in Chrome, where [`PasteEnd::Continued`] keeps the `<br>` to
/// break the line before the text after the caret. A copy that ends in
/// `<br class="Apple-interchange-newline">` ends in a paragraph end: the text
/// after the caret is a paragraph of its own (the slice is closed at its
/// end), and with nothing after the caret it adds no empty paragraph.
pub fn slice_from_pasted_html_ending(
    schema: &Schema,
    html: &str,
    end: PasteEnd,
) -> Result<Slice, EditorError> {
    read_slice(schema, html, DataAttrSource::MarkedOnly, end)
}

fn read_slice(
    schema: &Schema,
    html: &str,
    data: DataAttrSource,
    end: PasteEnd,
) -> Result<Slice, EditorError> {
    let mut forest = HtmlFragmentParser::new(html).parse();
    let paragraph_end = take_interchange_newline(&mut forest);
    drop_source_default_colours(&mut forest);
    let mut parser = HtmlParser::new(schema)?;
    parser.data_attrs = data;
    let has_blocks = parser.mark_blocks(&mut forest);
    // Markup of inline content alone is the one line that continues the
    // caret's, as a selection inside one line copies: its `<br>`s are kept
    // wherever it goes (Chrome pastes `a<br>` at a paragraph's end as `xa`
    // and an empty line), so rinch's own copy of `ab` and a break reads back.
    let line_end = if paragraph_end || (end == PasteEnd::Ended && has_blocks) {
        LineEnd::Closed
    } else {
        LineEnd::Open
    };
    let blocks = parser.parse_fragment(&forest, line_end)?;
    if blocks.is_empty() {
        return Ok(Slice::empty());
    }
    let open_start = open_depth(blocks.first());
    // A paragraph end with text after the caret: that text keeps a
    // paragraph of its own, so nothing is open at the end.
    let open_end = if paragraph_end && end == PasteEnd::Continued {
        0
    } else {
        open_depth_end(blocks.last())
    };
    Ok(Slice::new(
        Fragment::from_children(blocks),
        open_start,
        open_end,
    ))
}

/// How many nodes are open at the start of content whose first node is
/// `node`: every node down to and including the first textblock, when the
/// way there crosses no leaf and no isolating node (a table cell); else none.
fn open_depth(node: Option<&Node>) -> usize {
    edge_depth(node, |n| n.content().children().first())
}

/// [`open_depth`] for the end of content whose last node is `node`.
fn open_depth_end(node: Option<&Node>) -> usize {
    edge_depth(node, |n| n.content().children().last())
}

fn edge_depth(mut node: Option<&Node>, next: impl Fn(&Node) -> Option<&Node>) -> usize {
    let mut depth = 0;
    while let Some(n) = node {
        if n.is_textblock() {
            return depth + 1;
        }
        if n.is_leaf() || n.node_type().is_isolating() {
            return 0;
        }
        depth += 1;
        node = next(n);
    }
    0
}
