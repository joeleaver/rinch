//! Block virtualization for mounted (M5+) editors — the re-homed `CeVirtualWindow`
//! driver. Off-screen blocks of a *scroll-container* editor get a fixed estimated
//! height so Taffy skips their Parley measurement entirely.
//!
//! `pre_layout`/`post_layout` are called from `resolve_and_repaint` (design A3 two
//! phase). `pre_layout` runs **before** the resolve short-circuit so creating a
//! window (and its initial collapse) un-short-circuits the frame — otherwise a
//! selection-only first interaction would never trigger the collapse (the bug that
//! reverted the first attempt; see `project-editor-virtualization`).
//!
//! Only the container's `data-pm-type` children are modeled — caret / selection /
//! node-outline / placeholder overlays are container siblings and must never be
//! collapsed (`CeVirtualWindow::new_filtered(.., true)`).

use std::cell::RefCell;

use rinch_core::dom::DomDocument; // take_dirty_nodes / resolve_layout trait methods
use rinch_dom::RinchDocument;

use rinch_editor_view::EditorHandle;
use rinch_editor_view::registry;

use super::virtual_window::CeVirtualWindow;

/// One virtualized editor: `(doc_key, container id)`, its window, and whether
/// this frame's caret scroll follows the caret while the window measures the
/// blocks the scroll reaches (#845; set by [`pre_layout`], read by
/// [`post_layout`]).
type WindowEntry = ((u64, usize), CeVirtualWindow, bool);

thread_local! {
    /// `((doc_key, container id), window)` for each scroll-container editor being
    /// virtualized. Keyed per document: container ids are per-document slab
    /// indices, so two documents on one thread would otherwise share (and stomp)
    /// a window at a colliding id (issue #134).
    static WINDOWS: RefCell<Vec<WindowEntry>> =
        const { RefCell::new(Vec::new()) };
}

/// Phase 1 (before layout): for every mounted editor that is a scroll container,
/// ensure a virtual window exists and update the materialized range from the
/// previous frame's positions. Creating a window (or moving the range) sets the
/// document's style/layout dirty flags, so the caller's short-circuit will not fire
/// and the upcoming resolve applies the collapse.
pub(crate) fn pre_layout(doc: &mut RinchDocument, focused: Option<usize>) {
    // This pass asks the editor handles for their caret while it holds
    // `WINDOWS` and the document mutably; a handle read may run pending effects
    // (`rinch_core::flush_pending_effects`) when called inside a batch, and an
    // effect touching a `NodeHandle` would then re-borrow the document. It runs
    // from the frame pass, outside any batch, today — the guard keeps it that
    // way if a caller ever changes (PR #882 audit).
    let _no_flush = rinch_core::reactive::suppress_effect_flush();
    let doc_key = doc.doc_key();
    // Only this document's editors: another document's editors are driven by its
    // own runtime pass, against its own tree (issue #134).
    let editors: Vec<(usize, EditorHandle)> = registry::all_editors()
        .into_iter()
        .filter(|(dk, _, _)| *dk == doc_key)
        .map(|(_, id, h)| (id, h))
        .collect();
    WINDOWS.with(|w| {
        let mut windows = w.borrow_mut();
        // Drop THIS document's windows whose editor unmounted; other documents'
        // windows are managed by their own pass.
        windows
            .retain(|((dk, id), _, _)| *dk != doc_key || editors.iter().any(|(eid, _)| eid == id));
        for (container, handle) in &editors {
            let container = *container;
            if container == 0 {
                continue;
            }
            let key = (doc_key, container);
            let idx = match windows.iter().position(|(k, _, _)| *k == key) {
                Some(i) => i,
                None => {
                    if !is_scroll_container(doc, container) {
                        continue;
                    }
                    let vw = CeVirtualWindow::new_filtered(container, doc, true);
                    windows.push((key, vw, false));
                    windows.len() - 1
                }
            };
            let (_, vw, follow) = &mut windows[idx];
            // A follow lasts one frame: the frame of the caret scroll it is for.
            *follow = false;
            // Re-sync only on a block-count change (insert/delete) — doing it every
            // frame would stomp measured heights with the default estimate.
            if block_count(doc, container) != vw.block_count() {
                vw.on_blocks_changed(doc);
            }
            if !vw.is_active() {
                continue;
            }
            let protected = protected_block(doc, handle, container, focused);
            vw.pre_layout_update(doc, &protected);
            // A caret move that owes a scroll needs the block holding the
            // caret laid out, wherever the window is (#845): Ctrl+End or a
            // `set_selection` far below lands in a collapsed block, which has
            // no caret to scroll to, and the window — which follows the
            // scroll position — would never reach it. Only the focused
            // editor's caret pass fulfils that scroll, so only its caret. The
            // scroll lands where the block is by the estimated heights of the
            // blocks above it, so `post_layout` follows the caret as the
            // blocks the scroll reaches are measured, whether or not this call
            // was what laid the block out. Known limit (#2019's review, F3): a
            // caret move into a block that is laid out but sits below
            // unmeasured blocks (an arrow key in a deep link's caret block) is
            // scrolled by the input path's caret pass before any frame, so it
            // is never owed here and not followed.
            //
            // Not while a `scroll_into_view` is pending: the caret pass runs
            // the caret scroll first and the reveal after it, so the reveal
            // wins, and following the caret would pull the view back off it.
            let owed = Some(container) == focused && handle.caret_scroll_owed();
            if owed
                && handle.pending_reveal_range().is_none()
                && let Some(block) = block_at(doc, handle, handle.selection().head(), container)
            {
                vw.materialize(doc, block);
                *follow = true;
            }
            // A pending `scroll_into_view` needs the blocks holding its start
            // and its end laid out, wherever the window is: the reveal is what
            // moves the window there. Both, because a range whose end block
            // stayed collapsed had no geometry for its end (#922's review, D3);
            // the view reveals the start alone if the end still has none.
            if let Some((from, to)) = handle.pending_reveal_range() {
                for pos in [from, to] {
                    if let Some(block) = block_at(doc, handle, pos, container) {
                        vw.materialize(doc, block);
                    }
                }
            }
        }
    });
}

/// Phase 2 (after layout): cache measured heights, then re-verify the materialized
/// range with fresh positions; if it changed (a big scroll jump), re-layout once.
pub(crate) fn post_layout(doc: &mut RinchDocument, focused: Option<usize>, vw_w: f32, vw_h: f32) {
    // Same reason as `pre_layout`.
    let _no_flush = rinch_core::reactive::suppress_effect_flush();
    let doc_key = doc.doc_key();
    let editors: Vec<(usize, EditorHandle)> = registry::all_editors()
        .into_iter()
        .filter(|(dk, _, _)| *dk == doc_key)
        .map(|(_, id, h)| (id, h))
        .collect();
    WINDOWS.with(|w| {
        let mut windows = w.borrow_mut();
        for (container, handle) in &editors {
            let container = *container;
            let key = (doc_key, container);
            let Some(idx) = windows.iter().position(|(k, _, _)| *k == key) else {
                continue;
            };
            let (_, vw, follow) = &mut windows[idx];
            if !vw.is_active() {
                continue;
            }
            vw.post_layout_cache(doc);
            let protected = protected_block(doc, handle, container, focused);
            let moved = vw.pre_layout_update(doc, &protected);
            if moved {
                let _ = doc.take_dirty_nodes();
                doc.resolve_layout(vw_w, vw_h);
                vw.post_layout_cache(doc);
            }
            // A caret scrolled to by estimated heights (#845): the scroll moved
            // the window, the blocks it reached were measured, and their real
            // heights moved the caret — owe its scroll again, and the caret
            // pass right after this in `after_layout` follows it (a no-op
            // when the caret is still in view). This runs in the frame's
            // layout pass and in each bounds-settle pass after it — a pass
            // runs only while the layout is dirty, so those passes bound the
            // corrections; the next frame's `pre_layout` ends the follow, and
            // a window still moving after the last pass is not followed.
            if *follow && !handle.caret_scroll_owed() {
                handle.owe_caret_scroll();
            }
        }
    });
}

/// The top-level blocks that must never be collapsed: the cursor's, for the
/// focused editor, and — for any editor — the ones holding the start and the
/// end of a pending `EditorHandle::scroll_into_view`, which needs them laid
/// out to know where to scroll (focused or not: a reveal does not need focus).
fn protected_block(
    doc: &RinchDocument,
    handle: &EditorHandle,
    container: usize,
    focused: Option<usize>,
) -> Vec<usize> {
    let mut out = Vec::new();
    let mut protect = |pos| {
        if let Some((textblock, _)) = handle.caret_address(pos)
            && let Some(block) = top_block(doc, textblock, container)
            && !out.contains(&block)
        {
            out.push(block);
        }
    };
    if Some(container) == focused {
        protect(handle.selection().head());
    }
    if let Some((from, to)) = handle.pending_reveal_range() {
        protect(from);
        protect(to);
    }
    out
}

/// The direct child of `container` holding the model position `pos`.
fn block_at(
    doc: &RinchDocument,
    handle: &EditorHandle,
    pos: rinch_editor_core::Pos,
    container: usize,
) -> Option<usize> {
    let (textblock, _) = handle.caret_address(pos)?;
    top_block(doc, textblock, container)
}

/// Walk up from `id` to the direct child of `container`.
fn top_block(doc: &RinchDocument, mut id: usize, container: usize) -> Option<usize> {
    loop {
        let parent = doc.tree.nodes.get(id)?.parent?;
        if parent == container {
            return Some(id);
        }
        id = parent;
    }
}

/// The number of `data-pm-type` block children of `container`.
fn block_count(doc: &RinchDocument, container: usize) -> usize {
    doc.tree
        .nodes
        .get(container)
        .map(|n| {
            n.children
                .iter()
                .filter(|&&id| {
                    doc.tree
                        .nodes
                        .get(id)
                        .is_some_and(|c| c.attributes.contains_key("data-pm-type"))
                })
                .count()
        })
        .unwrap_or(0)
}

/// Whether `container`'s computed `overflow-y` makes it a scrollable viewport.
fn is_scroll_container(doc: &RinchDocument, id: usize) -> bool {
    doc.tree.nodes.get(id).is_some_and(|n| n.scrolls_y())
}
