//! Breadcrumbs component.
//!
//! Navigation trail showing the current page location.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Breadcrumbs navigation component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Breadcrumbs {
///         BreadcrumbsItem { href: "/", "Home" }
///         BreadcrumbsItem { href: "/docs", "Docs" }
///         BreadcrumbsItem { "Current Page" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Breadcrumbs {
    /// Separator between items.
    pub separator: String,
    /// Spacing between items.
    pub separator_margin: String,
}

impl Component for Breadcrumbs {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let separator = if self.separator.is_empty() { "/" } else { &self.separator };

        let nav = rinch_macros::rsx! { nav { class: "rinch-breadcrumbs" } };
        nav.set_attribute("aria-label", "Breadcrumb");

        if !self.separator_margin.is_empty() {
            nav.set_attribute("style", &format!("--rinch-breadcrumbs-margin: {}", self.separator_margin));
        }

        let ol = rinch_macros::rsx! { ol { class: "rinch-breadcrumbs__list" } };
        ol.set_attribute("data-separator", &html_escape(separator));

        for child in children {
            ol.append_child(child);
        }

        nav.append_child(&ol);
        nav
    }
}

/// Individual breadcrumb item.
#[derive(Debug, Default)]
pub struct BreadcrumbsItem {
    /// Link href (if clickable).
    pub href: String,
}

impl Component for BreadcrumbsItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let li = rinch_macros::rsx! { li { class: "rinch-breadcrumbs__item" } };

        let content = if !self.href.is_empty() {
            let link = rinch_macros::rsx! { a { class: "rinch-breadcrumbs__link" } };
            link.set_attribute("href", &self.href);
            for child in children {
                link.append_child(child);
            }
            link
        } else {
            let span = rinch_macros::rsx! { span { class: "rinch-breadcrumbs__item--active" } };
            for child in children {
                span.append_child(child);
            }
            span
        };

        li.append_child(&content);
        li
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
