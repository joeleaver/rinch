//! List widget.
//!
//! Styled ordered and unordered lists.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Icon, Widget};

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
    pub r#type: Option<String>,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Spacing between items (xs, sm, md, lg, xl).
    pub spacing: Option<String>,
    /// Whether to center list items.
    pub center: bool,
    /// Custom icon for list items.
    pub icon: Option<Icon>,
    /// Whether to show list markers.
    pub with_padding: bool,
}

impl List {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-list"];

        let list_type: ListType = self
            .r#type
            .as_ref()
            .and_then(|t| t.parse().ok())
            .unwrap_or_default();
        classes.push(list_type.class_name());

        let size: ListSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        if let Some(ref spacing) = self.spacing {
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

impl Widget for List {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        let list_type: ListType = self
            .r#type
            .as_ref()
            .and_then(|t| t.parse().ok())
            .unwrap_or_default();

        let container = match list_type {
            ListType::Ordered => rinch_macros::rsx! { ol { class: "rinch-list" } },
            ListType::Unordered => rinch_macros::rsx! { ul { class: "rinch-list" } },
        };
        container.set_attribute("class", &class);

        for child in children {
            container.append_child(child);
        }
        container
    }
}

/// A list item.
#[derive(Debug, Default)]
pub struct ListItem {
    /// Custom icon for this item.
    pub icon: Option<Icon>,
}

impl Widget for ListItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { li { class: "rinch-list__item" } };

        if let Some(icon) = self.icon {
            container.set_attribute("class", "rinch-list__item rinch-list__item--with-icon");

            let icon_span = rinch_macros::rsx! { span { class: "rinch-list__item-icon" } };
            let icon_el = crate::icons::render_icon(__scope, icon);
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
