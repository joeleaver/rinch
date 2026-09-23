//! Which link a character belongs to — the model half of link activation and
//! link hover.
//!
//! A pointer is over a *character*, not over a caret position, and the two
//! differ exactly at a link's edges. [`ResolvedPos::marks`] answers what a
//! caret at a position would inherit — and a link is non-inclusive, so a caret
//! at either edge of one inherits no link — while the character on one side of
//! each edge is linked: the one starting at the link's start, and the one
//! ending at its end. [`link_at`] therefore asks about the character that
//! **starts** at `pos` — the inline content covering `pos..pos + 1` — and never
//! about the marks a caret at `pos` would inherit.
//!
//! [`ResolvedPos::marks`]: crate::ResolvedPos::marks

use crate::model::{Mark, Node};
use crate::pos::Pos;

/// One link as the reader sees it: the whole run of adjacent inline content
/// carrying a `link` mark with the same `href`.
///
/// The run is taken across every other mark change, so a link whose middle
/// word is bold is **one** span, not three. Two adjacent links with different
/// `href`s are two spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    /// The link's `href` attribute.
    pub href: String,
    /// The link's `title` attribute, `None` when absent or empty. Taken from
    /// the character the lookup started at.
    pub title: Option<String>,
    /// The document position before the run's first character.
    pub from: Pos,
    /// The document position after the run's last character.
    pub to: Pos,
}

/// The `link` mark on an inline node, if it carries one.
fn link_mark(node: &Node) -> Option<&Mark> {
    node.marks().iter().find(|m| m.type_name() == "link")
}

/// The `href` of the `link` mark on an inline node.
fn link_href(node: &Node) -> Option<&str> {
    link_mark(node).and_then(|m| m.attrs.get_str("href"))
}

/// The link carrying the character that **starts at** `pos` (the inline
/// content covering `pos..pos + 1`), as the whole run of adjacent content with
/// the same `href`.
///
/// `None` when that character carries no `link` mark (or one without an
/// `href`), when `pos` is not inside a textblock's content, and at the end of a
/// textblock (there is no character there). In particular a position just
/// **after** a link's last character is not on the link, and the position at
/// its **start** is, although a caret at either reports no link (a link is
/// non-inclusive): pointer interaction is about the character under the
/// pointer, so a platform asks for the position before that character.
///
/// The mark is found by its name, `"link"`, as
/// `EditorHandle::active_link_href` finds it; a schema without one has no
/// links.
pub fn link_at(doc: &Node, pos: Pos) -> Option<LinkSpan> {
    let resolved = doc.resolve(pos).ok()?;
    let parent = resolved.parent();
    if !parent.is_textblock() {
        return None;
    }
    let content_start = resolved.start(resolved.depth());
    let offset = resolved.parent_offset();
    let children = parent.content().children();

    // The child covering `offset..offset + 1`, and where it starts.
    let mut start = 0;
    let mut hit = None;
    for (i, child) in children.iter().enumerate() {
        let end = start + child.node_size();
        if offset < end {
            hit = Some((i, start));
            break;
        }
        start = end;
    }
    let (index, hit_start) = hit?;
    let mark = link_mark(&children[index])?;
    let href = mark.attrs.get_str("href")?;

    let mut from = hit_start;
    let mut first = index;
    while first > 0 && link_href(&children[first - 1]) == Some(href) {
        first -= 1;
        from -= children[first].node_size();
    }
    let mut to = hit_start + children[index].node_size();
    let mut last = index;
    while last + 1 < children.len() && link_href(&children[last + 1]) == Some(href) {
        last += 1;
        to += children[last].node_size();
    }

    Some(LinkSpan {
        href: href.to_string(),
        title: mark
            .attrs
            .get_str("title")
            .filter(|t| !t.is_empty())
            .map(str::to_string),
        from: Pos(content_start + from),
        to: Pos(content_start + to),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::serialize::slice_from_html;

    fn doc(html: &str) -> Node {
        let schema = Schema::starter_kit();
        let slice = slice_from_html(&schema, html).expect("parses");
        schema.branch("doc", slice.content).expect("a document")
    }

    // `<p>go <a>here</a> now</p>`: "go " is 1..4, "here" 4..8, " now" 8..12.
    const PLAIN: &str = r#"<p>go <a href="pimble:a/b" title="B">here</a> now</p>"#;

    #[test]
    fn every_character_of_a_link_answers_the_whole_run() {
        let d = doc(PLAIN);
        for pos in 4..8 {
            let span = link_at(&d, Pos(pos)).unwrap_or_else(|| panic!("{pos} is on the link"));
            assert_eq!(span.href, "pimble:a/b");
            assert_eq!(span.title.as_deref(), Some("B"));
            assert_eq!((span.from, span.to), (Pos(4), Pos(8)), "at {pos}");
        }
    }

    #[test]
    fn the_position_after_the_last_character_is_not_on_the_link() {
        let d = doc(PLAIN);
        assert_eq!(link_at(&d, Pos(8)), None, "the space after the link");
        assert_eq!(link_at(&d, Pos(3)), None, "the space before the link");
    }

    #[test]
    fn a_link_that_ends_its_paragraph_has_nothing_after_it() {
        let d = doc(r#"<p>go <a href="x">here</a></p><p>next</p>"#);
        assert_eq!(link_at(&d, Pos(7)).map(|s| s.to), Some(Pos(8)));
        assert_eq!(link_at(&d, Pos(8)), None, "the paragraph's end");
        assert_eq!(link_at(&d, Pos(9)), None, "between paragraphs");
        assert_eq!(link_at(&d, Pos(10)), None, "the next paragraph");
    }

    #[test]
    fn a_link_partly_bold_is_one_span() {
        // "a " 1..3, "one " 3..7, "two" 7..10 (bold), " three" 10..16, "!" 16..17.
        let d = doc(r#"<p>a <a href="u">one <strong>two</strong> three</a>!</p>"#);
        for pos in [3, 7, 9, 15] {
            let span = link_at(&d, Pos(pos)).expect("on the link");
            assert_eq!((span.from, span.to), (Pos(3), Pos(16)), "at {pos}");
        }
        assert_eq!(link_at(&d, Pos(16)), None);
    }

    #[test]
    fn adjacent_links_with_different_hrefs_are_two_spans() {
        let d = doc(r#"<p><a href="a">ab</a><a href="b">cd</a></p>"#);
        let first = link_at(&d, Pos(2)).expect("on a");
        assert_eq!(
            (first.href.as_str(), first.from, first.to),
            ("a", Pos(1), Pos(3))
        );
        let second = link_at(&d, Pos(3)).expect("on b");
        assert_eq!(
            (second.href.as_str(), second.from, second.to),
            ("b", Pos(3), Pos(5))
        );
    }

    #[test]
    fn an_empty_title_is_none_and_positions_outside_text_are_none() {
        let d = doc(r#"<ul><li><p><a href="x">in</a></p></li></ul>"#);
        let span = link_at(&d, Pos(3)).expect("a link in a list item");
        assert_eq!(span.title, None);
        assert_eq!((span.from, span.to), (Pos(3), Pos(5)));
        assert_eq!(link_at(&d, Pos(0)), None, "before the list");
        assert_eq!(link_at(&d, Pos(1)), None, "inside the list, outside text");
        assert_eq!(link_at(&d, Pos(999)), None, "past the document");
    }
}
