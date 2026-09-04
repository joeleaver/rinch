//! NumberInput component.
//!
//! A numeric input field with increment/decrement controls.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component, InputCallback};
use std::cell::Cell;
use std::rc::Rc;

/// Reactive callback type for string state.
pub type ReactiveString = Rc<dyn Fn() -> String>;

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
/// Works in two modes (#501):
///
/// - **Uncontrolled** (no `value_fn`): the component owns the field. A
///   stepper click steps the displayed number itself — seeded from `value`
///   (or `default_value`), moved by every parsed keystroke — clamped to
///   `[min, max]`, and reports the written text through `oninput`.
///   `onincrement`/`ondecrement` remain pure notifications.
/// - **Controlled** (`value_fn` supplied): the signal → effect → DOM chain is
///   the field's single write path (#264). The steppers stay callback-only —
///   write the signal in `onincrement`/`ondecrement` and the effect carries
///   it to the field.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     NumberInput { label: "Quantity", min: 0.0, max: 100.0, step: 1.0 }
///     NumberInput { label: "Price", value: 9.99, decimal_scale: 2, prefix: "$" }
/// }
/// ```
#[derive(Default)]
pub struct NumberInput {
    /// Input label.
    pub label: String,
    /// Description text.
    pub description: String,
    /// Error message.
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Current value.
    pub value: Option<f64>,
    /// Reactive value getter - use this for fine-grained updates.
    /// When provided, the input value updates automatically when the signal
    /// changes, and it is the field's single write path: the steppers become
    /// callback-only (write the signal in `onincrement`/`ondecrement` and the
    /// effect carries it to the DOM).
    pub value_fn: Option<ReactiveString>,
    /// Initial value of an uncontrolled field when `value` is absent.
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
    pub prefix: String,
    /// Suffix text (e.g., "kg").
    pub suffix: String,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Whether to hide the controls.
    pub hide_controls: bool,
    /// Whether the input is required.
    pub required: bool,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Callback when increment button is clicked. A notification, not a
    /// value: uncontrolled, the component steps the field itself and reports
    /// the written text through `oninput` after this fires (#501).
    pub onincrement: Option<Callback>,
    /// Callback when decrement button is clicked (see `onincrement`).
    pub ondecrement: Option<Callback>,
    /// Callback when value changes (from direct input, and — uncontrolled —
    /// from a stepper write, which reports the text it wrote).
    pub oninput: Option<InputCallback>,
    /// Callback when the typed gesture commits (focus leaves the input after a
    /// modification, or Enter) — HTML `change` semantics, receives the final
    /// value. Fires only if the value changed since focus (issue #226).
    pub onchange: Option<InputCallback>,
}

impl std::fmt::Debug for NumberInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumberInput")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field("value", &self.value)
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("default_value", &self.default_value)
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("decimal_scale", &self.decimal_scale)
            .field("prefix", &self.prefix)
            .field("suffix", &self.suffix)
            .field("disabled", &self.disabled)
            .field("hide_controls", &self.hide_controls)
            .field("required", &self.required)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field(
                "onincrement",
                &self.onincrement.as_ref().map(|_| "<callback>"),
            )
            .field(
                "ondecrement",
                &self.ondecrement.as_ref().map(|_| "<callback>"),
            )
            .field("oninput", &self.oninput.as_ref().map(|_| "<callback>"))
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl NumberInput {
    /// Generate the CSS class string for this number input.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-number-input"];

        // Size class
        let size: NumberInputSize = if self.size.is_empty() {
            NumberInputSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if !self.error.is_empty() {
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

/// The text the component itself writes into the field for `v` (#501):
/// `decimal_scale` fixes the number of decimals; without it, the value is
/// rounded to 10 decimal places first so repeated ±step arithmetic cannot
/// surface float dust ("0.30000000000000004").
fn format_shown(v: f64, decimal_scale: Option<u32>) -> String {
    if let Some(scale) = decimal_scale {
        return format!("{:.*}", scale as usize, v);
    }
    let rounded = (v * 1e10).round() / 1e10;
    if rounded.is_finite() {
        rounded.to_string()
    } else {
        v.to_string()
    }
}

/// Clamp to `[min, max]`, each bound optional. `min` is applied last so the
/// degenerate `min > max` favors `min` rather than panicking like
/// `f64::clamp`.
fn clamp_to(v: f64, min: Option<f64>, max: Option<f64>) -> f64 {
    let v = match max {
        Some(mx) => v.min(mx),
        None => v,
    };
    match min {
        Some(mn) => v.max(mn),
        None => v,
    }
}

impl Component for NumberInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-number-input" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let required_mark = if self.required { " *" } else { "" };
            let label = rinch_macros::rsx! { label { class: "rinch-number-input__label", {format!("{}{}", label_text, required_mark)} } };
            container.append_child(&label);
        }

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-number-input__description", {desc} } };
            container.append_child(&desc_div);
        }

        // Input wrapper
        let wrapper = rinch_macros::rsx! { div { class: "rinch-number-input__wrapper" } };

        // Prefix
        if !self.prefix.is_empty() {
            let prefix = &self.prefix;
            let prefix_span =
                rinch_macros::rsx! { span { class: "rinch-number-input__prefix", {prefix} } };
            wrapper.append_child(&prefix_span);
        }

        // Input element
        let input =
            rinch_macros::rsx! { input { class: "rinch-number-input__input", r#type: "number" } };

        // Uncontrolled display state (#501). With no `value_fn` nothing else
        // writes the field, so the steppers own it: `shown` is the number the
        // field currently displays, as this component last heard it — the
        // mount value, then every stepper write and every parsed keystroke.
        // `None` is an empty field. With a `value_fn`, `shown` is never read:
        // the signal → effect → DOM chain below is the field's single write
        // path (#264), and a second writer here is exactly how controlled
        // inputs desync.
        let controlled = self.value_fn.is_some();
        let shown: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(self.value.or(self.default_value)));

        // Reactive value binding
        if let Some(ref value_fn) = self.value_fn {
            // Set initial value
            let initial_value = value_fn();
            input.set_attribute("value", &initial_value);

            // Create Effect for reactive updates
            let value_fn = value_fn.clone();
            let input_clone = input.clone();
            __scope.create_effect(move || {
                let current_value = value_fn();
                input_clone.set_attribute("value", &current_value);
            });
        } else if let Some(v) = shown.get() {
            input.set_attribute("value", &format_shown(v, self.decimal_scale));
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
        if !self.placeholder.is_empty() {
            input.set_attribute("placeholder", &self.placeholder);
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if self.required {
            input.set_attribute("required", "");
        }

        wrapper.append_child(&input);

        // Input handler for direct text entry — registered whenever any commit
        // consumer exists (oninput OR onchange), so the runtime can route
        // focus and typed text to this element (an onchange-only NumberInput
        // must still be focusable). An instance with NEITHER callback stays
        // inert like pre-#226: registering anyway would make a spinner-only
        // NumberInput tab-focusable and freely editable, with the edits
        // reported nowhere (#244 review).
        if self.oninput.is_some() || self.onchange.is_some() {
            let callback = self.oninput.clone();
            let typed_record = (!controlled).then(|| shown.clone());
            let handler_id = __scope.register_input_handler(move |value: String| {
                // A parsed keystroke moves the uncontrolled stepping base
                // (#501): after typing "3", + must write 4, not step the
                // mount value. An unparseable partial ("", "1e") leaves the
                // base where it was — the next stepper write replaces it.
                if let Some(record) = &typed_record {
                    if let Ok(n) = value.trim().parse::<f64>() {
                        record.set(Some(n));
                    }
                }
                if let Some(cb) = &callback {
                    cb.invoke(value);
                }
            });
            input.set_attribute("data-oninput", &handler_id.to_string());
        }

        // Change handler — the commit boundary (issue #226)
        if let Some(callback) = &self.onchange {
            let callback = callback.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                callback.invoke(value);
            });
            input.set_attribute("data-onchange", &handler_id.to_string());
        }

        // Suffix
        if !self.suffix.is_empty() {
            let suffix = &self.suffix;
            let suffix_span =
                rinch_macros::rsx! { span { class: "rinch-number-input__suffix", {suffix} } };
            wrapper.append_child(&suffix_span);
        }

        // Controls (unless hidden)
        if !self.hide_controls {
            let controls = rinch_macros::rsx! { div { class: "rinch-number-input__controls" } };

            // The stepper write (#501). Uncontrolled, a click computes the
            // next value from `shown`, clamps it to [min, max], writes it,
            // and reports the written text through `oninput` — the same
            // channel a keystroke reports through, and the report is
            // deliberately last so it carries the text the field ends on. A
            // clamped click that moves nothing writes nothing and reports
            // nothing (HTML's input event fires only when the value changes);
            // `onincrement`/`ondecrement` still fire either way, as they
            // always have. Controlled or disabled, this is inert: the
            // value_fn effect stays the single write path, and a disabled
            // field's number must not move.
            let step = self.step.unwrap_or(1.0);
            let min = self.min;
            let max = self.max;
            let apply_step = {
                let shown = shown.clone();
                let input = input.clone();
                let oninput = self.oninput.clone();
                let decimal_scale = self.decimal_scale;
                let disabled = self.disabled;
                Rc::new(move |delta: f64| {
                    if controlled || disabled {
                        return;
                    }
                    let base = shown.get().unwrap_or(0.0);
                    let next = clamp_to(base + delta, min, max);
                    if shown.get() == Some(next) {
                        return;
                    }
                    shown.set(Some(next));
                    let text = format_shown(next, decimal_scale);
                    input.set_attribute("value", &text);
                    if let Some(cb) = &oninput {
                        cb.invoke(text);
                    }
                })
            };

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

            // Registered whenever the stepper has work to do: uncontrolled it
            // writes the field even with no callback (the docs' bare
            // `NumberInput { label: "Quantity" }` must step); controlled it
            // is callback-only, so with no callback it stays inert like
            // pre-#501.
            if !controlled || self.onincrement.is_some() {
                let cb = self.onincrement.clone();
                let apply_step = apply_step.clone();
                let handler_id = __scope.register_handler(move || {
                    if let Some(cb) = &cb {
                        cb.invoke();
                    }
                    apply_step(step);
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

            if !controlled || self.ondecrement.is_some() {
                let cb = self.ondecrement.clone();
                let apply_step = apply_step.clone();
                let handler_id = __scope.register_handler(move || {
                    if let Some(cb) = &cb {
                        cb.invoke();
                    }
                    apply_step(-step);
                });
                down_btn.set_attribute("data-rid", &handler_id.to_string());
            }

            controls.append_child(&up_btn);
            controls.append_child(&down_btn);
            wrapper.append_child(&controls);
        }

        container.append_child(&wrapper);

        // Error message
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-number-input__error", {err} } };
            container.append_child(&err_div);
        }

        container
    }
}
