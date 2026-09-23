//! Resolving a [`Pos`] into a [`ResolvedPos`].
//!
//! A faithful port of ProseMirror's `Node.resolve` / `ResolvedPos`. Resolving a
//! position walks down the tree from the document root, recording at each depth
//! the node, the child index the position falls at, and the document position
//! just before that child. From that path we can answer all the geometric
//! questions the transform engine and selection need: the parent node, the offset
//! within it, and the positions before/after/inside any ancestor.

use crate::EditorError;
use crate::model::{Mark, Node};
use crate::pos::Pos;

/// One level of the resolved path.
#[derive(Clone, Debug)]
struct PathEntry {
    /// The node at this depth.
    node: Node,
    /// The index, within this node's content, of the child the position falls at.
    index: usize,
    /// The document position immediately before that child (i.e. before
    /// `node`'s child #`index`). For an ancestor we descended through, this is the
    /// open-token position of the next deeper node.
    before: usize,
}

/// A position resolved against a document: its ancestor path, depth, and the
/// offset within the immediate parent node.
#[derive(Clone, Debug)]
pub struct ResolvedPos {
    pos: Pos,
    /// One entry per depth, `path[0]` = the document root.
    path: Vec<PathEntry>,
    depth: usize,
    /// Offset (in the parent's content position space) of the resolved position
    /// within its immediate parent — i.e. within a text node, the char offset.
    parent_offset: usize,
}

impl ResolvedPos {
    /// Resolve `pos` interpreting `doc` as the document root.
    pub(crate) fn resolve(doc: &Node, pos: Pos) -> Result<ResolvedPos, EditorError> {
        let target = pos.0;
        let content_size = doc.content_size();
        if target > content_size {
            return Err(EditorError::InvalidPosition(target, content_size));
        }

        let mut path: Vec<PathEntry> = Vec::new();
        let mut start = 0usize; // content-start of the current node
        let mut parent_offset = target;
        let mut node = doc.clone();

        loop {
            let (index, offset) = node.content().find_index(parent_offset);
            let rem = parent_offset - offset;
            path.push(PathEntry {
                node: node.clone(),
                index,
                before: start + offset,
            });
            if rem == 0 {
                break;
            }
            let child = node.child(index).clone();
            if child.is_text() {
                // The position is inside this text node; the immediate parent is
                // `node`, and `parent_offset` is the offset within its content.
                break;
            }
            parent_offset = rem - 1; // step past the child's open token
            start += offset + 1;
            node = child;
        }

        let depth = path.len() - 1;
        Ok(ResolvedPos {
            pos,
            path,
            depth,
            parent_offset,
        })
    }

    /// The resolved position.
    pub fn pos(&self) -> Pos {
        self.pos
    }

    /// The depth (0 = directly in the document root).
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// The immediate parent node of the position.
    pub fn parent(&self) -> &Node {
        &self.path[self.depth].node
    }

    /// The ancestor node at `depth`.
    pub fn node(&self, depth: usize) -> &Node {
        &self.path[depth].node
    }

    /// The child index, within the node at `depth`, that the position falls at.
    pub fn index(&self, depth: usize) -> usize {
        self.path[depth].index
    }

    /// The child index *after* the position within the node at `depth`. Equal to
    /// [`Self::index`] except at the deepest level when the position sits inside a
    /// text node (then it is the next index). Port of `ResolvedPos.indexAfter`.
    pub fn index_after(&self, depth: usize) -> usize {
        self.index(depth)
            + if depth == self.depth && self.text_offset() == 0 {
                0
            } else {
                1
            }
    }

    /// The offset of the position within its immediate parent's content (within a
    /// text node, the char offset).
    pub fn parent_offset(&self) -> usize {
        self.parent_offset
    }

    /// The document position where the content of the node at `depth` begins.
    pub fn start(&self, depth: usize) -> usize {
        if depth == 0 {
            0
        } else {
            self.path[depth - 1].before + 1
        }
    }

    /// The document position where the content of the node at `depth` ends.
    pub fn end(&self, depth: usize) -> usize {
        self.start(depth) + self.node(depth).content_size()
    }

    /// The document position directly before the node at `depth` (its open
    /// token). `None` at depth 0 (the doc has no boundary token).
    pub fn before(&self, depth: usize) -> Option<usize> {
        if depth == 0 {
            None
        } else {
            Some(self.path[depth - 1].before)
        }
    }

    /// The document position directly after the node at `depth`. `None` at depth 0.
    pub fn after(&self, depth: usize) -> Option<usize> {
        self.before(depth).map(|b| b + self.node(depth).node_size())
    }

    /// The offset of the position within the deepest child boundary — i.e. the
    /// char offset into the text node the position sits in, or `0` at a non-text
    /// boundary. Port of `ResolvedPos.textOffset`.
    pub fn text_offset(&self) -> usize {
        self.pos.0 - self.path[self.depth].before
    }

    /// The node directly after the position within its parent, cut at the position
    /// if it falls inside a text node; `None` at the parent's end. Port of
    /// `ResolvedPos.nodeAfter`.
    pub fn node_after(&self) -> Option<Node> {
        let parent = self.parent();
        let index = self.index(self.depth);
        if index == parent.child_count() {
            return None;
        }
        let d_off = self.text_offset();
        let child = parent.child(index);
        if d_off > 0 {
            let to = if child.is_text() {
                child.text_len()
            } else {
                child.content_size()
            };
            Some(child.cut(d_off, to))
        } else {
            Some(child.clone())
        }
    }

    /// The node directly before the position within its parent, cut at the
    /// position if it falls inside a text node; `None` at the parent's start. Port
    /// of `ResolvedPos.nodeBefore`.
    pub fn node_before(&self) -> Option<Node> {
        let index = self.index(self.depth);
        let d_off = self.text_offset();
        if d_off > 0 {
            Some(self.parent().child(index).cut(0, d_off))
        } else if index == 0 {
            None
        } else {
            Some(self.parent().child(index - 1).clone())
        }
    }

    /// The deepest ancestor whose content range contains both this position and
    /// `pos` — the depth at which a slice between the two stays closed. Port of
    /// `ResolvedPos.sharedDepth`.
    pub fn shared_depth(&self, pos: Pos) -> usize {
        let p = pos.0;
        let mut depth = self.depth;
        while depth > 0 {
            if self.start(depth) <= p && self.end(depth) >= p {
                return depth;
            }
            depth -= 1;
        }
        0
    }

    /// The set of marks active *at* this position — the marks a character typed
    /// here would inherit. Port of `ResolvedPos.marks`, including the `inclusive`
    /// spec flag ([`MarkSpec::inclusive`](crate::MarkSpec::inclusive)):
    ///
    /// - **Inside** a text node: that node's own marks, inclusive or not.
    /// - **At a boundary** between two inline nodes: the marks of the node *before*
    ///   the position, minus every non-inclusive mark the node *after* does not carry
    ///   too. So a caret right after a link (a non-inclusive mark) reports no link,
    ///   while one right after bold text reports bold.
    /// - **At a link's start** mid-paragraph: the node before decides, as it does for
    ///   every mark, so the link is not reported there either.
    /// - **At the start** of a textblock (no node before): the marks of the node after,
    ///   minus every non-inclusive one (there is nothing before it to continue).
    pub fn marks(&self) -> Vec<Mark> {
        let parent = self.parent();
        if parent.content().size() == 0 {
            return Vec::new();
        }
        if self.text_offset() > 0 {
            return parent.child(self.index(self.depth)).marks().to_vec();
        }
        let index = self.index(self.depth);
        let before = index
            .checked_sub(1)
            .and_then(|i| parent.content().maybe_child(i));
        let after = parent.content().maybe_child(index);
        let (main, other) = match before {
            Some(b) => (Some(b), after),
            None => (after, None),
        };
        let Some(main) = main else {
            return Vec::new();
        };
        main.marks()
            .iter()
            .filter(|m| m.typ.spec().inclusive || other.is_some_and(|o| m.is_in(o.marks())))
            .cloned()
            .collect()
    }

    /// If the position is inside (or at the left edge of) a text node, return that
    /// node and the char offset within it; otherwise `None`.
    ///
    /// Seam rule: a position exactly between two adjacent text runs resolves to
    /// `Some(next_run, 0)` (the left edge of the following run); a position at the
    /// end of the last run, or at a non-text boundary, resolves to `None`. M2's
    /// cursor-mark and split logic relies on this defined rule.
    pub fn text_node(&self) -> Option<(&Node, usize)> {
        let parent = self.parent();
        let idx = self.index(self.depth);
        let child = parent.content().maybe_child(idx)?;
        if !child.is_text() {
            return None;
        }
        let start = self.start(self.depth);
        let before = self.path[self.depth].before;
        debug_assert!(before >= start, "text_node: content position underflow");
        let child_start_offset = before - start;
        Some((child, self.parent_offset - child_start_offset))
    }
}

#[cfg(test)]
mod tests {
    use crate::model::{Attrs, Fragment, Mark, Node};
    use crate::pos::Pos;
    use crate::{AttrValue, Schema};

    fn link(s: &Schema, href: &str) -> Mark {
        let attrs = Attrs::from_iter([("href", AttrValue::from(href.to_string()))]);
        Mark::new(s.mark_type("link").unwrap().clone(), attrs)
    }

    fn bold(s: &Schema) -> Mark {
        Mark::simple(s.mark_type("bold").unwrap().clone())
    }

    /// A one-paragraph doc from `(text, marks)` runs. The paragraph opens at 0, so
    /// its content starts at position 1.
    fn doc(s: &Schema, runs: Vec<(&str, Vec<Mark>)>) -> Node {
        let inline: Vec<Node> = runs
            .into_iter()
            .map(|(t, m)| s.text_with_marks(t, m).unwrap())
            .collect();
        let p = s
            .branch("paragraph", Fragment::from_children(inline))
            .unwrap();
        s.branch("doc", Fragment::from_node(p)).unwrap()
    }

    fn names(d: &Node, pos: usize) -> Vec<String> {
        d.resolve(Pos(pos))
            .unwrap()
            .marks()
            .iter()
            .map(|m| m.type_name().to_string())
            .collect()
    }

    // "see " 1..5, link "here" 5..9, " now" 9..13.
    fn linked(s: &Schema, m: Vec<Mark>) -> Node {
        doc(s, vec![("see ", vec![]), ("here", m), (" now", vec![])])
    }

    #[test]
    fn a_link_is_reported_inside_it_but_not_at_either_edge() {
        let s = Schema::starter_kit();
        let d = linked(&s, vec![link(&s, "https://a.example")]);
        assert!(
            names(&d, 5).is_empty(),
            "at its start the text before decides"
        );
        assert_eq!(names(&d, 6), ["link"], "inside");
        assert_eq!(names(&d, 8), ["link"], "before its last character");
        assert!(names(&d, 9).is_empty(), "right after its last character");
    }

    #[test]
    fn bold_stays_inclusive_at_its_end() {
        let s = Schema::starter_kit();
        let d = linked(&s, vec![bold(&s)]);
        assert!(
            names(&d, 5).is_empty(),
            "at its start the text before decides"
        );
        assert_eq!(names(&d, 9), ["bold"], "right after its last character");
    }

    #[test]
    fn at_a_link_end_the_inclusive_marks_on_it_are_kept() {
        let s = Schema::starter_kit();
        let d = linked(&s, vec![bold(&s), link(&s, "https://a.example")]);
        assert_eq!(names(&d, 9), ["bold"]);
    }

    #[test]
    fn a_link_that_goes_on_after_the_boundary_is_kept() {
        // The same link, with a bold part: the boundary between the two runs is
        // inside the link, not at its end.
        let s = Schema::starter_kit();
        let l = link(&s, "https://a.example");
        let d = doc(&s, vec![("ab", vec![l.clone()]), ("cd", vec![bold(&s), l])]);
        assert_eq!(names(&d, 3), ["link"]);
    }

    #[test]
    fn a_different_link_after_the_boundary_does_not_continue_the_first() {
        let s = Schema::starter_kit();
        let d = doc(
            &s,
            vec![
                ("ab", vec![link(&s, "https://a.example")]),
                ("cd", vec![link(&s, "https://b.example")]),
            ],
        );
        assert!(names(&d, 3).is_empty());
    }

    #[test]
    fn at_a_textblock_edge_a_link_is_not_reported() {
        let s = Schema::starter_kit();
        let d = doc(
            &s,
            vec![("here", vec![bold(&s), link(&s, "https://a.example")])],
        );
        assert_eq!(
            names(&d, 1),
            ["bold"],
            "at the start: the node after, minus the link"
        );
        assert_eq!(
            names(&d, 5),
            ["bold"],
            "at the end: the node before, minus the link"
        );
    }
}
