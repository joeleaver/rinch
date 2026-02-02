//! Paper widget.
//!
//! A card-like container with optional shadow and border.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// Paper shadow size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperShadow {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl PaperShadow {
    /// Get the CSS class name for this shadow.
    pub fn class_name(&self) -> &'static str {
        match self {
            PaperShadow::Xs => "rinch-paper--shadow-xs",
            PaperShadow::Sm => "rinch-paper--shadow-sm",
            PaperShadow::Md => "rinch-paper--shadow-md",
            PaperShadow::Lg => "rinch-paper--shadow-lg",
            PaperShadow::Xl => "rinch-paper--shadow-xl",
        }
    }
}

impl std::str::FromStr for PaperShadow {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(PaperShadow::Xs),
            "sm" => Ok(PaperShadow::Sm),
            "md" => Ok(PaperShadow::Md),
            "lg" => Ok(PaperShadow::Lg),
            "xl" => Ok(PaperShadow::Xl),
            _ => Err(()),
        }
    }
}

/// Padding size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaperPadding {
    Xs,
    Sm,
    Md,
    Lg,
    Xl,
}

impl PaperPadding {
    /// Get the CSS class name for this padding.
    pub fn class_name(&self) -> &'static str {
        match self {
            PaperPadding::Xs => "rinch-paper--p-xs",
            PaperPadding::Sm => "rinch-paper--p-sm",
            PaperPadding::Md => "rinch-paper--p-md",
            PaperPadding::Lg => "rinch-paper--p-lg",
            PaperPadding::Xl => "rinch-paper--p-xl",
        }
    }
}

impl std::str::FromStr for PaperPadding {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(PaperPadding::Xs),
            "sm" => Ok(PaperPadding::Sm),
            "md" => Ok(PaperPadding::Md),
            "lg" => Ok(PaperPadding::Lg),
            "xl" => Ok(PaperPadding::Xl),
            _ => Err(()),
        }
    }
}

/// A card-like container with optional shadow and border.
#[derive(Debug, Default)]
pub struct Paper {
    /// Shadow size (xs, sm, md, lg, xl).
    pub shadow: Option<String>,
    /// Padding size (xs, sm, md, lg, xl). Use `p` as shorthand in RSX.
    pub p: Option<String>,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: Option<String>,
    /// Whether to show a border.
    pub with_border: bool,
}

impl Paper {
    /// Generate the CSS class string for this paper.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-paper"];

        // Shadow class
        if let Some(ref shadow) = self.shadow {
            if let Ok(s) = shadow.parse::<PaperShadow>() {
                classes.push(s.class_name());
            }
        }

        // Padding class
        if let Some(ref p) = self.p {
            if let Ok(padding) = p.parse::<PaperPadding>() {
                classes.push(padding.class_name());
            }
        }

        // Radius class
        if let Some(ref radius) = self.radius {
            match radius.as_str() {
                "xs" => classes.push("rinch-paper--radius-xs"),
                "sm" => classes.push("rinch-paper--radius-sm"),
                "md" => classes.push("rinch-paper--radius-md"),
                "lg" => classes.push("rinch-paper--radius-lg"),
                "xl" => classes.push("rinch-paper--radius-xl"),
                _ => {}
            }
        }

        // Border
        if self.with_border {
            classes.push("rinch-paper--with-border");
        }

        classes.join(" ")
    }
}

impl Widget for Paper {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-paper" }
        };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
