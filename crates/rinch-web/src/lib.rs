//! Browser-native DOM backend for rinch (WASM target).
//!
//! Instead of painting to a Canvas 2D or WebGPU surface, this crate creates real
//! browser DOM elements via `web_sys`. The browser handles layout, CSS, text
//! rendering, and painting natively. The reactive system (Signal/Effect) and all
//! components work through [`rinch_core::dom::NodeHandle`] →
//! [`rinch_core::dom::DomDocument`], so everything works automatically.
//!
//! It provides:
//! - [`WebDocument`] — a [`DomDocument`](rinch_core::dom::DomDocument)
//!   implementation over `web_sys`.
//! - [`setup_event_delegation`] — document-level listeners that bridge browser
//!   events into rinch's event/drag/render-surface systems.
//! - [`mount`] — boot a single whole-page app under `#rinch-body`.
//! - [`mount_into`] / [`mount_selector`] — mount one or more **independent**
//!   component trees ("islands") into existing page elements. Each returns a
//!   [`RootHandle`] that can later [`unmount`](RootHandle::unmount) that root.
//! - [`mount_with_menu_bar`] and its `_into` / `_selector` twins — the same,
//!   under a menu bar built from [`Menu`] / [`MenuItem`].
//! - `AppExt` — run a [`rinch::App`] builder chain in the browser, so one
//!   `App::new(app).title(..).theme(..).menu(..).run()` serves desktop and web.
//! - [`set_suppress_native_context_menu`] — for a whole-page app that renders
//!   its own right-click menus.
//! - [`install_runtime`] — timers, `spawn_local` tasks and polled signals for
//!   a page that mounts no rinch root.
//!
//! ## Menu bar
//!
//! An app that declares menus for the desktop gets the same bar in the browser
//! from the same declaration — no second code path, and no native menu bar to
//! fall back to:
//!
//! ```ignore
//! let file = Menu::new()
//!     .item(MenuItem::new("New").shortcut("Ctrl+N").on_click(|| new_doc()))
//!     .separator()
//!     .item(MenuItem::new("Close").on_click(|| close_doc()));
//!
//! rinch_web::mount_with_menu_bar(theme, vec![("File", file)], app);
//! ```
//!
//! Clicks, hover-to-switch and click-outside dismissal come from the shared
//! renderer; the shortcuts are matched on a capture-phase `window` `keydown`,
//! and a chord the menus claim is consumed — `preventDefault`, so the browser's
//! own handling of `Ctrl+K` does not run alongside the app's, and the event is
//! stopped before it reaches anything else in the app, which is what the desktop
//! does by returning ahead of the event loop. Some chords are the browser's
//! alone and cannot be claimed — which ones depends on the browser and the
//! platform; see the [WASM
//! guide](https://github.com/joeleaver/rinch/blob/main/docs/src/guide/wasm.md).
//!
//! ## One entry point for desktop and web
//!
//! The desktop's [`rinch::App`] builder runs here too, through `AppExt`; the
//! only per-target line is the import:
//!
//! ```ignore
//! use rinch::prelude::*;
//! #[cfg(target_arch = "wasm32")]
//! use rinch_web::AppExt as _;
//!
//! fn main() {
//!     App::new(app).title("My App").size(800, 600).theme(theme()).run();
//! }
//! ```
//!
//! ## Whole-page app
//!
//! ```ignore
//! #[wasm_bindgen(start)]
//! pub fn start() {
//!     rinch_web::mount(ThemeProviderProps::default(), app);
//! }
//! ```
//!
//! ## Islands
//!
//! Hydrate independent widgets into placeholder elements on a server-rendered page:
//!
//! ```ignore
//! #[wasm_bindgen(start)]
//! pub fn start() {
//!     let theme = ThemeProviderProps::default();
//!     rinch_web::mount_selector("#comments", theme.clone(), comments_app);
//!     rinch_web::mount_selector("#search", theme, search_app);
//! }
//! ```

#[cfg(target_arch = "wasm32")]
mod app_ext;
mod bounds;
mod editor_input;
mod entry_points;
mod event_delegation;
mod highlight;
mod images;
mod key_layout;
mod menu_bar;
mod page_init;
mod pointer_lock;
mod roots;
pub mod web_document;

#[cfg(target_arch = "wasm32")]
pub use app_ext::{AppExt, apply_spellcheck};
pub use entry_points::{
    mount, mount_into, mount_into_with_menu_bar, mount_selector, mount_selector_with_menu_bar,
    mount_with_menu_bar,
};
pub use page_init::install_runtime;
pub use roots::RootHandle;
pub(crate) use roots::{mark_event_root, root_doc_of};

#[doc(hidden)]
pub use bounds::{__bounds_watch_count, __bounds_watch_installed};
pub use event_delegation::setup_event_delegation;
#[doc(hidden)]
pub use event_delegation::{
    __drag_auto_scroll_running, __force_trusted_clicks, __reset_activation_state,
};
// Whether a right-click anywhere suppresses the browser's own menu. Off by
// default: an island mounted into somebody else's page must not take the
// right-click away from the rest of it.
/// Test-only: take the page as macOS, or not, for the editor's key bindings
/// (`navigator.platform` otherwise), so their macOS half runs in CI.
#[doc(hidden)]
pub use editor_input::__set_mac_platform;
pub use event_delegation::{set_suppress_native_context_menu, suppresses_native_context_menu};
#[doc(hidden)]
pub use images::__reset_image_sources;
/// App-resolved image sources: an `<img>` whose `src` has a scheme the app
/// registered shows what the app resolves it to. `rinch::image::
/// register_image_scheme` and `rinch::image::reload_image` are the same calls as
/// on desktop; `register_image_url_scheme` is the browser-only form that answers
/// with a URL.
pub use images::{LOGICAL_SRC_ATTR, register_image_url_scheme, unregister_image_url_scheme};
#[doc(hidden)]
pub use key_layout::{__forget_key_layout, __known_unmodified_key};
/// The menu declaration types, re-exported so a web app names them in one place
/// (`rinch_web::{Menu, MenuItem}`) while its desktop twin builds the very same
/// values from `rinch::menu`. They are the same types, not a parallel set.
pub use rinch::menu::{Menu, MenuEntryRef, MenuItem};
/// How a letter chord's key is matched against a keystroke (issue #1170), and
/// the setter for it. The desktop twin is `App::shortcut_matching`; on the
/// web there is no builder to hang it off, so call
/// `rinch_web::set_shortcut_matching` directly before mounting (it takes
/// effect on the next keystroke either way — there is nothing to rebuild).
pub use rinch::menu::{ShortcutMatching, set_shortcut_matching};
pub use web_document::WebDocument;
/// Test-only handles on the page scroll lock (#474), so a fixture that fails
/// between a lock and its unlock cannot leave `<html>` hidden for every test
/// after it.
#[doc(hidden)]
pub use web_document::{__reset_scroll_lock, __scroll_lock_depth};

/// How this browser splits a line's leading (see the editor's margin
/// anchors, #1677): `true` when the half above a text run is floored to a
/// whole pixel. Test-only.
#[doc(hidden)]
pub fn __leading_floors() -> bool {
    web_document::leading_floors()
}

// The renderer-agnostic rich-text editor, re-exported so a web app uses the *same*
// API as desktop: `create_editor()` → `EditorHandle`, mounted via `Editor {}`. The
// browser input glue (keyboard/pointer/IME/clipboard → the handle) lives in
// `editor_input`, installed once alongside event delegation.
pub use rinch_editor_view::{
    CaretAffinity, CaretFrom, Editor, EditorHandle, EditorKey, ImageHover, ImageInput,
    ImageInputSource, KeyBindError, KeyBinding, LinkClick, LinkHover, LinkSpan, MarginAnchor,
    MarginAssoc, MarginAt, MarginLine, NODE_VIEW_ATTR, NodeViewCtx, NodeViewRegistration,
    NodeViewRender, NodeViewSpec, OVERLAY_SLOT_ATTR, ScrollAlign, SelectionAnchor, TabBehavior,
    clear_image_source, create_editor, history_now_ms, host_is_macos, set_image_source,
};

// Collaborative editing (M9), behind the `collaboration` feature. The collab methods
// live on `EditorHandle` (lit up by the feature); these add the error type and the
// by-container inbound entry point. On web there is no thread marshalling — a JS
// transport callback runs on the main thread, so it calls `handle.collab_receive`
// (or `collab_receive_for(container_id, ..)`) directly; there is no `post_remote_delta`
// (that is the desktop runtime's off-thread marshaller).
#[cfg(feature = "collaboration")]
pub use rinch_editor_view::{CollabError, collab_receive_for};
// Reconciliation for poll/reconnect transports (`EditorHandle::collab_state_vector` /
// `collab_sync_diff`) needs no extra imports: state vectors and diffs are opaque
// `Vec<u8>`, and a diff is applied through the same `collab_receive` as a delta.
