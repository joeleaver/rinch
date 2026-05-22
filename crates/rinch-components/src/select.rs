//! Select component.
//!
//! Custom dropdown select with label, description, error, sizes, and reactive value binding.
//! Renders a styled trigger button with a dropdown overlay — no native `<select>` element.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback, Signal, events::get_click_context};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::rc::Rc;

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// An option in a Select dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption {
    /// The value submitted when this option is selected.
    pub value: String,
    /// The display label. If empty, `value` is used.
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }

    /// Display text: label if non-empty, otherwise value.
    pub fn display(&self) -> &str {
        if self.label.is_empty() {
            &self.value
        } else {
            &self.label
        }
    }
}

/// A dropdown select input.
#[derive(Default)]
pub struct Select {
    /// Label displayed above the select.
    pub label: String,
    /// Description displayed below the select.
    pub description: String,
    /// Error message to display.
    pub error: String,
    /// Placeholder text when no value is selected.
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
    /// The list of selectable options.
    pub data: Vec<SelectOption>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("label", &self.label)
            .field("placeholder", &self.placeholder)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("value", &self.value)
            .field("data", &self.data)
            .finish()
    }
}

impl Component for Select {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let opened = Signal::new(false);
        // `flip_above` flips the dropdown to open upward when there isn't enough
        // room below the trigger. Decided at click time using the cursor's y vs
        // viewport height (the actual dropdown height isn't known until after
        // layout). The estimate matches the dropdown's CSS `max-height: 200px`
        // plus margin/border slop. Conservative — biases toward flipping rather
        // than clipping.
        let flip_above = Signal::new(false);
        const DROPDOWN_RESERVE_PX: f32 = 220.0;

        // Determine the current value
        let current_value = if let Some(ref vf) = self.value_fn {
            vf()
        } else {
            self.value.clone()
        };
        let selected_value = Signal::new(current_value);

        // If value_fn is provided, sync from it.
        // `set_if_changed` no-ops on the initial run (selected_value was just
        // seeded from the same value_fn() above), which avoids re-entering
        // `flush_effects` when mounted during a parent flush. See GH #24.
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            __scope.create_effect(move || {
                selected_value.set_if_changed(value_fn());
            });
        }

        let options: Rc<Vec<SelectOption>> = Rc::new(self.data.clone());

        // Classes
        let size_class = match self.size.as_str() {
            "xs" => " rinch-select--xs",
            "sm" => " rinch-select--sm",
            "lg" => " rinch-select--lg",
            "xl" => " rinch-select--xl",
            _ => "",
        };
        let error_class = if !self.error.is_empty() {
            " rinch-select--error"
        } else {
            ""
        };
        let disabled_class = if self.disabled {
            " rinch-select--disabled"
        } else {
            ""
        };
        let wrapper_class =
            format!("rinch-select{size_class}{error_class}{disabled_class}").to_string();
        let container = __scope.create_element("div");
        container.set_attribute("class", &wrapper_class);

        // Label
        if !self.label.is_empty() {
            let label = __scope.create_element("label");
            label.set_attribute("class", "rinch-select__label");
            let label_text = __scope.create_text(&self.label);
            label.append_child(&label_text);
            if self.required {
                let star = __scope.create_element("span");
                star.set_attribute("class", "rinch-select__required");
                let star_text = __scope.create_text(" *");
                star.append_child(&star_text);
                label.append_child(&star);
            }
            container.append_child(&label);
        }

        // Trigger wrapper (position: relative for dropdown positioning)
        let trigger_wrapper = __scope.create_element("div");
        trigger_wrapper.set_attribute("class", "rinch-select__wrapper");

        // Trigger button
        let trigger = __scope.create_element("div");
        if self.disabled {
            trigger.set_attribute("class", "rinch-select__input rinch-select__input--disabled");
        } else {
            trigger.set_attribute("class", "rinch-select__input");
        }

        // Display text
        let display_span = __scope.create_element("span");
        display_span.set_attribute("class", "rinch-select__display");
        let display_text = __scope.create_text("");
        display_span.append_child(&display_text);

        let placeholder = self.placeholder.clone();
        let opts_for_display = options.clone();
        let placeholder2 = placeholder.clone();
        let display_text_c = display_text.clone();
        __scope.create_effect(move || {
            let val = selected_value.get();
            let text = if val.is_empty() {
                placeholder2.clone()
            } else {
                opts_for_display
                    .iter()
                    .find(|o| o.value == val)
                    .map(|o| o.display().to_string())
                    .unwrap_or(val)
            };
            display_text_c.set_text(&text);
        });

        // Placeholder styling
        let placeholder_empty = placeholder.is_empty();
        let display_span_c = display_span.clone();
        __scope.create_effect(move || {
            let val = selected_value.get();
            if val.is_empty() && !placeholder_empty {
                display_span_c.set_attribute(
                    "class",
                    "rinch-select__display rinch-select__display--placeholder",
                );
            } else {
                display_span_c.set_attribute("class", "rinch-select__display");
            }
        });

        trigger.append_child(&display_span);

        // Chevron icon
        let chevron =
            render_tabler_icon(__scope, TablerIcon::ChevronDown, TablerIconStyle::Outline);
        chevron.set_attribute("class", "rinch-select__chevron");
        let chevron_c = chevron.clone();
        __scope.create_effect(move || {
            if opened.get() {
                chevron_c.set_style("transform", "rotate(180deg)");
            } else {
                chevron_c.set_style("transform", "rotate(0deg)");
            }
        });
        trigger.append_child(&chevron);

        // Click handler on trigger
        if !self.disabled {
            let handler_id = __scope.register_handler(move || {
                // Decide flip direction at toggle time using viewport bounds
                // (we don't know the dropdown's actual size until after layout).
                if !opened.get() {
                    let ctx = get_click_context();
                    let space_below = ctx.viewport_height - ctx.element_y - ctx.element_height;
                    flip_above.set(
                        space_below < DROPDOWN_RESERVE_PX && ctx.element_y > DROPDOWN_RESERVE_PX,
                    );
                }
                opened.update(|v| *v = !*v);
            });
            trigger.set_attribute("data-rid", &handler_id.0.to_string());
        }

        trigger_wrapper.append_child(&trigger);

        // Dropdown options list
        let dropdown = __scope.create_element("div");
        dropdown.set_attribute("class", "rinch-select__dropdown");
        dropdown.set_style("display", "none");

        let dropdown_c = dropdown.clone();
        __scope.create_effect(move || {
            if opened.get() {
                dropdown_c.set_style("display", "flex");
            } else {
                dropdown_c.set_style("display", "none");
            }
        });

        let dropdown_flip = dropdown.clone();
        __scope.create_effect(move || {
            if flip_above.get() {
                dropdown_flip.add_class("rinch-select__dropdown--above");
            } else {
                dropdown_flip.remove_class("rinch-select__dropdown--above");
            }
        });

        // Render option items
        let onchange = self.onchange.clone();
        for opt in self.data.iter() {
            let item = __scope.create_element("div");
            item.set_attribute("class", "rinch-select__option");
            let item_text = __scope.create_text(opt.display());
            item.append_child(&item_text);

            // Highlight selected option
            let opt_value = opt.value.clone();
            let opt_value2 = opt_value.clone();
            let item_c = item.clone();
            __scope.create_effect(move || {
                let val = selected_value.get();
                if val == opt_value2 {
                    item_c.set_attribute(
                        "class",
                        "rinch-select__option rinch-select__option--selected",
                    );
                } else {
                    item_c.set_attribute("class", "rinch-select__option");
                }
            });

            // Click handler for this option
            let onchange_clone = onchange.clone();
            let handler_id = __scope.register_handler(move || {
                selected_value.set(opt_value.clone());
                opened.set(false);
                if let Some(ref cb) = onchange_clone {
                    cb.invoke(opt_value.clone());
                }
            });
            item.set_attribute("data-rid", &handler_id.0.to_string());

            dropdown.append_child(&item);
        }

        trigger_wrapper.append_child(&dropdown);

        // Backdrop to close on click outside
        let backdrop = __scope.create_element("div");
        backdrop.set_attribute("class", "rinch-select__backdrop");
        backdrop.set_style("display", "none");

        let backdrop_c = backdrop.clone();
        __scope.create_effect(move || {
            if opened.get() {
                backdrop_c.set_style("display", "block");
            } else {
                backdrop_c.set_style("display", "none");
            }
        });
        let backdrop_handler = __scope.register_handler(move || {
            opened.set(false);
        });
        backdrop.set_attribute("data-rid", &backdrop_handler.0.to_string());
        trigger_wrapper.append_child(&backdrop);

        container.append_child(&trigger_wrapper);

        // Description
        if !self.description.is_empty() {
            let desc_div = __scope.create_element("div");
            desc_div.set_attribute("class", "rinch-select__description");
            let desc_text = __scope.create_text(&self.description);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error
        if !self.error.is_empty() {
            let err_div = __scope.create_element("div");
            err_div.set_attribute("class", "rinch-select__error");
            let err_text = __scope.create_text(&self.error);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
