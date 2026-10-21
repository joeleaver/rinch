//! Margin anchors (#1677): where the lines holding a set of places in the
//! document are drawn, for an app that draws something beside them (a comment
//! marker, a bookmark flag, a change bar) — see
//! [`EditorHandle::set_margin_anchors`](crate::EditorHandle::set_margin_anchors)
//! and [`EditorHandle::on_margin_layout`](crate::EditorHandle::on_margin_layout).
//!
//! Nothing here touches the document: an anchor is view state beside it, like
//! a [`SelectionAnchor`](crate::SelectionAnchor), so it changes no undo history
//! and nothing a collaborator receives, and works on a read-only editor.

use std::cell::Cell;

use rinch_core::dom::NodeHandle;
use rinch_editor_core::transform::Mapping;
use rinch_editor_core::{Node, Pos};

/// A place an app wants the line of: a `key` of its own choosing (a comment
/// id) and where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct MarginAnchor {
    /// The app's name for this anchor, reported back in [`MarginLine::key`].
    /// Two anchors may share a key; each is reported.
    pub key: String,
    /// Where it is.
    pub at: MarginAt,
}

impl MarginAnchor {
    /// An anchor at model position `pos`, which stays before text inserted
    /// right at it ([`MarginAssoc::After`]: it sticks to what follows).
    pub fn at(key: impl Into<String>, pos: Pos) -> MarginAnchor {
        MarginAnchor::at_with(key, pos, MarginAssoc::After)
    }

    /// An anchor at model position `pos`, on `assoc`'s side of text inserted
    /// right at it.
    pub fn at_with(key: impl Into<String>, pos: Pos, assoc: MarginAssoc) -> MarginAnchor {
        MarginAnchor {
            key: key.into(),
            at: MarginAt::Pos { pos, assoc },
        }
    }

    /// An anchor at a collaboration sticky index — bytes from
    /// [`EditorHandle::collab_sticky_index_with`](crate::EditorHandle::collab_sticky_index_with)
    /// or [`collab_sticky_index`](crate::EditorHandle::collab_sticky_index),
    /// made by this editor or a peer's — resolved afresh on every margin pass,
    /// so it means the same character on every replica.
    #[cfg(feature = "collaboration")]
    pub fn sticky(key: impl Into<String>, bytes: Vec<u8>) -> MarginAnchor {
        MarginAnchor {
            key: key.into(),
            at: MarginAt::Sticky(bytes),
        }
    }
}

/// Where a [`MarginAnchor`] is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarginAt {
    /// A model position, carried through every change the editor applies —
    /// local edits by their mapping, a peer's change the way a
    /// [`SelectionAnchor`](crate::SelectionAnchor) is carried. It is lost
    /// (never reported again, until the set is replaced) when the document is
    /// replaced (`load_doc` / `load_html`) or a peer removes, splits, joins or
    /// changes the kind of its block.
    Pos {
        /// The position, in the document current when the set was given.
        pos: Pos,
        /// Which side of text inserted right at `pos` it stays on.
        assoc: MarginAssoc,
    },
    /// A collaboration sticky index; see [`MarginAnchor::sticky`]. Reported
    /// wherever it resolves — the place its block stood, once that block is
    /// deleted — and not at all while the editor is not collaborating, or its
    /// session is stalled or poisoned.
    #[cfg(feature = "collaboration")]
    Sticky(Vec<u8>),
}

/// Which side of text inserted right at a [`MarginAt::Pos`] anchor it stays
/// on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarginAssoc {
    /// With the text before it: an insertion at the anchor lands after it.
    Before,
    /// With the text after it: an insertion at the anchor lands before it.
    #[default]
    After,
}

impl MarginAssoc {
    fn bias(self) -> i32 {
        match self {
            MarginAssoc::Before => -1,
            MarginAssoc::After => 1,
        }
    }
}

/// Where an anchor's line is drawn, as
/// [`EditorHandle::on_margin_layout`](crate::EditorHandle::on_margin_layout)
/// reports it.
///
/// `top` and `height` are the **line box** of the line the anchor sits in (its
/// leading included), in the editor container's content coordinates: the frame
/// the editor's own overlays and the `Editor { … }` overlay slot are placed in
/// — the container's padding box, with the content unscrolled. A child of the
/// overlay slot at `top: {top}px` sits level with the line whatever scrolls.
/// Anchors on one line report the same box.
#[derive(Clone, Debug, PartialEq)]
pub struct MarginLine {
    /// The anchor's [`MarginAnchor::key`].
    pub key: String,
    /// The line box's top, in CSS px.
    pub top: f32,
    /// The line box's height, in CSS px.
    pub height: f32,
}

/// One anchor of the live set.
pub(crate) struct Live {
    pub(crate) key: String,
    pub(crate) at: LiveAt,
}

pub(crate) enum LiveAt {
    /// `None` once lost (see [`MarginAt::Pos`]).
    Pos(Option<Pos>, MarginAssoc),
    #[cfg(feature = "collaboration")]
    Sticky(Vec<u8>),
}

/// An editor's margin anchors and what its last margin pass reported.
#[derive(Default)]
pub(crate) struct Margins {
    pub(crate) anchors: Vec<Live>,
    /// The last report (lines, text column's right edge); `None` before the
    /// first, and again once the callback or the mount changed, so the next
    /// pass reports whatever it finds.
    pub(crate) last: Option<(Vec<MarginLine>, Option<f32>)>,
    /// The container a platform watches for size changes while anchors are
    /// set (`registry::notify_node_view_host`), so a reflow the editor did not
    /// make (a window resize) reports.
    pub(crate) observed: Option<(u64, NodeHandle)>,
    /// What the last measure read its lines from
    /// ([`RinchDomEditorView::margin_geometry_key`](crate::RinchDomEditorView)):
    /// a pass that finds the same skips measuring.
    pub(crate) key: Option<(u64, Node)>,
}

impl Margins {
    pub(crate) fn set(&mut self, anchors: Vec<MarginAnchor>) {
        self.key = None;
        self.anchors = anchors
            .into_iter()
            .map(|a| Live {
                key: a.key,
                at: match a.at {
                    MarginAt::Pos { pos, assoc } => LiveAt::Pos(Some(pos), assoc),
                    #[cfg(feature = "collaboration")]
                    MarginAt::Sticky(bytes) => LiveAt::Sticky(bytes),
                },
            })
            .collect();
    }

    /// Forget the last report and what it was measured from, so the next
    /// pass measures and reports whatever it finds (a new callback, a
    /// re-mount).
    pub(crate) fn forget_report(&mut self) {
        self.last = None;
        self.key = None;
    }

    /// Whether `key` is what the last measure read its lines from.
    pub(crate) fn key_is(&self, key: &(u64, Node)) -> bool {
        self.key
            .as_ref()
            .is_some_and(|(h, doc)| *h == key.0 && doc.same_ref(&key.1))
    }

    /// Whether a margin pass has anything to do: anchors to measure, a
    /// non-empty report to take back, or a container still watched.
    pub(crate) fn is_idle(&self) -> bool {
        self.anchors.is_empty()
            && self.observed.is_none()
            && self.last.as_ref().is_none_or(|(l, _)| l.is_empty())
    }

    /// Carry the position anchors across a local document change: by
    /// `mapping`, or lost with no mapping (a load).
    // One variant without `collaboration`.
    #[allow(irrefutable_let_patterns)]
    pub(crate) fn carry(&mut self, mapping: Option<&Mapping>) {
        for a in &mut self.anchors {
            if let LiveAt::Pos(pos, assoc) = &mut a.at {
                *pos = match (pos.take(), mapping) {
                    (Some(p), Some(m)) => Some(Pos(m.map(p.0, assoc.bias()))),
                    _ => None,
                };
            }
        }
    }

    /// Carry the position anchors across a peer's change (see
    /// [`MarginAt::Pos`]).
    #[cfg(feature = "collaboration")]
    pub(crate) fn carry_remote(&mut self, carry: &rinch_editor_collab::RemoteCarry) {
        for a in &mut self.anchors {
            if let LiveAt::Pos(pos, _) = &mut a.at {
                *pos = pos.take().and_then(|p| {
                    carry
                        .selection(&rinch_editor_core::Selection::cursor(p))
                        .map(|s| s.head())
                });
            }
        }
    }

    pub(crate) fn has_pos_anchors(&self) -> bool {
        self.anchors
            .iter()
            .any(|a| matches!(a.at, LiveAt::Pos(Some(_), _)))
    }
}

thread_local! {
    static PASSES: Cell<u64> = const { Cell::new(0) };
    static MEASURES: Cell<u64> = const { Cell::new(0) };
}

/// Count one margin pass that measured `anchors` anchors.
pub(crate) fn count_pass(anchors: usize) {
    PASSES.with(|c| c.set(c.get() + 1));
    MEASURES.with(|c| c.set(c.get() + anchors as u64));
}

/// `(margin passes that measured, anchors measured)` on this thread since it
/// started: how the tests pin that an editor with no margin anchors measures
/// nothing, and that `N` anchors cost one pass per layout.
#[doc(hidden)]
pub fn __margin_counters() -> (u64, u64) {
    (PASSES.with(Cell::get), MEASURES.with(Cell::get))
}
