//! Select component.
//!
//! Dropdown select input with label support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};
use std::rc::Rc;

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// A dropdown select input.
#[derive(Default)]
pub struct Select {
    /// Label displayed above the select.
    pub label: String,
    /// Description displayed below the select.
    pub description: String,
    /// Error message to display.
    pub error: String,
    /// Placeholder text for empty state.
    pub placeholder: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Whether the select is disabled.
    pub disabled: bool,
    /// Whether the select is required.
    pub required: bool,
    /// Currently selected value.
    pub value: String,
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
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-select--xs"),
                "sm" => classes.push("rinch-select--sm"),
                "md" => classes.push("rinch-select--md"),
                "lg" => classes.push("rinch-select--lg"),
                "xl" => classes.push("rinch-select--xl"),
                _ => {}
            }
        }

        // Error state
        if !self.error.is_empty() {
            classes.push("rinch-select--error");
        }

        classes.join(" ")
    }
}

impl Component for Select {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-select" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let label = rinch_macros::rsx! { label { class: "rinch-select__label", {label_text} } };

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
        } else if !self.value.is_empty() {
            select.set_attribute("value", &self.value);
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
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-select__description", {desc} } };
            container.append_child(&desc_div);
        }

        // Error
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-select__error", {err} } };
            container.append_child(&err_div);
        }

        container
    }
}
