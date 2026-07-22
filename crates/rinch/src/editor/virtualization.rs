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
use rinch_dom::computed_style::OverflowValue;

use rinch_editor_view::EditorHandle;
use rinch_editor_view::registry;

use super::virtual_window::CeVirtualWindow;

thread_local! {
    /// `((doc_key, container id), window)` for each scroll-container editor being
    /// virtualized. Keyed per document: container ids are per-document slab
    /// indices, so two documents on one thread would otherwise share (and stomp)
    /// a window at a colliding id (issue #134).
    static WINDOWS: RefCell<Vec<((u64, usize), CeVirtualWindow)>> =
        const { RefCell::new(Vec::new()) };
}

/// Phase 1 (before layout): for every mounted editor that is a scroll container,
/// ensure a virtual window exists and update the materialized range from the
/// previous frame's positions. Creating a window (or moving the range) sets the
/// document's style/layout dirty flags, so the caller's short-circuit will not fire
/// and the upcoming resolve applies the collapse.
pub(crate) fn pre_layout(doc: &mut RinchDocument, focused: Option<usize>) {
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
        windows.retain(|((dk, id), _)| *dk != doc_key || editors.iter().any(|(eid, _)| eid == id));
        for (container, handle) in &editors {
            let container = *container;
            if container == 0 {
                continue;
            }
            let key = (doc_key, container);
            let idx = match windows.iter().position(|(k, _)| *k == key) {
                Some(i) => i,
                None => {
                    if !is_scroll_container(doc, container) {
                        continue;
                    }
                    let vw = CeVirtualWindow::new_filtered(container, doc, true);
                    windows.push((key, vw));
                    windows.len() - 1
                }
            };
            let vw = &mut windows[idx].1;
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
        }
    });
}

/// Phase 2 (after layout): cache measured heights, then re-verify the materialized
/// range with fresh positions; if it changed (a big scroll jump), re-layout once.
pub(crate) fn post_layout(doc: &mut RinchDocument, focused: Option<usize>, vw_w: f32, vw_h: f32) {
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
            let Some(idx) = windows.iter().position(|(k, _)| *k == key) else {
                continue;
            };
            let vw = &mut windows[idx].1;
            if !vw.is_active() {
                continue;
            }
            vw.post_layout_cache(doc);
            let protected = protected_block(doc, handle, container, focused);
            if vw.pre_layout_update(doc, &protected) {
                let _ = doc.take_dirty_nodes();
                doc.resolve_layout(vw_w, vw_h);
                vw.post_layout_cache(doc);
            }
        }
    });
}

/// The cursor's top-level block (must never be collapsed) for the focused editor.
fn protected_block(
    doc: &RinchDocument,
    handle: &EditorHandle,
    container: usize,
    focused: Option<usize>,
) -> Vec<usize> {
    if Some(container) != focused {
        return Vec::new();
    }
    let Some((textblock, _)) = handle.caret_address(handle.selection().head()) else {
        return Vec::new();
    };
    top_block(doc, textblock, container)
        .map(|b| vec![b])
        .unwrap_or_default()
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
    doc.tree.nodes.get(id).is_some_and(|n| {
        matches!(
            n.computed_style.overflow_y,
            OverflowValue::Auto | OverflowValue::Scroll
        )
    })
}
