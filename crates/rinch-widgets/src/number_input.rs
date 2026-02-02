//! NumberInput widget.
//!
//! A numeric input field with increment/decrement controls.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Widget, WidgetCallback};

/// NumberInput size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberInputSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl NumberInputSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            NumberInputSize::Xs => "rinch-number-input--xs",
            NumberInputSize::Sm => "rinch-number-input--sm",
            NumberInputSize::Md => "rinch-number-input--md",
            NumberInputSize::Lg => "rinch-number-input--lg",
            NumberInputSize::Xl => "rinch-number-input--xl",
        }
    }
}

impl std::str::FromStr for NumberInputSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(NumberInputSize::Xs),
            "sm" => Ok(NumberInputSize::Sm),
            "md" => Ok(NumberInputSize::Md),
            "lg" => Ok(NumberInputSize::Lg),
            "xl" => Ok(NumberInputSize::Xl),
            _ => Err(()),
        }
    }
}

/// A numeric input component with increment/decrement controls.
///
/// Allows users to input numbers with optional step controls.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     NumberInput { label: "Quantity", min: 0.0, max: 100.0, step: 1.0 }
///     NumberInput { label: "Price", value: 9.99, decimal_scale: 2, prefix: "$" }
/// }
/// ```
#[derive(Debug, Default)]
pub struct NumberInput {
    /// Input label.
    pub label: Option<String>,
    /// Description text.
    pub description: Option<String>,
    /// Error message.
    pub error: Option<String>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Current value.
    pub value: Option<f64>,
    /// Default value.
    pub default_value: Option<f64>,
    /// Minimum value.
    pub min: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// Step increment.
    pub step: Option<f64>,
    /// Number of decimal places.
    pub decimal_scale: Option<u32>,
    /// Prefix text (e.g., "$").
    pub prefix: Option<String>,
    /// Suffix text (e.g., "kg").
    pub suffix: Option<String>,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Whether to hide the controls.
    pub hide_controls: bool,
    /// Whether the input is required.
    pub required: bool,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: Option<String>,
    /// Callback when increment button is clicked.
    pub onincrement: Option<WidgetCallback>,
    /// Callback when decrement button is clicked.
    pub ondecrement: Option<WidgetCallback>,
    /// Callback when value changes (from direct input).
    pub onchange: Option<WidgetCallback>,
}

impl NumberInput {
    /// Generate the CSS class string for this number input.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-number-input"];

        // Size class
        let size: NumberInputSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        if self.error.is_some() {
            classes.push("rinch-number-input--error");
        }
        if self.disabled {
            classes.push("rinch-number-input--disabled");
        }
        if self.hide_controls {
            classes.push("rinch-number-input--no-controls");
        }

        classes.join(" ")
    }
}

impl Widget for NumberInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-number-input" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if let Some(label_text) = &self.label {
            let required_mark = if self.required { " *" } else { "" };
            let label = rinch_macros::rsx! { label { class: "rinch-number-input__label" } };
            let label_text_node =
                __scope.create_text(&format!("{}{}", label_text, required_mark));
            label.append_child(&label_text_node);
            container.append_child(&label);
        }

        // Description
        if let Some(desc) = &self.description {
            let desc_div = rinch_macros::rsx! { div { class: "rinch-number-input__description" } };
            let desc_text = __scope.create_text(desc);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Input wrapper
        let wrapper = rinch_macros::rsx! { div { class: "rinch-number-input__wrapper" } };

        // Prefix
        if let Some(prefix) = &self.prefix {
            let prefix_span = rinch_macros::rsx! { span { class: "rinch-number-input__prefix" } };
            let prefix_text = __scope.create_text(prefix);
            prefix_span.append_child(&prefix_text);
            wrapper.append_child(&prefix_span);
        }

        // Input element
        let input =
            rinch_macros::rsx! { input { class: "rinch-number-input__input", r#type: "number" } };

        if let Some(v) = self.value {
            input.set_attribute("value", &v.to_string());
        }
        if let Some(min) = self.min {
            input.set_attribute("min", &min.to_string());
        }
        if let Some(max) = self.max {
            input.set_attribute("max", &max.to_string());
        }
        if let Some(step) = self.step {
            input.set_attribute("step", &step.to_string());
        } else {
            input.set_attribute("step", "any");
        }
        if let Some(placeholder) = &self.placeholder {
            input.set_attribute("placeholder", placeholder);
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if self.required {
            input.set_attribute("required", "");
        }

        wrapper.append_child(&input);

        // Suffix
        if let Some(suffix) = &self.suffix {
            let suffix_span = rinch_macros::rsx! { span { class: "rinch-number-input__suffix" } };
            let suffix_text = __scope.create_text(suffix);
            suffix_span.append_child(&suffix_text);
            wrapper.append_child(&suffix_span);
        }

        // Controls (unless hidden)
        if !self.hide_controls {
            let controls = rinch_macros::rsx! { div { class: "rinch-number-input__controls" } };

            // Increment button
            let up_btn = rinch_macros::rsx! {
                button {
                    class: "rinch-number-input__control rinch-number-input__control--up",
                    r#type: "button",
                    tabindex: "-1"
                }
            };
            up_btn.set_attribute("aria-label", "Increment");
            up_btn.append_child(&crate::icons::chevron_up_dom(__scope));

            if let Some(cb) = &self.onincrement {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                up_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            // Decrement button
            let down_btn = rinch_macros::rsx! {
                button {
                    class: "rinch-number-input__control rinch-number-input__control--down",
                    r#type: "button",
                    tabindex: "-1"
                }
            };
            down_btn.set_attribute("aria-label", "Decrement");
            down_btn.append_child(&crate::icons::chevron_down_small_dom(__scope));

            if let Some(cb) = &self.ondecrement {
                let handler_id = __scope.register_handler({
                    let cb = cb.clone();
                    move || cb.invoke()
                });
                down_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            controls.append_child(&up_btn);
            controls.append_child(&down_btn);
            wrapper.append_child(&controls);
        }

        container.append_child(&wrapper);

        // Error message
        if let Some(err) = &self.error {
            let err_div = rinch_macros::rsx! { div { class: "rinch-number-input__error" } };
            let err_text = __scope.create_text(err);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
