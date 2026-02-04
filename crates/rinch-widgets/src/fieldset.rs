//! Fieldset widget.
//!
//! Form grouping component with legend.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// A fieldset for grouping form elements.
#[derive(Debug, Default)]
pub struct Fieldset {
    /// Legend text.
    pub legend: Option<String>,
    /// Variant ("default", "filled", "unstyled").
    pub variant: Option<String>,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Whether the fieldset is disabled.
    pub disabled: bool,
}

impl Fieldset {
    /// Generate the CSS class string for this fieldset.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-fieldset"];

        // Variant
        if let Some(ref variant) = self.variant {
            match variant.as_str() {
                "filled" => classes.push("rinch-fieldset--filled"),
                "unstyled" => classes.push("rinch-fieldset--unstyled"),
                _ => {} // default has no extra class
            }
        }

        // Size
        if let Some(ref size) = self.size {
            match size.as_str() {
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

impl Widget for Fieldset {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { fieldset { class: "rinch-fieldset" } };
        container.set_attribute("class", &self.class_string());

        if self.disabled {
            container.set_attribute("disabled", "");
        }

        if let Some(legend) = &self.legend {
            let legend_elem = rinch_macros::rsx! { legend { class: "rinch-fieldset__legend" } };
            let text_node = __scope.create_text(legend);
            legend_elem.append_child(&text_node);
            container.append_child(&legend_elem);
        }

        for child in children {
            container.append_child(child);
        }
        container
    }
}
