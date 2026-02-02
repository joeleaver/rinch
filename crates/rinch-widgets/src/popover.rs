//! Popover widget.
//!
//! Positioned popup content relative to a target element.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::Widget;

/// Popover position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopoverPosition {
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

impl PopoverPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            PopoverPosition::Top => "rinch-popover--top",
            PopoverPosition::TopStart => "rinch-popover--top-start",
            PopoverPosition::TopEnd => "rinch-popover--top-end",
            PopoverPosition::Bottom => "rinch-popover--bottom",
            PopoverPosition::BottomStart => "rinch-popover--bottom-start",
            PopoverPosition::BottomEnd => "rinch-popover--bottom-end",
            PopoverPosition::Left => "rinch-popover--left",
            PopoverPosition::LeftStart => "rinch-popover--left-start",
            PopoverPosition::LeftEnd => "rinch-popover--left-end",
            PopoverPosition::Right => "rinch-popover--right",
            PopoverPosition::RightStart => "rinch-popover--right-start",
            PopoverPosition::RightEnd => "rinch-popover--right-end",
        }
    }
}

impl std::str::FromStr for PopoverPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "top" => Ok(PopoverPosition::Top),
            "top_start" | "topstart" => Ok(PopoverPosition::TopStart),
            "top_end" | "topend" => Ok(PopoverPosition::TopEnd),
            "bottom" => Ok(PopoverPosition::Bottom),
            "bottom_start" | "bottomstart" => Ok(PopoverPosition::BottomStart),
            "bottom_end" | "bottomend" => Ok(PopoverPosition::BottomEnd),
            "left" => Ok(PopoverPosition::Left),
            "left_start" | "leftstart" => Ok(PopoverPosition::LeftStart),
            "left_end" | "leftend" => Ok(PopoverPosition::LeftEnd),
            "right" => Ok(PopoverPosition::Right),
            "right_start" | "rightstart" => Ok(PopoverPosition::RightStart),
            "right_end" | "rightend" => Ok(PopoverPosition::RightEnd),
            _ => Err(()),
        }
    }
}

/// A popover for displaying content relative to a target.
///
/// # Example
///
/// ```ignore
/// let show_popover = use_signal(|| false);
///
/// rsx! {
///     Popover { opened: show_popover.get(), position: "bottom",
///         PopoverTarget {
///             Button { onclick: move || show_popover.update(|v| *v = !*v),
///                 "Toggle Popover"
///             }
///         }
///         PopoverDropdown {
///             Text { "Popover content here" }
///         }
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Popover {
    /// Whether the popover is open.
    pub opened: bool,
    /// Position relative to target.
    pub position: Option<String>,
    /// Offset from target in pixels.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: Option<String>,
    /// Whether popover has shadow.
    pub shadow: Option<String>,
    /// Whether to show arrow pointing to target.
    pub with_arrow: bool,
    /// Arrow size in pixels.
    pub arrow_size: Option<f32>,
    /// Arrow offset from edge.
    pub arrow_offset: Option<f32>,
    /// Whether clicking outside closes popover.
    pub close_on_click_outside: bool,
    /// Whether pressing Escape closes popover.
    pub close_on_escape: bool,
    /// Width of the dropdown.
    pub width: Option<String>,
    /// Z-index for the popover.
    pub z_index: Option<i32>,
    /// Whether to trap focus.
    pub trap_focus: bool,
    /// Callback when popover should close.
    pub onclose: Option<rinch_core::WidgetCallback>,
}

impl Default for Popover {
    fn default() -> Self {
        Self {
            opened: false,
            position: None,
            offset: None,
            radius: None,
            shadow: None,
            with_arrow: false,
            arrow_size: None,
            arrow_offset: None,
            close_on_click_outside: true,
            close_on_escape: true,
            width: None,
            z_index: None,
            trap_focus: false,
            onclose: None,
        }
    }
}

impl Popover {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-popover"];

        if let Some(ref p) = self.position {
            if let Ok(pos) = p.parse::<PopoverPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(PopoverPosition::Bottom.class_name());
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-popover--radius-xs"),
                "sm" => classes.push("rinch-popover--radius-sm"),
                "md" => classes.push("rinch-popover--radius-md"),
                "lg" => classes.push("rinch-popover--radius-lg"),
                "xl" => classes.push("rinch-popover--radius-xl"),
                _ => {}
            }
        }

        if let Some(ref s) = self.shadow {
            match s.as_str() {
                "xs" => classes.push("rinch-popover--shadow-xs"),
                "sm" => classes.push("rinch-popover--shadow-sm"),
                "md" => classes.push("rinch-popover--shadow-md"),
                "lg" => classes.push("rinch-popover--shadow-lg"),
                "xl" => classes.push("rinch-popover--shadow-xl"),
                _ => {}
            }
        }

        if self.with_arrow {
            classes.push("rinch-popover--with-arrow");
        }

        if self.opened {
            classes.push("rinch-popover--opened");
        }

        classes.join(" ")
    }
}

impl Widget for Popover {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-popover-offset: {}px", offset));
        }
        if let Some(ref w) = self.width {
            style_parts.push(format!("--rinch-popover-width: {}", w));
        }
        if let Some(size) = self.arrow_size {
            style_parts.push(format!("--rinch-popover-arrow-size: {}px", size));
        }
        if let Some(offset) = self.arrow_offset {
            style_parts.push(format!("--rinch-popover-arrow-offset: {}px", offset));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-popover" } };
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

/// Target element that the popover is anchored to.
#[derive(Debug, Default)]
pub struct PopoverTarget;

impl Widget for PopoverTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-popover__target" } };
        for child in children {
            target.append_child(child);
        }
        target
    }
}

/// Dropdown content of the popover.
#[derive(Debug, Default)]
pub struct PopoverDropdown;

impl Widget for PopoverDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-popover__dropdown" } };
        for child in children {
            dropdown.append_child(child);
        }
        dropdown
    }
}
