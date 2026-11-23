//! Breadcrumbs component.
//!
//! Navigation trail showing the current page location.
//!
//! Each separator is a real element, `<span class="rinch-breadcrumbs__separator"
//! aria-hidden="true">`, appended to every item but the last (issue #2193). It
//! used to be an `::after` pseudo-element whose `content` came from a fixed list
//! of five attribute selectors, so any other separator (`›`, `|`) was drawn as
//! `/`. An element can hold any text, a Tabler icon, or whatever the app renders,
//! and the item that is last changes when a `for` loop adds or removes crumbs, so
//! the separators are re-settled whenever an item arrives or leaves.

use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

use crate::SectionRenderer;

const ITEM_CLASS: &str = "rinch-breadcrumbs__item";
const SEPARATOR_CLASS: &str = "rinch-breadcrumbs__separator";

/// Breadcrumbs navigation component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Breadcrumbs { separator: "›",
///         BreadcrumbsItem { href: "/", "Home" }
///         BreadcrumbsItem { onclick: move || route.set(Route::Docs), "Docs" }
///         BreadcrumbsItem { "Current Page" }
///     }
/// }
/// ```
#[derive(Default)]
pub struct Breadcrumbs {
    /// Separator text drawn between items. Empty means `/`.
    pub separator: String,
    /// A Tabler icon drawn as the separator, in place of `separator`'s text.
    pub separator_icon: Option<TablerIcon>,
    /// The escape hatch: renders each separator's content (called once per
    /// separator). Wins over `separator_icon` and `separator`.
    pub separator_fn: Option<SectionRenderer>,
    /// Spacing between items.
    pub separator_margin: String,
    /// The `<nav>`'s accessible name. Empty means `"Breadcrumb"`.
    pub aria_label: String,
}

impl std::fmt::Debug for Breadcrumbs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Breadcrumbs")
            .field("separator", &self.separator)
            .field("separator_icon", &self.separator_icon)
            .field(
                "separator_fn",
                &self.separator_fn.as_ref().map(|_| "<renderer>"),
            )
            .field("separator_margin", &self.separator_margin)
            .field("aria_label", &self.aria_label)
            .finish()
    }
}

impl Breadcrumbs {
    /// What one separator's content is drawn from.
    fn separator_content(&self) -> SeparatorContent {
        if let Some(render) = &self.separator_fn {
            SeparatorContent::Custom(render.clone())
        } else if let Some(icon) = self.separator_icon {
            SeparatorContent::Icon(icon)
        } else if self.separator.is_empty() {
            SeparatorContent::Text("/".to_string())
        } else {
            SeparatorContent::Text(self.separator.clone())
        }
    }
}

#[derive(Clone)]
enum SeparatorContent {
    Text(String),
    Icon(TablerIcon),
    Custom(SectionRenderer),
}

impl SeparatorContent {
    fn build(&self, scope: &mut RenderScope) -> NodeHandle {
        let span = scope.create_element("span");
        span.set_attribute("class", SEPARATOR_CLASS);
        span.set_attribute("aria-hidden", "true");
        let content = match self {
            SeparatorContent::Text(text) => scope.create_text(text),
            SeparatorContent::Icon(icon) => {
                render_tabler_icon(scope, *icon, TablerIconStyle::Outline)
            }
            SeparatorContent::Custom(render) => render(scope),
        };
        span.append_child(&content);
        span
    }
}

impl Component for Breadcrumbs {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let nav = rinch_macros::rsx! { nav { class: "rinch-breadcrumbs" } };
        let label = if self.aria_label.is_empty() {
            "Breadcrumb"
        } else {
            &self.aria_label
        };
        nav.set_attribute("aria-label", label);

        if !self.separator_margin.is_empty() {
            nav.set_attribute(
                "style",
                &format!("--rinch-breadcrumbs-margin: {}", self.separator_margin),
            );
        }

        let ol = rinch_macros::rsx! { ol { class: "rinch-breadcrumbs__list" } };

        for child in children {
            ol.append_child(child);
        }

        let content = Rc::new(self.separator_content());
        settle_separators(&ol, &content, __scope);

        // A crumb that arrives or leaves later (a `for` loop over a route)
        // changes which item is last: re-settle on both (#716, #745).
        {
            let ol_handle = ol.clone();
            let content = content.clone();
            crate::late_children::adopt_late_children(
                __scope,
                &ol,
                &[ITEM_CLASS],
                move |_, scope| settle_separators(&ol_handle, &content, scope),
            );
        }
        {
            let ol_handle = ol.clone();
            crate::late_children::adopt_child_removals(__scope, &ol, &[ITEM_CLASS], move |scope| {
                settle_separators(&ol_handle, &content, scope)
            });
        }

        nav.append_child(&ol);
        nav
    }
}

/// Give every item under `list` but the last a trailing separator, and take
/// the last one's away.
///
/// Idempotent: it writes only where an item's separator is missing or extra,
/// so the insertion and removal it makes (each declined by the observers, since
/// it happens inside an item) never start another pass.
fn settle_separators(list: &NodeHandle, content: &SeparatorContent, scope: &mut RenderScope) {
    let mut items = Vec::new();
    collect_items(list, &mut items);
    let count = items.len();
    for (i, item) in items.iter().enumerate() {
        let existing = item
            .children()
            .into_iter()
            .find(|c| has_class(c, SEPARATOR_CLASS));
        let wanted = i + 1 < count;
        match (wanted, existing) {
            (true, None) => item.append_child(&content.build(scope)),
            (false, Some(sep)) => sep.remove(),
            _ => {}
        }
    }
}

/// Every breadcrumb item under `node`, in document order, not descending into
/// an item.
fn collect_items(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
    for child in node.children() {
        if has_class(&child, ITEM_CLASS) {
            out.push(child);
        } else {
            collect_items(&child, out);
        }
    }
}

fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

/// Individual breadcrumb item.
///
/// - With `href`, it is a link (`<a>`).
/// - With `onclick` and no `href`, it is a `<button type="button">` styled as
///   the link, for routing by callback in a desktop app. With both, the link
///   carries the handler too.
/// - With neither, it is the current page: a `<span>` with
///   `aria-current="page"`.
///
/// `active` overrides that last guess either way: `Some(true)` marks a link or
/// button as the current page (styled as the current item, with
/// `aria-current="page"`), `Some(false)` leaves an inert item unmarked.
#[derive(Default)]
pub struct BreadcrumbsItem {
    /// Link href (if clickable).
    pub href: String,
    /// Click handler (in-app routing).
    pub onclick: Option<Callback>,
    /// Whether this item is the current page. `None` means "it is when it has
    /// neither `href` nor `onclick`".
    pub active: Option<bool>,
}

impl std::fmt::Debug for BreadcrumbsItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BreadcrumbsItem")
            .field("href", &self.href)
            .field("onclick", &self.onclick.as_ref().map(|_| "<callback>"))
            .field("active", &self.active)
            .finish()
    }
}

impl Component for BreadcrumbsItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let li = rinch_macros::rsx! { li { class: "rinch-breadcrumbs__item" } };

        let active = self
            .active
            .unwrap_or(self.href.is_empty() && self.onclick.is_none());

        let content = if !self.href.is_empty() {
            let link = rinch_macros::rsx! { a { class: "rinch-breadcrumbs__link" } };
            link.set_attribute("href", &self.href);
            link
        } else if self.onclick.is_some() {
            let button = rinch_macros::rsx! { button { class: "rinch-breadcrumbs__link" } };
            button.set_attribute("type", "button");
            button
        } else {
            rinch_macros::rsx! { span {} }
        };

        if active {
            content.add_class("rinch-breadcrumbs__item--active");
            content.set_attribute("aria-current", "page");
        }

        if let Some(cb) = &self.onclick {
            let cb = cb.clone();
            let handler_id = __scope.register_handler(move || cb.invoke());
            content.set_attribute("data-rid", &handler_id.0.to_string());
        }

        for child in children {
            content.append_child(child);
        }

        li.append_child(&content);
        li
    }
}
