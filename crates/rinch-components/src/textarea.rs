//! Textarea component.
//!
//! Multi-line text input with label and description support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};
use std::rc::Rc;

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// A multi-line text input field.
#[derive(Default)]
pub struct Textarea {
    /// Label displayed above the textarea.
    pub label: String,
    /// Description displayed below the textarea.
    pub description: String,
    /// Error message to display.
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
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
    /// Current value.
    pub value: String,
    /// Reactive value getter for fine-grained updates.
    pub value_fn: Option<ReactiveString>,
    /// Callback when textarea content changes.
    pub oninput: Option<InputCallback>,
}

impl std::fmt::Debug for Textarea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Textarea")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("required", &self.required)
            .field("autosize", &self.autosize)
            .field("min_rows", &self.min_rows)
            .field("max_rows", &self.max_rows)
            .field("value", &self.value)
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("oninput", &self.oninput.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Textarea {
    /// Generate the CSS class string for this textarea.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-textarea"];

        // Size
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-textarea--xs"),
                "sm" => classes.push("rinch-textarea--sm"),
                "md" => classes.push("rinch-textarea--md"),
                "lg" => classes.push("rinch-textarea--lg"),
                "xl" => classes.push("rinch-textarea--xl"),
                _ => {}
            }
        }

        // Error state
        if !self.error.is_empty() {
            classes.push("rinch-textarea--error");
        }

        // Autosize
        if self.autosize {
            classes.push("rinch-textarea--autosize");
        }

        classes.join(" ")
    }
}

impl Component for Textarea {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-textarea" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
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

        if !self.placeholder.is_empty() {
            textarea.set_attribute("placeholder", &self.placeholder);
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

        // Reactive value binding
        if let Some(ref value_fn) = self.value_fn {
            let initial_value = value_fn();
            textarea.set_attribute("value", &initial_value);

            let value_fn = value_fn.clone();
            let textarea_clone = textarea.clone();
            __scope.create_effect(move || {
                let current_value = value_fn();
                textarea_clone.set_attribute("value", &current_value);
            });
        } else if !self.value.is_empty() {
            textarea.set_attribute("value", &self.value);
        }

        // Input handler — always register so the runtime can route text input
        // to this element, even without an explicit oninput callback.
        {
            let callback = self.oninput.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                if let Some(cb) = &callback {
                    cb.invoke(value);
                }
            });
            textarea.set_attribute("data-oninput", &handler_id.to_string());
        }

        container.append_child(&textarea);

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div = rinch_macros::rsx! { div { class: "rinch-textarea__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-textarea__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
