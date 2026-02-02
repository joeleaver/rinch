//! Checkbox widget.
//!
//! Checkbox input with label support.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive checked state without re-rendering, use the `checked_fn` prop:
//!
//! ```ignore
//! let is_checked = use_signal(|| false);
//!
//! rsx! {
//!     Checkbox {
//!         checked_fn: Some(Rc::new(move || is_checked.get())),
//!         onchange: move || is_checked.update(|v| *v = !*v),
//!         label: "Accept terms"
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Widget, WidgetCallback};
use std::rc::Rc;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// A checkbox input with optional label.
pub struct Checkbox {
    /// Label displayed next to the checkbox.
    pub label: Option<String>,
    /// Description displayed below the checkbox.
    pub description: Option<String>,
    /// Size (xs, sm, md, lg).
    pub size: Option<String>,
    /// Whether the checkbox is disabled.
    pub disabled: bool,
    /// Whether the checkbox is checked (static, for initial render or non-reactive use).
    pub checked: bool,
    /// Reactive checked getter - use this for fine-grained updates.
    /// When provided, the checkbox class updates automatically when the signal changes.
    pub checked_fn: Option<ReactiveBool>,
    /// Whether the checkbox is in indeterminate state.
    pub indeterminate: bool,
    /// Callback when checkbox is toggled.
    pub onchange: Option<WidgetCallback>,
}

impl std::fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checkbox")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("checked", &self.checked)
            .field("checked_fn", &self.checked_fn.as_ref().map(|_| "<reactive>"))
            .field("indeterminate", &self.indeterminate)
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for Checkbox {
    fn default() -> Self {
        Self {
            label: None,
            description: None,
            size: None,
            disabled: false,
            checked: false,
            checked_fn: None,
            indeterminate: false,
            onchange: None,
        }
    }
}

impl Checkbox {
    /// Generate the base CSS class string for this checkbox (without checked state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-checkbox"];

        // Size
        if let Some(size) = &self.size {
            match size.as_str() {
                "xs" => classes.push("rinch-checkbox--xs"),
                "sm" => classes.push("rinch-checkbox--sm"),
                "md" => classes.push("rinch-checkbox--md"),
                "lg" => classes.push("rinch-checkbox--lg"),
                _ => {}
            }
        }

        // Disabled
        if self.disabled {
            classes.push("rinch-checkbox--disabled");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this checkbox (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.checked {
            class.push_str(" rinch-checkbox--checked");
        }
        class
    }
}

impl Widget for Checkbox {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let base_class = self.base_class_string();

        // Determine if we have a reactive checked state
        let is_checked = if let Some(ref checked_fn) = self.checked_fn {
            checked_fn()
        } else {
            self.checked
        };

        // Build the class string
        let class = if is_checked {
            format!("{} rinch-checkbox--checked", base_class)
        } else {
            base_class
        };

        // Create label container
        let label_node = scope.create_element("label");
        label_node.set_attribute("class", &class);

        // Register handler
        if let Some(cb) = &self.onchange {
            let handler_id = scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            label_node.set_attribute("data-rid", &handler_id.to_string());
        }

        // Input element (hidden, used for accessibility)
        let input = scope.create_element("input");
        input.set_attribute("type", "checkbox");
        input.set_attribute("class", "rinch-checkbox__input");

        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if is_checked {
            input.set_attribute("checked", "");
        }

        label_node.append_child(&input);

        // Checkbox icon (checkmark or indeterminate line)
        let icon = if self.indeterminate {
            crate::icons::indeterminate_dom(scope)
        } else {
            crate::icons::checkmark_dom(scope)
        };

        // Box with icon
        let box_node = scope.create_element("span");
        box_node.set_attribute("class", "rinch-checkbox__box");

        let icon_span = scope.create_element("span");
        icon_span.set_attribute("class", "rinch-checkbox__icon");
        icon_span.append_child(&icon);

        box_node.append_child(&icon_span);
        label_node.append_child(&box_node);

        // Label text
        if let Some(label_text) = &self.label {
            let label_span = scope.create_element("span");
            label_span.set_attribute("class", "rinch-checkbox__label");
            let label_text_node = scope.create_text(label_text);
            label_span.append_child(&label_text_node);
            label_node.append_child(&label_span);
        }

        // If reactive checked_fn is provided, create an Effect to toggle checked class
        if let Some(ref checked_fn) = self.checked_fn {
            let checked_fn = checked_fn.clone();
            let label_clone = label_node.clone();
            let base_class = self.base_class_string();

            scope.create_effect(move || {
                let is_checked = checked_fn();
                if is_checked {
                    label_clone.set_attribute("class", &format!("{} rinch-checkbox--checked", base_class));
                } else {
                    label_clone.set_attribute("class", &base_class);
                }
            });
        }

        label_node
    }
}
