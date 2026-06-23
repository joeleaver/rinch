//! An ordered, sized, structurally-shared list of child nodes.
//!
//! A [`Fragment`] is the content of a node. It caches its total **size** in the
//! position space (the sum of its children's `node_size`) so that position math
//! is O(1) per level, and it is `Rc`-shared so cloning a fragment (and therefore
//! a node subtree) is a refcount bump — the basis for the persistent tree and the
//! cheap `Rc::ptr_eq` view diff (amendment A12).

use crate::model::Node;
use std::fmt;
use std::rc::Rc;

struct FragmentInner {
    children: Vec<Node>,
    /// Sum of `children[i].node_size()`. Cached so `size()` is O(1).
    size: usize,
}

/// The content (ordered children) of a node.
#[derive(Clone)]
pub struct Fragment(Rc<FragmentInner>);

impl Fragment {
    /// The empty fragment (no children, size 0).
    pub fn empty() -> Fragment {
        Fragment(Rc::new(FragmentInner {
            children: Vec::new(),
            size: 0,
        }))
    }

    /// Build a fragment from an ordered list of children, computing its size.
    pub fn from_children(children: Vec<Node>) -> Fragment {
        let size = children.iter().map(Node::node_size).sum();
        Fragment(Rc::new(FragmentInner { children, size }))
    }

    /// Build a single-child fragment.
    pub fn from_node(node: Node) -> Fragment {
        Fragment::from_children(vec![node])
    }

    /// Number of children.
    pub fn child_count(&self) -> usize {
        self.0.children.len()
    }

    /// True if there are no children.
    pub fn is_empty(&self) -> bool {
        self.0.children.is_empty()
    }

    /// Total size of the content in the position space.
    pub fn size(&self) -> usize {
        self.0.size
    }

    /// Child at index `i` (panics if out of range).
    pub fn child(&self, i: usize) -> &Node {
        &self.0.children[i]
    }

    /// Child at index `i`, or `None` if out of range.
    pub fn maybe_child(&self, i: usize) -> Option<&Node> {
        self.0.children.get(i)
    }

    /// The children as a slice.
    pub fn children(&self) -> &[Node] {
        &self.0.children
    }

    /// Iterate the children.
    pub fn iter(&self) -> std::slice::Iter<'_, Node> {
        self.0.children.iter()
    }

    /// Find the child index that a content position falls into.
    ///
    /// Returns `(index, offset)` where `offset` is the content position at the
    /// **start** of child `index`. Following ProseMirror's `findIndex` (round =
    /// -1): a position exactly on a child boundary resolves to the *gap after* the
    /// preceding child, i.e. `(index+1, end)`. Assumes `0 <= pos <= size`.
    pub fn find_index(&self, pos: usize) -> (usize, usize) {
        debug_assert!(
            pos <= self.0.size,
            "find_index position {pos} out of range (content size {})",
            self.0.size
        );
        if pos == 0 {
            return (0, 0);
        }
        if pos >= self.0.size {
            return (self.0.children.len(), self.0.size);
        }
        let mut cur = 0usize;
        for (i, child) in self.0.children.iter().enumerate() {
            let end = cur + child.node_size();
            if end >= pos {
                if end == pos {
                    return (i + 1, end);
                }
                return (i, cur);
            }
            cur = end;
        }
        // Unreachable for an in-range `pos`, but stay total.
        (self.0.children.len(), self.0.size)
    }

    /// Internal constructor with a pre-computed size (avoids the O(n) re-sum when
    /// the caller already knows the size — every content-surgery primitive does).
    fn from_parts(children: Vec<Node>, size: usize) -> Fragment {
        debug_assert_eq!(
            size,
            children.iter().map(Node::node_size).sum::<usize>(),
            "from_parts size mismatch"
        );
        Fragment(Rc::new(FragmentInner { children, size }))
    }

    /// Cut the content between two **content positions** `from..to`, slicing
    /// through text nodes and one level into non-text nodes where the boundary
    /// falls inside a child. A faithful port of ProseMirror's `Fragment.cut`.
    ///
    /// `from`/`to` are positions in this fragment's content space
    /// (`0..=self.size()`); the result is the content that lies strictly between
    /// them.
    pub fn cut(&self, from: usize, to: usize) -> Fragment {
        if from == 0 && to == self.0.size {
            return self.clone();
        }
        let mut result: Vec<Node> = Vec::new();
        let mut size = 0usize;
        if to > from {
            let mut pos = 0usize;
            for child in &self.0.children {
                if pos >= to {
                    break;
                }
                let end = pos + child.node_size();
                if end > from {
                    let piece = if pos < from || end > to {
                        if child.is_text() {
                            // char offsets within the text node
                            child.cut(from.saturating_sub(pos), (to - pos).min(child.text_len()))
                        } else {
                            // content offsets within the child (skip its open token)
                            child.cut(
                                from.saturating_sub(pos + 1),
                                (to - pos - 1).min(child.content_size()),
                            )
                        }
                    } else {
                        child.clone()
                    };
                    size += piece.node_size();
                    result.push(piece);
                }
                pos = end;
            }
        }
        Fragment::from_parts(result, size)
    }

    /// Concatenate `other` onto the end of this fragment, merging the boundary
    /// pair if both are text nodes with identical markup (so adjacent runs never
    /// fragment). Faithful port of ProseMirror's `Fragment.append`.
    pub fn append(&self, other: &Fragment) -> Fragment {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let mut content = self.0.children.clone();
        let first = &other.0.children[0];
        // `content` is non-empty (we returned early when `self` is empty), so the
        // last child always exists.
        let last_idx = content.len() - 1;
        let mut start = 0usize;
        if content[last_idx].is_text() && first.is_text() && content[last_idx].same_markup(first) {
            let merged = format!(
                "{}{}",
                content[last_idx].text().unwrap(),
                first.text().unwrap()
            );
            content[last_idx] = content[last_idx].with_text(merged.into());
            start = 1;
        }
        content.extend(other.0.children[start..].iter().cloned());
        Fragment::from_parts(content, self.0.size + other.0.size)
    }

    /// Return a copy with the child at `index` replaced by `node` (the rest
    /// structurally shared). Faithful port of `Fragment.replaceChild`.
    pub fn replace_child(&self, index: usize, node: Node) -> Fragment {
        let current = &self.0.children[index];
        if current.same_ref(&node) {
            return self.clone();
        }
        let size = self.0.size + node.node_size() - current.node_size();
        let mut copy = self.0.children.clone();
        copy[index] = node;
        Fragment::from_parts(copy, size)
    }
}

impl PartialEq for Fragment {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
            || (self.0.size == other.0.size && self.0.children == other.0.children)
    }
}
impl Eq for Fragment {}

impl fmt::Debug for Fragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.children.iter()).finish()
    }
}

impl Default for Fragment {
    fn default() -> Self {
        Fragment::empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::Schema;
    use crate::model::{Fragment, Mark};

    /// paragraph("abcd") — content size 4.
    fn para_abcd(s: &Schema) -> Fragment {
        s.branch("paragraph", Fragment::from_node(s.text("abcd").unwrap()))
            .unwrap()
            .content()
            .clone()
    }

    #[test]
    fn cut_text_within_a_single_run() {
        let s = Schema::starter_kit();
        let frag = para_abcd(&s); // [text "abcd"]
        // cut content positions 1..3 -> "bc"
        let c = frag.cut(1, 3);
        assert_eq!(c.size(), 2);
        assert_eq!(c.child(0).text(), Some("bc"));
    }

    #[test]
    fn cut_whole_is_identity_ref() {
        let s = Schema::starter_kit();
        let frag = para_abcd(&s);
        let c = frag.cut(0, frag.size());
        assert!(std::rc::Rc::ptr_eq(&frag.0, &c.0)); // shares the Rc
    }

    #[test]
    fn cut_empty_range_is_empty() {
        let s = Schema::starter_kit();
        let frag = para_abcd(&s);
        assert!(frag.cut(2, 2).is_empty());
    }

    #[test]
    fn cut_spanning_two_blocks_opens_them() {
        // doc content = [paragraph "ab" (0..4), paragraph "cd" (4..8)]
        let s = Schema::starter_kit();
        let p1 = s
            .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
            .unwrap();
        let p2 = s
            .branch("paragraph", Fragment::from_node(s.text("cd").unwrap()))
            .unwrap();
        let doc_content = Fragment::from_children(vec![p1, p2]);
        // cut 2..6 -> "b" from p1 and "c" from p2 (each block kept, sliced)
        let c = doc_content.cut(2, 6);
        assert_eq!(c.child_count(), 2);
        assert_eq!(c.child(0).child(0).text(), Some("b"));
        assert_eq!(c.child(1).child(0).text(), Some("c"));
    }

    #[test]
    fn append_merges_adjacent_text_with_same_marks() {
        let s = Schema::starter_kit();
        let a = Fragment::from_node(s.text("foo").unwrap());
        let b = Fragment::from_node(s.text("bar").unwrap());
        let merged = a.append(&b);
        assert_eq!(merged.child_count(), 1);
        assert_eq!(merged.child(0).text(), Some("foobar"));
        assert_eq!(merged.size(), 6);
    }

    #[test]
    fn append_does_not_merge_text_with_different_marks() {
        let s = Schema::starter_kit();
        let bold = Mark::simple(s.mark_type("bold").unwrap().clone());
        let a = Fragment::from_node(s.text("foo").unwrap());
        let b = Fragment::from_node(s.text_with_marks("bar", vec![bold]).unwrap());
        let merged = a.append(&b);
        assert_eq!(merged.child_count(), 2);
        assert_eq!(merged.size(), 6);
    }

    #[test]
    fn append_with_empty_is_identity() {
        let s = Schema::starter_kit();
        let a = Fragment::from_node(s.text("x").unwrap());
        assert!(std::rc::Rc::ptr_eq(&a.append(&Fragment::empty()).0, &a.0));
        assert!(std::rc::Rc::ptr_eq(&Fragment::empty().append(&a).0, &a.0));
    }

    #[test]
    fn replace_child_preserves_size_accounting() {
        let s = Schema::starter_kit();
        let p1 = s
            .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
            .unwrap();
        let p2 = s
            .branch("paragraph", Fragment::from_node(s.text("cde").unwrap()))
            .unwrap();
        let frag = Fragment::from_children(vec![p1, p2]); // 4 + 5 = 9
        assert_eq!(frag.size(), 9);
        let new_p = s
            .branch("paragraph", Fragment::from_node(s.text("z").unwrap()))
            .unwrap(); // size 3
        let replaced = frag.replace_child(0, new_p);
        assert_eq!(replaced.size(), 3 + 5);
        assert_eq!(replaced.child(0).child(0).text(), Some("z"));
    }
}
