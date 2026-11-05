//! How deep a document could nest, counted before it is parsed.
//!
//! html5ever's tree builder slows with depth (it walks its stack of open
//! elements: 320 KB of unclosed `<div>`s took 6 s), so [`open_bound`] runs
//! first, in linear time, and says where more than [`MAX_OPEN`] elements could
//! be open at once; [`cap`](crate::depth) cuts the input there.
//!
//! It is driven by html5ever's own tokenizer, so tags, quoted attributes,
//! comments (abrupt and bogus ones too) and character references are read as
//! the parser reads them, and it switches the tokenizer into raw text for
//! `<style>`, `<title>`, `<textarea>`, `<script>` and the rest exactly where
//! the tree builder would: outside SVG and MathML. Its count of open elements
//! is generous, never short:
//!
//! - every start tag of an element that is not void opens one (`<div/>` too;
//!   in SVG or MathML a self-closing tag opens nothing);
//! - an end tag closes only an element of its name it can reach: one of the
//!   top 64, with no table, cell, caption, template or object in between;
//! - formatting elements closed out from under a formatting element are
//!   reopened (`<b><i></b>` leaves an `<i>` open), as the parser reopens them;
//! - the common implied closes close down to their element, within its bound
//!   (a `<td>` the open cell, an `<li>` the open item); a new `<option>`
//!   closes only an option on top.

use std::cell::{Cell, RefCell};

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// Past this many elements that could be open at once, the input is cut.
pub const MAX_OPEN: usize = 1024;

/// How far down the stack an end tag or an implied close looks.
const REACH: usize = 64;

/// The input is fed in pieces of about this many bytes; a cut falls at the
/// start of the piece where the bound was passed (a little early, never
/// late).
const PIECE: usize = 1024;

/// Where in `document` more than [`MAX_OPEN`] elements could be open at
/// once, if anywhere (see the module docs).
pub(crate) fn open_bound(document: &str) -> Option<usize> {
    let tokenizer = Tokenizer::new(Counter::default(), TokenizerOpts::default());
    let queue = BufferQueue::default();
    let mut at = 0;
    while at < document.len() {
        let mut end = (at + PIECE).min(document.len());
        while !document.is_char_boundary(end) {
            end += 1;
        }
        queue.push_back(StrTendril::from_slice(&document[at..end]));
        // The sink never asks for a script to run, so one feed reads the
        // whole piece.
        let _ = tokenizer.feed(&queue);
        if tokenizer.sink.over.get() {
            return Some(at);
        }
        at = end;
    }
    tokenizer.end();
    None
}

/// An open element, as far as the count needs it.
struct Open {
    name: String,
    /// In SVG or MathML, where tags are not HTML's.
    foreign: bool,
}

#[derive(Default)]
struct Counter {
    stack: RefCell<Vec<Open>>,
    over: Cell<bool>,
}

impl Counter {
    /// Whether what follows the top of the stack is foreign content.
    fn in_foreign(&self) -> bool {
        self.stack
            .borrow()
            .last()
            .is_some_and(|o| o.foreign && !is_integration_point(&o.name))
    }

    fn start(&self, tag: &Tag) -> TokenSinkResult<()> {
        let name = tag.name.to_string();
        let mut foreign = self.in_foreign();
        if foreign && breaks_out_of_foreign(&name) {
            // The parser pops back to HTML content first.
            let mut stack = self.stack.borrow_mut();
            while stack.last().is_some_and(|o| o.foreign) {
                stack.pop();
            }
            foreign = false;
        }
        let opens_foreign = !foreign && matches!(name.as_str(), "svg" | "math");
        if foreign || opens_foreign {
            if !tag.self_closing {
                self.push(name, true);
            }
            return TokenSinkResult::Continue;
        }
        if is_void(&name) {
            return TokenSinkResult::Continue;
        }
        close_implied(&mut self.stack.borrow_mut(), &name);
        let raw = match name.as_str() {
            "title" | "textarea" => Some(TokenSinkResult::RawData(RawKind::Rcdata)),
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                Some(TokenSinkResult::RawData(RawKind::Rawtext))
            }
            "script" => Some(TokenSinkResult::RawData(RawKind::ScriptData)),
            "plaintext" => Some(TokenSinkResult::Plaintext),
            _ => None,
        };
        self.push(name, false);
        raw.unwrap_or(TokenSinkResult::Continue)
    }

    fn push(&self, name: String, foreign: bool) {
        let mut stack = self.stack.borrow_mut();
        stack.push(Open { name, foreign });
        if stack.len() > MAX_OPEN {
            self.over.set(true);
        }
    }

    fn end(&self, tag: &Tag) {
        let name = tag.name.to_string();
        let mut stack = self.stack.borrow_mut();
        let reach = stack
            .iter()
            .rev()
            .take(REACH)
            .position(|o| o.name == name || is_scope_boundary(&o.name));
        let Some(from_top) = reach else {
            return;
        };
        let at = stack.len() - 1 - from_top;
        if stack[at].name != name {
            return;
        }
        // Formatting elements closed out from under a formatting element are
        // reopened and stay open; under a block they are only rebuilt inside
        // what follows it.
        let reopened: Vec<Open> = if is_formatting(&name) {
            stack
                .drain(at + 1..)
                .filter(|o| is_formatting(&o.name))
                .collect()
        } else {
            Vec::new()
        };
        stack.truncate(at);
        stack.extend(reopened);
        if stack.len() > MAX_OPEN {
            self.over.set(true);
        }
    }
}

impl TokenSink for Counter {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        match token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => self.start(&tag),
            Token::TagToken(tag) => {
                self.end(&tag);
                TokenSinkResult::Continue
            }
            _ => TokenSinkResult::Continue,
        }
    }

    /// `<![CDATA[` is a section only in foreign content.
    fn adjusted_current_node_present_but_not_in_html_namespace(&self) -> bool {
        self.in_foreign()
    }
}

/// Close what opening `name` closes in the parser: the nearest open element
/// of the kinds it ends, with everything above it, unless one of the
/// elements that bound it comes first.
fn close_implied(stack: &mut Vec<Open>, name: &str) {
    let (ends, bounds): (&[&str], &[&str]) = match name {
        "td" | "th" => (&["td", "th"], &["table"]),
        "tr" => (&["tr"], &["table"]),
        "tbody" | "thead" | "tfoot" => (&["tbody", "thead", "tfoot"], &["table"]),
        "li" => (&["li"], &["ul", "ol", "table", "td", "th"]),
        "dt" | "dd" => (&["dt", "dd"], &["dl", "table", "td", "th"]),
        // Only an option that is the current node.
        "option" | "optgroup" => {
            if stack.last().is_some_and(|o| o.name == "option") {
                stack.pop();
            }
            return;
        }
        "p" | "div" | "table" | "ul" | "ol" | "dl" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        | "blockquote" | "pre" | "center" | "section" | "header" | "footer" | "form" => (
            &["p"],
            &["table", "td", "th", "button", "caption", "template", "html"],
        ),
        _ => return,
    };
    let found = stack
        .iter()
        .rev()
        .take(REACH)
        .position(|o| ends.contains(&o.name.as_str()) || bounds.contains(&o.name.as_str()));
    if let Some(from_top) = found {
        let at = stack.len() - 1 - from_top;
        if ends.contains(&stack[at].name.as_str()) {
            stack.truncate(at);
        }
    }
}

/// The formatting elements HTML's parser reopens when they are misnested.
fn is_formatting(name: &str) -> bool {
    matches!(
        name,
        "a" | "b"
            | "big"
            | "code"
            | "em"
            | "font"
            | "i"
            | "nobr"
            | "s"
            | "small"
            | "strike"
            | "strong"
            | "tt"
            | "u"
    )
}

fn is_scope_boundary(name: &str) -> bool {
    matches!(
        name,
        "table" | "td" | "th" | "caption" | "template" | "object" | "applet" | "marquee" | "html"
    )
}

/// Where SVG or MathML holds HTML content again.
fn is_integration_point(name: &str) -> bool {
    matches!(
        name,
        "foreignObject"
            | "foreignobject"
            | "desc"
            | "title"
            | "mi"
            | "mo"
            | "mn"
            | "ms"
            | "mtext"
            | "annotation-xml"
    )
}

/// HTML start tags that end foreign content in the parser.
fn breaks_out_of_foreign(name: &str) -> bool {
    matches!(
        name,
        "b" | "big"
            | "blockquote"
            | "body"
            | "br"
            | "center"
            | "code"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "em"
            | "embed"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "hr"
            | "i"
            | "img"
            | "li"
            | "listing"
            | "menu"
            | "meta"
            | "nobr"
            | "ol"
            | "p"
            | "pre"
            | "ruby"
            | "s"
            | "small"
            | "span"
            | "strong"
            | "strike"
            | "sub"
            | "sup"
            | "table"
            | "tt"
            | "u"
            | "ul"
            | "var"
    )
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "source"
            | "track"
            | "wbr"
    )
}

#[cfg(test)]
mod tests {
    use super::open_bound;

    #[test]
    fn real_mails_unclosed_cells_and_paragraphs_do_not_count_as_depth() {
        let rows = "<tr><td><p>one<td><p>two<li>x<li>y".repeat(5_000);
        let sloppy = "<div><font>x</div><p><b>y<i>z</b></i>".repeat(3_000);
        let select = "<select><option>a<option>b</select>".repeat(2_000);
        let svg = "<svg><title>t</title><path/><path/></svg><style>p{}</style>".repeat(2_000);
        let doc = format!("<table>{rows}</table><font><b>bold</font></b>{sloppy}{select}{svg}");
        assert_eq!(open_bound(&doc), None);
    }

    #[test]
    fn every_known_way_past_the_count_is_counted() {
        // Each nests deep in the parser. A prefix that fooled the hand
        // scanner of the last round, then ~10 KB of `<div>`s; untimed, so a
        // debug build checks the mechanism.
        for prefix in [
            "",
            "<!-->",
            r#"<p a="><!--">"#,
            r#"<p a="><style>">"#,
            "<svg><style>",
            "<math><style>",
            "<svg><title>",
            "<select><option>",
            "<b><i></b>",
            "<template>",
        ] {
            let doc = format!("{prefix}{}", "<div>".repeat(2_000));
            assert!(open_bound(&doc).is_some(), "{prefix:?} was not counted");
        }
        for (unit, n) in [
            ("<div/>", 2_000),
            ("<span><table><tbody><tr><td></span>", 400),
            ("<div><table><tr><td></div>", 400),
            ("<b><i></b>", 2_000),
            ("<template><div>", 1_000),
            ("<option><div>", 2_000),
        ] {
            assert!(
                open_bound(&unit.repeat(n)).is_some(),
                "{unit} was not counted"
            );
        }
    }
}
