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
