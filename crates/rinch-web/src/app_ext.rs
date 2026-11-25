//! The desktop `App` builder, run in the browser (#1769).

use rinch_core::dom::{NodeHandle, RenderScope};

use crate::{Menu, mount, mount_with_menu_bar};

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
    /// - [`spellcheck`](rinch::App::spellcheck): the browser checks the
    ///   fields; `false` puts `spellcheck="false"` on `<body>` (see
    ///   [`apply_spellcheck`]). The rich-text editor is checked by rinch,
    ///   with the configuration's dictionaries, only when the app supplied
    ///   some (`SpellcheckConfig::checker`); a field's dictionaries and skip
    ///   rules are the browser's.
    /// - The GPU configuration (`gpu_config`, `external_gpu`, `renderer`): does
    ///   not exist on a web build — the browser composites, and a
    ///   `RenderSurface` app owns its own canvas context.
    fn run(self);
}

/// Sets `document.title` from the builder, when it names one, and applies
/// its spellcheck switch.
#[cfg(target_arch = "wasm32")]
fn apply_title<F>(parts: &rinch::app_builder::WebParts<F>) {
    if let (Some(title), Some(doc)) = (&parts.title, web_sys::window().and_then(|w| w.document())) {
        doc.set_title(title);
    }
    apply_spellcheck(&parts.spellcheck);
}

/// The app's spellcheck switch on the web (#2862): the browser checks the
/// page's fields itself, and rinch's checker never runs on them. Off, `<body>`
/// gets `spellcheck="false"`, which every field and editable region on the
/// page inherits (HTML's rule) unless one sets its own; on, the page is left
/// to the browser's default (fields checked). For fields the rest of the
/// configuration (skip rules, the interaction and blur options) is the
/// browser's. The rich-text editor, which the browser cannot check, is
/// checked by rinch with the configuration (switch, dictionaries, skip rules,
/// debounce, slices) when it holds the app's own dictionaries; with none it
/// is not checked.
///
/// It also keeps `<body>` in step with every later
/// [`rinch::spellcheck::configure`] / [`rinch::spellcheck::set_enabled`].
#[cfg(target_arch = "wasm32")]
pub fn apply_spellcheck(config: &rinch::spellcheck::SpellcheckConfig) {
    thread_local! {
        static LISTENING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !LISTENING.with(|l| l.replace(true)) {
        rinch::spellcheck::on_config_change(sync_body_spellcheck);
    }
    rinch::spellcheck::configure(config.clone());
    sync_body_spellcheck();
}

/// `<body>`'s `spellcheck` for the current configuration.
#[cfg(target_arch = "wasm32")]
fn sync_body_spellcheck() {
    let enabled = rinch::spellcheck::config().is_none_or(|c| c.enabled);
    let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    else {
        return;
    };
    if enabled {
        if body.get_attribute("spellcheck").as_deref() == Some("false") {
            let _ = body.remove_attribute("spellcheck");
        }
    } else {
        let _ = body.set_attribute("spellcheck", "false");
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
