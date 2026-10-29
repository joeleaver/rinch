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
//! - [`AppExt`] — run a [`rinch::App`] builder chain in the browser, so one
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
//! The desktop's [`rinch::App`] builder runs here too, through [`AppExt`]; the
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

mod bounds;
mod editor_input;
mod event_delegation;
mod images;
mod menu_bar;
pub mod web_document;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rinch_core::dom::{DomDocument, NodeHandle, RenderScope, clear_render_scope, set_render_scope};
use rinch_core::element::ThemeProviderProps;
use rinch_core::events;

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
pub use event_delegation::{set_suppress_native_context_menu, suppresses_native_context_menu};
#[doc(hidden)]
pub use images::__reset_image_sources;
/// App-resolved image sources: an `<img>` whose `src` has a scheme the app
/// registered shows what the app resolves it to. `rinch::image::
/// register_image_scheme` and `rinch::image::reload_image` are the same calls as
/// on desktop; `register_image_url_scheme` is the browser-only form that answers
/// with a URL.
pub use images::{LOGICAL_SRC_ATTR, register_image_url_scheme, unregister_image_url_scheme};
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
    ImageInputSource, LinkClick, LinkHover, LinkSpan, MarginAnchor, MarginAssoc, MarginAt,
    MarginLine, NODE_VIEW_ATTR, NodeViewCtx, NodeViewRegistration, NodeViewRender, NodeViewSpec,
    OVERLAY_SLOT_ATTR, ScrollAlign, SelectionAnchor, clear_image_source, create_editor,
    history_now_ms, set_image_source,
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
            // Before the borrow: a browser runs its focus listeners inside
            // the removal of a focused node, and a key entry that outlives
            // this root still reads the document from one (issue #1478).
            r.root.release_focus_before_detach();
            r.web_doc.borrow_mut().remove_node(r.root.node_id());
        }
    }
}

// ============================================================================
// One-time page-global setup
// ============================================================================

/// Install the web runtime without mounting anything (issue #1956): the
/// timer backend `set_timeout` and `sleep` run on, the scheduler that polls
/// `spawn_local` tasks, and the drain that samples `poll_signal`s.
///
/// Every `mount*` calls this, so a page that mounts a root needs nothing more.
/// It is for a page that mounts no rinch DOM but runs rinch code that arms
/// timers, spawns tasks or polls — a game that owns the canvas, say, or
/// `rinch-ws` / `rinch-rtc` code with no UI of its own. Call it at startup,
/// before the first timer: a `set_timeout` armed earlier never fires (a task
/// spawned earlier is picked up). Idempotent, page-global, and it builds no
/// DOM: no event delegation, no theme `<style>`, no root.
///
/// The same function as `rinch::install_web_runtime`, for an app that
/// depends on `rinch` alone. An embedded `rinch::embed::RinchContext` installs
/// the timer backend and task scheduler itself on wasm32.
pub fn install_runtime() {
    // A no-op on host builds of this crate (the workspace's clippy and check):
    // `rinch::install_web_runtime` installs nothing off wasm32.
    rinch::install_web_runtime();
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

        // Timers, tasks and polls: the same install a page with no root makes
        // (issue #1956). Idempotent, so a page that called it first is fine.
        install_runtime();

        // Theme CSS is page-global (one shared `<style>`). On any signal change
        // (e.g. dark-mode toggle), refresh it once — regardless of which root
        // changed. Fine-grained DOM updates are handled by effects directly.
        //
        // Keyed on the theme's generation, so a flush that did not touch the
        // theme costs one integer compare rather than a clone of the whole
        // stylesheet string.
        rinch_core::set_on_signal_change(|| {
            thread_local! {
                static APPLIED_THEME_GENERATION: std::cell::Cell<Option<u64>> =
                    const { std::cell::Cell::new(None) };
            }
            let generation = rinch_core::current_theme_css_generation();
            if APPLIED_THEME_GENERATION.with(|g| g.replace(Some(generation))) == Some(generation) {
                return;
            }
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

    // Ask the app for the image sources this build gave its `<img>`s, now
    // that no borrow of the document is held (`images.rs`).
    images::resolve_pending();

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

/// The page's document, or `None` where there is none: a Web Worker, where
/// `rinch::worker` runs a second instance of the app's module and so its start
/// function (issue #1772).
fn page_document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

/// Mount a single whole-page rinch app under `#rinch-body` (appended to
/// `document.body`).
///
/// `build` receives the active [`RenderScope`]; a `#[component]` function (which
/// expands to `fn(&mut RenderScope) -> NodeHandle`) can be passed directly. The
/// root lives for the lifetime of the page.
///
/// Where there is no `document` — inside a `rinch::worker` Web Worker, which
/// runs the app's start function too — it returns without mounting.
pub fn mount<F>(theme: ThemeProviderProps, build: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    // No page: a worker instance of this module (`rinch::worker`, #1772),
    // which runs the app's start function too. Nothing to mount.
    let Some(browser_doc) = page_document() else {
        return;
    };
    ensure_global_init();
    rinch::setup_theme_css(&theme);

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
/// accumulate chords. Some chords are the browser's own — `Ctrl+N`, `Ctrl+T`
/// and `Ctrl+W` in Chrome and Firefox, and more depending on the browser and
/// platform — and cannot be claimed; declare them anyway if the same `Menu`
/// drives a desktop build, just do not rely on them here.
///
/// The menus are read during this call and nothing is kept borrowed afterwards,
/// which is why the labels may be borrowed `&str`.
///
/// Like [`mount`], it returns without mounting where there is no `document`.
pub fn mount_with_menu_bar<F>(theme: ThemeProviderProps, menus: Vec<(&str, Menu)>, build: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle,
{
    // No page: a worker instance (see `mount`).
    let Some(browser_doc) = page_document() else {
        return;
    };
    ensure_global_init();
    rinch::setup_theme_css(&theme);

    let web_doc = Rc::new(RefCell::new(WebDocument::new(browser_doc)));

    let _ = mount_tree(web_doc, move |scope| {
        let content = build(scope);
        menu_bar::wrap(scope, &menus, content)
    });
}

/// Mount an island into `host` under a menu bar. See [`mount_into`] and
/// [`mount_with_menu_bar`].
///
/// The bar fills the host element's width and the content sits below it. It
/// does **not** fit itself to the host's height: the wrapper it builds is
/// `height: 100vh` from the component stylesheet, so a host shorter than the
/// viewport is overflowed by it, and giving the host a height does not change
/// that. Measured in Chrome 153: a `height: 120px` host gets a 437px wrapper in
/// a 437px viewport. Tracked separately. Until it is fixed the workaround is an
/// author CSS rule more specific than the component sheet's bare class —
/// measured: `div.rinch-app-menu-bar-wrapper { height: 100% }` gives that 120px
/// host a 120px wrapper.
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

// ============================================================================
// The desktop `App` builder, run in the browser (#1769)
// ============================================================================

/// Runs a [`rinch::App`] builder chain in the browser, so a cross-platform app
/// configures itself once and calls `run()` on both targets (#1769).
///
/// ```ignore
/// use rinch::prelude::*;
/// #[cfg(target_arch = "wasm32")]
/// use rinch_web::AppExt as _;
///
/// fn main() {
///     App::new(app).title("My App").size(800, 600).theme(theme()).menu(menus()).run();
/// }
/// ```
///
/// On a desktop build `App::run` is the inherent method and this trait is not
/// consulted; import it only on `wasm32`, the one target this crate builds it
/// for.
///
/// Forgetting the import fails with `error[E0599]: no method named `run`
/// found for struct `rinch::App``; the compiler suggests this trait only if the
/// crate already names `rinch_web` elsewhere.
///
/// Implemented once per menu-bar state ([`NoMenuBar`](rinch::app_builder::NoMenuBar),
/// [`WithMenuBar`](rinch::app_builder::WithMenuBar)), so an app with no
/// `.menu(..)` does not link the DOM menu bar.
#[cfg(target_arch = "wasm32")]
pub trait AppExt {
    /// Mount the app as a whole-page app under `#rinch-body` and return.
    ///
    /// What each builder setting does here:
    /// - [`theme`](rinch::App::theme): installed, as by [`mount`].
    /// - [`menu`](rinch::App::menu): the app is mounted under the DOM menu bar,
    ///   as by [`mount_with_menu_bar`]; without one, as by [`mount`].
    /// - [`title`](rinch::App::title) (or a non-default `WindowProps::title`):
    ///   becomes `document.title`. Without one the page keeps its own `<title>`.
    /// - [`image_scheme`](rinch::App::image_scheme): registered when called, as
    ///   on the desktop.
    /// - [`size`](rinch::App::size) and the rest of
    ///   [`window_props`](rinch::App::window_props): ignored — the page, not the
    ///   app, sizes a browser window.
    /// - [`fonts`](rinch::App::fonts): accepted and ignored — the browser loads
    ///   fonts from the page's CSS (`@font-face`).
    /// - The GPU configuration (`gpu_config`, `external_gpu`, `renderer`): does
    ///   not exist on a web build — the browser composites, and a
    ///   `RenderSurface` app owns its own canvas context.
    fn run(self);
}

/// Sets `document.title` from the builder, when it names one.
#[cfg(target_arch = "wasm32")]
fn apply_title<F>(parts: &rinch::app_builder::WebParts<F>) {
    if let (Some(title), Some(doc)) = (&parts.title, web_sys::window().and_then(|w| w.document())) {
        doc.set_title(title);
    }
}

/// No `.menu()` on the chain: mounted as by [`mount`]. Implemented apart from
/// the [`WithMenuBar`](rinch::app_builder::WithMenuBar) state so that an app
/// with no menu bar does not link it.
#[cfg(target_arch = "wasm32")]
impl<F> AppExt for rinch::App<F, rinch::app_builder::NoMenuBar>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    fn run(self) {
        let parts = self.into_web_parts();
        apply_title(&parts);
        mount(parts.theme, parts.component);
    }
}

/// `.menu(..)` on the chain: mounted under the DOM menu bar, as by
/// [`mount_with_menu_bar`].
#[cfg(target_arch = "wasm32")]
impl<F> AppExt for rinch::App<F, rinch::app_builder::WithMenuBar>
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    fn run(self) {
        let parts = self.into_web_parts();
        apply_title(&parts);
        let (labels, menus): (Vec<String>, Vec<Menu>) =
            parts.menus.unwrap_or_default().into_iter().unzip();
        let menus: Vec<(&str, Menu)> = labels.iter().map(String::as_str).zip(menus).collect();
        mount_with_menu_bar(parts.theme, menus, parts.component);
    }
}
