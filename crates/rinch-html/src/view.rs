//! `HtmlView`: a document someone else wrote, in the app's DOM.
//!
//! Give it the document and a [`Policy`](crate::Policy), and it cleans the
//! document at its own width, and again whenever a resize crosses one of the
//! document's `@media` breakpoints, so a narrow view gets the document's
//! narrow layout (what a phone shows) and a wide one its wide layout:
//!
//! ```ignore
//! rsx! {
//!     HtmlView {
//!         document: source,
//!         policy: Policy::default(),
//!         onlink: move |href: String| open_if_web(&href),
//!     }
//! }
//! ```
//!
//! Or give it HTML already cleaned ([`clean`](crate::clean)): `html` and
//! `body_style`, drawn as they are at any width.
//!
//! It renders `div.rinch-html-view` holding `div.rinch-html-body` (the
//! document's body: its style on it, its content inside). A click on a link
//! in the document calls `onlink` with the link's `href` (as written, after
//! the cleaner's scheme check); the view never navigates or opens anything
//! itself, and on the web the browser does not follow it either: a middle
//! click comes to `onlink` too (`rinch_core::click_context().button` says
//! which button), and the browser's link menu and link dragging are off
//! unless `link_menu` allows them. Links show
//! a pointer cursor and, by default, their address as a tooltip, with a
//! warning when it hides or disguises where it goes ([`Tooltips`]).
//!
//! To show another document, render a new view (key it on the document, in
//! a `for` loop).

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};

use crate::{Cleaned, Policy, Tooltips};

/// Shows a document someone else wrote; see the module docs.
pub struct HtmlView {
    /// The document as written, cleaned by the view at its own width under
    /// `policy`. Takes the place of `html` and `body_style`.
    pub document: String,
    /// What the view's cleaning lets through (pictures, link schemes); the
    /// default policy when `None`.
    pub policy: Option<Policy>,
    /// HTML already cleaned ([`Cleaned::html`]), when `document` is empty.
    /// Put only cleaned HTML here: it goes into the DOM as it is.
    pub html: String,
    /// Its body's style ([`Cleaned::body_style`]).
    pub body_style: String,
    /// Called with a clicked link's `href`.
    pub onlink: Option<InputCallback>,
    /// How links are titled: their address and a warning when it hides or
    /// disguises where it goes ([`Tooltips::Default`], in English), the
    /// app's own words, or nothing.
    pub tooltips: Tooltips,
    /// On the web, let the browser's own link menu ("Open link in new tab")
    /// and dragging a link to the tab bar work on the document's links:
    /// both open the address without `onlink`, so they are off by default.
    pub link_menu: bool,
    /// Whether the document's text can be selected and copied, as in a
    /// browser (the default). `false` makes it unselectable, as an app's own
    /// text is. This is the switch on desktop: a style sheet's `user-select`
    /// is not read there yet, only an element's own `style`.
    pub selectable: bool,
}

impl Default for HtmlView {
    fn default() -> Self {
        Self {
            document: String::new(),
            policy: None,
            html: String::new(),
            body_style: String::new(),
            onlink: None,
            tooltips: Tooltips::default(),
            link_menu: false,
            selectable: true,
        }
    }
}

impl std::fmt::Debug for HtmlView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlView")
            .field("document", &format_args!("<{} bytes>", self.document.len()))
            .field("policy", &self.policy)
            .field("html", &format_args!("<{} bytes>", self.html.len()))
            .field("body_style", &self.body_style)
            .field("onlink", &self.onlink.as_ref().map(|_| "<callback>"))
            .field("tooltips", &self.tooltips)
            .field("link_menu", &self.link_menu)
            .field("selectable", &self.selectable)
            .finish()
    }
}

/// The view's own CSS, scoped to the view.
///
/// - Nothing the document draws reaches outside the view, to paint over the
///   app or take its clicks: the view clips on both axes and is the
///   stacking context and containing block of everything in it. A document
///   wider than the view scrolls sideways, as a browser scrolls a page wider
///   than its window. (The cleaner already leaves out `position: fixed` and
///   `absolute` and `z-index`; this holds for whatever moves content
///   otherwise: relative offsets, transforms, negative margins.)
/// - A document that sets no colours is drawn as a browser draws a page:
///   black on white, whatever the app's own theme (a dark theme's light text
///   would otherwise reach mail written for a white page).
/// - Links show a pointer.
///
/// An app may restyle `.rinch-html-view` through the cascade (its own
/// colours, its font); one that lets content overflow takes that on.
const CSS: &str = "\
.rinch-html-view { position: relative; z-index: 0; overflow-x: auto; overflow-y: hidden; \
background-color: #ffffff; color: #000000; }
.rinch-html-view a[href] { cursor: pointer; }";

/// The width a document is first cleaned at, before the view has one: wide,
/// as a desktop shows mail.
const FIRST_WIDTH: f32 = 10_000.0;

impl Component for HtmlView {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let view = scope.create_element("div");
        view.set_attribute("class", "rinch-html-view");
        // Its links are the app's to follow (`onlink`): on the web, the
        // browser never navigates the app's own tab to one.
        view.set_attribute("data-claim-links", if self.link_menu { "menu" } else { "" });
        // A document's text can be selected and copied, as in a browser; an
        // app's own controls cannot, by default (`selectable`). rinch reads
        // `user-select` from the `style` attribute on desktop, and the
        // document's own elements take it from here unless they say otherwise.
        view.set_attribute(
            "style",
            if self.selectable {
                "user-select: text"
            } else {
                "user-select: none"
            },
        );
        let style = scope.create_element("style");
        style.append_child(&scope.create_text(CSS));
        view.append_child(&style);

        let body = scope.create_element("div");
        body.set_attribute("class", "rinch-html-body");
        view.append_child(&body);

        if self.document.is_empty() {
            show(
                &body,
                &Cleaned {
                    html: self.html.clone(),
                    body_style: self.body_style.clone(),
                },
                &self.tooltips,
            );
        } else {
            let document = Rc::new(self.document.clone());
            let policy = self.policy.clone().unwrap_or_default();
            let tooltips = self.tooltips.clone();
            show(
                &body,
                &crate::clean_at(&document, &policy, FIRST_WIDTH),
                &tooltips,
            );
            // Clean again when the view's width crosses a breakpoint. The
            // queries are read once; a resize only evaluates them, and only
            // when the width changed.
            let queries = crate::media::queries(&document);
            let shown = Rc::new(RefCell::new((FIRST_WIDTH, queries.at(FIRST_WIDTH))));
            let bounds = view.bounds_signal();
            let body = body.clone();
            scope.create_effect(move || {
                let width = bounds.get().width;
                if width <= 0.0 || shown.borrow().0 == width {
                    return;
                }
                let now = queries.at(width);
                if shown.borrow().1 == now {
                    shown.borrow_mut().0 = width;
                    return;
                }
                show(
                    &body,
                    &crate::clean_at(&document, &policy, width),
                    &tooltips,
                );
                *shown.borrow_mut() = (width, now);
            });
        }

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

fn show(body: &NodeHandle, cleaned: &Cleaned, tooltips: &Tooltips) {
    if cleaned.body_style.is_empty() {
        body.remove_attribute("style");
    } else {
        body.set_attribute("style", &cleaned.body_style);
    }
    body.set_inner_html(&cleaned.html);
    // Each link says where it really goes, as its tooltip.
    for link in body.query_selector_all("a[href]") {
        let Some(href) = link.get_attribute("href") else {
            continue;
        };
        let text = link.text_content().unwrap_or_default();
        if let Some(title) = tooltips.title(&href, &text) {
            link.set_attribute("title", &title);
        }
    }
}
