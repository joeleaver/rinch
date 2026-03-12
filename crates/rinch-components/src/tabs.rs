//! Tabs component.
//!
//! Tab-based navigation for switching between content panels.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::Signal;
use rinch_core::{Callback, Component};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};

/// Tab variant style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsVariant {
    #[default]
    Default,
    Outline,
    Pills,
}

impl TabsVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            TabsVariant::Default => "rinch-tabs--default",
            TabsVariant::Outline => "rinch-tabs--outline",
            TabsVariant::Pills => "rinch-tabs--pills",
        }
    }
}

impl std::str::FromStr for TabsVariant {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(TabsVariant::Default),
            "outline" => Ok(TabsVariant::Outline),
            "pills" => Ok(TabsVariant::Pills),
            _ => Err(()),
        }
    }
}

/// Tab orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsOrientation {
    #[default]
    Horizontal,
    Vertical,
}

impl TabsOrientation {
    pub fn class_name(&self) -> &'static str {
        match self {
            TabsOrientation::Horizontal => "rinch-tabs--horizontal",
            TabsOrientation::Vertical => "rinch-tabs--vertical",
        }
    }
}

impl std::str::FromStr for TabsOrientation {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "horizontal" => Ok(TabsOrientation::Horizontal),
            "vertical" => Ok(TabsOrientation::Vertical),
            _ => Err(()),
        }
    }
}

/// Tab position relative to content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TabsPosition {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

impl TabsPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            TabsPosition::Top => "rinch-tabs--position-top",
            TabsPosition::Bottom => "rinch-tabs--position-bottom",
            TabsPosition::Left => "rinch-tabs--position-left",
            TabsPosition::Right => "rinch-tabs--position-right",
        }
    }
}

impl std::str::FromStr for TabsPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "top" => Ok(TabsPosition::Top),
            "bottom" => Ok(TabsPosition::Bottom),
            "left" => Ok(TabsPosition::Left),
            "right" => Ok(TabsPosition::Right),
            _ => Err(()),
        }
    }
}

/// A tabs container for organizing content into switchable panels.
///
/// Wires up tab switching: clicking a Tab button shows the corresponding
/// TabsPanel and hides others.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Tabs { value: "tab1",
///         TabsList {
///             Tab { value: "tab1", "First" }
///             Tab { value: "tab2", "Second" }
///         }
///         TabsPanel { value: "tab1", "First panel content" }
///         TabsPanel { value: "tab2", "Second panel content" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct Tabs {
    /// Currently active tab value.
    pub value: String,
    /// Default active tab (uncontrolled).
    pub default_value: String,
    /// Visual variant (default, outline, pills).
    pub variant: String,
    /// Tab orientation (horizontal, vertical).
    pub orientation: String,
    /// Tab position (top, bottom, left, right).
    pub position: String,
    /// Whether tabs grow to fill container.
    pub grow: bool,
    /// Color for active tab.
    pub color: String,
    /// Border radius.
    pub radius: String,
}

impl Tabs {
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-tabs"];

        if !self.variant.is_empty() {
            if let Ok(variant) = self.variant.parse::<TabsVariant>() {
                classes.push(variant.class_name());
            }
        } else {
            classes.push(TabsVariant::Default.class_name());
        }

        if !self.orientation.is_empty() {
            if let Ok(orientation) = self.orientation.parse::<TabsOrientation>() {
                classes.push(orientation.class_name());
            }
        } else {
            classes.push(TabsOrientation::Horizontal.class_name());
        }

        if !self.position.is_empty()
            && let Ok(position) = self.position.parse::<TabsPosition>()
        {
            classes.push(position.class_name());
        }

        if self.grow {
            classes.push("rinch-tabs--grow");
        }

        classes.join(" ")
    }
}

impl Component for Tabs {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-tabs" } };
        container.set_attribute("class", &self.class_string());

        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-tabs-color: {}", c)
            } else {
                format!("--rinch-tabs-color: var(--rinch-color-{}-6)", c)
            };
            container.set_attribute("style", &style);
        }

        let active_value = if !self.value.is_empty() {
            self.value.clone()
        } else {
            self.default_value.clone()
        };

        let active_tab = Signal::new(active_value);

        // Collect tab buttons and panels from children
        let mut tab_buttons: Vec<(String, NodeHandle)> = Vec::new();
        let mut panels: Vec<(String, NodeHandle)> = Vec::new();

        for child in children {
            if let Some(cls) = child.get_attribute("class") {
                if cls.contains("rinch-tabs__list") {
                    // Walk the TabsList's children to find Tab buttons
                    for tab_btn in child.children() {
                        if let Some(val) = tab_btn.get_attribute("data-tab-value") {
                            tab_buttons.push((val, tab_btn));
                        }
                    }
                } else if cls.contains("rinch-tabs__panel") {
                    if let Some(val) = child.get_attribute("data-panel-value") {
                        panels.push((val, child.clone()));
                    }
                }
            }
            container.append_child(child);
        }

        // Wire up click handlers on tab buttons
        for (value, btn) in &tab_buttons {
            let val = value.clone();
            let active = active_tab;
            // Preserve any user-provided onclick handler
            let user_rid = btn
                .get_attribute("data-user-rid")
                .and_then(|s| s.parse::<usize>().ok())
                .map(rinch_core::events::EventHandlerId);
            let handler_id = __scope.register_handler(move || {
                active.set(val.clone());
                // Also fire user's onclick callback if present
                if let Some(rid) = user_rid {
                    rinch_core::events::dispatch_event(rid);
                }
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Determine variant for styling
        let variant = if !self.variant.is_empty() {
            self.variant.parse::<TabsVariant>().unwrap_or_default()
        } else {
            TabsVariant::Default
        };

        // Reactive: update active tab styling via inline styles.
        // We must set color on the label span (not the button) because the button
        // is display:flex and set_style on it won't invalidate child text layouts.
        for (value, btn) in &tab_buttons {
            let val = value.clone();
            let btn = btn.clone();
            let active = active_tab;

            // Find the label span inside the button
            let label_span = btn.children().into_iter().find(|c| {
                c.get_attribute("class")
                    .is_some_and(|cls| cls.contains("rinch-tabs__tab-label"))
            });

            // Create underline indicator element for Default variant
            let indicator = if variant == TabsVariant::Default {
                let el = __scope.create_element("div");
                el.set_attribute(
                    "style",
                    "position: absolute; bottom: -2px; left: 0; right: 0; height: 2px;",
                );
                btn.append_child(&el);
                Some(el)
            } else {
                None
            };

            __scope.create_effect(move || {
                let is_active = active.get() == val;
                if is_active {
                    // Set color on label span for proper text invalidation
                    let active_color = match variant {
                        TabsVariant::Pills => "white",
                        _ => "var(--rinch-tabs-color, var(--rinch-primary-color))",
                    };
                    if let Some(ref span) = label_span {
                        span.set_style("color", active_color);
                    }
                    match variant {
                        TabsVariant::Pills => {
                            btn.set_style(
                                "background-color",
                                "var(--rinch-tabs-color, var(--rinch-primary-color))",
                            );
                        }
                        TabsVariant::Outline => {
                            btn.set_style("border-color", "var(--rinch-color-border)");
                            btn.set_style("background-color", "var(--rinch-color-body)");
                        }
                        _ => {}
                    }
                    if let Some(ref ind) = indicator {
                        ind.set_style(
                            "background-color",
                            "var(--rinch-tabs-color, var(--rinch-primary-color))",
                        );
                    }
                } else {
                    if let Some(ref span) = label_span {
                        span.set_style("color", "");
                    }
                    match variant {
                        TabsVariant::Pills => {
                            btn.set_style("background-color", "");
                        }
                        TabsVariant::Outline => {
                            btn.set_style("border-color", "transparent");
                            btn.set_style("background-color", "");
                        }
                        _ => {}
                    }
                    if let Some(ref ind) = indicator {
                        ind.set_style("background-color", "transparent");
                    }
                }
            });
        }

        // Reactive: show/hide panels
        for (value, panel) in &panels {
            let val = value.clone();
            let panel = panel.clone();
            let active = active_tab;
            __scope.create_effect(move || {
                if active.get() == val {
                    panel.set_style("display", "");
                } else {
                    panel.set_style("display", "none");
                }
            });
        }

        container
    }
}

/// Container for tab buttons.
#[derive(Debug, Default)]
pub struct TabsList {
    /// Whether tabs should grow to fill width.
    pub grow: bool,
    /// Justify content (start, center, end, space-between).
    pub justify: String,
}

impl Component for TabsList {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-tabs__list"];

        if self.grow {
            classes.push("rinch-tabs__list--grow");
        }

        let list = rinch_macros::rsx! { div { class: "rinch-tabs__list", role: "tablist" } };
        list.set_attribute("class", &classes.join(" "));

        if !self.justify.is_empty() {
            list.set_attribute("style", &format!("justify-content: {}", self.justify));
        }

        for child in children {
            list.append_child(child);
        }

        list
    }
}

/// Individual tab button.
#[derive(Default)]
pub struct Tab {
    /// Tab identifier value.
    pub value: String,
    /// Whether tab is disabled.
    pub disabled: bool,
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon.
    pub right_section: Option<TablerIcon>,
    /// Click callback.
    pub onclick: Option<Callback>,
}

impl std::fmt::Debug for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tab")
            .field("value", &self.value)
            .field("disabled", &self.disabled)
            .field("left_section", &self.left_section)
            .field("right_section", &self.right_section)
            .finish_non_exhaustive()
    }
}

impl Component for Tab {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-tabs__tab"];

        if self.disabled {
            classes.push("rinch-tabs__tab--disabled");
        }

        let btn = rinch_macros::rsx! { button { class: "rinch-tabs__tab", role: "tab" } };
        btn.set_attribute("class", &classes.join(" "));

        if !self.value.is_empty() {
            btn.set_attribute("data-tab-value", &self.value);
        }

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // User-provided click callback (toggle logic is wired by Tabs parent)
        if let Some(ref cb) = self.onclick
            && !self.disabled
        {
            // Store as a secondary attribute; Tabs will overwrite data-rid for switching
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            btn.set_attribute("data-user-rid", &handler_id.0.to_string());
        }

        // Left section icon
        if let Some(left) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-tabs__tab-left" } };
            let icon_el = render_tabler_icon(__scope, left, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            btn.append_child(&left_span);
        }

        // Label
        let label_span = rinch_macros::rsx! { span { class: "rinch-tabs__tab-label" } };
        for child in children {
            label_span.append_child(child);
        }
        btn.append_child(&label_span);

        // Right section icon
        if let Some(right) = self.right_section {
            let right_span = rinch_macros::rsx! { span { class: "rinch-tabs__tab-right" } };
            let icon_el = render_tabler_icon(__scope, right, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            btn.append_child(&right_span);
        }

        btn
    }
}

/// Content panel for a tab.
#[derive(Debug, Default)]
pub struct TabsPanel {
    /// Tab value this panel belongs to.
    pub value: String,
}

impl Component for TabsPanel {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let panel = rinch_macros::rsx! { div { class: "rinch-tabs__panel", role: "tabpanel" } };

        if !self.value.is_empty() {
            panel.set_attribute("data-panel-value", &self.value);
        }

        for child in children {
            panel.append_child(child);
        }

        panel
    }
}
