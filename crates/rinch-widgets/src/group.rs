//! Group widget.
//!
//! A horizontal flex container with consistent spacing.

use rinch_core::Widget;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Group gap size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupGap {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl GroupGap {
    /// Get the CSS class name for this gap.
    pub fn class_name(&self) -> &'static str {
        match self {
            GroupGap::Xs => "rinch-group--gap-xs",
            GroupGap::Sm => "rinch-group--gap-sm",
            GroupGap::Md => "rinch-group--gap-md",
            GroupGap::Lg => "rinch-group--gap-lg",
            GroupGap::Xl => "rinch-group--gap-xl",
        }
    }
}

impl std::str::FromStr for GroupGap {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(GroupGap::Xs),
            "sm" => Ok(GroupGap::Sm),
            "md" => Ok(GroupGap::Md),
            "lg" => Ok(GroupGap::Lg),
            "xl" => Ok(GroupGap::Xl),
            _ => Err(()),
        }
    }
}

/// Alignment options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupAlign {
    Stretch,
    Start,
    #[default]
    Center,
    End,
    Baseline,
}

impl GroupAlign {
    /// Get the CSS class name for this alignment.
    pub fn class_name(&self) -> &'static str {
        match self {
            GroupAlign::Stretch => "rinch-group--align-stretch",
            GroupAlign::Start => "rinch-group--align-start",
            GroupAlign::Center => "rinch-group--align-center",
            GroupAlign::End => "rinch-group--align-end",
            GroupAlign::Baseline => "rinch-group--align-baseline",
        }
    }
}

impl std::str::FromStr for GroupAlign {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stretch" => Ok(GroupAlign::Stretch),
            "start" | "flex-start" => Ok(GroupAlign::Start),
            "center" => Ok(GroupAlign::Center),
            "end" | "flex-end" => Ok(GroupAlign::End),
            "baseline" => Ok(GroupAlign::Baseline),
            _ => Err(()),
        }
    }
}

/// Justification options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupJustify {
    #[default]
    Start,
    Center,
    End,
    Between,
    Around,
}

impl GroupJustify {
    /// Get the CSS class name for this justification.
    pub fn class_name(&self) -> &'static str {
        match self {
            GroupJustify::Start => "rinch-group--justify-start",
            GroupJustify::Center => "rinch-group--justify-center",
            GroupJustify::End => "rinch-group--justify-end",
            GroupJustify::Between => "rinch-group--justify-between",
            GroupJustify::Around => "rinch-group--justify-around",
        }
    }
}

impl std::str::FromStr for GroupJustify {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "start" | "flex-start" => Ok(GroupJustify::Start),
            "center" => Ok(GroupJustify::Center),
            "end" | "flex-end" => Ok(GroupJustify::End),
            "between" | "space-between" => Ok(GroupJustify::Between),
            "around" | "space-around" => Ok(GroupJustify::Around),
            _ => Err(()),
        }
    }
}

/// A horizontal flex container with consistent spacing.
#[derive(Debug, Default)]
pub struct Group {
    /// Gap between children (xs, sm, md, lg, xl).
    pub gap: Option<String>,
    /// Alignment of children (stretch, start, center, end, baseline).
    pub align: Option<String>,
    /// Justification of children (start, center, end, between, around).
    pub justify: Option<String>,
    /// Whether items should wrap.
    pub wrap: bool,
    /// Whether children should grow to fill available space.
    pub grow: bool,
}

impl Group {
    /// Generate the CSS class string for this group.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-group"];

        // Gap class
        let gap: GroupGap = self
            .gap
            .as_ref()
            .and_then(|g| g.parse().ok())
            .unwrap_or_default();
        classes.push(gap.class_name());

        // Alignment class
        let align: GroupAlign = self
            .align
            .as_ref()
            .and_then(|a| a.parse().ok())
            .unwrap_or_default();
        classes.push(align.class_name());

        // Justification class
        if let Some(ref justify) = self.justify
            && let Ok(j) = justify.parse::<GroupJustify>()
        {
            classes.push(j.class_name());
        }

        // Wrap
        if self.wrap {
            classes.push("rinch-group--wrap");
        }

        // Grow
        if self.grow {
            classes.push("rinch-group--grow");
        }

        classes.join(" ")
    }
}

impl Widget for Group {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! {
            div { class: "rinch-group" }
        };
        container.set_attribute("class", &self.class_string());
        for child in children {
            container.append_child(child);
        }
        container
    }
}
