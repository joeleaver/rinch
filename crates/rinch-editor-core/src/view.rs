//! The view seam — the renderer-agnostic boundary between the pure editor state
//! and a platform's host tree (design §6).
//!
//! "The view is a pure function of state" (non-negotiable principle #3): an
//! [`EditorView`] projects `EditorState` onto a host (desktop rinch-dom now, web
//! `contentEditable` later) and is **never read back** for content. The caret is
//! rendered from `state.selection`; the host is derived from `state.doc`. There is
//! no DOM-direct mutation path.
//!
//! ## Two-phase update (design A3)
//!
//! Projection is split to straddle the host's *mutate → layout → measure*
//! pipeline, the same seam the desktop block-virtualizer already lives on:
//!
//! 1. [`update_dom`](EditorView::update_dom) runs **before** layout. It diffs
//!    `prev.doc` vs `next.doc` and patches the host, and diffs
//!    `prev.decorations()` vs `next.decorations()` and patches overlay nodes
//!    **independently of the document diff** (design A4) — so an IME-preedit
//!    transaction that changes only decorations still produces a visible update.
//! 2. [`update_caret`](EditorView::update_caret) runs **after** layout, once the
//!    host has fresh geometry. Caret/selection rectangles and the IME candidate
//!    box are computed here from `next.selection`. **Caret geometry must never be
//!    computed in the call that mutates the host**, because the layout it would
//!    read is stale.
//!
//! Each phase returns the [`ViewRequest`]s the runtime must fulfil on the
//! platform window (scrolling the caret into view now; enabling IME and
//! positioning the candidate box from M6). The view itself owns no window — only
//! the runtime does — so these cross back as data.
//!
//! Input translation (key/pointer/IME events → commands/transactions) is **not**
//! on this trait: it is inherently platform-specific (desktop reads Parley
//! geometry for pointer→`Pos`; web delegates to the browser), so each platform's
//! runtime glue owns it. The trait covers only the state→host projection.

use crate::state::EditorState;

/// A platform request a view emits for the runtime to fulfil on the window. The
/// runtime owns the window; the view does not, so these cross back as data from
/// the two-phase update (design §6). Grows with later milestones: IME
/// enable/candidate-box positioning (M6) and accessibility refresh (M7b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRequest {
    /// Scroll the host so the current selection's caret is visible. Emitted after
    /// a selection-changing transaction.
    ScrollSelectionIntoView,
}

/// Projects [`EditorState`] onto a platform host tree (design §6). Implemented by
/// the desktop view in the `rinch` crate and the web view in `rinch-web`; nothing
/// else knows the renderer.
///
/// See the [module docs](self) for the two-phase contract. The initial render is
/// the implementor's responsibility (its constructor builds the host from the
/// starting state); [`update_dom`](Self::update_dom) handles every subsequent
/// transaction.
pub trait EditorView {
    /// Phase 1 — **before** layout. Diff `prev` vs `next` (document and
    /// decorations) and patch the host accordingly. Returns the platform requests
    /// to fulfil before measuring (e.g. IME enable, from M6).
    fn update_dom(&mut self, prev: &EditorState, next: &EditorState) -> Vec<ViewRequest>;

    /// Phase 2 — **after** layout. Render caret/selection geometry from
    /// `next.selection` using the host's now-fresh layout, and return the
    /// geometry-dependent platform requests (scroll into view; the IME candidate
    /// box, from M6).
    fn update_caret(&mut self, next: &EditorState) -> Vec<ViewRequest>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Schema, default_plugins};
    use std::rc::Rc;

    /// A no-op view that records how many times each phase ran and echoes a
    /// request — enough to exercise the seam in the pure core (the real
    /// projection lives in the platform crates).
    #[derive(Default)]
    struct RecordingView {
        dom_updates: usize,
        caret_updates: usize,
    }

    impl EditorView for RecordingView {
        fn update_dom(&mut self, _prev: &EditorState, _next: &EditorState) -> Vec<ViewRequest> {
            self.dom_updates += 1;
            Vec::new()
        }

        fn update_caret(&mut self, _next: &EditorState) -> Vec<ViewRequest> {
            self.caret_updates += 1;
            vec![ViewRequest::ScrollSelectionIntoView]
        }
    }

    fn empty_state() -> EditorState {
        let schema = Rc::new(Schema::starter_kit());
        let doc = schema
            .branch(
                "doc",
                crate::model::Fragment::from_node(
                    schema
                        .branch("paragraph", crate::model::Fragment::empty())
                        .unwrap(),
                ),
            )
            .unwrap();
        EditorState::create(schema, doc, default_plugins())
    }

    #[test]
    fn two_phase_update_runs_both_phases() {
        let mut view = RecordingView::default();
        let prev = empty_state();
        let mut tr = prev.tr();
        tr.insert_text("hi").unwrap();
        let next = prev.apply(tr);

        let dom_reqs = view.update_dom(&prev, &next);
        let caret_reqs = view.update_caret(&next);

        assert_eq!(view.dom_updates, 1);
        assert_eq!(view.caret_updates, 1);
        assert!(dom_reqs.is_empty());
        assert_eq!(caret_reqs, vec![ViewRequest::ScrollSelectionIntoView]);
    }
}
