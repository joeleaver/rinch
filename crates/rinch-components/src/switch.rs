//! Switch component.
//!
//! Toggle switch input.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive checked state without re-rendering, use the `checked_fn` prop:
//!
//! ```ignore
//! let is_on = use_signal(|| false);
//!
//! rsx! {
//!     Switch {
//!         checked_fn: Some(Rc::new(move || is_on.get())),
//!         onchange: move || is_on.update(|v| *v = !*v),
//!         label: "Enable notifications"
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, Callback};
use std::rc::Rc;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// A toggle switch input.
#[derive(Default)]
pub struct Switch {
    /// Label displayed next to the switch.
    pub label: String,
    /// Description displayed below the switch.
    pub description: String,
    /// Size (xs, sm, md, lg).
    pub size: String,
    /// Whether the switch is disabled.
    pub disabled: bool,
    /// Whether the switch is checked (static, for initial render or non-reactive use).
    pub checked: bool,
    /// Reactive checked getter - use this for fine-grained updates.
    /// When provided, the switch class updates automatically when the signal changes.
    pub checked_fn: Option<ReactiveBool>,
    /// Label position (start, end). Default is end.
    pub label_position: String,
    /// Callback when switch is toggled.
    pub onchange: Option<Callback>,
}

impl std::fmt::Debug for Switch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Switch")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("checked", &self.checked)
            .field(
                "checked_fn",
                &self.checked_fn.as_ref().map(|_| "<reactive>"),
            )
            .field("label_position", &self.label_position)
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Switch {
    /// Generate the base CSS class string for this switch (without checked state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-switch"];

        // Size
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-switch--xs"),
                "sm" => classes.push("rinch-switch--sm"),
                "md" => classes.push("rinch-switch--md"),
                "lg" => classes.push("rinch-switch--lg"),
                _ => {}
            }
        }

        // Disabled
        if self.disabled {
            classes.push("rinch-switch--disabled");
        }

        // Label position
        if !self.label_position.is_empty() && self.label_position == "start" {
            classes.push("rinch-switch--label-start");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this switch (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.checked {
            class.push_str(" rinch-switch--checked");
        }
        class
    }
}

impl Component for Switch {
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
            format!("{} rinch-switch--checked", base_class)
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
        input.set_attribute("class", "rinch-switch__input");

        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if is_checked {
            input.set_attribute("checked", "");
        }

        label_node.append_child(&input);

        // Track with thumb
        let track = scope.create_element("span");
        track.set_attribute("class", "rinch-switch__track");

        let thumb = scope.create_element("span");
        thumb.set_attribute("class", "rinch-switch__thumb");
        track.append_child(&thumb);

        label_node.append_child(&track);

        // Label text
        if !self.label.is_empty() {
            let label_text = &self.label;
            let label_span = scope.create_element("span");
            label_span.set_attribute("class", "rinch-switch__label");
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
                    label_clone
                        .set_attribute("class", &format!("{} rinch-switch--checked", base_class));
                } else {
                    label_clone.set_attribute("class", &base_class);
                }
            });
        }

        label_node
    }
}
