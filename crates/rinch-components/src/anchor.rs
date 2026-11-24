//! Anchor component.
//!
//! Styled link component.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};

/// A styled anchor/link element.
///
/// With no `href` it writes none: the `<a>` is a placeholder link, as in a
/// browser (not focusable, nothing to follow). With an `onclick` too it is a
/// button: `tabindex="0"` and `role="button"`, so Tab reaches it and Enter or
/// Space runs the handler.
/// With an `onclick`, the click is the app's: on the web the browser does not
/// follow `href` (the link is marked `data-claim-links`), so a desktop and a
/// web app route in-app the same way (a middle click comes to it too, with
/// `click_context().button` `Middle`); the handler can read the link with
/// `click_link()`. The browser's own link menu stays.
#[derive(Debug, Default)]
pub struct Anchor {
    /// Link URL. Empty writes no `href` (a placeholder link).
    pub href: String,
    /// Link target (_blank, _self, etc.).
    pub target: String,
    /// The link's `rel` (`noreferrer`, `nofollow`, ...), written as given.
    /// A browser already treats `target="_blank"` as `noopener`.
    pub rel: String,
    /// Runs on a click (or Enter on a focused link). When set, the browser
    /// does not follow `href` on the web.
    pub onclick: Option<Callback>,
    /// Size (xs, sm, md, lg).
    pub size: String,
    /// Whether to always show underline.
    pub underline: bool,
}

impl Anchor {
    /// Generate the CSS class string for this anchor.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-anchor"];

        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-anchor--xs"),
                "sm" => classes.push("rinch-anchor--sm"),
                "md" => classes.push("rinch-anchor--md"),
                "lg" => classes.push("rinch-anchor--lg"),
                _ => {}
            }
        }

        if self.underline {
            classes.push("rinch-anchor--underline");
        }

        classes.join(" ")
    }
}

impl Component for Anchor {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { a { class: "rinch-anchor" } };
        container.set_attribute("class", &self.class_string());
        // No `href` is a placeholder link (#2206); it used to be `#`, which a
        // browser follows to the top of the page and into its history.
        if !self.href.is_empty() {
            container.set_attribute("href", &self.href);
        }

        if !self.target.is_empty() {
            container.set_attribute("target", &self.target);
        }
        if !self.rel.is_empty() {
            container.set_attribute("rel", &self.rel);
        }

        if let Some(cb) = &self.onclick {
            let cb = cb.clone();
            let handler_id = __scope.register_handler(move || cb.invoke());
            container.set_attribute("data-rid", &handler_id.0.to_string());
            // The click is the app's: rinch-web cancels the browser's
            // navigation for a link inside a `data-claim-links` element.
            // `menu` keeps the browser's own link menu and drag to the tab
            // bar, which a click listener leaves alone in a browser too.
            container.set_attribute("data-claim-links", "menu");
            // With no `href` the `<a>` is a placeholder, which neither a
            // browser nor rinch focuses: the keyboard could not reach the
            // handler (`href="#"` used to make it focusable). A `tabindex`
            // makes it a stop, and Enter / Space activate a focused
            // `tabindex` element on both backends, as for a `role="button"`.
            if self.href.is_empty() {
                container.set_attribute("tabindex", "0");
                container.set_attribute("role", "button");
            }
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
