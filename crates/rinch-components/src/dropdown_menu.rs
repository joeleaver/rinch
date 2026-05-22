//! DropdownMenu component.
//!
//! A dropdown menu component (distinct from native AppMenu/Menu).

use rinch_core::Component;
use rinch_core::Signal;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::events::get_click_context;
use rinch_core::reactive::Effect;
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::rc::Rc;

/// Thread-local signal that menu containers (ContextMenu, DropdownMenu) set
/// before their children render. DropdownMenuItem reads it to close the menu
/// on item click.
///
/// This must be thread-local (not context) because Component::render receives
/// pre-rendered children — context set in render() is too late.
use std::cell::RefCell;
thread_local! {
    static MENU_CLOSE_SIGNAL: RefCell<Option<Signal<bool>>> = const { RefCell::new(None) };
}

/// Set the menu close signal. Call before rendering menu item children.
pub fn set_menu_close_signal(signal: Signal<bool>) {
    MENU_CLOSE_SIGNAL.with(|s| *s.borrow_mut() = Some(signal));
}

/// Clear the menu close signal. Call after rendering menu item children.
pub fn clear_menu_close_signal() {
    MENU_CLOSE_SIGNAL.with(|s| *s.borrow_mut() = None);
}

/// Get the current menu close signal, if any.
pub fn get_menu_close_signal() -> Option<Signal<bool>> {
    MENU_CLOSE_SIGNAL.with(|s| *s.borrow())
}

/// Reactive callback type for opened state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Dropdown menu position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DropdownMenuPosition {
    #[default]
    Bottom,
    BottomStart,
    BottomEnd,
    Top,
    TopStart,
    TopEnd,
    Left,
    LeftStart,
    LeftEnd,
    Right,
    RightStart,
    RightEnd,
}

impl DropdownMenuPosition {
    pub fn class_name(&self) -> &'static str {
        match self {
            DropdownMenuPosition::Bottom => "rinch-dropdown-menu--bottom",
            DropdownMenuPosition::BottomStart => "rinch-dropdown-menu--bottom-start",
            DropdownMenuPosition::BottomEnd => "rinch-dropdown-menu--bottom-end",
            DropdownMenuPosition::Top => "rinch-dropdown-menu--top",
            DropdownMenuPosition::TopStart => "rinch-dropdown-menu--top-start",
            DropdownMenuPosition::TopEnd => "rinch-dropdown-menu--top-end",
            DropdownMenuPosition::Left => "rinch-dropdown-menu--left",
            DropdownMenuPosition::LeftStart => "rinch-dropdown-menu--left-start",
            DropdownMenuPosition::LeftEnd => "rinch-dropdown-menu--left-end",
            DropdownMenuPosition::Right => "rinch-dropdown-menu--right",
            DropdownMenuPosition::RightStart => "rinch-dropdown-menu--right-start",
            DropdownMenuPosition::RightEnd => "rinch-dropdown-menu--right-end",
        }
    }
}

impl std::str::FromStr for DropdownMenuPosition {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "bottom" => Ok(DropdownMenuPosition::Bottom),
            "bottom_start" | "bottomstart" => Ok(DropdownMenuPosition::BottomStart),
            "bottom_end" | "bottomend" => Ok(DropdownMenuPosition::BottomEnd),
            "top" => Ok(DropdownMenuPosition::Top),
            "top_start" | "topstart" => Ok(DropdownMenuPosition::TopStart),
            "top_end" | "topend" => Ok(DropdownMenuPosition::TopEnd),
            "left" => Ok(DropdownMenuPosition::Left),
            "left_start" | "leftstart" => Ok(DropdownMenuPosition::LeftStart),
            "left_end" | "leftend" => Ok(DropdownMenuPosition::LeftEnd),
            "right" => Ok(DropdownMenuPosition::Right),
            "right_start" | "rightstart" => Ok(DropdownMenuPosition::RightStart),
            "right_end" | "rightend" => Ok(DropdownMenuPosition::RightEnd),
            _ => Err(()),
        }
    }
}

/// A dropdown menu component.
///
/// # Example
///
/// ```ignore
/// let show_menu = Signal::new(false);
///
/// rsx! {
///     DropdownMenu { opened: show_menu.get(),
///         DropdownMenuTarget {
///             Button { onclick: move || show_menu.update(|v| *v = !*v),
///                 "Options"
///             }
///         }
///         DropdownMenuDropdown {
///             DropdownMenuItem { onclick: || println!("Edit"), "Edit" }
///             DropdownMenuItem { onclick: || println!("Delete"), "Delete" }
///             DropdownMenuDivider {}
///             DropdownMenuItem { color: "red", "Delete All" }
///         }
///     }
/// }
/// ```
pub struct DropdownMenu {
    /// Whether the menu is open (static, for initial render or non-reactive use).
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    pub opened_fn: Option<ReactiveBool>,
    /// Position relative to target.
    pub position: String,
    /// Offset from target.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: String,
    /// Shadow size.
    pub shadow: String,
    /// Whether clicking outside closes menu.
    pub close_on_click_outside: bool,
    /// Whether clicking item closes menu.
    pub close_on_item_click: bool,
    /// Width of the dropdown.
    pub width: String,
    /// Z-index.
    pub z_index: Option<i32>,
}

impl std::fmt::Debug for DropdownMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DropdownMenu")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("position", &self.position)
            .finish()
    }
}

impl Default for DropdownMenu {
    fn default() -> Self {
        Self {
            opened: false,
            opened_fn: None,
            position: String::new(),
            offset: None,
            radius: String::new(),
            shadow: String::new(),
            close_on_click_outside: true,
            close_on_item_click: true,
            width: String::new(),
            z_index: None,
        }
    }
}

impl DropdownMenu {
    /// Resolved user position (or default `Bottom`).
    fn resolved_position(&self) -> DropdownMenuPosition {
        if self.position.is_empty() {
            DropdownMenuPosition::Bottom
        } else {
            self.position
                .parse::<DropdownMenuPosition>()
                .unwrap_or(DropdownMenuPosition::Bottom)
        }
    }

    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-dropdown-menu"];

        classes.push(self.resolved_position().class_name());

        if !self.radius.is_empty() {
            match self.radius.as_str() {
                "xs" => classes.push("rinch-dropdown-menu--radius-xs"),
                "sm" => classes.push("rinch-dropdown-menu--radius-sm"),
                "md" => classes.push("rinch-dropdown-menu--radius-md"),
                "lg" => classes.push("rinch-dropdown-menu--radius-lg"),
                "xl" => classes.push("rinch-dropdown-menu--radius-xl"),
                _ => {}
            }
        }

        if !self.shadow.is_empty() {
            match self.shadow.as_str() {
                "xs" => classes.push("rinch-dropdown-menu--shadow-xs"),
                "sm" => classes.push("rinch-dropdown-menu--shadow-sm"),
                "md" => classes.push("rinch-dropdown-menu--shadow-md"),
                "lg" => classes.push("rinch-dropdown-menu--shadow-lg"),
                "xl" => classes.push("rinch-dropdown-menu--shadow-xl"),
                _ => {}
            }
        }

        if self.opened {
            classes.push("rinch-dropdown-menu--opened");
        }

        classes.join(" ")
    }
}

impl Component for DropdownMenu {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let is_opened = if let Some(ref opened_fn) = self.opened_fn {
            opened_fn()
        } else {
            self.opened
        };

        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-dropdown-menu-offset: {}px", offset));
        }
        if !self.width.is_empty() {
            style_parts.push(format!("--rinch-dropdown-menu-width: {}", self.width));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-dropdown-menu" } };
        root.set_attribute("class", &self.class_string());

        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        // Collect dropdown content children (everything after the first child/target)
        let mut dropdown_handles = Vec::new();
        for (i, child) in children.iter().enumerate() {
            root.append_child(child);
            if i > 0 {
                dropdown_handles.push(child.clone());
            }
        }

        // Set initial inline visibility on dropdown children.
        // Use display:none to hide — visibility/pointer-events don't propagate
        // reliably to descendants through Stylo's style recomputation.
        let vis_style = if is_opened {
            "display: block"
        } else {
            "display: none"
        };
        for handle in &dropdown_handles {
            handle.set_attribute("style", vis_style);
        }

        // If reactive opened_fn is provided, create an Effect to toggle visibility
        // and reposition near the viewport edge.
        if let Some(ref opened_fn) = self.opened_fn {
            let opened_fn_vis = opened_fn.clone();
            let dropdown_handles_vis = dropdown_handles.clone();
            Effect::new(move || {
                let style = if opened_fn_vis() {
                    "display: block"
                } else {
                    "display: none"
                };
                for handle in &dropdown_handles_vis {
                    handle.set_attribute("style", style);
                }
            });

            // Auto-flip: when the menu opens within ~220px of the viewport's
            // bottom edge (and has room above), swap a Bottom*/Top* position
            // class so the dropdown opens upward instead of clipping. Decided
            // at open time using ClickContext — the click that triggered the
            // open populated the trigger's bounds and viewport size, which is
            // the best info available before layout has measured the dropdown.
            const DROPDOWN_RESERVE_PX: f32 = 220.0;
            let user_pos = self.resolved_position();
            let root_for_flip = root.clone();
            let opened_fn_flip = opened_fn.clone();
            let was_open = std::cell::Cell::new(false);
            let flipped = std::cell::Cell::new(false);
            Effect::new(move || {
                let is_open = opened_fn_flip();
                if is_open && !was_open.get() {
                    let ctx = get_click_context();
                    let space_below = ctx.viewport_height - ctx.element_y - ctx.element_height;
                    let space_above = ctx.element_y;
                    let want_flip = space_below < DROPDOWN_RESERVE_PX && space_above > space_below;
                    if want_flip != flipped.get() {
                        let old_class = if flipped.get() {
                            flip_position(user_pos)
                        } else {
                            user_pos
                        };
                        let new_class = if want_flip {
                            flip_position(user_pos)
                        } else {
                            user_pos
                        };
                        root_for_flip.remove_class(old_class.class_name());
                        root_for_flip.add_class(new_class.class_name());
                        flipped.set(want_flip);
                    }
                }
                was_open.set(is_open);
            });
        }

        root
    }
}

/// Flip a position from Bottom* ↔ Top* (other positions return unchanged).
fn flip_position(pos: DropdownMenuPosition) -> DropdownMenuPosition {
    match pos {
        DropdownMenuPosition::Bottom => DropdownMenuPosition::Top,
        DropdownMenuPosition::BottomStart => DropdownMenuPosition::TopStart,
        DropdownMenuPosition::BottomEnd => DropdownMenuPosition::TopEnd,
        DropdownMenuPosition::Top => DropdownMenuPosition::Bottom,
        DropdownMenuPosition::TopStart => DropdownMenuPosition::BottomStart,
        DropdownMenuPosition::TopEnd => DropdownMenuPosition::BottomEnd,
        other => other,
    }
}

/// Target element for the dropdown menu.
#[derive(Debug, Default)]
pub struct DropdownMenuTarget;

impl Component for DropdownMenuTarget {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let target = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__target" } };

        for child in children {
            target.append_child(child);
        }

        target
    }
}

/// Dropdown content container.
#[derive(Debug, Default)]
pub struct DropdownMenuDropdown;

impl Component for DropdownMenuDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__dropdown" } };

        for child in children {
            dropdown.append_child(child);
        }

        dropdown
    }
}

/// A menu item in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuItem {
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon.
    pub right_section: Option<TablerIcon>,
    /// Color variant.
    pub color: String,
    /// Whether item is disabled.
    pub disabled: bool,
    /// Click callback.
    pub onclick: Option<rinch_core::Callback>,
}

impl Component for DropdownMenuItem {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut classes = vec!["rinch-dropdown-menu__item"];

        if self.disabled {
            classes.push("rinch-dropdown-menu__item--disabled");
        }

        let color_style = if self.color.is_empty() {
            None
        } else {
            let c = &self.color;
            if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                Some(format!("--rinch-dropdown-menu-item-color: {}", c))
            } else {
                Some(format!(
                    "--rinch-dropdown-menu-item-color: var(--rinch-color-{}-6)",
                    c
                ))
            }
        };

        let btn = rinch_macros::rsx! { button { class: "rinch-dropdown-menu__item" } };
        btn.set_attribute("class", &classes.join(" "));

        if let Some(ref style) = color_style {
            btn.set_attribute("style", style);
        }

        if self.disabled {
            btn.set_attribute("disabled", "");
        }

        // Click handler — also closes the parent menu if one exists
        if let Some(ref cb) = self.onclick {
            let close_signal = get_menu_close_signal();
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || {
                    cb.invoke();
                    if let Some(signal) = close_signal {
                        signal.set(false);
                    }
                }
            });
            btn.set_attribute("data-rid", &handler_id.0.to_string());
        }

        // Left section
        if let Some(icon) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-left" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            btn.append_child(&left_span);
        }

        // Label
        let label_span = rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-label" } };
        for child in children {
            label_span.append_child(child);
        }
        btn.append_child(&label_span);

        // Right section
        if let Some(icon) = self.right_section {
            let right_span =
                rinch_macros::rsx! { span { class: "rinch-dropdown-menu__item-right" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            btn.append_child(&right_span);
        }

        btn
    }
}

/// A label/header in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuLabel;

impl Component for DropdownMenuLabel {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let label = rinch_macros::rsx! { div { class: "rinch-dropdown-menu__label" } };

        for child in children {
            label.append_child(child);
        }

        label
    }
}

/// A divider in the dropdown.
#[derive(Debug, Default)]
pub struct DropdownMenuDivider;

impl Component for DropdownMenuDivider {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        rinch_macros::rsx! { div { class: "rinch-dropdown-menu__divider" } }
    }
}
