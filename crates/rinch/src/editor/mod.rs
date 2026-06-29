//! Desktop wiring for the rich-text editor (part of the `desktop` feature).
//!
//! The renderer-agnostic editor — [`RinchDomEditorView`], [`EditorHandle`], the
//! [`Editor`] component, the mounted-editor registry, and the caret-blink clock —
//! lives in the shared [`rinch_editor_view`] crate (reused by the web editor in
//! `rinch-web`). This module re-exports it and adds the **desktop-only** pieces that
//! reach past the [`DomDocument`](rinch_core::dom::DomDocument) seam into the
//! `rinch-dom` renderer or a native platform adapter:
//!
//! - block [`virtualization`] (Parley `estimated_height` over the concrete layout tree),
//! - AccessKit [`a11y`] (the AT-SPI adapter; the derivation itself is portable), and
//! - the thread-safe collaboration inbound entry point [`post_remote_delta`].

#[cfg(feature = "a11y")]
pub(crate) mod a11y;
mod virtual_window;
mod virtualization;

pub use rinch_editor_view::*;
pub(crate) use virtualization::{
    post_layout as virtualization_post_layout, pre_layout as virtualization_pre_layout,
};

/// Post a remote collaboration `delta` to the editor at `container_id` from **any**
/// thread — the `Send`-safe inbound entry point for a network transport. The delta is
/// marshalled onto the main thread (via [`run_on_main_thread`](crate::shell::run_on_main_thread)),
/// where [`collab_receive_for`] integrates it and re-projects the editor; the runtime
/// is woken so the change paints promptly.
#[cfg(feature = "collaboration")]
pub fn post_remote_delta(container_id: usize, delta: Vec<u8>) {
    crate::shell::run_on_main_thread(move || {
        rinch_editor_view::collab_receive_for(container_id, &delta);
    });
}
