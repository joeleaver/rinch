//! Text widget.
//!
//! A typography component for displaying text with various styles.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Text size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl TextSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextSize::Xs => "rinch-text--xs",
            TextSize::Sm => "rinch-text--sm",
            TextSize::Md => "rinch-text--md",
            TextSize::Lg => "rinch-text--lg",
            TextSize::Xl => "rinch-text--xl",
        }
    }
}

impl std::str::FromStr for TextSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(TextSize::Xs),
            "sm" => Ok(TextSize::Sm),
            "md" => Ok(TextSize::Md),
            "lg" => Ok(TextSize::Lg),
            "xl" => Ok(TextSize::Xl),
            _ => Err(()),
        }
    }
}

/// Text weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextWeight {
    Thin,
    ExtraLight,
    Light,
    Normal,
    Medium,
    SemiBold,
    Bold,
    ExtraBold,
    Black,
}

impl TextWeight {
    /// Get the CSS class name for this weight.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextWeight::Thin => "rinch-text--thin",
            TextWeight::ExtraLight => "rinch-text--extralight",
            TextWeight::Light => "rinch-text--light",
            TextWeight::Normal => "rinch-text--normal",
            TextWeight::Medium => "rinch-text--medium",
            TextWeight::SemiBold => "rinch-text--semibold",
            TextWeight::Bold => "rinch-text--bold",
            TextWeight::ExtraBold => "rinch-text--extrabold",
            TextWeight::Black => "rinch-text--black",
        }
    }
}

impl std::str::FromStr for TextWeight {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "thin" | "100" => Ok(TextWeight::Thin),
            "extralight" | "200" => Ok(TextWeight::ExtraLight),
            "light" | "300" => Ok(TextWeight::Light),
            "normal" | "400" => Ok(TextWeight::Normal),
            "medium" | "500" => Ok(TextWeight::Medium),
            "semibold" | "600" => Ok(TextWeight::SemiBold),
            "bold" | "700" => Ok(TextWeight::Bold),
            "extrabold" | "800" => Ok(TextWeight::ExtraBold),
            "black" | "900" => Ok(TextWeight::Black),
            _ => Err(()),
        }
    }
}

/// Text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl TextAlign {
    /// Get the CSS class name for this alignment.
    pub fn class_name(&self) -> &'static str {
        match self {
            TextAlign::Left => "rinch-text--left",
            TextAlign::Center => "rinch-text--center",
            TextAlign::Right => "rinch-text--right",
            TextAlign::Justify => "rinch-text--justify",
        }
    }
}

impl std::str::FromStr for TextAlign {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "left" => Ok(TextAlign::Left),
            "center" => Ok(TextAlign::Center),
            "right" => Ok(TextAlign::Right),
            "justify" => Ok(TextAlign::Justify),
            _ => Err(()),
        }
    }
}

/// A typography component for displaying text.
#[derive(Debug, Default)]
pub struct Text {
    /// Text size (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Font weight (thin, light, normal, medium, semibold, bold, etc.).
    pub weight: Option<String>,
    /// Text color (primary, dimmed, or any color name).
    pub color: Option<String>,
    /// Text alignment (left, center, right, justify).
    pub align: Option<String>,
    /// Whether to render as a span instead of p (inline).
    pub inline: bool,
}

impl Text {
    /// Generate the CSS class string for this text.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-text"];

        // Size class
        if let Some(ref size) = self.size {
            if let Ok(s) = size.parse::<TextSize>() {
                classes.push(s.class_name());
            }
        } else {
            classes.push(TextSize::Md.class_name());
        }

        // Weight class
        if let Some(ref weight) = self.weight
            && let Ok(w) = weight.parse::<TextWeight>()
        {
            classes.push(w.class_name());
        }

        // Color class
        if let Some(ref color) = self.color {
            match color.as_str() {
                "primary" => classes.push("rinch-text--primary"),
                "dimmed" => classes.push("rinch-text--dimmed"),
                "inherit" => classes.push("rinch-text--inherit"),
                "red" => classes.push("rinch-text--red"),
                "pink" => classes.push("rinch-text--pink"),
                "grape" => classes.push("rinch-text--grape"),
                "violet" => classes.push("rinch-text--violet"),
                "indigo" => classes.push("rinch-text--indigo"),
                "blue" => classes.push("rinch-text--blue"),
                "cyan" => classes.push("rinch-text--cyan"),
                "teal" => classes.push("rinch-text--teal"),
                "green" => classes.push("rinch-text--green"),
                "lime" => classes.push("rinch-text--lime"),
                "yellow" => classes.push("rinch-text--yellow"),
                "orange" => classes.push("rinch-text--orange"),
                "gray" => classes.push("rinch-text--gray"),
                "dark" => classes.push("rinch-text--dark"),
                _ => {}
            }
        }

        // Alignment class
        if let Some(ref align) = self.align
            && let Ok(a) = align.parse::<TextAlign>()
        {
            classes.push(a.class_name());
        }

        classes.join(" ")
    }
}

impl Widget for Text {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();
        let container = if self.inline {
            rinch_macros::rsx! { span { class: "rinch-text" } }
        } else {
            rinch_macros::rsx! { p { class: "rinch-text" } }
        };
        container.set_attribute("class", &class);
        for child in children {
            container.append_child(child);
        }
        container
    }
}
