//! Select widget.
//!
//! Dropdown select input with label support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{InputCallback, Widget};
use std::rc::Rc;

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// A dropdown select input.
#[derive(Default)]
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
    /// Currently selected value.
    pub value: Option<String>,
    /// Reactive value getter for fine-grained updates.
    pub value_fn: Option<ReactiveString>,
    /// Callback when selection changes (receives selected value).
    pub onchange: Option<InputCallback>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("required", &self.required)
            .field("value", &self.value)
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
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

        // Reactive value binding
        if let Some(ref value_fn) = self.value_fn {
            let initial_value = value_fn();
            select.set_attribute("value", &initial_value);

            let value_fn = value_fn.clone();
            let select_clone = select.clone();
            __scope.create_effect(move || {
                let current_value = value_fn();
                select_clone.set_attribute("value", &current_value);
            });
        } else if let Some(value) = &self.value {
            select.set_attribute("value", value);
        }

        // Change handler
        if let Some(callback) = &self.onchange {
            let callback = callback.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                callback.invoke(value);
            });
            select.set_attribute("data-oninput", &handler_id.to_string());
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
