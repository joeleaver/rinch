//! Resolving a [`Pos`] into a [`ResolvedPos`].
//!
//! A faithful port of ProseMirror's `Node.resolve` / `ResolvedPos`. Resolving a
//! position walks down the tree from the document root, recording at each depth
//! the node, the child index the position falls at, and the document position
//! just before that child. From that path we can answer all the geometric
//! questions the transform engine and selection need: the parent node, the offset
//! within it, and the positions before/after/inside any ancestor.

use crate::EditorError;
use crate::model::Node;
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
