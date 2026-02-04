//! Select widget.
//!
//! Dropdown select input with label support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A dropdown select input.
#[derive(Debug, Default)]
pub struct Select {
    /// Label displayed above the select.
    pub label: Option<String>,
    /// Description displayed below the select.
    pub description: Option<String>,
    /// Error message to display.
    pub error: Option<String>,
    /// Placeholder text for empty state.
    pub placeholder: Option<String>,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Whether the select is disabled.
    pub disabled: bool,
    /// Whether the select is required.
    pub required: bool,
}

impl Select {
    /// Generate the CSS class string for this select.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-select"];

        // Size
        if let Some(size) = &self.size {
            match size.as_str() {
                "xs" => classes.push("rinch-select--xs"),
                "sm" => classes.push("rinch-select--sm"),
                "md" => classes.push("rinch-select--md"),
                "lg" => classes.push("rinch-select--lg"),
                "xl" => classes.push("rinch-select--xl"),
                _ => {}
            }
        }

        // Error state
        if self.error.is_some() {
            classes.push("rinch-select--error");
        }

        classes.join(" ")
    }
}

impl Widget for Select {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-select" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if let Some(label_text) = &self.label {
            let label = rinch_macros::rsx! { label { class: "rinch-select__label" } };
            let label_text_node = __scope.create_text(label_text);
            label.append_child(&label_text_node);

            if self.required {
                let required_span =
                    rinch_macros::rsx! { span { class: "rinch-select__required", "*" } };
                label.append_child(&required_span);
            }

            container.append_child(&label);
        }

        // Select element
        let select = rinch_macros::rsx! { select { class: "rinch-select__input" } };

        if self.disabled {
            select.set_attribute("disabled", "");
        }
        if self.required {
            select.set_attribute("required", "");
        }

        // Append children (option elements)
        for child in children {
            select.append_child(child);
        }

        container.append_child(&select);

        // Description
        if let Some(desc) = &self.description {
            let desc_div = rinch_macros::rsx! { div { class: "rinch-select__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error
        if let Some(err) = &self.error {
            let err_div = rinch_macros::rsx! { div { class: "rinch-select__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
