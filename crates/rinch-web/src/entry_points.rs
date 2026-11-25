//! The mount entry points: a whole-page app, islands, and both under a
//! menu bar.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;

use crate::page_init::ensure_global_init;
use crate::roots::mount_tree;
use crate::{Menu, RootHandle, WebDocument, menu_bar};

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
/// Its root never ends, so its document is never swept from the registries
/// keyed by document ([`rinch_core::forget_doc`]): what it registers there
/// stays for the page's life. A root that can go is made with a `mount*`
/// that answers a [`RootHandle`], whose [`unmount`](RootHandle::unmount)
/// sweeps its document, what its cleanups register as it goes included.
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
    let _ = mount_tree(web_doc, true, build);
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

    mount_tree(web_doc, false, build)
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
/// uses — and the menus are the same [`Menu`] / [`MenuItem`](crate::MenuItem)
/// values a desktop build hands to `App::menu`, so one declaration serves both
/// targets.
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

    let _ = mount_tree(web_doc, true, move |scope| {
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

    mount_tree(web_doc, false, move |scope| {
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
