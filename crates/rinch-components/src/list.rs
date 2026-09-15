//! List component.
//!
//! Styled ordered and unordered lists.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// List type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListType {
    #[default]
    Unordered,
    Ordered,
}

impl ListType {
    pub fn class_name(&self) -> &'static str {
        match self {
            ListType::Unordered => "rinch-list--unordered",
            ListType::Ordered => "rinch-list--ordered",
        }
    }
}

impl std::str::FromStr for ListType {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "unordered" | "ul" => Ok(ListType::Unordered),
            "ordered" | "ol" => Ok(ListType::Ordered),
            _ => Err(()),
        }
    }
}

/// List size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl ListSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            ListSize::Xs => "rinch-list--xs",
            ListSize::Sm => "rinch-list--sm",
            ListSize::Md => "rinch-list--md",
            ListSize::Lg => "rinch-list--lg",
            ListSize::Xl => "rinch-list--xl",
        }
    }
}

impl std::str::FromStr for ListSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ListSize::Xs),
            "sm" => Ok(ListSize::Sm),
            "md" => Ok(ListSize::Md),
            "lg" => Ok(ListSize::Lg),
            "xl" => Ok(ListSize::Xl),
            _ => Err(()),
        }
    }
}

/// A styled list component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     List {
///         ListItem { "First item" }
///         ListItem { "Second item" }
///         ListItem { "Third item" }
///     }
///     List { r#type: "ordered",
///         ListItem { "Step one" }
///         ListItem { "Step two" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct List {
    /// List type (ordered, unordered).
    pub r#type: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Spacing between items (xs, sm, md, lg, xl).
    pub spacing: String,
    /// Whether to center list items.
    pub center: bool,
    /// Default icon for this list's items.
    ///
    /// Applied to every [`ListItem`] in this list that sets no `icon` of its own
    /// — the item's icon wins. Items are found and restyled after they have
    /// rendered, since a parent component renders *after* its children; an item
    /// that arrives **later**, by a `for` reconcile, a `show_dom` branch or a
    /// hand-rolled `append_child`, is restyled as it lands (issue #716).
    ///
    /// An item inside a *nested* `List` belongs to that list, not this one.
    pub icon: Option<TablerIcon>,
    /// Whether to show list markers.
    pub with_padding: bool,
}

impl List {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-list"];

        let list_type: ListType = if self.r#type.is_empty() {
            ListType::default()
        } else {
            self.r#type.parse().unwrap_or_default()
        };
        classes.push(list_type.class_name());

        let size: ListSize = if self.size.is_empty() {
            ListSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if !self.spacing.is_empty() {
            let spacing = &self.spacing;
            classes.push(match spacing.as_str() {
                "xs" => "rinch-list--spacing-xs",
                "sm" => "rinch-list--spacing-sm",
                "md" => "rinch-list--spacing-md",
                "lg" => "rinch-list--spacing-lg",
                "xl" => "rinch-list--spacing-xl",
                _ => "",
            });
        }

        if self.center {
            classes.push("rinch-list--center");
        }

        if self.with_padding {
            classes.push("rinch-list--with-padding");
        }

        classes.join(" ")
    }
}

impl Component for List {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        let list_type: ListType = if self.r#type.is_empty() {
            ListType::default()
        } else {
            self.r#type.parse().unwrap_or_default()
        };

        let container = match list_type {
            ListType::Ordered => rinch_macros::rsx! { ol { class: "rinch-list" } },
            ListType::Unordered => rinch_macros::rsx! { ul { class: "rinch-list" } },
        };
        container.set_attribute("class", &class);

        for child in children {
            container.append_child(child);
        }

        if let Some(icon) = self.icon {
            give_items_a_default_icon(&container, icon, __scope);
            // And again for every item that lands later — a `for` reconcile, a
            // `show_dom` branch, a hand-rolled `append_child` (issue #716). The
            // same function does both halves, so the two cannot drift.
            crate::late_children::adopt_late_children(
                __scope,
                &container,
                "rinch-list__item",
                move |inserted, scope| give_items_a_default_icon(inserted, icon, scope),
            );
        }

        container
    }
}

/// Give every item in `node`'s subtree that has no icon of its own the list's
/// `icon`, in the layout [`ListItem`] would have built for it.
///
/// The walk stops at each item rather than descending into it: an item's own
/// content — including a nested `List`, which has already applied its own
/// default — is not this list's to restyle.
///
/// Called twice over: once on the container at render, and once per subtree
/// that lands beneath it afterwards (issue #716). It is idempotent — an item
/// that already carries `rinch-list__item--with-icon` is left alone — which is
/// what lets the second caller hand it a subtree the first one already walked.
fn give_items_a_default_icon(node: &NodeHandle, icon: TablerIcon, scope: &mut RenderScope) {
    let classes = node.get_attribute("class").unwrap_or_default();
    let mut tokens = classes.split_whitespace();
    if tokens.clone().any(|c| c == "rinch-list__item") {
        // The item set an icon of its own; a child's value wins over the
        // parent's default.
        if !tokens.any(|c| c == "rinch-list__item--with-icon") {
            adopt_icon(node, icon, scope);
        }
        return;
    }

    for child in node.children() {
        give_items_a_default_icon(&child, icon, scope);
    }
}

/// Rebuild `item` into the icon layout, moving whatever it already holds into
/// the content span.
fn adopt_icon(item: &NodeHandle, icon: TablerIcon, scope: &mut RenderScope) {
    let existing = item.children();

    let icon_span = scope.create_element("span");
    icon_span.set_attribute("class", "rinch-list__item-icon");
    let icon_el = render_tabler_icon(scope, icon, TablerIconStyle::Outline);
    icon_span.append_child(&icon_el);

    let content_span = scope.create_element("span");
    content_span.set_attribute("class", "rinch-list__item-content");
    // `append_child` detaches first, so this is a move out of `item`.
    for child in &existing {
        content_span.append_child(child);
    }

    item.add_class("rinch-list__item--with-icon");
    item.append_child(&icon_span);
    item.append_child(&content_span);
}

/// A list item.
#[derive(Debug, Default)]
pub struct ListItem {
    /// Custom icon for this item.
    pub icon: Option<TablerIcon>,
}

impl Component for ListItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { li { class: "rinch-list__item" } };

        if let Some(icon) = self.icon {
            container.set_attribute("class", "rinch-list__item rinch-list__item--with-icon");

            let icon_span = rinch_macros::rsx! { span { class: "rinch-list__item-icon" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            icon_span.append_child(&icon_el);
            container.append_child(&icon_span);

            let content_span = rinch_macros::rsx! { span { class: "rinch-list__item-content" } };
            for child in children {
                content_span.append_child(child);
            }
            container.append_child(&content_span);
        } else {
            for child in children {
                container.append_child(child);
            }
        }

        container
    }
}
