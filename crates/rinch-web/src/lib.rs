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
//! - [`mount_with_menu_bar`] and its `_into` / `_selector` twins — the same,
//!   under a menu bar built from [`Menu`] / [`MenuItem`].
//! - [`set_suppress_native_context_menu`] — for a whole-page app that renders
//!   its own right-click menus.
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
//! does by returning ahead of the event loop. A handful of chords (`Ctrl+N`,
//! `Ctrl+T`, `Ctrl+W`, `Ctrl+Q` and their `Shift` variants) are the browser's
//! alone and cannot be claimed; see the [WASM
//! guide](https://github.com/joeleaver/rinch/blob/main/docs/src/guide/wasm.md).
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
mod menu_bar;
pub mod web_document;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;

pub use event_delegation::setup_event_delegation;
#[doc(hidden)]
pub use event_delegation::{__force_trusted_clicks, __reset_activation_state};
// Whether a right-click anywhere suppresses the browser's own menu. Off by
// default: an island mounted into somebody else's page must not take the
// right-click away from the rest of it.
pub use event_delegation::{set_suppress_native_context_menu, suppresses_native_context_menu};
/// The menu declaration types, re-exported so a web app names them in one place
/// (`rinch_web::{Menu, MenuItem}`) while its desktop twin builds the very same
/// values from `rinch::menu`. They are the same types, not a parallel set.
pub use rinch::menu::{Menu, MenuEntryRef, MenuItem};
pub use web_document::WebDocument;
/// Test-only handles on the page scroll lock (#474), so a fixture that fails
/// between a lock and its unlock cannot leave `<html>` hidden for every test
/// after it.
#[doc(hidden)]
pub use web_document::{__reset_scroll_lock, __scroll_lock_depth};

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
// Reconciliation for poll/reconnect transports (`EditorHandle::collab_state_vector` /
// `collab_sync_diff`) needs no extra imports: state vectors and diffs are opaque
// `Vec<u8>`, and a diff is applied through the same `collab_receive` as a delta.

// ============================================================================
// Mounted-root registry + per-page guards
// ============================================================================

/// A live root and everything that must stay alive for it to keep reacting.
struct MountedRoot {
    web_doc: Rc<RefCell<WebDocument>>,
    /// Held so the render scope (and the effects that reference it) stay alive.
    /// It also *owns* everything the build created — handlers included — so
    /// dropping it is what reclaims the root (issue #141).
    scope: Rc<RefCell<RenderScope>>,
    root: NodeHandle,
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
    ///
    /// Disposing the scope is also what releases what the build registered
    /// *page-globally* — a menu bar's Escape handler and its keyboard chords —
    /// so an island can be taken out of somebody else's page without leaving
    /// anything of its own behind.
    pub fn unmount(self) {
        let root = MOUNTED_ROOTS.with(|m| m.borrow_mut().remove(&self.id));
        if let Some(r) = root {
            // Dispose the scope *first*, while the DOM it was built against is
            // still standing: disposal runs this root's cleanups and drops its
            // effect closures, and those legitimately touch their own nodes.
            // Tearing the tree down first left them patching a corpse.
            //
            // Disposal is also what deregisters this root's event handlers now,
            // replacing the old `handler_id_watermark` range. The range was both
            // incomplete and unsound: it only covered ids allocated during the
            // synchronous build (missing every handler a later effect run
            // registers), and a synchronous effect flush inside the build lets
            // an *unrelated* root allocate an id inside the recorded window,
            // which unmounting this root would then delete. A scope's own record
            // has neither problem (issue #141).
            match Rc::try_unwrap(r.scope) {
                // The expected path: nothing else can reach the scope, so its
                // cleanups run with no borrow outstanding anywhere.
                Ok(cell) => cell.into_inner().dispose(),
                Err(shared) => shared.borrow_mut().dispose_in_place(),
            }
            r.web_doc.borrow_mut().remove_node(r.root.node_id());
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

    // The root scope owns everything this build creates — signals, memos,
    // effects and event handlers — and `unmount` releases them by disposing it
    // (issue #141). Scoped to the build alone: theme injection, event delegation
    // and root registration below are page-global setup with app lifetime.
    let root = {
        let _owner = scope.borrow().push_owner();
        let mut scope_ref = scope.borrow_mut();
        build(&mut scope_ref)
    };

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

// ============================================================================
// Mount entry points, under a menu bar
// ============================================================================

/// Mount a whole-page app under a menu bar built from `menus`.
///
/// [`mount`], plus the DOM menu bar above the app: each `(label, menu)` pair
/// becomes one top-level menu, in order. The bar is
/// [`rinch::menu::render_with_menu_bar`] — the same renderer the Linux desktop
/// uses — and the menus are the same [`Menu`] / [`MenuItem`] values a desktop
/// build hands to `App::menu`, so one declaration serves both targets.
///
/// Every item's `shortcut` is armed against the page: pressing it runs the
/// item's `on_click` and consumes the keystroke, so neither the browser's own
/// handling of that combination nor anything else in the app also acts on it.
/// Arming replaces whatever a previous call armed, so remounting does not
/// accumulate chords. A handful of chords — `Ctrl+N`, `Ctrl+T`, `Ctrl+W`,
/// `Ctrl+Q` and their `Shift` variants — are the browser's own and cannot be
/// claimed at all; declare them anyway if the same `Menu` drives a desktop
/// build, just do not rely on them here.
///
/// The bar is laid out above the content *inside* the page, and the content
/// wrapper is `height: 100%` — so `html, body` want a height, or everything
/// below the bar collapses.
///
/// The menus are read during this call and nothing is kept borrowed afterwards,
/// which is why the labels may be borrowed `&str`.
pub fn mount_with_menu_bar<F>(theme: ThemeProviderProps, menus: Vec<(&str, Menu)>, build: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    ensure_global_init();
    rinch::setup_theme_css(&theme);

    let browser_doc = web_sys::window().unwrap().document().unwrap();
    let web_doc = Rc::new(RefCell::new(WebDocument::new(browser_doc)));

    let _ = mount_tree(web_doc, move |scope| {
        let content = build(scope);
        menu_bar::wrap(scope, &menus, content)
    });
}

/// Mount an island into `host` under a menu bar. See [`mount_into`] and
/// [`mount_with_menu_bar`].
///
/// The bar fills the host element's width and the content sits below it, so the
/// host wants a height of its own — the bar is laid out inside the island, not
/// over the page, and the content wrapper below it is `height: 100%`, which
/// collapses without one.
///
/// **One page, one set of chords.** Clicks are per-bar, but keyboard shortcuts
/// are matched against a single page-global registry, and arming a bar releases
/// whatever the previous one armed. So two islands that each declare menus will
/// find only the second one's shortcuts live. Give one island the shortcuts, or
/// declare the chords in a single bar.
///
/// [`unmount`](RootHandle::unmount) takes this island's chords back down —
/// they are armed against the whole page, and an island removed from somebody
/// else's page must not go on eating a key combination from it. Release is
/// "only if these are still the live ones", so an island unmounting after a
/// *later* one armed leaves the later one's chords alone.
pub fn mount_into_with_menu_bar<F>(
    host: &web_sys::Element,
    theme: ThemeProviderProps,
    menus: Vec<(&str, Menu)>,
    build: F,
) -> RootHandle
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

    mount_tree(web_doc, move |scope| {
        let content = build(scope);
        menu_bar::wrap(scope, &menus, content)
    })
}

/// Mount an island under a menu bar into the first element matching `selector`.
/// Returns `None` if no element matches. See [`mount_selector`].
pub fn mount_selector_with_menu_bar<F>(
    selector: &str,
    theme: ThemeProviderProps,
    menus: Vec<(&str, Menu)>,
    build: F,
) -> Option<RootHandle>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    let doc = web_sys::window()?.document()?;
    let host = doc.query_selector(selector).ok().flatten()?;
    Some(mount_into_with_menu_bar(&host, theme, menus, build))
}
