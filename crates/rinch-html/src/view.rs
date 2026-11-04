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
//! itself. Links show a pointer cursor and their address as a tooltip.
//!
//! To show another document, render a new view (key it on the document, in
//! a `for` loop).

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};

use crate::{Cleaned, Policy};

/// Shows a document someone else wrote; see the module docs.
#[derive(Default)]
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
}

impl std::fmt::Debug for HtmlView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HtmlView")
            .field("document", &format_args!("<{} bytes>", self.document.len()))
            .field("policy", &self.policy)
            .field("html", &format_args!("<{} bytes>", self.html.len()))
            .field("body_style", &self.body_style)
            .field("onlink", &self.onlink.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

/// The view's own CSS: only what links need, scoped to the view.
const CSS: &str = ".rinch-html-view a[href] { cursor: pointer; }";

/// The width a document is first cleaned at, before the view has one: wide,
/// as a desktop shows mail.
const FIRST_WIDTH: f32 = 10_000.0;

impl Component for HtmlView {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let view = scope.create_element("div");
        view.set_attribute("class", "rinch-html-view");
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
            );
        } else {
            let document = Rc::new(self.document.clone());
            let policy = self.policy.clone().unwrap_or_default();
            show(&body, &crate::clean_at(&document, &policy, FIRST_WIDTH));
            // Clean again when the view's width crosses a breakpoint.
            let shown = Rc::new(RefCell::new(crate::signature(&document, FIRST_WIDTH)));
            let bounds = view.bounds_signal();
            let body = body.clone();
            scope.create_effect(move || {
                let width = bounds.get().width;
                if width <= 0.0 {
                    return;
                }
                let now = crate::signature(&document, width);
                if *shown.borrow() == now {
                    return;
                }
                show(&body, &crate::clean_at(&document, &policy, width));
                *shown.borrow_mut() = now;
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

fn show(body: &NodeHandle, cleaned: &Cleaned) {
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
        link.set_attribute("title", &link_title(&href, &text));
    }
}

/// A link's tooltip: its address, and a warning when its text names another
/// site than the one it opens (`paypal.com` written over a link to
/// `paypa1-login.example`).
pub(crate) fn link_title(href: &str, text: &str) -> String {
    let opens = host(href);
    let named = text
        .split_whitespace()
        .filter_map(host)
        .find(|h| h.contains('.'));
    match (opens, named) {
        (Some(opens), Some(named)) if site(&opens) != site(&named) => {
            format!("{href}\nCareful: the text says {named}, but this opens {opens}.")
        }
        _ => href.to_owned(),
    }
}

/// The host of a web address, or of a bare domain (`www.example.com/x`).
fn host(s: &str) -> Option<String> {
    let s = s
        .trim()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '/');
    let lower = s.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .unwrap_or(&lower);
    let host = rest
        .split(['/', '?', '#', ':'])
        .next()?
        .trim_start_matches("www.");
    let looks = host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        && host
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    looks.then(|| host.to_owned())
}

/// A host's last two labels: `mail.google.com` and `google.com` are one
/// site. (No public-suffix list: `example.co.uk` hosts all compare as
/// `co.uk`, so they never warn.)
fn site(host: &str) -> String {
    let labels: Vec<&str> = host.rsplit('.').take(2).collect();
    labels.into_iter().rev().collect::<Vec<_>>().join(".")
}

#[cfg(test)]
mod tests {
    use super::link_title;

    #[test]
    fn a_link_names_where_it_goes_and_warns_on_a_mismatch() {
        assert_eq!(
            link_title("https://shop.example/sale?x=1", "See the sale"),
            "https://shop.example/sale?x=1"
        );
        assert_eq!(
            link_title("https://mail.google.com/x", "www.google.com"),
            "https://mail.google.com/x"
        );
        let warned = link_title(
            "https://paypa1-login.example/",
            "https://www.paypal.com/signin",
        );
        assert!(
            warned.contains("Careful: the text says paypal.com"),
            "{warned}"
        );
        assert!(warned.contains("opens paypa1-login.example"), "{warned}");
        assert_eq!(
            link_title("mailto:ada@example.org", "Mail Ada"),
            "mailto:ada@example.org"
        );
    }
}
