//! The **scoped** IFC structural pass.
//!
//! # What it replaces
//!
//! A structural change — an append, a removal, a reorder, an `if` toggle, a
//! component re-render, a `display` or `position` flip — used to set one
//! document-global flag, `NodeTree::ifc_dirty`, and the next layout re-ran the
//! whole structural pass over the **whole document**: every `display: contents`
//! wrapper re-spliced (dirtying every row that held one), every anonymous
//! block box destroyed and minted again, every split inline restored and split
//! again, every measure leaf rebuilt, every IFC root re-marked, every text
//! context rewritten and a separate Taffy compute run for **every** atomic
//! inline in the document. Appending one row to a 40-row list with a chip per
//! row cost 40 inline-block computes; at 2000 rows the pass dominated the
//! frame.
//!
//! # The unit of work: a formatting container and its region
//!
//! Every decision that pass makes about a node is taken **by one formatting
//! container**, from facts about that container's *region* and nothing else:
//!
//! - a **formatting container** is any node whose children lay out in a
//!   context of its own: a block container, a flex or grid container, an
//!   atomic inline, an out-of-flow box, a `display: none` element, the
//!   document. [`is_formatting_container`] is the predicate. It is exactly the
//!   complement of the "descends" test `recompute_contributes_in_flow_block`,
//!   `collect_run_units` and `mark_inline_descendants` all walk by: a
//!   `display: contents` wrapper or a non-atomic `display: inline` element is
//!   *transparent* — the walk goes through it — and anything else stops it.
//! - a container's **region** is every node reached from its children by
//!   walking through transparent nodes: its inline content, its wrappers, the
//!   out-of-flow boxes hoisted to it, and the **stop nodes** — the formatting
//!   containers directly beneath it (a row in a list, a chip in a paragraph, a
//!   block inside a split inline). A stop node's *own* interior belongs to its
//!   own region; its *role* in this one (its `ifc_root` mark, its hoisting, its
//!   place in a run) belongs here.
//!
//! What depends on what, checked pass by pass:
//!
//! | pass | decided per | reads |
//! |---|---|---|
//! | `sync_display_contents` | the wrapper's nearest box-generating ancestor | the wrapper chain, all inside one region |
//! | `recompute_contributes_in_flow_block` | each region node | its children in the region; a container resets the context |
//! | anonymous boxes, splits, hoisting | the container | its run units, which are its region |
//! | IFC roots, measure leaves, marking | the container (and its boxes) | its units; marking stops at stop nodes |
//! | content signatures | each root | its members, all in its container's region |
//!
//! A formatting container's own role never depends on its content: its
//! `inline_flow_role` is a function of its own computed style, and its
//! `contributes_in_flow_block` of that role alone (only a transparent node's
//! value is derived from its descendants). So a change *inside* a container
//! reaches no decision outside its region — except its **size**, which an
//! enclosing IFC lines up against when the container is an atomic inline, and
//! that already has its own path: `dirty_atomic_inlines` and
//! `remeasure_dirty_atomic_inlines`, which invalidates the enclosing root when
//! the size moves.
//!
//! # Seeds
//!
//! So the mutation verbs record *where* the tree changed, as seeds
//! ([`IfcSeed`], `NodeTree::ifc_seeds`), and the next layout sets up again only
//! the containers they reach ([`RinchDocument::compute_ifc_scope`]):
//!
//! - [`IfcSeed::Children`] on `n` — `n`'s child list changed. Scope: the
//!   container whose region `n`'s children are in, `fc(n)`, the nearest
//!   formatting container at or above `n`.
//! - [`IfcSeed::Subtree`] on `n` — `n` itself is new here, moved, or its own box
//!   changed (`display`, `position`, the display mode, a crossing into or out
//!   of `display: contents`). Scope: `fc(parent(n))`, whose region holds `n`'s
//!   role, **and every formatting container inside `n`'s subtree** — a moved
//!   subtree has had its `ifc_root` marks cleared by the verb that moved it, so
//!   its interior is set up again as if new. Conservative for a display flip,
//!   which rarely needs its interior redone; correct, and a flip is rare.
//!
//! A seed on a node that is **not connected** to the document when the pass
//! runs is dropped: a detached subtree is not laid out, and the verb that
//! attaches it seeds it again. (The whole-document pass *did* set up detached
//! subtrees that are still in the slab, and re-did it on every pass. Nothing
//! reads that state until the subtree is attached, and attaching re-seeds it.)
//!
//! # The fallback
//!
//! `NodeTree::ifc_dirty` still asks for the whole-document pass, and every
//! writer that does not say *where* the tree changed gets it — a test forcing a
//! pass, a site nobody converted, the first layout, a whole-document restyle.
//! Each is counted under an `ifc_full_*` reason ([`IfcFullReason`]).
//!
//! A scoped pass is also robust to state left behind by a subtree freed from
//! the slab (`set_inner_html`, pseudo-element churn): anonymous boxes, splits
//! and measure leaves whose owner is gone are swept at its start (an O(boxes)
//! check, not O(document)).

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::RinchDocument;
use crate::node::{DisplayMode, InlineFlowRole, Node, NodeKind, NodeTree, RawNodeId};
use crate::perf::Counter;

/// Where the tree changed; see the [module docs](self).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfcSeed {
    /// The node's child list changed: an insertion, a removal or a reorder
    /// under it, or its children replaced.
    Children,
    /// The node is new here, moved, or its own box changed (`display`,
    /// `position`, display mode, a `display: contents` crossing): its parent's
    /// region and its whole subtree are set up again.
    Subtree,
}

/// Why a structural pass covered the whole document; see the `ifc_full_*`
/// counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfcFullReason {
    /// The first layout of a document.
    Initial,
    /// A whole-document restyle (`recompute_all_styles_full`).
    Theme,
}

impl IfcFullReason {
    pub(crate) const fn counter(self) -> Counter {
        match self {
            Self::Initial => Counter::IfcFullInitial,
            Self::Theme => Counter::IfcFullTheme,
        }
    }
}

/// More seeds than this between two layouts and the next pass is a whole
/// document one anyway: only a document that keeps mutating and is never laid
/// out gets here, and the seeds must not grow without bound.
const MAX_SEEDS: usize = 1 << 16;

impl NodeTree {
    /// Record that the tree changed at `node` ([`IfcSeed`]), for the next
    /// layout's scoped structural pass. The caller also sets `layout_dirty`.
    pub fn seed_ifc(&mut self, node: RawNodeId, seed: IfcSeed) {
        if self.ifc_dirty {
            // A whole-document pass is already pending; it covers this.
            return;
        }
        if self.ifc_seeds.len() >= MAX_SEEDS {
            self.ifc_seeds.clear();
            self.ifc_dirty = true;
            return;
        }
        self.ifc_seeds.push((node, seed));
    }

    /// Ask for a whole-document structural pass, saying why.
    pub fn request_full_ifc(&mut self, reason: IfcFullReason) {
        if !self.ifc_dirty || self.ifc_full_reason.is_none() {
            self.ifc_full_reason = Some(reason);
        }
        self.ifc_dirty = true;
        self.ifc_seeds.clear();
    }

    /// Whether a structural pass — whole-document or scoped — is pending.
    pub fn structural_pass_pending(&self) -> bool {
        self.ifc_dirty || !self.ifc_seeds.is_empty()
    }
}

/// A node the IFC passes walk **through** rather than stopping at: a
/// `display: contents` wrapper, or a non-atomic `display: inline` element.
/// The same test `recompute_contributes_in_flow_block` calls "descends".
pub(crate) fn is_inline_transparent(node: &Node) -> bool {
    let role = node.inline_flow_role();
    role == InlineFlowRole::Contents
        || (role == InlineFlowRole::Inline
            && node.is_element()
            && node.display_mode == DisplayMode::Inline)
}

/// A node whose children lay out in a context of its own; see the
/// [module docs](self). Text and comments are leaves, never containers.
pub(crate) fn is_formatting_container(node: &Node) -> bool {
    match node.kind {
        NodeKind::Text(_) | NodeKind::Comment(_) => false,
        NodeKind::Element(_) => !node.is_anonymous_block_box && !is_inline_transparent(node),
        NodeKind::Document => true,
    }
}

/// The containers and nodes one scoped structural pass sets up again.
#[derive(Debug, Default)]
pub(crate) struct IfcScope {
    /// The formatting containers, ascending id.
    pub containers: Vec<RawNodeId>,
    pub container_set: HashSet<RawNodeId>,
    /// Every region node: `(node, its container, is a stop node)`, each
    /// container's region in DOM pre-order.
    pub region: Vec<(RawNodeId, RawNodeId, bool)>,
    /// The containers, every region node, and every anonymous block box the
    /// containers held when the pass began.
    pub nodes: HashSet<RawNodeId>,
    /// The nodes whose **IFC-root status** this pass decides: [`Self::nodes`]
    /// less the stop nodes that are not containers of the scope. A stop node's
    /// own IFC is its own region's business — a row in a list keeps its root,
    /// its measure leaf, its cached measures and its paint layout when the list
    /// gains a row.
    pub rootable: HashSet<RawNodeId>,
}

impl IfcScope {
    pub fn is_empty(&self) -> bool {
        self.containers.is_empty()
    }

    /// Region nodes that are not stop nodes: the transparent wrappers and
    /// inline elements, and the text and comments.
    pub fn transparent_region(&self) -> impl Iterator<Item = RawNodeId> + '_ {
        self.region.iter().filter(|r| !r.2).map(|r| r.0)
    }
}

impl RinchDocument {
    /// Drain `ifc_seeds` into the scope of a structural pass; see the
    /// [module docs](self).
    pub(crate) fn compute_ifc_scope(&mut self) -> IfcScope {
        let seeds = std::mem::take(&mut self.tree.ifc_seeds);
        let nodes = &self.tree.nodes;
        let root = self.tree.root_id;

        // Connectivity, memoised along every walk.
        let mut connected: HashMap<RawNodeId, bool> = HashMap::default();
        connected.insert(root, true);
        let mut is_connected = |id: RawNodeId| -> bool {
            let mut path = Vec::new();
            let mut cur = Some(id);
            let answer = loop {
                let Some(c) = cur else { break false };
                if let Some(&known) = connected.get(&c) {
                    break known;
                }
                path.push(c);
                cur = nodes.get(c).and_then(|n| n.parent);
            };
            for p in path {
                connected.insert(p, answer);
            }
            answer
        };
        // The nearest formatting container at or above `id`.
        let fc = |id: RawNodeId| -> Option<RawNodeId> {
            let mut cur = Some(id);
            while let Some(c) = cur {
                let n = nodes.get(c)?;
                if is_formatting_container(n) {
                    return Some(c);
                }
                cur = n.parent;
            }
            None
        };

        let mut containers: HashSet<RawNodeId> = HashSet::default();
        let mut walked: HashSet<RawNodeId> = HashSet::default();
        for (id, seed) in seeds {
            if !nodes.contains(id) || !is_connected(id) {
                continue;
            }
            match seed {
                IfcSeed::Children => {
                    if let Some(c) = fc(id) {
                        containers.insert(c);
                    }
                }
                IfcSeed::Subtree => {
                    if let Some(c) = nodes[id].parent.and_then(fc) {
                        containers.insert(c);
                    }
                    let mut stack = vec![id];
                    while let Some(x) = stack.pop() {
                        if !walked.insert(x) {
                            continue;
                        }
                        let Some(n) = nodes.get(x) else { continue };
                        if is_formatting_container(n) {
                            containers.insert(x);
                        }
                        stack.extend(n.children.iter().copied());
                    }
                }
            }
        }

        let mut scope = IfcScope::default();
        let mut sorted: Vec<RawNodeId> = containers.into_iter().collect();
        sorted.sort_unstable();
        for &c in &sorted {
            scope.nodes.insert(c);
            let cn = &nodes[c];
            scope.nodes.extend(cn.run_boxes.iter().copied());
            let mut stack: Vec<RawNodeId> = cn.children.iter().rev().copied().collect();
            while let Some(x) = stack.pop() {
                let Some(n) = nodes.get(x) else { continue };
                let stop = is_formatting_container(n);
                scope.region.push((x, c, stop));
                scope.nodes.insert(x);
                if !stop {
                    // A transparent node holds no boxes of its own — unless it
                    // was a block container until this change. Those need no
                    // entry here: `cleanup_anonymous_block_boxes` takes every
                    // box whose parent is `rootable`, which this node is. (An
                    // explicit entry was a mutant nothing could kill: the only
                    // other reader it reached is a measure leaf, and a run never
                    // holds the out-of-flow box that would give one a leaf.)
                    stack.extend(n.children.iter().rev().copied());
                }
            }
        }
        scope.container_set = sorted.iter().copied().collect();
        let stop_only: HashSet<RawNodeId> = scope
            .region
            .iter()
            .filter(|r| r.2 && !scope.container_set.contains(&r.0))
            .map(|r| r.0)
            .collect();
        scope.rootable = scope
            .nodes
            .iter()
            .copied()
            .filter(|id| !stop_only.contains(id))
            .collect();
        scope.containers = sorted;
        scope
    }
}
