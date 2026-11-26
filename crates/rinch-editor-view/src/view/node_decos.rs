//! The view's half of [`Decoration::Node`]: a CSS class list on the host element
//! of a whole node.
//!
//! Each [`ViewDesc`] remembers what was put on its own element
//! ([`HostClasses`]), so only the words the decorations added are ever taken
//! off, and a host built anew starts with nothing remembered and is written by
//! the pass that follows the document diff.
//!
//! [`Decoration::Node`]: rinch_editor_core::decoration::Decoration::Node

use rinch_core::dom::NodeHandle;
use rinch_editor_core::decoration::DecorationSet;

use super::ViewDesc;

/// One node decoration flattened for the view: the range it claims to span and
/// its class list.
#[derive(Clone, Debug)]
pub(super) struct NodeDeco {
    from: usize,
    to: usize,
    class: String,
}

/// The node decorations of `set`, sorted by `from` (stable, so decorations of
/// one node keep their draw order).
pub(super) fn node_decos(set: &DecorationSet) -> Vec<NodeDeco> {
    let mut out: Vec<NodeDeco> = set
        .iter()
        .filter_map(|d| {
            let class = d.node_class()?;
            let (from, to) = d.range();
            Some(NodeDeco {
                from: from.0,
                to: to.0,
                class: class.to_string(),
            })
        })
        .collect();
    out.sort_by_key(|d| d.from);
    out
}

/// What node decorations have put on one host element's `class` attribute.
#[derive(Debug, Default)]
pub(super) struct HostClasses {
    /// The words asked for, each once, in draw order.
    wanted: Vec<String>,
    /// Those of `wanted` the element did not already have: the only words that
    /// are ever removed from it.
    added: Vec<String>,
}

impl HostClasses {
    /// Make `el` carry `wanted` besides whatever else its class list holds.
    /// Writes nothing when `wanted` is what was asked for last time. Returns
    /// whether the element was written.
    fn set(&mut self, el: &NodeHandle, wanted: Vec<String>) -> bool {
        if self.wanted == wanted {
            return false;
        }
        let current = el.get_attribute("class");
        let before: Vec<&str> = current
            .as_deref()
            .unwrap_or("")
            .split_whitespace()
            .collect();
        let mut after: Vec<&str> = before
            .iter()
            .copied()
            .filter(|w| !self.added.iter().any(|a| a == w) || wanted.iter().any(|n| n == w))
            .collect();
        let mut added = Vec::new();
        for word in &wanted {
            if !after.contains(&word.as_str()) {
                after.push(word);
                added.push(word.clone());
            } else if self.added.contains(word) {
                added.push(word.clone());
            }
        }
        let write = after != before;
        if write {
            if after.is_empty() {
                el.remove_attribute("class");
            } else {
                el.set_attribute("class", &after.join(" "));
            }
        }
        self.added = added;
        self.wanted = wanted;
        write
    }

    /// The host element was replaced by `el` (a node view's inline host swap):
    /// put the same classes on the new one.
    pub(super) fn move_to(&mut self, el: &NodeHandle) {
        let wanted = std::mem::take(&mut self.wanted);
        self.added.clear();
        self.set(el, wanted);
    }
}

/// Re-project the node decorations over `desc`'s descendants. `decos` is sorted
/// by `from` and `content_start` is the position just inside `desc`'s content (0
/// for the document). Returns whether any descendant carries a decoration class
/// (the caller's [`ViewDesc::has_node_deco`]); `wrote` is set when any element
/// was written.
///
/// A decoration applies to the node that starts at its `from` and ends at its
/// `to`. A subtree is descended into only when a decoration starts inside it or
/// it still carries a class to take off.
pub(super) fn sync_node_decos(
    desc: &mut ViewDesc,
    decos: &[NodeDeco],
    content_start: usize,
    wrote: &mut bool,
) -> bool {
    let mut any = false;
    let mut pos = content_start;
    let mut next = 0;
    for child in &mut desc.children {
        let end = pos + child.node.node_size();
        while decos.get(next).is_some_and(|d| d.from < pos) {
            next += 1;
        }
        let first = next;
        while decos.get(next).is_some_and(|d| d.from < end) {
            next += 1;
        }
        let reaching = &decos[first..next];
        // A text run has no element of its own to carry a class.
        if !child.is_text && (!reaching.is_empty() || child.has_node_deco) {
            let at_start = reaching.partition_point(|d| d.from == pos);
            let (own, inside) = reaching.split_at(at_start);
            let mut wanted: Vec<String> = Vec::new();
            for deco in own.iter().filter(|d| d.to == end) {
                for word in deco.class.split_whitespace() {
                    if !wanted.iter().any(|w| w == word) {
                        wanted.push(word.to_string());
                    }
                }
            }
            let decorated = !wanted.is_empty();
            *wrote |= child.host_classes.set(&child.dom, wanted);
            let below = sync_node_decos(child, inside, pos + 1, wrote);
            child.has_node_deco = decorated || below;
        }
        any |= child.has_node_deco;
        pos = end;
    }
    any
}
