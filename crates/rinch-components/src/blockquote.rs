//! Blockquote component.
//!
//! A styled quotation block.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// A blockquote component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Blockquote { cite: "Albert Einstein", icon: TablerIcon::Quote,
///         "Imagination is more important than knowledge."
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Blockquote {
    /// Citation/source.
    pub cite: String,
    /// Icon to display.
    pub icon: Option<TablerIcon>,
    /// Color for the left border.
    pub color: String,
    /// Border radius.
    pub radius: String,
}

impl Blockquote {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-blockquote"];

        if self.icon.is_some() {
            classes.push("rinch-blockquote--with-icon");
        }

        if !self.radius.is_empty() {
            classes.push(match self.radius.as_str() {
                "xs" => "rinch-blockquote--radius-xs",
                "sm" => "rinch-blockquote--radius-sm",
                "md" => "rinch-blockquote--radius-md",
                "lg" => "rinch-blockquote--radius-lg",
                "xl" => "rinch-blockquote--radius-xl",
                _ => "",
            });
        }

        classes.join(" ")
    }
}

impl Component for Blockquote {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { blockquote { class: "rinch-blockquote" } };
        container.set_attribute("class", &self.class_string());

        // Color style
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-blockquote-color: {}", c)
            } else {
                format!("--rinch-blockquote-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        // Icon element
        if let Some(icon) = self.icon {
            let icon_wrapper = rinch_macros::rsx! { span { class: "rinch-blockquote__icon" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            icon_wrapper.append_child(&icon_el);
            container.append_child(&icon_wrapper);
        }

        // Body element with children
        let body = rinch_macros::rsx! { p { class: "rinch-blockquote__body" } };
        for child in children {
            body.append_child(child);
        }
        container.append_child(&body);

        // Citation element
        if !self.cite.is_empty() {
            let cite_elem = rinch_macros::rsx! { cite { class: "rinch-blockquote__cite", {self.cite.clone()} } };
            container.append_child(&cite_elem);
        }

        container
    }
}
