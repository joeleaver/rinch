//! Browser-native DOM backend for rinch (WASM target).
//!
//! Instead of painting to a Canvas 2D or WebGPU surface, this crate creates real
//! browser DOM elements via `web_sys`. The browser handles layout, CSS, text
//! rendering, and painting natively. The reactive system (Signal/Effect) and all
//! components work through [`rinch_core::dom::NodeHandle`] →
//! [`rinch_core::dom::DomDocument`], so everything works automatically.
//!
//! It provides:
//! - [`WebDocument`] — a [`DomDocument`](rinch_core::dom::DomDocument) implementation over `web_sys`.
//! - [`setup_event_delegation`] — document-level listeners that bridge browser
//!   events into rinch's event/drag/render-surface systems.
//! - [`mount`] — boot a single whole-page app under `#rinch-body`.
//! - [`mount_into`] / [`mount_selector`] — mount one or more **independent**
//!   component trees ("islands") into existing page elements. Each returns a
//!   [`RootHandle`] that can later [`unmount`](RootHandle::unmount) that root.
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

mod editor_input;
mod event_delegation;
pub mod web_document;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;

pub use event_delegation::setup_event_delegation;
pub use web_document::WebDocument;

// The renderer-agnostic rich-text editor, re-exported so a web app uses the *same*
// API as desktop: `create_editor()` → `EditorHandle`, mounted via `Editor {}`. The
// browser input glue (keyboard/pointer/IME/clipboard → the handle) lives in
// `editor_input`, installed once alongside event delegation.
pub use rinch_editor_view::{Editor, EditorHandle, create_editor};

// Collaborative editing (M9), behind the `collaboration` feature. The collab methods
// live on `EditorHandle` (lit up by the feature); these add the error type and the
// by-container inbound entry point. On web there is no thread marshalling — a JS
// transport callback runs on the main thread, so it calls `handle.collab_receive`
// (or `collab_receive_for(container_id, ..)`) directly; there is no `post_remote_delta`
// (that is the desktop runtime's off-thread marshaller).
#[cfg(feature = "collaboration")]
pub use rinch_editor_view::{CollabError, collab_receive_for};
// The Automerge sync-protocol types, for a caller driving
// `EditorHandle::collab_generate_sync_message` / `collab_receive_sync_message` (the
// reconciling alternative to delta broadcast, for poll/reconnect transports).
#[cfg(feature = "collaboration")]
pub use rinch_editor_view::{ChangeHash, SyncMessage, SyncState};

// ============================================================================
// Mounted-root registry + per-page guards
// ============================================================================

/// A live root and everything that must stay alive for it to keep reacting.
struct MountedRoot {
    web_doc: Rc<RefCell<WebDocument>>,
    /// Held so the render scope (and the effects that reference it) stay alive.
    #[allow(dead_code)]
    scope: Rc<RefCell<RenderScope>>,
    root: NodeHandle,
    handler_start: events::EventHandlerId,
    handler_end: events::EventHandlerId,
}

thread_local! {
    /// All currently-mounted roots, keyed by [`RootHandle`] id. Owning the roots
    /// here (rather than leaking them) keeps each alive even if the caller drops
    /// its handle, and lets [`RootHandle::unmount`] tear an individual one down.
    static MOUNTED_ROOTS: RefCell<HashMap<u64, MountedRoot>> = RefCell::new(HashMap::new());
    static NEXT_ROOT_ID: Cell<u64> = const { Cell::new(0) };
    /// Page-global, first-mount-only initialization guard.
    static GLOBAL_INIT: Cell<bool> = const { Cell::new(false) };
    /// Document-level event listeners are installed exactly once per page.
    static DELEGATION_INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// A handle to a mounted root. Drop it freely (the root stays mounted); call
/// [`unmount`](Self::unmount) to remove that root's DOM and handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RootHandle {
    id: u64,
}

impl RootHandle {
    /// The stable numeric id of this root.
    pub fn id(self) -> u64 {
        self.id
    }

    /// Unmount this root: remove its component subtree from the host element and
    /// deregister the event handlers it created at build time. The host element
    /// itself is left in place. A no-op if the root was already unmounted.
    pub fn unmount(self) {
        let root = MOUNTED_ROOTS.with(|m| m.borrow_mut().remove(&self.id));
        if let Some(r) = root {
            r.web_doc.borrow_mut().remove_node(r.root.node_id());
            events::remove_handlers_in_range(r.handler_start, r.handler_end);
            // Dropping `r` releases the WebDocument and RenderScope for this root.
        }
    }
}

// ============================================================================
// One-time page-global setup
// ============================================================================

/// The web timer backend: schedule `fire_timeout(id)` via `window.setTimeout`.
///
/// Only `id` is captured, so the parked callback stays in rinch-core's
/// main-thread registry. The closure is created with `Closure::once_into_js`,
/// which hands ownership to the JS GC and drops the boxed `FnOnce` after it fires
/// — so, unlike `Closure::forget()`, arming thousands of timers over a long
/// session does not leak a closure per timer. A cancelled timeout
/// (`clear_timeout`) still wakes here, then finds nothing parked and no-ops.
fn web_set_timeout(id: u64, delay_ms: u32) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    let cb = Closure::once_into_js(move || rinch_core::fire_timeout(id));
    if let Some(window) = web_sys::window() {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            cb.unchecked_ref(),
            delay_ms as i32,
        );
    }
}

/// First-mount-only global init shared by every mount. Clearing handlers/context
/// must NOT run per-root, or a second island would wipe the first.
fn ensure_global_init() {
    GLOBAL_INIT.with(|f| {
        if f.get() {
            return;
        }
        f.set(true);

        // Clear stale handlers/context from any previous run of this wasm module.
        events::clear_handlers();
        rinch_core::clear_context();

        // Drive `rinch_core::set_timeout` off `window.setTimeout`. The browser has
        // no threads, so rinch-core's built-in shared-thread scheduler cannot run
        // here — the web runtime supplies the scheduling half.
        rinch_core::set_timer_backend(web_set_timeout);

        // Theme CSS is page-global (one shared `<style>`). On any signal change
        // (e.g. dark-mode toggle), refresh it once — regardless of which root
        // changed. Fine-grained DOM updates are handled by effects directly.
        rinch_core::set_on_signal_change(|| {
            if let Some(css) = rinch_core::get_current_theme_css() {
                web_document::update_theme_style_global(&css);
            }
        });
    });
}

/// Install document-level event delegation exactly once. The listeners are
/// page-global (they key off `[data-rid]` / `closest(...)`, not any one
/// document), so a single install serves every root.
fn ensure_event_delegation(doc: &WebDocument) {
    DELEGATION_INSTALLED.with(|f| {
        if f.get() {
            return;
        }
        f.set(true);
        setup_event_delegation(doc);
    });
}

// ============================================================================
// Mount entry points
// ============================================================================

/// Shared mount core: build the tree into `web_doc`'s body, wire theme + event
/// delegation, register the root, and return its handle.
fn mount_tree<F>(web_doc: Rc<RefCell<WebDocument>>, build: F) -> RootHandle
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    let doc_as_dom: Rc<RefCell<dyn DomDocument>> = web_doc.clone();
    let body_id = web_doc.borrow().body();
    let scope = Rc::new(RefCell::new(RenderScope::new(doc_as_dom, body_id)));

    set_render_scope(scope.clone());

    // Handler ids allocated during this build belong to this root (builds are
    // sequential, so the range is contiguous and exclusively ours).
    let handler_start = events::handler_id_watermark();
    let root = {
        let mut scope_ref = scope.borrow_mut();
        build(&mut scope_ref)
    };
    let handler_end = events::handler_id_watermark();

    web_doc.borrow_mut().append_child(body_id, root.node_id());

    clear_render_scope();

    // Inject/refresh the page-global theme `<style>` (idempotent across roots).
    if let Some(css) = rinch_core::get_current_theme_css() {
        web_document::update_theme_style_global(&css);
    }

    ensure_event_delegation(&web_doc.borrow());
    editor_input::install(web_doc.borrow().browser_document());

    let id = NEXT_ROOT_ID.with(|c| {
        let id = c.get();
        c.set(id + 1);
        id
    });
    MOUNTED_ROOTS.with(|m| {
        m.borrow_mut().insert(
            id,
            MountedRoot {
                web_doc,
                scope,
                root,
                handler_start,
                handler_end,
            },
        );
    });
    RootHandle { id }
}

/// Mount a single whole-page rinch app under `#rinch-body` (appended to
/// `document.body`).
///
/// `build` receives the active [`RenderScope`]; a `#[component]` function (which
/// expands to `fn(&mut RenderScope) -> NodeHandle`) can be passed directly. The
/// root lives for the lifetime of the page.
pub fn mount<F>(theme: ThemeProviderProps, build: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    ensure_global_init();
    rinch::setup_theme_css(&theme);

    let browser_doc = web_sys::window().unwrap().document().unwrap();
    let web_doc = Rc::new(RefCell::new(WebDocument::new(browser_doc)));

    // Whole-page app: keep it for the page lifetime (owned by MOUNTED_ROOTS).
    let _ = mount_tree(web_doc, build);
}

/// Mount an independent rinch component tree (an "island") into an existing
/// browser element. Multiple islands can coexist on one page.
///
/// Returns a [`RootHandle`] that can later [`unmount`](RootHandle::unmount) this
/// root. The handle may be dropped — the root stays mounted until unmounted.
pub fn mount_into<F>(host: &web_sys::Element, theme: ThemeProviderProps, build: F) -> RootHandle
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    ensure_global_init();
    rinch::setup_theme_css(&theme);

    let browser_doc = web_sys::window().unwrap().document().unwrap();
    let web_doc = Rc::new(RefCell::new(WebDocument::new_into(
        browser_doc,
        host.clone(),
    )));

    mount_tree(web_doc, build)
}

/// Mount an island into the first element matching a CSS `selector`
/// (e.g. `"#comments"`). Returns `None` if no element matches.
pub fn mount_selector<F>(selector: &str, theme: ThemeProviderProps, build: F) -> Option<RootHandle>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    let doc = web_sys::window()?.document()?;
    let host = doc.query_selector(selector).ok().flatten()?;
    Some(mount_into(&host, theme, build))
}
