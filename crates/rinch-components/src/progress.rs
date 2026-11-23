//! Progress component.
//!
//! A progress bar indicator showing completion status.
//!
//! Like Chrome's `<progress>`: a bar with no `value` (and no `value_fn`) is
//! indeterminate, and the accessible value (`aria-valuenow`) follows every
//! change. The range is `0..=max`, with `max` 100 by default (Chrome's
//! `<progress>` defaults to 1.0; 100 keeps every existing `value` meaning
//! a percentage).
//!
//! # Fine-Grained Reactivity
//!
//! For reactive progress value without re-rendering, use the `value_fn` prop:
//!
//! ```ignore
//! let progress = Signal::new(0.0);
//!
//! rsx! {
//!     Progress {
//!         value_fn: Some(Rc::new(move || progress.get()))
//!     }
//! }
//! ```

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::rc::Rc;

/// Reactive callback type for f32 values.
pub type ReactiveF32 = Rc<dyn Fn() -> f32>;

/// Reactive callback type for bool values.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// The `max` used when none is given (or it is not a positive number).
pub const DEFAULT_MAX: f32 = 100.0;

/// Progress size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl ProgressSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            ProgressSize::Xs => "rinch-progress--xs",
            ProgressSize::Sm => "rinch-progress--sm",
            ProgressSize::Md => "rinch-progress--md",
            ProgressSize::Lg => "rinch-progress--lg",
            ProgressSize::Xl => "rinch-progress--xl",
        }
    }
}

impl std::str::FromStr for ProgressSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ProgressSize::Xs),
            "sm" => Ok(ProgressSize::Sm),
            "md" => Ok(ProgressSize::Md),
            "lg" => Ok(ProgressSize::Lg),
            "xl" => Ok(ProgressSize::Xl),
            _ => Err(()),
        }
    }
}

/// Progress radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressRadius {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl ProgressRadius {
    /// Get the CSS class name for this radius.
    pub fn class_name(&self) -> &'static str {
        match self {
            ProgressRadius::Xs => "rinch-progress--radius-xs",
            ProgressRadius::Sm => "rinch-progress--radius-sm",
            ProgressRadius::Md => "rinch-progress--radius-md",
            ProgressRadius::Lg => "rinch-progress--radius-lg",
            ProgressRadius::Xl => "rinch-progress--radius-xl",
        }
    }
}

impl std::str::FromStr for ProgressRadius {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(ProgressRadius::Xs),
            "sm" => Ok(ProgressRadius::Sm),
            "md" => Ok(ProgressRadius::Md),
            "lg" => Ok(ProgressRadius::Lg),
            "xl" => Ok(ProgressRadius::Xl),
            _ => Err(()),
        }
    }
}

/// A progress bar component.
///
/// Displays progress as a horizontal bar.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Progress { value: 65.0 }
///     Progress { value: 100.0, color: "green", size: "lg" }
///     Progress { value: 30.0, striped: true, animated: true }
/// }
/// ```
#[derive(Default)]
pub struct Progress {
    /// Progress value (`0..=max`) - static, for initial render or
    /// non-reactive use. `None` with no `value_fn` is an indeterminate bar.
    pub value: Option<f32>,
    /// Reactive progress value getter - use this for fine-grained updates.
    /// When provided, the progress bar width and `aria-valuenow` update
    /// automatically when the signal changes.
    pub value_fn: Option<ReactiveF32>,
    /// The value of a full bar. `None` (or a value that is not a positive
    /// number) is 100.
    pub max: Option<f32>,
    /// Show an indeterminate bar (an animated segment, no `aria-valuenow`)
    /// whatever `value` says.
    pub indeterminate: bool,
    /// Reactive `indeterminate`: the bar switches between indeterminate and
    /// the value without re-rendering. Wins over `indeterminate`.
    pub indeterminate_fn: Option<ReactiveBool>,
    /// The bar's accessible name (`aria-label`). Empty writes none.
    pub aria_label: String,
    /// Progress bar color.
    pub color: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether to show striped pattern.
    pub striped: bool,
    /// Whether to animate the stripes.
    pub animated: bool,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("value", &self.value)
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("max", &self.max)
            .field("indeterminate", &self.indeterminate)
            .field(
                "indeterminate_fn",
                &self.indeterminate_fn.as_ref().map(|_| "<reactive>"),
            )
            .field("aria_label", &self.aria_label)
            .field("color", &self.color)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("striped", &self.striped)
            .field("animated", &self.animated)
            .finish()
    }
}

impl Progress {
    /// Generate the CSS class string for this progress bar.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-progress"];

        // Size class
        let size: ProgressSize = if self.size.is_empty() {
            ProgressSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        // Radius class
        if !self.radius.is_empty() {
            if let Ok(radius) = self.radius.parse::<ProgressRadius>() {
                classes.push(radius.class_name());
            }
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for the progress bar fill.
    pub fn bar_class_string(&self) -> String {
        let mut classes = vec!["rinch-progress__bar"];

        if self.striped {
            classes.push("rinch-progress__bar--striped");
        }

        if self.animated {
            classes.push("rinch-progress__bar--animated");
        }

        classes.join(" ")
    }
}

impl Progress {
    /// The effective `max`: the prop when it is a positive number, else 100.
    pub fn effective_max(&self) -> f32 {
        match self.max {
            Some(m) if m.is_finite() && m > 0.0 => m,
            _ => DEFAULT_MAX,
        }
    }
}

/// Writes one state of the bar: indeterminate, or `value` of `max`.
fn apply(
    container: &NodeHandle,
    bar: &NodeHandle,
    color_style: &str,
    max: f32,
    indeterminate: bool,
    value: f32,
) {
    if indeterminate {
        container.add_class("rinch-progress--indeterminate");
        container.remove_attribute("aria-valuenow");
        bar.set_attribute("style", color_style);
    } else {
        container.remove_class("rinch-progress--indeterminate");
        // NaN clamps to NaN; read it as 0, the bar's empty state.
        let value = if value.is_nan() {
            0.0
        } else {
            value.clamp(0.0, max)
        };
        container.set_attribute("aria-valuenow", &value.to_string());
        // `100 / max` is exactly 1 at the default max, so a percentage value
        // is written unchanged.
        let percent = value * (100.0 / max);
        bar.set_attribute("style", &format!("width: {percent}%;{color_style}"));
    }
}

impl Component for Progress {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();
        let bar_class = self.bar_class_string();
        let max = self.effective_max();

        // Color style
        let color_style = if self.color.is_empty() {
            String::new()
        } else {
            let t = crate::color_utils::color_tokens(&self.color);
            format!("--rinch-progress-color: {};", t.base)
        };

        let container = rinch_macros::rsx! { div { class: "rinch-progress", role: "progressbar" } };
        container.set_attribute("class", &class);
        container.set_attribute("aria-valuemin", "0");
        container.set_attribute("aria-valuemax", &max.to_string());
        if !self.aria_label.is_empty() {
            container.set_attribute("aria-label", &self.aria_label);
        }

        let bar = rinch_macros::rsx! { div { class: "rinch-progress__bar" } };
        bar.set_attribute("class", &bar_class);
        container.append_child(&bar);

        // Like `<progress>`: no value at all is indeterminate.
        let static_indeterminate =
            self.indeterminate || (self.value.is_none() && self.value_fn.is_none());

        if self.value_fn.is_none() && self.indeterminate_fn.is_none() {
            apply(
                &container,
                &bar,
                &color_style,
                max,
                static_indeterminate,
                self.value.unwrap_or(0.0),
            );
            return container;
        }

        let value_fn = self.value_fn.clone();
        let static_value = self.value.unwrap_or(0.0);
        let indeterminate_fn = self.indeterminate_fn.clone();
        let container_clone = container.clone();
        let bar_clone = bar.clone();
        __scope.create_effect(move || {
            let indeterminate = match &indeterminate_fn {
                Some(f) => f(),
                None => static_indeterminate,
            };
            let value = match &value_fn {
                Some(f) => f(),
                None => static_value,
            };
            apply(
                &container_clone,
                &bar_clone,
                &color_style,
                max,
                indeterminate,
                value,
            );
        });

        container
    }
}
