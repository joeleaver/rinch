//! Textarea widget.
//!
//! Multi-line text input with label and description support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// A multi-line text input field.
#[derive(Debug, Default)]
pub struct Textarea {
    /// Label displayed above the textarea.
    pub label: Option<String>,
    /// Description displayed below the textarea.
    pub description: Option<String>,
    /// Error message to display.
    pub error: Option<String>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Whether the textarea is disabled.
    pub disabled: bool,
    /// Whether the textarea is required.
    pub required: bool,
    /// Whether the textarea should auto-resize.
    pub autosize: bool,
    /// Minimum number of rows.
    pub min_rows: Option<u32>,
    /// Maximum number of rows.
    pub max_rows: Option<u32>,
}

impl Textarea {
    /// Generate the CSS class string for this textarea.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-textarea"];

        // Size
        if let Some(size) = &self.size {
            match size.as_str() {
                "xs" => classes.push("rinch-textarea--xs"),
                "sm" => classes.push("rinch-textarea--sm"),
                "md" => classes.push("rinch-textarea--md"),
                "lg" => classes.push("rinch-textarea--lg"),
                "xl" => classes.push("rinch-textarea--xl"),
                _ => {}
            }
        }

        // Error state
        if self.error.is_some() {
            classes.push("rinch-textarea--error");
        }

        // Autosize
        if self.autosize {
            classes.push("rinch-textarea--autosize");
        }

        classes.join(" ")
    }
}

impl Widget for Textarea {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-textarea" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if let Some(label_text) = &self.label {
            let label = rinch_macros::rsx! { label { class: "rinch-textarea__label" } };
            let label_text_node = __scope.create_text(label_text);
            label.append_child(&label_text_node);

            if self.required {
                let required_span =
                    rinch_macros::rsx! { span { class: "rinch-textarea__required", "*" } };
                label.append_child(&required_span);
            }

            container.append_child(&label);
        }

        // Textarea element
        let textarea = rinch_macros::rsx! { textarea { class: "rinch-textarea__input" } };

        if let Some(placeholder) = &self.placeholder {
            textarea.set_attribute("placeholder", placeholder);
        }
        if self.disabled {
            textarea.set_attribute("disabled", "");
        }
        if self.required {
            textarea.set_attribute("required", "");
        }
        if let Some(rows) = self.min_rows {
            textarea.set_attribute("rows", &rows.to_string());
        }

        container.append_child(&textarea);

        // Description
        if let Some(desc) = &self.description {
            let desc_div = rinch_macros::rsx! { div { class: "rinch-textarea__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error
        if let Some(err) = &self.error {
            let err_div = rinch_macros::rsx! { div { class: "rinch-textarea__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
