//! Loader component.
//!
//! A loading indicator with multiple animation types.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

use crate::borderless_window::SectionRenderer;

/// Loader type (animation style).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoaderType {
    /// Spinning oval/circle (default).
    #[default]
    Oval,
    /// Animated bars.
    Bars,
    /// Animated dots.
    Dots,
}

impl LoaderType {
    /// Get the CSS class name for this type.
    pub fn class_name(&self) -> &'static str {
        match self {
            LoaderType::Oval => "rinch-loader--oval",
            LoaderType::Bars => "rinch-loader--bars",
            LoaderType::Dots => "rinch-loader--dots",
        }
    }
}

impl std::str::FromStr for LoaderType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "oval" | "spinner" | "circle" => Ok(LoaderType::Oval),
            "bars" => Ok(LoaderType::Bars),
            "dots" => Ok(LoaderType::Dots),
            _ => Err(()),
        }
    }
}

/// Loader size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoaderSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl LoaderSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            LoaderSize::Xs => "rinch-loader--xs",
            LoaderSize::Sm => "rinch-loader--sm",
            LoaderSize::Md => "rinch-loader--md",
            LoaderSize::Lg => "rinch-loader--lg",
            LoaderSize::Xl => "rinch-loader--xl",
        }
    }
}

impl std::str::FromStr for LoaderSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(LoaderSize::Xs),
            "sm" => Ok(LoaderSize::Sm),
            "md" => Ok(LoaderSize::Md),
            "lg" => Ok(LoaderSize::Lg),
            "xl" => Ok(LoaderSize::Xl),
            _ => Err(()),
        }
    }
}

/// The app-wide loader: what every [`Loader`] that names no type of its own
/// draws, including the ones [`Button`](crate::Button),
/// [`ActionIcon`](crate::ActionIcon) and [`StepperStep`](crate::StepperStep)
/// render while loading (issue #2205).
///
/// Provide it once, near the root, with `create_context`:
///
/// ```ignore
/// create_context(LoaderDefaults { r#type: "dots".into(), ..Default::default() });
/// ```
///
/// A `Loader`'s own `r#type` or `custom` wins over it. Without one, a loader
/// is an oval, as before.
#[derive(Clone, Default)]
pub struct LoaderDefaults {
    /// The loader type (`oval`, `bars`, `dots`) a loader with no `r#type`
    /// draws. Empty means oval.
    pub r#type: String,
    /// A custom loader drawn instead of the built-in kinds by every loader that
    /// has neither a `r#type` nor a `custom` of its own.
    pub custom: Option<SectionRenderer>,
}

impl std::fmt::Debug for LoaderDefaults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoaderDefaults")
            .field("type", &self.r#type)
            .field("custom", &self.custom.as_ref().map(|_| "<renderer>"))
            .finish()
    }
}

/// The accessible name a loader has when its `aria_label` is empty.
pub const DEFAULT_LOADER_LABEL: &str = "Loading";

/// A loading indicator component.
///
/// Displays an animated spinner to indicate loading state.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Loader {}
///     Loader { r#type: "bars", size: "lg" }
///     Loader { r#type: "dots", color: "red", aria_label: "Chargement" }
/// }
/// ```
#[derive(Default)]
pub struct Loader {
    /// Loader type (oval, bars, dots). Empty uses the app's [`LoaderDefaults`],
    /// and without one, oval.
    pub r#type: String,
    /// Loader size (xs, sm, md, lg, xl).
    pub size: String,
    /// Color (uses theme colors or custom).
    pub color: String,
    /// The loader's accessible name (`aria-label`). Empty (the default) is
    /// `"Loading"`.
    pub aria_label: String,
    /// A custom loader, drawn inside the sized `.rinch-loader` box in place of
    /// the built-in kinds. Wins over `r#type` and over [`LoaderDefaults`].
    pub custom: Option<SectionRenderer>,
}

impl std::fmt::Debug for Loader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loader")
            .field("type", &self.r#type)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("aria_label", &self.aria_label)
            .field("custom", &self.custom.as_ref().map(|_| "<renderer>"))
            .finish()
    }
}

/// What a loader draws: a custom renderer, or one of the built-in kinds.
enum LoaderKind {
    Custom(SectionRenderer),
    Builtin(LoaderType),
}

impl Loader {
    /// What this loader draws, after its own props and then the app's
    /// [`LoaderDefaults`].
    fn kind(&self) -> LoaderKind {
        if let Some(custom) = &self.custom {
            return LoaderKind::Custom(custom.clone());
        }
        if !self.r#type.is_empty() {
            return LoaderKind::Builtin(self.r#type.parse().unwrap_or_default());
        }
        match rinch_core::try_use_context::<LoaderDefaults>() {
            Some(LoaderDefaults {
                custom: Some(custom),
                ..
            }) => LoaderKind::Custom(custom),
            Some(defaults) => LoaderKind::Builtin(defaults.r#type.parse().unwrap_or_default()),
            None => LoaderKind::Builtin(LoaderType::default()),
        }
    }

    fn class_for(&self, kind: &LoaderKind) -> String {
        let type_class = match kind {
            LoaderKind::Custom(_) => "rinch-loader--custom",
            LoaderKind::Builtin(t) => t.class_name(),
        };
        let size: LoaderSize = self.size.parse().unwrap_or_default();
        format!("rinch-loader {type_class} {}", size.class_name())
    }

    /// Generate the CSS class string for this loader.
    pub fn class_string(&self) -> String {
        self.class_for(&self.kind())
    }
}

impl Component for Loader {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let kind = self.kind();
        let container = rinch_macros::rsx! { span { class: "rinch-loader", role: "status" } };
        container.set_attribute("class", &self.class_for(&kind));
        let label = if self.aria_label.is_empty() {
            DEFAULT_LOADER_LABEL
        } else {
            &self.aria_label
        };
        container.set_attribute("aria-label", label);

        // Color style attribute
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-loader-color: {}", c)
            } else {
                format!("--rinch-loader-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        match kind {
            LoaderKind::Custom(render) => {
                let node = render(__scope);
                container.append_child(&node);
            }
            LoaderKind::Builtin(LoaderType::Oval) => {
                let svg_placeholder = rinch_macros::rsx! { span { class: "rinch-loader__oval" } };
                container.append_child(&svg_placeholder);
            }
            LoaderKind::Builtin(LoaderType::Bars) => {
                for _ in 0..4 {
                    let bar = rinch_macros::rsx! { span { class: "rinch-loader__bar" } };
                    container.append_child(&bar);
                }
            }
            LoaderKind::Builtin(LoaderType::Dots) => {
                for _ in 0..3 {
                    let dot = rinch_macros::rsx! { span { class: "rinch-loader__dot" } };
                    container.append_child(&dot);
                }
            }
        }

        container
    }
}

/// The loader a loading [`Button`](crate::Button), [`ActionIcon`](crate::ActionIcon)
/// or [`StepperStep`](crate::StepperStep) draws: the shared [`Loader`], with that
/// component's `loader_type` and `loader` props (issue #2205).
pub(crate) fn render_component_loader(
    scope: &mut RenderScope,
    loader_type: &str,
    custom: Option<&SectionRenderer>,
    size: &str,
) -> NodeHandle {
    Loader {
        r#type: loader_type.to_string(),
        size: size.to_string(),
        custom: custom.cloned(),
        ..Default::default()
    }
    .render(scope, &[])
}
