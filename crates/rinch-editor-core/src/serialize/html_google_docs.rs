//! What the HTML reader knows about Google Docs' copies (#3096).
//!
//! Docs wraps everything it puts on the clipboard in
//! `<b style="font-weight:normal" id="docs-internal-guid-…">` and writes the
//! full character formatting of every run as CSS on a `<span>`, its defaults
//! included. Most defaults read as nothing already (`font-weight:400`,
//! `font-style:normal`, `text-decoration:none`, `vertical-align:baseline`,
//! `background-color:transparent`). The default text colour is the one that
//! does not: Docs writes `color:#000000` for text whose colour is
//! "automatic", which is not a colour the writer chose. Read as a
//! `text_color` mark, it would pin every pasted word black (unreadable on a
//! dark theme) where the editor's own text colour belongs.

use super::html_tree::ParsedNode;

/// The id prefix of the element Google Docs wraps a copy in.
const DOCS_GUID_PREFIX: &str = "docs-internal-guid-";

/// Whether `id` is the id Google Docs gives the element it wraps a copy in.
/// The tokenizer keeps this one `id` for [`is_google_docs_copy`].
pub(super) fn is_docs_guid(id: &str) -> bool {
    id.starts_with(DOCS_GUID_PREFIX)
}

/// Whether `forest` is a Google Docs copy: some element in it carries Docs'
/// `docs-internal-guid-…` id. The wrapper is near the top, after a
/// `<meta charset>` (and under `<html><body>` where a platform's clipboard
/// format adds them), so the walk stops at the first one it meets.
pub(super) fn is_google_docs_copy(forest: &[ParsedNode]) -> bool {
    let mut stack: Vec<&ParsedNode> = forest.iter().rev().collect();
    while let Some(node) = stack.pop() {
        let ParsedNode::Element {
            attributes,
            children,
            ..
        } = node
        else {
            continue;
        };
        if attributes.iter().any(|(k, v)| k == "id" && is_docs_guid(v)) {
            return true;
        }
        stack.extend(children.iter().rev());
    }
    false
}

/// Whether `colour`, a span's CSS `color`, is the colour Google Docs writes
/// for text with no colour of its own.
pub(super) fn is_docs_default_text_colour(colour: &str) -> bool {
    colour.trim().eq_ignore_ascii_case("#000000")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::html_tree::HtmlFragmentParser;

    fn forest(html: &str) -> Vec<ParsedNode> {
        HtmlFragmentParser::new(html).parse()
    }

    #[test]
    fn a_docs_copy_is_known_by_its_wrapper_id() {
        assert!(is_google_docs_copy(&forest(
            "<meta charset=\"utf-8\"><b style=\"font-weight:normal;\" \
             id=\"docs-internal-guid-1a2b\"><p>a</p></b>"
        )));
        assert!(is_google_docs_copy(&forest(
            "<html><body><!--StartFragment--><b id=\"docs-internal-guid-1\"><p>a</p></b></body></html>"
        )));
        assert!(!is_google_docs_copy(&forest(
            "<b style=\"font-weight:normal;\"><p>a</p></b>"
        )));
        assert!(!is_google_docs_copy(&forest("<p id=\"docs\">a</p>")));
    }

    #[test]
    fn only_docs_black_is_its_default_colour() {
        assert!(is_docs_default_text_colour("#000000"));
        assert!(is_docs_default_text_colour(" #000000 "));
        assert!(!is_docs_default_text_colour("#0000ff"));
        assert!(!is_docs_default_text_colour("#000001"));
    }
}
