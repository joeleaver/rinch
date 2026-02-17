//! Card component.
//!
//! A container component with sections (header, body, footer).

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Card padding size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardPadding {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl CardPadding {
    pub fn class_name(&self) -> &'static str {
        match self {
            CardPadding::Xs => "rinch-card--p-xs",
            CardPadding::Sm => "rinch-card--p-sm",
            CardPadding::Md => "rinch-card--p-md",
            CardPadding::Lg => "rinch-card--p-lg",
            CardPadding::Xl => "rinch-card--p-xl",
        }
    }
}

impl std::str::FromStr for CardPadding {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(CardPadding::Xs),
            "sm" => Ok(CardPadding::Sm),
            "md" => Ok(CardPadding::Md),
            "lg" => Ok(CardPadding::Lg),
            "xl" => Ok(CardPadding::Xl),
            _ => Err(()),
        }
    }
}

/// A card container component.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Card { shadow: "sm", padding: "lg",
///         CardSection {
///             img { src: "/image.jpg" }
///         }
///         "Card content here"
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Card {
    /// Shadow size (xs, sm, md, lg, xl).
    pub shadow: String,
    /// Padding size (xs, sm, md, lg, xl).
    pub padding: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether to show a border.
    pub with_border: bool,
}

impl Card {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-card"];

        if !self.shadow.is_empty() {
            classes.push(match self.shadow.as_str() {
                "xs" => "rinch-card--shadow-xs",
                "sm" => "rinch-card--shadow-sm",
                "md" => "rinch-card--shadow-md",
                "lg" => "rinch-card--shadow-lg",
                "xl" => "rinch-card--shadow-xl",
                _ => "",
            });
        }

        let padding: CardPadding = if self.padding.is_empty() {
            CardPadding::default()
        } else {
            self.padding.parse().unwrap_or_default()
        };
        classes.push(padding.class_name());

        if !self.radius.is_empty() {
            classes.push(match self.radius.as_str() {
                "xs" => "rinch-card--radius-xs",
                "sm" => "rinch-card--radius-sm",
                "md" => "rinch-card--radius-md",
                "lg" => "rinch-card--radius-lg",
                "xl" => "rinch-card--radius-xl",
                _ => "",
            });
        }

        if self.with_border {
            classes.push("rinch-card--with-border");
        }

        classes.join(" ")
    }
}

impl Component for Card {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-card" } };
        container.set_attribute("class", &self.class_string());

        for child in children {
            container.append_child(child);
        }
        container
    }
}

/// A section within a Card that spans full width.
#[derive(Debug, Default)]
pub struct CardSection {
    /// Whether this section inherits card padding.
    pub inherit_padding: bool,
    /// Whether to add a border at the top.
    pub with_border: bool,
}

impl CardSection {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-card__section"];

        if self.inherit_padding {
            classes.push("rinch-card__section--inherit-padding");
        }

        if self.with_border {
            classes.push("rinch-card__section--with-border");
        }

        classes.join(" ")
    }
}

impl Component for CardSection {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-card__section" } };
        container.set_attribute("class", &self.class_string());

        for child in children {
            container.append_child(child);
        }
        container
    }
}
