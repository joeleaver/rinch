//! Fieldset component.
//!
//! Form grouping component with legend.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A fieldset for grouping form elements.
#[derive(Debug, Default)]
pub struct Fieldset {
    /// Legend text.
    pub legend: String,
    /// Variant ("default", "filled", "unstyled").
    pub variant: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Whether the fieldset is disabled.
    pub disabled: bool,
}

impl Fieldset {
    /// Generate the CSS class string for this fieldset.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-fieldset"];

        // Variant
        if !self.variant.is_empty() {
            match self.variant.as_str() {
                "filled" => classes.push("rinch-fieldset--filled"),
                "unstyled" => classes.push("rinch-fieldset--unstyled"),
                _ => {} // default has no extra class
            }
        }

        // Size
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-fieldset--xs"),
                "sm" => classes.push("rinch-fieldset--sm"),
                "md" => classes.push("rinch-fieldset--md"),
                "lg" => classes.push("rinch-fieldset--lg"),
                "xl" => classes.push("rinch-fieldset--xl"),
                _ => {}
            }
        }

        classes.join(" ")
    }
}

impl Component for Fieldset {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { fieldset { class: "rinch-fieldset" } };
        container.set_attribute("class", &self.class_string());

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        if !self.legend.is_empty() {
            let legend_str = self.legend.clone();
            let legend_elem =
                rinch_macros::rsx! { legend { class: "rinch-fieldset__legend", {legend_str} } };
            container.append_child(&legend_elem);
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
