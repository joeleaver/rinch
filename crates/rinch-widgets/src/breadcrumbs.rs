//! Breadcrumbs widget.
//!
//! Navigation trail showing the current page location.

use rinch_core::Widget;
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
    pub separator: Option<String>,
    /// Spacing between items.
    pub separator_margin: Option<String>,
}

impl Widget for Breadcrumbs {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let separator = self.separator.as_deref().unwrap_or("/");

        let nav = rinch_macros::rsx! { nav { class: "rinch-breadcrumbs" } };
        nav.set_attribute("aria-label", "Breadcrumb");

        if let Some(ref margin) = self.separator_margin {
            nav.set_attribute("style", &format!("--rinch-breadcrumbs-margin: {}", margin));
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
    pub href: Option<String>,
}

impl Widget for BreadcrumbsItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let li = rinch_macros::rsx! { li { class: "rinch-breadcrumbs__item" } };

        let content = if let Some(ref href) = self.href {
            let link = rinch_macros::rsx! { a { class: "rinch-breadcrumbs__link" } };
            link.set_attribute("href", href);
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
