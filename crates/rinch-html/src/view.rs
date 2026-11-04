//! `HtmlView`: a [`Cleaned`](crate::Cleaned) document in the app's DOM.
//!
//! ```ignore
//! let cleaned = rinch::html::clean(&document, &Policy::default());
//! rsx! {
//!     HtmlView {
//!         html: cleaned.html,
//!         body_style: cleaned.body_style,
//!         onlink: move |href: String| open_if_web(&href),
//!     }
//! }
//! ```
//!
//! It renders `div.rinch-html-view` holding `div.rinch-html-body` (the
//! document's body: `body_style` on it, `html` inside). A click on a link in
//! the document calls `onlink` with the link's `href` (as written, after
//! [`clean`](crate::clean)'s scheme check); the view never navigates or
//! opens anything itself. Links show a pointer cursor.
//!
//! The view is drawn once: to show another document, render a new view
//! (key it on the document, in a `for` loop).

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};

/// Shows cleaned HTML; see the module docs.
#[derive(Default)]
pub struct HtmlView {
    /// The cleaned document's body ([`Cleaned::html`](crate::Cleaned::html)).
    /// Put only cleaned HTML here: it goes into the DOM as it is.
    pub html: String,
    /// The body's style ([`Cleaned::body_style`](crate::Cleaned::body_style)).
    pub body_style: String,
    /// Called with a clicked link's `href`.
    pub onlink: Option<InputCallback>,
}

impl std::fmt::Debug for HtmlView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlView")
            .field("html", &format_args!("<{} bytes>", self.html.len()))
            .field("body_style", &self.body_style)
            .field("onlink", &self.onlink.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

/// The view's own CSS: only what links need, scoped to the view.
const CSS: &str = ".rinch-html-view a[href] { cursor: pointer; }";

impl Component for HtmlView {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let view = scope.create_element("div");
        view.set_attribute("class", "rinch-html-view");
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        view.append_child(&style);

        let body = scope.create_element("div");
        body.set_attribute("class", "rinch-html-body");
        if !self.body_style.is_empty() {
            body.set_attribute("style", &self.body_style);
        }
        view.append_child(&body);
        body.set_inner_html(&self.html);

        if let Some(onlink) = self.onlink.clone() {
            // One handler for the whole document: a click anywhere in it
            // lands here, and `click_link` says which link it went through.
            let handler = scope.register_handler(move || {
                if let Some(href) = rinch_core::click_link() {
                    onlink.invoke(href);
                }
            });
            view.set_attribute("data-rid", &handler.0.to_string());
        }
        view
    }
}
