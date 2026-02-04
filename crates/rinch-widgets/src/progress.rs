//! Progress widget.
//!
//! A progress bar indicator showing completion status.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive progress value without re-rendering, use the `value_fn` prop:
//!
//! ```ignore
//! let progress = use_signal(|| 0.0);
//!
//! rsx! {
//!     Progress {
//!         value_fn: Some(Rc::new(move || progress.get()))
//!     }
//! }
//! ```

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};
use std::rc::Rc;

/// Reactive callback type for f32 values.
pub type ReactiveF32 = Rc<dyn Fn() -> f32>;

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
    /// Progress value (0-100) - static, for initial render or non-reactive use.
    pub value: Option<f32>,
    /// Reactive progress value getter - use this for fine-grained updates.
    /// When provided, the progress bar width updates automatically when the signal changes.
    pub value_fn: Option<ReactiveF32>,
    /// Progress bar color.
    pub color: Option<String>,
    /// Size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: Option<String>,
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
        let size: ProgressSize = self
            .size
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        classes.push(size.class_name());

        // Radius class
        if let Some(ref r) = self.radius
            && let Ok(radius) = r.parse::<ProgressRadius>()
        {
            classes.push(radius.class_name());
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

impl Widget for Progress {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();
        let bar_class = self.bar_class_string();

        // Determine value - prefer reactive getter if available
        let current_value = if let Some(ref value_fn) = self.value_fn {
            value_fn()
        } else {
            self.value.unwrap_or(0.0)
        };
        let value = current_value.clamp(0.0, 100.0);

        // Color style prefix
        let color_prefix = self.color.as_ref().map(|c| {
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-progress-color: {};", c)
            } else {
                format!("--rinch-progress-color: var(--rinch-color-{}-6);", c)
            }
        });

        let container = rinch_macros::rsx! { div { class: "rinch-progress", role: "progressbar" } };
        container.set_attribute("class", &class);
        container.set_attribute("aria-valuenow", &value.to_string());
        container.set_attribute("aria-valuemin", "0");
        container.set_attribute("aria-valuemax", "100");

        let bar = rinch_macros::rsx! { div { class: "rinch-progress__bar" } };
        bar.set_attribute("class", &bar_class);

        // Set the bar style
        let bar_style = format!(
            "width: {}%;{}",
            value,
            color_prefix.as_deref().unwrap_or("")
        );
        bar.set_attribute("style", &bar_style);

        container.append_child(&bar);
        container
    }
}
