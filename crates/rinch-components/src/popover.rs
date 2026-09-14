//! Popover component.
//!
//! Positioned popup content relative to a target element.

use std::rc::Rc;

use rinch_core::Component;
use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::reactive::Effect;

/// A reactive `opened` getter, spelled as `Modal`, `Drawer` and `DropdownMenu`
/// spell theirs.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

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
/// let show_popover = Signal::new(false);
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
pub struct Popover {
    /// Whether the popover is open.
    pub opened: bool,
    /// Reactive opened getter - use this for fine-grained updates.
    /// When provided, the popover's dismissal tracks the signal automatically.
    pub opened_fn: Option<ReactiveBool>,
    /// Position relative to target.
    pub position: String,
    /// Offset from target in pixels.
    pub offset: Option<i32>,
    /// Border radius.
    pub radius: String,
    /// Whether popover has shadow.
    pub shadow: String,
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
    pub width: String,
    /// Z-index for the popover.
    pub z_index: Option<i32>,
    /// Whether to trap focus.
    pub trap_focus: bool,
    /// Callback when the popover should close.
    ///
    /// Both dismissal props need somewhere to send the request: a `Popover`
    /// with no `onclose` cannot close itself, so `close_on_escape` and
    /// `close_on_click_outside` are no-ops without it (issue #474).
    pub onclose: Option<rinch_core::Callback>,
}

impl std::fmt::Debug for Popover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popover")
            .field("opened", &self.opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field("position", &self.position)
            .field("offset", &self.offset)
            .field("radius", &self.radius)
            .field("shadow", &self.shadow)
            .field("with_arrow", &self.with_arrow)
            .field("arrow_size", &self.arrow_size)
            .field("arrow_offset", &self.arrow_offset)
            .field("close_on_click_outside", &self.close_on_click_outside)
            .field("close_on_escape", &self.close_on_escape)
            .field("width", &self.width)
            .field("z_index", &self.z_index)
            .field("trap_focus", &self.trap_focus)
            .field("onclose", &self.onclose.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for Popover {
    fn default() -> Self {
        Self {
            opened: false,
            opened_fn: None,
            position: String::new(),
            offset: None,
            radius: String::new(),
            shadow: String::new(),
            with_arrow: false,
            arrow_size: None,
            arrow_offset: None,
            close_on_click_outside: true,
            close_on_escape: true,
            width: String::new(),
            z_index: None,
            trap_focus: false,
            onclose: None,
        }
    }
}

impl Popover {
    /// The root's classes for the **closed** state.
    ///
    /// Split out from [`class_string`](Self::class_string) so the `opened_fn`
    /// effect can rebuild the class list without re-deriving position, radius
    /// and shadow on every toggle.
    pub fn class_string_closed(&self) -> String {
        let mut classes = vec!["rinch-popover"];

        if !self.position.is_empty() {
            if let Ok(pos) = self.position.parse::<PopoverPosition>() {
                classes.push(pos.class_name());
            }
        } else {
            classes.push(PopoverPosition::Bottom.class_name());
        }

        let radius_cls = crate::class_utils::radius_class("rinch-popover", &self.radius);
        classes.extend(radius_cls.as_deref());

        if !self.shadow.is_empty() {
            match self.shadow.as_str() {
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

        classes.join(" ")
    }

    /// The root's classes, including `rinch-popover--opened` when it is open.
    pub fn class_string(&self) -> String {
        let base = self.class_string_closed();
        if self.opened {
            format!("{base} rinch-popover--opened")
        } else {
            base
        }
    }
}

impl Component for Popover {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let mut style_parts = Vec::new();
        if let Some(offset) = self.offset {
            style_parts.push(format!("--rinch-popover-offset: {}px", offset));
        }
        if !self.width.is_empty() {
            style_parts.push(format!("--rinch-popover-width: {}", self.width));
        }
        if let Some(size) = self.arrow_size {
            style_parts.push(format!("--rinch-popover-arrow-size: {}px", size));
        }
        if let Some(offset) = self.arrow_offset {
            style_parts.push(format!("--rinch-popover-arrow-offset: {}px", offset));
        }
        // z_index (#474): the dropdown is a separate component, so the level
        // travels to it the way this component's width and offsets already do —
        // as an inherited custom property the stylesheet reads.
        if let Some(z) = self.z_index {
            style_parts.push(format!("--rinch-popover-z-index: {}", z));
        }

        let root = rinch_macros::rsx! { div { class: "rinch-popover" } };
        root.set_attribute("class", &self.class_string());
        if !style_parts.is_empty() {
            root.set_attribute("style", &style_parts.join("; "));
        }

        // The initial open state, from `opened_fn` when there is one — the same
        // precedence `Modal`, `Drawer` and `Notification` use.
        let is_opened = match &self.opened_fn {
            Some(f) => f(),
            None => self.opened,
        };

        // opened_fn (#474): keep the `--opened` class in step with the signal,
        // surgically, without re-rendering the component.
        if let Some(opened_fn) = self.opened_fn.clone() {
            let root_c = root.clone();
            let base = self.class_string_closed();
            __scope.create_effect(move || {
                if opened_fn() {
                    root_c.set_attribute("class", &format!("{base} rinch-popover--opened"));
                } else {
                    root_c.set_attribute("class", &base);
                }
            });
        }

        for child in children {
            root.append_child(child);
        }

        // close_on_click_outside (#474): an invisible full-viewport backdrop
        // that fires `onclose` when clicked, one z-level under the dropdown so
        // a click *inside* the popover still reaches its own content. Copied
        // from `DropdownMenu`, whose stylesheet note explains why `fixed` is
        // load-bearing rather than incidental — an `absolute` box is clipped by
        // an `overflow` ancestor, so "outside" would stop at whatever panel the
        // popover lives in.
        if self.close_on_click_outside && self.onclose.is_some() {
            let backdrop = rinch_macros::rsx! { div { class: "rinch-popover__backdrop" } };
            backdrop.set_attribute(
                "style",
                if is_opened {
                    "display: block"
                } else {
                    "display: none"
                },
            );

            if let Some(opened_fn) = self.opened_fn.clone() {
                let backdrop_c = backdrop.clone();
                Effect::new(move || {
                    backdrop_c.set_attribute(
                        "style",
                        if opened_fn() {
                            "display: block"
                        } else {
                            "display: none"
                        },
                    );
                });
            }

            let cb = self.onclose.clone().unwrap();
            let handler_id = __scope.register_handler(move || cb.invoke());
            backdrop.set_attribute("data-rid", &handler_id.0.to_string());

            root.append_child(&backdrop);
        }

        // close_on_escape (#474). See `overlay_dismiss` for why the open check
        // is at dispatch time and not here.
        crate::overlay_dismiss::arm_close_on_escape(
            __scope,
            &root,
            self.close_on_escape,
            self.opened,
            self.opened_fn.as_ref(),
            self.onclose.as_ref(),
        );

        root
    }
}

/// Target element that the popover is anchored to.
#[derive(Debug, Default)]
pub struct PopoverTarget;

impl Component for PopoverTarget {
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

impl Component for PopoverDropdown {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let dropdown = rinch_macros::rsx! { div { class: "rinch-popover__dropdown" } };
        for child in children {
            dropdown.append_child(child);
        }
        dropdown
    }
}
