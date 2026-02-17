//! Anchor widget.
//!
//! Styled link component.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A styled anchor/link element.
#[derive(Debug, Default)]
pub struct Anchor {
    /// Link URL.
    pub href: String,
    /// Link target (_blank, _self, etc.).
    pub target: String,
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

impl Widget for Anchor {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { a { class: "rinch-anchor" } };
        container.set_attribute("class", &self.class_string());
        let href = if self.href.is_empty() { "#" } else { &self.href };
        container.set_attribute("href", href);

        if !self.target.is_empty() {
            container.set_attribute("target", &self.target);
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
