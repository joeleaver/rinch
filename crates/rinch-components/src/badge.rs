//! Badge component.
//!
//! A small status indicator with text.

use crate::borderless_window::SectionRenderer;
use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Badge variant styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeVariant {
    /// Solid background with primary color.
    #[default]
    Filled,
    /// Light background with darker text.
    Light,
    /// Transparent background with colored border.
    Outline,
    /// Gray with a colored dot indicator.
    Dot,
}

impl BadgeVariant {
    /// Get the CSS class name for this variant.
    pub fn class_name(&self) -> &'static str {
        match self {
            BadgeVariant::Filled => "rinch-badge--filled",
            BadgeVariant::Light => "rinch-badge--light",
            BadgeVariant::Outline => "rinch-badge--outline",
            BadgeVariant::Dot => "rinch-badge--dot",
        }
    }
}

impl std::str::FromStr for BadgeVariant {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "filled" => Ok(BadgeVariant::Filled),
            "light" => Ok(BadgeVariant::Light),
            "outline" => Ok(BadgeVariant::Outline),
            "dot" => Ok(BadgeVariant::Dot),
            _ => Err(()),
        }
    }
}

/// Badge size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl BadgeSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            BadgeSize::Xs => "rinch-badge--xs",
            BadgeSize::Sm => "rinch-badge--sm",
            BadgeSize::Md => "rinch-badge--md",
            BadgeSize::Lg => "rinch-badge--lg",
            BadgeSize::Xl => "rinch-badge--xl",
        }
    }
}

impl std::str::FromStr for BadgeSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(BadgeSize::Xs),
            "sm" => Ok(BadgeSize::Sm),
            "md" => Ok(BadgeSize::Md),
            "lg" => Ok(BadgeSize::Lg),
            "xl" => Ok(BadgeSize::Xl),
            _ => Err(()),
        }
    }
}

/// The palette names the stylesheet has `[data-color]` rules for.
const BUILTIN_COLORS: [&str; 14] = [
    "dark", "gray", "red", "pink", "grape", "violet", "indigo", "blue", "cyan", "teal", "green",
    "lime", "yellow", "orange",
];

/// A small status indicator badge.
///
/// `color` takes a theme palette name, `primary`, or a raw CSS colour
/// (`#7048e8`, `rgb(…)`, `hsl(…)`). The filled variant's text is the
/// colour's contrast (`--rinch-color-{name}-contrast`, or white or black
/// computed from a raw colour; #2208); `--rinch-badge-text` changes it.
#[derive(Default)]
pub struct Badge {
    /// Badge variant (filled, light, outline, dot).
    pub variant: String,
    /// Badge size (xs, sm, md, lg, xl).
    pub size: String,
    /// The badge colour: a palette name, `primary`, or a raw CSS colour.
    /// Empty is the theme's primary colour.
    pub color: String,
    /// Border radius override.
    pub radius: String,
    /// Whether the badge should take full width.
    pub full_width: bool,
    /// Content before the label (a status dot, an icon), in
    /// `span.rinch-badge__section[data-position="left"]`.
    pub left_section: Option<SectionRenderer>,
    /// Content after the label (a remove "x"), in
    /// `span.rinch-badge__section[data-position="right"]`.
    pub right_section: Option<SectionRenderer>,
}

impl std::fmt::Debug for Badge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Badge")
            .field("variant", &self.variant)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("radius", &self.radius)
            .field("full_width", &self.full_width)
            .field(
                "left_section",
                &self.left_section.as_ref().map(|_| "<renderer>"),
            )
            .field(
                "right_section",
                &self.right_section.as_ref().map(|_| "<renderer>"),
            )
            .finish()
    }
}

impl Badge {
    /// Generate the CSS class string for this badge.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-badge"];

        // Size class
        let size: BadgeSize = if self.size.is_empty() {
            BadgeSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        // Variant class
        let variant: BadgeVariant = if self.variant.is_empty() {
            BadgeVariant::default()
        } else {
            self.variant.parse().unwrap_or_default()
        };
        classes.push(variant.class_name());

        // Full width
        if self.full_width {
            classes.push("rinch-badge--full-width");
        }

        // Radius class (#474): the base class is a pill, so a radius step has
        // to override it rather than fill a gap.
        let radius_cls = crate::class_utils::radius_class("rinch-badge", &self.radius);
        classes.extend(radius_cls.as_deref());

        classes.join(" ")
    }

    /// The inline colour variables for a colour the stylesheet has no
    /// `[data-color]` rule for: a raw CSS colour, `primary`, or another token
    /// family. `None` for no colour and for the built-in palette names.
    pub fn style_string(&self) -> Option<String> {
        if self.color.is_empty() || BUILTIN_COLORS.contains(&self.color.as_str()) {
            return None;
        }
        let t = crate::color_utils::color_tokens(&self.color);
        Some(format!(
            "--rinch-badge-color: {}; --rinch-badge-light: {}; \
             --rinch-badge-light-color: {}; --rinch-badge-text: {};",
            t.base, t.light, t.light_color, t.contrast
        ))
    }
}

/// One `span.rinch-badge__section` holding what `render` draws.
fn section(scope: &mut RenderScope, render: &SectionRenderer, position: &str) -> NodeHandle {
    let span = scope.create_element("span");
    span.set_attribute("class", "rinch-badge__section");
    span.set_attribute("data-position", position);
    span.append_child(&render(scope));
    span
}

impl Component for Badge {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { span { class: "rinch-badge" } };
        container.set_attribute("class", &self.class_string());

        if !self.color.is_empty() {
            container.set_attribute("data-color", &self.color);
        }
        if let Some(style) = self.style_string() {
            container.set_attribute("style", &style);
        }

        if self.left_section.is_none() && self.right_section.is_none() {
            for child in children {
                container.append_child(child);
            }
            return container;
        }

        // With a section, the label gets its own box so the sections sit
        // beside it, as Mantine's Badge lays them out.
        if let Some(render) = &self.left_section {
            container.append_child(&section(__scope, render, "left"));
        }
        let label = __scope.create_element("span");
        label.set_attribute("class", "rinch-badge__label");
        for child in children {
            label.append_child(child);
        }
        container.append_child(&label);
        if let Some(render) = &self.right_section {
            container.append_child(&section(__scope, render, "right"));
        }
        container
    }
}
