//! Skeleton component.
//!
//! A loading placeholder that mimics content shape.

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Skeleton radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkeletonRadius {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl SkeletonRadius {
    pub fn class_name(&self) -> &'static str {
        match self {
            SkeletonRadius::Xs => "rinch-skeleton--radius-xs",
            SkeletonRadius::Sm => "rinch-skeleton--radius-sm",
            SkeletonRadius::Md => "rinch-skeleton--radius-md",
            SkeletonRadius::Lg => "rinch-skeleton--radius-lg",
            SkeletonRadius::Xl => "rinch-skeleton--radius-xl",
        }
    }
}

impl std::str::FromStr for SkeletonRadius {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(SkeletonRadius::Xs),
            "sm" => Ok(SkeletonRadius::Sm),
            "md" => Ok(SkeletonRadius::Md),
            "lg" => Ok(SkeletonRadius::Lg),
            "xl" => Ok(SkeletonRadius::Xl),
            _ => Err(()),
        }
    }
}

/// A skeleton loading placeholder.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Skeleton { height: "1rem", width: "70%" }
///     Skeleton { height: "1rem", width: "50%" }
///     Skeleton { circle: true, height: "3rem" }
/// }
/// ```
#[derive(Debug)]
pub struct Skeleton {
    /// Width (CSS value).
    pub width: String,
    /// Height (CSS value).
    pub height: String,
    /// Border radius (xs, sm, md, lg, xl).
    pub radius: String,
    /// Whether to render as a circle.
    pub circle: bool,
    /// Whether to show animation.
    pub animate: bool,
    /// One sweep of the animation, as a CSS time (`"800ms"`, `"3s"`). Empty
    /// (the default) is the stylesheet's `1.5s`. Written to
    /// `--rinch-skeleton-duration`.
    pub duration: String,
    /// Whether content is still loading. When `false` the children are
    /// rendered instead, in a `<span class="rinch-skeleton__content">` with
    /// `display: contents`: it generates no box, so the children lay out as if
    /// they were the parent's own (a block stays a block, and a flex or grid
    /// parent sees each child as an item).
    pub visible: bool,
}

impl Default for Skeleton {
    fn default() -> Self {
        Self {
            width: String::new(),
            height: String::new(),
            radius: String::new(),
            circle: false,
            animate: true,
            duration: String::new(),
            visible: true,
        }
    }
}

impl Skeleton {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-skeleton"];

        if self.circle {
            classes.push("rinch-skeleton--circle");
        } else if !self.radius.is_empty() {
            if let Ok(radius) = self.radius.parse::<SkeletonRadius>() {
                classes.push(radius.class_name());
            }
        }

        if self.animate {
            classes.push("rinch-skeleton--animate");
        }

        classes.join(" ")
    }
}

impl Component for Skeleton {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        // If not visible (loaded), render children in a wrapper that generates
        // no box. It was a bare inline `<span>` (#2207): block children sat in
        // an inline box, and a flex or grid parent saw one item.
        if !self.visible {
            let wrapper = rinch_macros::rsx! { span { class: "rinch-skeleton__content" } };
            for child in children {
                wrapper.append_child(child);
            }
            return wrapper;
        }

        let container = rinch_macros::rsx! { div { class: "rinch-skeleton" } };
        container.set_attribute("class", &self.class_string());

        let mut styles = Vec::new();
        if !self.width.is_empty() {
            let w = &self.width;
            styles.push(format!("width: {}", w));
        }
        if !self.height.is_empty() {
            let h = &self.height;
            styles.push(format!("height: {}", h));
            if self.circle {
                styles.push(format!("width: {}", h)); // Circle needs equal width
            }
        }

        if !self.duration.trim().is_empty() {
            styles.push(format!(
                "--rinch-skeleton-duration: {}",
                self.duration.trim()
            ));
        }

        if !styles.is_empty() {
            container.set_attribute("style", &styles.join("; "));
        }

        // Skeleton doesn't render children when visible (it's a placeholder)
        let _ = children;

        container
    }
}
