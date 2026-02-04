//! Pagination widget.
//!
//! Page navigation controls.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{ValueCallback, Widget};

/// Pagination size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PaginationSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl PaginationSize {
    pub fn class_name(&self) -> &'static str {
        match self {
            PaginationSize::Xs => "rinch-pagination--xs",
            PaginationSize::Sm => "rinch-pagination--sm",
            PaginationSize::Md => "rinch-pagination--md",
            PaginationSize::Lg => "rinch-pagination--lg",
            PaginationSize::Xl => "rinch-pagination--xl",
        }
    }
}

impl std::str::FromStr for PaginationSize {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(PaginationSize::Xs),
            "sm" => Ok(PaginationSize::Sm),
            "md" => Ok(PaginationSize::Md),
            "lg" => Ok(PaginationSize::Lg),
            "xl" => Ok(PaginationSize::Xl),
            _ => Err(()),
        }
    }
}

/// Pagination component for navigating between pages.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Pagination {
///         total: 10,
///         value: 1,
///         siblings: 1,
///         boundaries: 1,
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Pagination {
    /// Total number of pages.
    pub total: u32,
    /// Current active page (1-indexed).
    pub value: u32,
    /// Number of siblings on each side of current page.
    pub siblings: u32,
    /// Number of pages at the start and end.
    pub boundaries: u32,
    /// Size variant (xs, sm, md, lg, xl).
    pub size: Option<String>,
    /// Border radius.
    pub radius: Option<String>,
    /// Whether to show first/last buttons.
    pub with_edges: bool,
    /// Whether to show prev/next controls.
    pub with_controls: bool,
    /// Color for active page.
    pub color: Option<String>,
    /// Whether pagination is disabled.
    pub disabled: bool,
    /// Gap between items.
    pub gap: Option<String>,
    /// Callback when page changes - receives the new page number.
    pub onchange: Option<ValueCallback<u32>>,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            total: 1,
            value: 1,
            siblings: 1,
            boundaries: 1,
            size: None,
            radius: None,
            with_edges: false,
            with_controls: true,
            color: None,
            disabled: false,
            gap: None,
            onchange: None,
        }
    }
}

impl Pagination {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-pagination"];

        if let Some(ref s) = self.size {
            if let Ok(size) = s.parse::<PaginationSize>() {
                classes.push(size.class_name());
            }
        }

        if let Some(ref r) = self.radius {
            match r.as_str() {
                "xs" => classes.push("rinch-pagination--radius-xs"),
                "sm" => classes.push("rinch-pagination--radius-sm"),
                "md" => classes.push("rinch-pagination--radius-md"),
                "lg" => classes.push("rinch-pagination--radius-lg"),
                "xl" => classes.push("rinch-pagination--radius-xl"),
                _ => {}
            }
        }

        if self.disabled {
            classes.push("rinch-pagination--disabled");
        }

        classes.join(" ")
    }

    fn generate_pages(&self) -> Vec<PageItem> {
        let total = self.total.max(1);
        let current = self.value.clamp(1, total);
        let siblings = self.siblings;
        let boundaries = self.boundaries;

        let mut pages = Vec::new();

        // Calculate range
        let left_siblings = (current as i32 - siblings as i32).max(boundaries as i32 + 2);
        let right_siblings =
            (current as i32 + siblings as i32).min(total as i32 - boundaries as i32 - 1);

        let show_left_dots = left_siblings > (boundaries as i32 + 2);
        let show_right_dots = right_siblings < (total as i32 - boundaries as i32 - 1);

        // Left boundary pages
        for i in 1..=boundaries.min(total) {
            pages.push(PageItem::Page(i));
        }

        // Left dots
        if show_left_dots {
            pages.push(PageItem::Dots);
        } else if boundaries + 1 < left_siblings as u32 {
            for i in (boundaries + 1)..(left_siblings as u32) {
                if i <= total {
                    pages.push(PageItem::Page(i));
                }
            }
        }

        // Middle pages (siblings + current)
        let middle_end = right_siblings.min(total as i32);
        if middle_end >= left_siblings.max(1) {
            for i in left_siblings.max(1)..=middle_end {
                let page = i as u32;
                if page > boundaries && page <= total.saturating_sub(boundaries) {
                    pages.push(PageItem::Page(page));
                }
            }
        }

        // Right dots
        if show_right_dots {
            pages.push(PageItem::Dots);
        } else if right_siblings >= 0 {
            let rs = right_siblings as u32;
            let end = total.saturating_sub(boundaries).saturating_add(1);
            if rs + 1 < end {
                for i in (rs + 1)..end {
                    if i <= total {
                        pages.push(PageItem::Page(i));
                    }
                }
            }
        }

        // Right boundary pages
        let start = total.saturating_sub(boundaries).saturating_add(1).max(boundaries.saturating_add(1));
        for i in start..=total {
            if i > 0 && i > boundaries {
                pages.push(PageItem::Page(i));
            }
        }

        // Deduplicate and sort
        pages.dedup();
        pages
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PageItem {
    Page(u32),
    Dots,
}

impl Widget for Pagination {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(ref c) = self.color {
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                style_parts.push(format!("--rinch-pagination-color: {}", c));
            } else {
                style_parts.push(format!(
                    "--rinch-pagination-color: var(--rinch-color-{}-6)",
                    c
                ));
            }
        }
        if let Some(ref g) = self.gap {
            style_parts.push(format!("gap: {}", g));
        }

        let nav = rinch_macros::rsx! { nav { class: "rinch-pagination" } };
        nav.set_attribute("class", &self.class_string());

        if !style_parts.is_empty() {
            nav.set_attribute("style", &style_parts.join("; "));
        }

        let controls = rinch_macros::rsx! { div { class: "rinch-pagination__controls" } };

        // First button
        if self.with_edges {
            let first_disabled = self.value <= 1 || self.disabled;
            let mut first_class = "rinch-pagination__control".to_string();
            if first_disabled {
                first_class.push_str(" disabled");
            }

            let first_btn = rinch_macros::rsx! { button { class: "rinch-pagination__control" } };
            first_btn.set_attribute("class", &first_class);
            first_btn.set_attribute("data-first", "");

            let icon = crate::icons::chevrons_left_dom(__scope);
            first_btn.append_child(&icon);

            if self.disabled {
                first_btn.set_attribute("disabled", "");
            }

            // Register click handler for first page
            if let Some(ref cb) = self.onchange {
                if !first_disabled {
                    let cb = cb.clone();
                    let handler_id = __scope.register_handler(move || cb.invoke(1));
                    first_btn.set_attribute("data-rid", &handler_id.0.to_string());
                }
            }
            controls.append_child(&first_btn);
        }

        // Previous button
        if self.with_controls {
            let prev_disabled = self.value <= 1 || self.disabled;
            let mut prev_class = "rinch-pagination__control".to_string();
            if prev_disabled {
                prev_class.push_str(" disabled");
            }

            let prev_btn = rinch_macros::rsx! { button { class: "rinch-pagination__control" } };
            prev_btn.set_attribute("class", &prev_class);
            prev_btn.set_attribute("data-prev", "");

            let icon = crate::icons::chevron_left_dom(__scope);
            prev_btn.append_child(&icon);

            if self.disabled {
                prev_btn.set_attribute("disabled", "");
            }

            // Register click handler for previous page
            if let Some(ref cb) = self.onchange {
                if !prev_disabled {
                    let cb = cb.clone();
                    let prev_page = self.value.saturating_sub(1).max(1);
                    let handler_id = __scope.register_handler(move || cb.invoke(prev_page));
                    prev_btn.set_attribute("data-rid", &handler_id.0.to_string());
                }
            }
            controls.append_child(&prev_btn);
        }

        // Page items container
        let items_container = rinch_macros::rsx! { div { class: "rinch-pagination__items" } };

        for item in self.generate_pages() {
            match item {
                PageItem::Page(n) => {
                    let mut page_class = "rinch-pagination__item".to_string();
                    if n == self.value {
                        page_class.push_str(" rinch-pagination__item--active");
                    }

                    let page_btn = rinch_macros::rsx! { button { class: "rinch-pagination__item" } };
                    page_btn.set_attribute("class", &page_class);
                    page_btn.set_attribute("data-page", &n.to_string());

                    let page_text = __scope.create_text(&n.to_string());
                    page_btn.append_child(&page_text);

                    if self.disabled {
                        page_btn.set_attribute("disabled", "");
                    }

                    // Register click handler for this page
                    if let Some(ref cb) = self.onchange {
                        if !self.disabled {
                            let cb = cb.clone();
                            let handler_id = __scope.register_handler(move || cb.invoke(n));
                            page_btn.set_attribute("data-rid", &handler_id.0.to_string());
                        }
                    }
                    items_container.append_child(&page_btn);
                }
                PageItem::Dots => {
                    let dots_span =
                        rinch_macros::rsx! { span { class: "rinch-pagination__dots", "..." } };
                    items_container.append_child(&dots_span);
                }
            }
        }

        controls.append_child(&items_container);

        // Next button
        if self.with_controls {
            let next_disabled = self.value >= self.total || self.disabled;
            let mut next_class = "rinch-pagination__control".to_string();
            if next_disabled {
                next_class.push_str(" disabled");
            }

            let next_btn = rinch_macros::rsx! { button { class: "rinch-pagination__control" } };
            next_btn.set_attribute("class", &next_class);
            next_btn.set_attribute("data-next", "");

            let icon = crate::icons::chevron_right_dom(__scope);
            next_btn.append_child(&icon);

            if self.disabled {
                next_btn.set_attribute("disabled", "");
            }

            // Register click handler for next page
            if let Some(ref cb) = self.onchange {
                if !next_disabled {
                    let cb = cb.clone();
                    let next_page = (self.value + 1).min(self.total);
                    let handler_id = __scope.register_handler(move || cb.invoke(next_page));
                    next_btn.set_attribute("data-rid", &handler_id.0.to_string());
                }
            }
            controls.append_child(&next_btn);
        }

        // Last button
        if self.with_edges {
            let last_disabled = self.value >= self.total || self.disabled;
            let mut last_class = "rinch-pagination__control".to_string();
            if last_disabled {
                last_class.push_str(" disabled");
            }

            let last_btn = rinch_macros::rsx! { button { class: "rinch-pagination__control" } };
            last_btn.set_attribute("class", &last_class);
            last_btn.set_attribute("data-last", "");

            let icon = crate::icons::chevrons_right_dom(__scope);
            last_btn.append_child(&icon);

            if self.disabled {
                last_btn.set_attribute("disabled", "");
            }

            // Register click handler for last page
            if let Some(ref cb) = self.onchange {
                if !last_disabled {
                    let cb = cb.clone();
                    let total = self.total;
                    let handler_id = __scope.register_handler(move || cb.invoke(total));
                    last_btn.set_attribute("data-rid", &handler_id.0.to_string());
                }
            }
            controls.append_child(&last_btn);
        }

        nav.append_child(&controls);

        nav
    }
}
