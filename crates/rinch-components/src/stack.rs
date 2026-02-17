//! Stack component.
//!
//! A vertical flex container with consistent spacing.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Stack gap size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackGap {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl StackGap {
    /// Get the CSS class name for this gap.
    pub fn class_name(&self) -> &'static str {
        match self {
            StackGap::Xs => "rinch-stack--gap-xs",
            StackGap::Sm => "rinch-stack--gap-sm",
            StackGap::Md => "rinch-stack--gap-md",
            StackGap::Lg => "rinch-stack--gap-lg",
            StackGap::Xl => "rinch-stack--gap-xl",
        }
    }
}

impl std::str::FromStr for StackGap {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(StackGap::Xs),
            "sm" => Ok(StackGap::Sm),
            "md" => Ok(StackGap::Md),
            "lg" => Ok(StackGap::Lg),
            "xl" => Ok(StackGap::Xl),
            _ => Err(()),
        }
    }
}

/// Alignment options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackAlign {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

impl StackAlign {
    /// Get the CSS class name for this alignment.
    pub fn class_name(&self) -> &'static str {
        match self {
            StackAlign::Stretch => "rinch-stack--align-stretch",
            StackAlign::Start => "rinch-stack--align-start",
            StackAlign::Center => "rinch-stack--align-center",
            StackAlign::End => "rinch-stack--align-end",
        }
    }
}

impl std::str::FromStr for StackAlign {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stretch" => Ok(StackAlign::Stretch),
            "start" | "flex-start" => Ok(StackAlign::Start),
            "center" => Ok(StackAlign::Center),
            "end" | "flex-end" => Ok(StackAlign::End),
            _ => Err(()),
        }
    }
}

/// Justification options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StackJustify {
    #[default]
    Start,
    Center,
    End,
    Between,
    Around,
}

impl StackJustify {
    /// Get the CSS class name for this justification.
    pub fn class_name(&self) -> &'static str {
        match self {
            StackJustify::Start => "rinch-stack--justify-start",
            StackJustify::Center => "rinch-stack--justify-center",
            StackJustify::End => "rinch-stack--justify-end",
            StackJustify::Between => "rinch-stack--justify-between",
            StackJustify::Around => "rinch-stack--justify-around",
        }
    }
}

impl std::str::FromStr for StackJustify {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "start" | "flex-start" => Ok(StackJustify::Start),
            "center" => Ok(StackJustify::Center),
            "end" | "flex-end" => Ok(StackJustify::End),
            "between" | "space-between" => Ok(StackJustify::Between),
            "around" | "space-around" => Ok(StackJustify::Around),
            _ => Err(()),
        }
    }
}

/// A vertical flex container with consistent spacing.
#[derive(Debug, Default)]
pub struct Stack {
    /// Gap between children (xs, sm, md, lg, xl).
    pub gap: String,
    /// Alignment of children (stretch, start, center, end).
    pub align: String,
    /// Justification of children (start, center, end, between, around).
    pub justify: String,
}

impl Stack {
    /// Generate the CSS class string for this stack.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-stack"];

        // Gap class
        let gap: StackGap = if self.gap.is_empty() {
            StackGap::default()
        } else {
            self.gap.parse().unwrap_or_default()
        };
        classes.push(gap.class_name());

        // Alignment class
        if !self.align.is_empty() {
            if let Ok(a) = self.align.parse::<StackAlign>() {
                classes.push(a.class_name());
            }
        }

        // Justification class
        if !self.justify.is_empty() {
            if let Ok(j) = self.justify.parse::<StackJustify>() {
                classes.push(j.class_name());
            }
        }

        classes.join(" ")
    }
}

impl Component for Stack {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-stack" }
        };
        // Set computed class after rsx! to avoid effect overhead
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
