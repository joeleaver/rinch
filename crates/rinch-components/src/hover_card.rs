//! HoverCard component.
//!
//! A card that appears on hover.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// HoverCard position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HoverCardPosition {
    Top,
    TopStart,
    TopEnd,
    #[default]
    Bottom,
    BottomStart,
    BottomEnd,
    Left,
    LeftStart,
    LeftEnd,
    Right,
    RightStart,
    RightEnd,
}

impl HoverCardPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            HoverCardPosition::Top => "rinch-hover-card--top",
            HoverCardPosition::TopStart => "rinch-hover-card--top-start",
            HoverCardPosition::TopEnd => "rinch-hover-card--top-end",
            HoverCardPosition::Bottom => "rinch-hover-card--bottom",
            HoverCardPosition::BottomStart => "rinch-hover-card--bottom-start",
            HoverCardPosition::BottomEnd => "rinch-hover-card--bottom-end",
            HoverCardPosition::Left => "rinch-hover-card--left",
            HoverCardPosition::LeftStart => "rinch-hover-card--left-start",
            HoverCardPosition::LeftEnd => "rinch-hover-card--left-end",
            HoverCardPosition::Right => "rinch-hover-card--right",
            HoverCardPosition::RightStart => "rinch-hover-card--right-start",
            HoverCardPosition::RightEnd => "rinch-hover-card--right-end",
        }
    }
}

impl std::str::FromStr for HoverCardPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "top" => Ok(HoverCardPosition::Top),
            "top_start" | "topstart" => Ok(HoverCardPosition::TopStart),
            "top_end" | "topend" => Ok(HoverCardPosition::TopEnd),
            "bottom" => Ok(HoverCardPosition::Bottom),
            "bottom_start" | "bottomstart" => Ok(HoverCardPosition::BottomStart),
            "bottom_end" | "bottomend" => Ok(HoverCardPosition::BottomEnd),
            "left" => Ok(HoverCardPosition::Left),
            "left_start" | "leftstart" => Ok(HoverCardPosition::LeftStart),
            "left_end" | "leftend" => Ok(HoverCardPosition::LeftEnd),
            "right" => Ok(HoverCardPosition::Right),
            "right_start" | "rightstart" => Ok(HoverCardPosition::RightStart),
            "right_end" | "rightend" => Ok(HoverCardPosition::RightEnd),
            _ => Err(()),
        }
    }
}

/// A card that appears when hovering over a target.
///
/// Uses CSS :hover for display, no JavaScript required.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     HoverCard { position: "bottom",
///         HoverCardTarget {
///             Anchor { href: "#", "@username" }
///         }
///         HoverCardDropdown {
///             Group {
///                 Avatar { src: "avatar.jpg", size: "lg" }
///                 Stack { gap: "xs",
///                     Text { weight: "bold", "User Name" }
///                     Text { size: "sm", color: "dimmed", "@username" }
///                 }
///             }
///         }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct HoverCard {
    /// Position relative to target.
    pub position: String,
    /// Offset from target.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: String,
    /// Shadow size.
    pub shadow: String,
    /// Width of the dropdown.
    pub width: String,
    /// Open delay in ms (CSS transition).
    pub open_delay: Option<u32>,
    /// Close delay in ms (CSS transition).
    pub close_delay: Option<u32>,
    /// Whether to show arrow.
    pub with_arrow: bool,
}

impl HoverCard {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-hover-card"];

        if !self.position.is_empty() {
            if let Ok(pos) = self.position.parse::<HoverCardPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(HoverCardPosition::Bottom.class_name());
        }

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-hover-card--radius-xs"),
                "sm" => classes.push("rinch-hover-card--radius-sm"),
                "md" => classes.push("rinch-hover-card--radius-md"),
                "lg" => classes.push("rinch-hover-card--radius-lg"),
                "xl" => classes.push("rinch-hover-card--radius-xl"),
                _ => {}
            }
        }

        if !self.shadow.is_empty() {
            match self.shadow.as_str() {
                "xs" => classes.push("rinch-hover-card--shadow-xs"),
                "sm" => classes.push("rinch-hover-card--shadow-sm"),
                "md" => classes.push("rinch-hover-card--shadow-md"),
                "lg" => classes.push("rinch-hover-card--shadow-lg"),
                "xl" => classes.push("rinch-hover-card--shadow-xl"),
                _ => {}
            }
        }

        if self.with_arrow {
            classes.push("rinch-hover-card--with-arrow");
        }

        classes.join(" ")
    }
}

impl Component for HoverCard {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-hover-card-offset: {}px", offset));
        }
        if !self.width.is_empty() {
            let w = &self.width;
            style_parts.push(format!("--rinch-hover-card-width: {}", w));
        }
        if let Some(delay) = self.open_delay {
            style_parts.push(format!("--rinch-hover-card-open-delay: {}ms", delay));
        }
        if let Some(delay) = self.close_delay {
            style_parts.push(format!("--rinch-hover-card-close-delay: {}ms", delay));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-hover-card" } };
        root.set_attribute("class", &self.class_string());
        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        for child in children {
            root.append_child(child);
        }

        root
    }
}

/// Target element for the hover card.
#[derive(Debug, Default)]
pub struct HoverCardTarget;

impl Component for HoverCardTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-hover-card__target" } };
        for child in children {
            target.append_child(child);
        }
        target
    }
}

/// Dropdown content for the hover card.
#[derive(Debug, Default)]
pub struct HoverCardDropdown;

impl Component for HoverCardDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-hover-card__dropdown" } };
        for child in children {
            dropdown.append_child(child);
        }
        dropdown
    }
}
