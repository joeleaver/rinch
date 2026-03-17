//! Slider component.
//!
//! A range input slider with fine-grained reactive updates.

use rinch_core::{
    Component, Drag, Signal, ValueCallback,
    dom::{NodeHandle, RenderScope},
    get_click_context,
};

/// Slider size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SliderSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl SliderSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            SliderSize::Xs => "rinch-slider--xs",
            SliderSize::Sm => "rinch-slider--sm",
            SliderSize::Md => "rinch-slider--md",
            SliderSize::Lg => "rinch-slider--lg",
            SliderSize::Xl => "rinch-slider--xl",
        }
    }
}

impl std::str::FromStr for SliderSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(SliderSize::Xs),
            "sm" => Ok(SliderSize::Sm),
            "md" => Ok(SliderSize::Md),
            "lg" => Ok(SliderSize::Lg),
            "xl" => Ok(SliderSize::Xl),
            _ => Err(()),
        }
    }
}

/// A range slider input with fine-grained reactive updates.
///
/// # Example (with signal for fine-grained updates)
///
/// ```ignore
/// let volume = Signal::new(50.0);
///
/// rsx! {
///     Slider {
///         min: 0.0,
///         max: 100.0,
///         value_signal: Some(volume),
///         onchange: move |v| volume.set(v)
///     }
/// }
/// ```
///
/// # Example (static value - legacy)
///
/// ```ignore
/// let volume = Signal::new(50.0);
///
/// rsx! {
///     Slider {
///         min: 0.0,
///         max: 100.0,
///         value: Some(volume.get()),
///         onchange: move |v| volume.set(v)
///     }
/// }
/// ```
#[derive(Default)]
pub struct Slider {
    /// Minimum value.
    pub min: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// Current value (static - requires re-render to update visuals).
    pub value: Option<f64>,
    /// Value signal for fine-grained reactive updates (preferred).
    /// When provided, the slider bar and thumb update surgically without full re-render.
    pub value_signal: Option<Signal<f64>>,
    /// Step increment.
    pub step: Option<f64>,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Color.
    pub color: String,
    /// Border radius.
    pub radius: String,
    /// Whether the slider is disabled.
    pub disabled: bool,
    /// Label format (e.g., "{value}%").
    pub label: String,
    /// Whether to show the label on hover.
    pub show_label_on_hover: bool,
    /// Label always visible.
    pub label_always_on: bool,
    /// Callback when value changes - receives the new value.
    pub onchange: Option<ValueCallback<f64>>,
}

impl std::fmt::Debug for Slider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slider")
            .field("min", &self.min)
            .field("max", &self.max)
            .field("value", &self.value)
            .field(
                "value_signal",
                &self.value_signal.as_ref().map(|_| "Signal<f64>"),
            )
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl Slider {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-slider"];

        let size: SliderSize = if self.size.is_empty() {
            SliderSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if !self.radius.is_empty() {
            let r = &self.radius;
            classes.push(match r.as_str() {
                "xs" => "rinch-slider--radius-xs",
                "sm" => "rinch-slider--radius-sm",
                "md" => "rinch-slider--radius-md",
                "lg" => "rinch-slider--radius-lg",
                "xl" => "rinch-slider--radius-xl",
                _ => "",
            });
        }

        if self.disabled {
            classes.push("rinch-slider--disabled");
        }

        if self.label_always_on {
            classes.push("rinch-slider--label-always-on");
        }

        classes.join(" ")
    }
}

impl Component for Slider {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        let min = self.min.unwrap_or(0.0);
        let max = self.max.unwrap_or(100.0);
        let step = self.step;

        // Get initial value from signal if provided, otherwise from value prop
        let value = if let Some(ref sig) = self.value_signal {
            sig.get()
        } else {
            self.value.unwrap_or(min)
        };

        // Calculate percentage for styling
        let percentage = if max > min {
            ((value - min) / (max - min)) * 100.0
        } else {
            0.0
        };

        let mut style_parts = Vec::new();

        if !self.color.is_empty() {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                style_parts.push(format!("--rinch-slider-color: {}", c));
            } else {
                style_parts.push(format!("--rinch-slider-color: var(--rinch-color-{}-6)", c));
            }
        }

        // Keep CSS variable for backwards compatibility (static styles)
        style_parts.push(format!("--rinch-slider-value: {}%", percentage));

        // Create root container
        let container = rinch_macros::rsx! { div {} };
        container.set_attribute("class", &class);
        container.set_attribute("style", &style_parts.join("; "));

        // Create track
        let track = rinch_macros::rsx! { div { class: "rinch-slider__track" } };

        // Create bar (the filled portion)
        // Uses transform: scaleX() instead of width for paint-only updates (no Taffy relayout)
        let bar = rinch_macros::rsx! { div { class: "rinch-slider__bar" } };
        bar.set_style("transform", &format!("scaleX({})", percentage / 100.0));
        track.append_child(&bar);

        // Create thumb wrapper
        // The wrapper is width: 100% and acts as a reference for the thumb position.
        // The thumb itself is positioned via transform on the wrapper:
        // translateX(X%) moves by X% of wrapper width (= track width), centering is
        // handled by the thumb's own negative margin.
        let thumb_wrapper = rinch_macros::rsx! { div { class: "rinch-slider__thumb-wrapper" } };
        thumb_wrapper.set_style("transform", &format!("translateX({}%)", percentage));

        // Create a zero-width anchor that centers content on the wrapper's left edge
        let thumb_anchor = rinch_macros::rsx! { div { class: "rinch-slider__thumb-anchor" } };

        // Add label if needed
        if !self.label.is_empty() || self.show_label_on_hover || self.label_always_on {
            let label_text = if self.label.is_empty() {
                format!("{}", value)
            } else {
                self.label.replace("{value}", &format!("{}", value))
            };

            let label = rinch_macros::rsx! { div { class: "rinch-slider__label", {label_text} } };
            thumb_anchor.append_child(&label);
        }

        // Create thumb
        let thumb = rinch_macros::rsx! { div { class: "rinch-slider__thumb" } };
        thumb_anchor.append_child(&thumb);
        thumb_wrapper.append_child(&thumb_anchor);

        // Create click overlay
        let overlay = rinch_macros::rsx! { div { class: "rinch-slider__overlay" } };

        // Register click handler if we have an onchange callback
        if let Some(ref value_cb) = self.onchange {
            let value_cb = value_cb.clone();
            let value_cb_drag = value_cb.clone();

            // Helper to calculate value from percentage
            let calc_value = move |percent: f64| -> f64 {
                let mut val = min + (percent * (max - min));
                // Apply step if specified
                if let Some(s) = step {
                    val = (val / s).round() * s;
                }
                val.clamp(min, max)
            };
            let calc_value_drag = calc_value;

            // Create the click handler that manages drag
            let handler_id = __scope.register_handler(move || {
                let ctx = get_click_context();
                let new_value = calc_value(ctx.percent_x() as f64);
                value_cb.invoke(new_value);

                // Start drag - the drag callback will continue updating
                let drag_cb = value_cb_drag.clone();
                let calc = calc_value_drag;
                Drag::percent()
                    .on_move(move |px, _py| {
                        let new_value = calc(px as f64);
                        drag_cb.invoke(new_value);
                    })
                    .start();
            });

            overlay.set_attribute("data-rid", &handler_id.to_string());
        }

        // Assemble the tree
        container.append_child(&track);
        container.append_child(&thumb_wrapper);
        container.append_child(&overlay);

        // If value_signal is provided, create an Effect to update bar and thumb reactively
        if let Some(ref value_signal) = self.value_signal {
            let value_signal = *value_signal;
            let bar_clone = bar.clone();
            let thumb_wrapper_clone = thumb_wrapper.clone();

            __scope.create_effect(move || {
                let current_value = value_signal.get();
                let pct = if max > min {
                    ((current_value - min) / (max - min)) * 100.0
                } else {
                    0.0
                };
                // Use transform instead of width/left — paint-only, no Taffy relayout
                bar_clone.set_style("transform", &format!("scaleX({})", pct / 100.0));
                thumb_wrapper_clone.set_style("transform", &format!("translateX({}%)", pct));
            });
        }

        container
    }
}
