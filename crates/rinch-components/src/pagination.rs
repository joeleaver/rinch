//! Pagination component.
//!
//! Page navigation controls.

use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, ValueCallback};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Renders a page button's content: called with the page number and whether it
/// is the active page. The button around it (its classes, `data-page`,
/// `aria-current` and click wiring) stays the component's.
pub type PaginationItemRenderer = Rc<dyn Fn(&mut RenderScope, u32, bool) -> NodeHandle>;

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
    pub size: String,
    /// Border radius.
    pub radius: String,
    /// Whether to show first/last buttons.
    pub with_edges: bool,
    /// Whether to show prev/next controls.
    pub with_controls: bool,
    /// Color for active page.
    pub color: String,
    /// Whether pagination is disabled.
    pub disabled: bool,
    /// Gap between items.
    pub gap: String,
    /// Callback when page changes - receives the new page number.
    pub onchange: Option<ValueCallback<u32>>,
    /// The `<nav>`'s accessible name. Empty means `"Pagination"`.
    pub aria_label: String,
    /// The first-page control's accessible name. Empty means `"First page"`.
    pub first_label: String,
    /// The previous-page control's accessible name. Empty means
    /// `"Previous page"`.
    pub previous_label: String,
    /// The next-page control's accessible name. Empty means `"Next page"`.
    pub next_label: String,
    /// The last-page control's accessible name. Empty means `"Last page"`.
    pub last_label: String,
    /// Icon for the first-page control, in place of the double chevron.
    pub first_icon: Option<TablerIcon>,
    /// Icon for the previous-page control, in place of the chevron.
    pub previous_icon: Option<TablerIcon>,
    /// Icon for the next-page control, in place of the chevron.
    pub next_icon: Option<TablerIcon>,
    /// Icon for the last-page control, in place of the double chevron.
    pub last_icon: Option<TablerIcon>,
    /// Text drawn for a gap in the page list. Empty means `"..."`.
    pub dots: String,
    /// The escape hatch: renders each page button's content in place of its
    /// number. See [`PaginationItemRenderer`].
    pub render_item: Option<PaginationItemRenderer>,
}

impl std::fmt::Debug for Pagination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pagination")
            .field("total", &self.total)
            .field("value", &self.value)
            .field("siblings", &self.siblings)
            .field("boundaries", &self.boundaries)
            .field("size", &self.size)
            .field("radius", &self.radius)
            .field("with_edges", &self.with_edges)
            .field("with_controls", &self.with_controls)
            .field("color", &self.color)
            .field("disabled", &self.disabled)
            .field("gap", &self.gap)
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .field("aria_label", &self.aria_label)
            .field("first_label", &self.first_label)
            .field("previous_label", &self.previous_label)
            .field("next_label", &self.next_label)
            .field("last_label", &self.last_label)
            .field("first_icon", &self.first_icon)
            .field("previous_icon", &self.previous_icon)
            .field("next_icon", &self.next_icon)
            .field("last_icon", &self.last_icon)
            .field("dots", &self.dots)
            .field(
                "render_item",
                &self.render_item.as_ref().map(|_| "<renderer>"),
            )
            .finish()
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            total: 1,
            value: 1,
            siblings: 1,
            boundaries: 1,
            size: String::new(),
            radius: String::new(),
            with_edges: false,
            with_controls: true,
            color: String::new(),
            disabled: false,
            gap: String::new(),
            onchange: None,
            aria_label: String::new(),
            first_label: String::new(),
            previous_label: String::new(),
            next_label: String::new(),
            last_label: String::new(),
            first_icon: None,
            previous_icon: None,
            next_icon: None,
            last_icon: None,
            dots: String::new(),
            render_item: None,
        }
    }
}

impl Pagination {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-pagination"];

        if !self.size.is_empty()
            && let Ok(size) = self.size.parse::<PaginationSize>()
        {
            classes.push(size.class_name());
        }

        let radius_cls = crate::class_utils::radius_class("rinch-pagination", &self.radius);
        classes.extend(radius_cls.as_deref());

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
        let start = total
            .saturating_sub(boundaries)
            .saturating_add(1)
            .max(boundaries.saturating_add(1));
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

/// Which of the four controls a button is.
#[derive(Clone, Copy)]
enum ControlKind {
    First,
    Previous,
    Next,
    Last,
}

/// `label` unless it is empty, then `default`.
fn or_default<'a>(label: &'a str, default: &'a str) -> &'a str {
    if label.is_empty() { default } else { label }
}

impl Pagination {
    /// One prev/next/first/last button.
    fn control(&self, scope: &mut RenderScope, kind: ControlKind) -> NodeHandle {
        let total = self.total;
        let (data, label, icon, at_edge, target) = match kind {
            ControlKind::First => (
                "data-first",
                or_default(&self.first_label, "First page"),
                self.first_icon,
                self.value <= 1,
                1,
            ),
            ControlKind::Previous => (
                "data-prev",
                or_default(&self.previous_label, "Previous page"),
                self.previous_icon,
                self.value <= 1,
                self.value.saturating_sub(1).max(1),
            ),
            ControlKind::Next => (
                "data-next",
                or_default(&self.next_label, "Next page"),
                self.next_icon,
                self.value >= total,
                (self.value + 1).min(total),
            ),
            ControlKind::Last => (
                "data-last",
                or_default(&self.last_label, "Last page"),
                self.last_icon,
                self.value >= total,
                total,
            ),
        };
        let disabled = at_edge || self.disabled;

        let btn = scope.create_element("button");
        btn.set_attribute(
            "class",
            if disabled {
                "rinch-pagination__control disabled"
            } else {
                "rinch-pagination__control"
            },
        );
        btn.set_attribute(data, "");
        btn.set_attribute("aria-label", label);

        let glyph = match icon {
            Some(icon) => render_tabler_icon(scope, icon, TablerIconStyle::Outline),
            None => match kind {
                ControlKind::First => crate::icons::chevrons_left_dom(scope),
                ControlKind::Previous => crate::icons::chevron_left_dom(scope),
                ControlKind::Next => crate::icons::chevron_right_dom(scope),
                ControlKind::Last => crate::icons::chevrons_right_dom(scope),
            },
        };
        btn.append_child(&glyph);

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        if let Some(ref cb) = self.onchange
            && !disabled
        {
            let cb = cb.clone();
            let handler_id = scope.register_handler(move || cb.invoke(target));
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }
        btn
    }
}

impl Component for Pagination {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if !self.color.is_empty() {
            style_parts.push(crate::color_utils::accent_vars(
                "--rinch-pagination",
                &self.color,
            ));
        }
        if !self.gap.is_empty() {
            style_parts.push(format!("gap: {}", self.gap));
        }

        let nav = rinch_macros::rsx! { nav { class: "rinch-pagination" } };
        nav.set_attribute("class", &self.class_string());
        nav.set_attribute("aria-label", or_default(&self.aria_label, "Pagination"));

        if !style_parts.is_empty() {
            nav.set_attribute("style", &style_parts.join("; "));
        }

        let controls = rinch_macros::rsx! { div { class: "rinch-pagination__controls" } };

        if self.with_edges {
            controls.append_child(&self.control(__scope, ControlKind::First));
        }
        if self.with_controls {
            controls.append_child(&self.control(__scope, ControlKind::Previous));
        }

        // Page items container
        let items_container = rinch_macros::rsx! { div { class: "rinch-pagination__items" } };

        for item in self.generate_pages() {
            match item {
                PageItem::Page(n) => {
                    let active = n == self.value;
                    let mut page_class = "rinch-pagination__item".to_string();
                    if active {
                        page_class.push_str(" rinch-pagination__item--active");
                    }

                    let page_btn =
                        rinch_macros::rsx! { button { class: "rinch-pagination__item" } };
                    page_btn.set_attribute("class", &page_class);
                    page_btn.set_attribute("data-page", &n.to_string());
                    if active {
                        page_btn.set_attribute("aria-current", "page");
                    }

                    let content = match &self.render_item {
                        Some(render) => render(__scope, n, active),
                        None => rinch_core::IntoNode::into_node(n.to_string(), __scope),
                    };
                    page_btn.append_child(&content);

                    if self.disabled {
                        page_btn.set_attribute("disabled", "");
                    }

                    // Register click handler for this page
                    if let Some(ref cb) = self.onchange
                        && !self.disabled
                    {
                        let cb = cb.clone();
                        let handler_id = __scope.register_handler(move || cb.invoke(n));
                        page_btn.set_attribute("data-rid", &handler_id.0.to_string());
                    }
                    items_container.append_child(&page_btn);
                }
                PageItem::Dots => {
                    let dots = or_default(&self.dots, "...");
                    let dots_span =
                        rinch_macros::rsx! { span { class: "rinch-pagination__dots", {dots} } };
                    items_container.append_child(&dots_span);
                }
            }
        }

        controls.append_child(&items_container);

        if self.with_controls {
            controls.append_child(&self.control(__scope, ControlKind::Next));
        }
        if self.with_edges {
            controls.append_child(&self.control(__scope, ControlKind::Last));
        }

        nav.append_child(&controls);

        nav
    }
}
