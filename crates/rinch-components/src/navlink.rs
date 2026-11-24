//! NavLink component.
//!
//! Navigation link with active state and nested support.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive active state without re-rendering, use the `active_fn` prop:
//!
//! ```ignore
//! let current_page = Signal::new("home".to_string());
//!
//! rsx! {
//!     NavLink {
//!         active_fn: Some(Rc::new(move || current_page.get() == "home")),
//!         onclick: move || current_page.set("home".to_string()),
//!         label: Some("Home".to_string())
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, Signal, ValueCallback};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::rc::Rc;

use crate::SectionRenderer;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// The class the active state adds to a navlink's anchor or button.
const ACTIVE_CLASS: &str = "rinch-navlink--active";

/// The class an open navlink's chevron (or rotating right section) carries.
const CHEVRON_OPENED_CLASS: &str = "rinch-navlink__chevron--opened";

/// The class a closed navlink's children container carries.
const CHILDREN_COLLAPSED_CLASS: &str = "rinch-navlink__children--collapsed";

/// NavLink variant style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavLinkVariant {
    #[default]
    Default,
    Light,
    Filled,
    Subtle,
}

impl NavLinkVariant {
    pub fn class_name(&self) -> &'static str {
        match self {
            NavLinkVariant::Default => "rinch-navlink--default",
            NavLinkVariant::Light => "rinch-navlink--light",
            NavLinkVariant::Filled => "rinch-navlink--filled",
            NavLinkVariant::Subtle => "rinch-navlink--subtle",
        }
    }
}

impl std::str::FromStr for NavLinkVariant {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "default" => Ok(NavLinkVariant::Default),
            "light" => Ok(NavLinkVariant::Light),
            "filled" => Ok(NavLinkVariant::Filled),
            "subtle" => Ok(NavLinkVariant::Subtle),
            _ => Err(()),
        }
    }
}

/// Navigation link component with active state.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     NavLink {
///         label: "Dashboard",
///         href: "/dashboard",
///         active: true,
///         left_section: TablerIcon::Settings,
///     }
///     NavLink {
///         label: "Settings",
///         href: "/settings",
///         description: "App configuration",
///     }
/// }
/// ```
#[derive(Default)]
pub struct NavLink {
    /// Link label text.
    pub label: String,
    /// Link description (secondary text).
    pub description: String,
    /// Link href.
    pub href: String,
    /// Whether link is active (static, for initial render or non-reactive use).
    pub active: bool,
    /// Reactive active getter - use this for fine-grained updates.
    /// When provided, the navlink class updates automatically when the signal changes.
    pub active_fn: Option<ReactiveBool>,
    /// Visual variant (default, light, filled, subtle).
    pub variant: String,
    /// Color when active.
    pub color: String,
    /// Left section icon.
    pub left_section: Option<TablerIcon>,
    /// Right section icon. On a navlink with children it takes the chevron's
    /// place, and rotates with it unless `disable_right_section_rotation`.
    pub right_section: Option<TablerIcon>,
    /// Icon drawn as the expand chevron of a navlink with children, in place of
    /// the built-in one. It points right when closed and turns 90° when open.
    pub chevron_icon: Option<TablerIcon>,
    /// Keep the right section (chevron included) still when the navlink opens.
    pub disable_right_section_rotation: bool,
    /// The escape hatch for the label: renders the label slot's content (a
    /// badge, a count) in place of `label`'s text.
    pub label_fn: Option<SectionRenderer>,
    /// Whether link is disabled.
    pub disabled: bool,
    /// Whether this navlink has nested children (makes it expandable).
    pub children_offset: String,
    /// Whether children start visible (expanded). With children, clicking the
    /// navlink toggles them from there; see `opened_fn` for a controlled one.
    pub opened: bool,
    /// Default opened state (uncontrolled). Means the same as `opened`.
    pub default_opened: bool,
    /// Controlled open state: the children follow this getter, and a click
    /// only reports the wanted state through `on_opened_change`.
    pub opened_fn: Option<ReactiveBool>,
    /// Called with the new open state when a click toggles the children.
    pub on_opened_change: Option<ValueCallback<bool>>,
    /// Disable text wrapping.
    pub no_wrap: bool,
    /// Click handler.
    pub onclick: Option<rinch_core::Callback>,
}

impl std::fmt::Debug for NavLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NavLink")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("href", &self.href)
            .field("active", &self.active)
            .field("active_fn", &self.active_fn.as_ref().map(|_| "<reactive>"))
            .field("variant", &self.variant)
            .field("color", &self.color)
            .field("left_section", &self.left_section)
            .field("right_section", &self.right_section)
            .field("chevron_icon", &self.chevron_icon)
            .field(
                "disable_right_section_rotation",
                &self.disable_right_section_rotation,
            )
            .field("label_fn", &self.label_fn.as_ref().map(|_| "<renderer>"))
            .field("disabled", &self.disabled)
            .field("children_offset", &self.children_offset)
            .field("opened", &self.opened)
            .field("default_opened", &self.default_opened)
            .field("opened_fn", &self.opened_fn.as_ref().map(|_| "<reactive>"))
            .field(
                "on_opened_change",
                &self.on_opened_change.as_ref().map(|_| "<callback>"),
            )
            .field("no_wrap", &self.no_wrap)
            .field("onclick", &self.onclick.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl NavLink {
    /// Generate the base CSS class string (without active state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-navlink"];

        if !self.variant.is_empty() {
            if let Ok(variant) = self.variant.parse::<NavLinkVariant>() {
                classes.push(variant.class_name());
            }
        } else {
            classes.push(NavLinkVariant::Default.class_name());
        }

        if self.disabled {
            classes.push("rinch-navlink--disabled");
        }

        if self.no_wrap {
            classes.push("rinch-navlink--no-wrap");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this navlink (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.active {
            class.push_str(" rinch-navlink--active");
        }
        class
    }
}

impl Component for NavLink {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let base_class = self.base_class_string();

        // Determine initial active state
        let initial_active = if let Some(ref active_fn) = self.active_fn {
            active_fn()
        } else {
            self.active
        };

        // Check if there are children for expandable behavior
        let has_children = !children.is_empty();

        // Build style parts
        let mut style_parts = Vec::new();
        if !self.color.is_empty() {
            style_parts.push(crate::color_utils::accent_vars(
                "--rinch-navlink",
                &self.color,
            ));
        }
        if !self.children_offset.is_empty() {
            style_parts.push(format!(
                "--rinch-navlink-children-offset: {}",
                self.children_offset
            ));
        }

        // Create wrapper element
        let wrapper = rinch_macros::rsx! { div { class: "rinch-navlink__wrapper" } };
        if !style_parts.is_empty() {
            wrapper.set_attribute("style", &style_parts.join("; "));
        }

        // Build the inner content
        let inner = rinch_macros::rsx! { span { class: "rinch-navlink__inner" } };

        // Left section icon
        if let Some(icon) = self.left_section {
            let left_span = rinch_macros::rsx! { span { class: "rinch-navlink__left" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            left_span.append_child(&icon_el);
            inner.append_child(&left_span);
        }

        // Body with label and optional description
        let label_span = if let Some(render) = &self.label_fn {
            let span = rinch_macros::rsx! { span { class: "rinch-navlink__label" } };
            let content = render(__scope);
            span.append_child(&content);
            Some(span)
        } else if !self.label.is_empty() {
            Some(
                rinch_macros::rsx! { span { class: "rinch-navlink__label", {self.label.clone()} } },
            )
        } else {
            None
        };
        if !self.description.is_empty() {
            let body = rinch_macros::rsx! { div { class: "rinch-navlink__body" } };

            if let Some(label_span) = &label_span {
                body.append_child(label_span);
            }

            let desc = &self.description;
            let desc_span =
                rinch_macros::rsx! { span { class: "rinch-navlink__description", {desc} } };
            body.append_child(&desc_span);

            inner.append_child(&body);
        } else if let Some(label_span) = &label_span {
            inner.append_child(label_span);
        }

        // The initial open state. With `opened_fn` it is the getter's; without,
        // `opened`/`default_opened` seed the navlink's own state, which a click
        // toggles (issue #2192: it used to choose the first class and nothing
        // more, so a nested NavLink never opened).
        let initial_open = match &self.opened_fn {
            Some(opened_fn) => opened_fn(),
            None => self.opened || self.default_opened,
        };

        // Right section - chevron for expandable items or custom icon. With
        // children, the app's `right_section` takes the chevron's place (as in
        // Mantine) where it used to be dropped silently (#2192).
        let mut rotating: Option<NodeHandle> = None;
        if has_children {
            let right_span = rinch_macros::rsx! { span { class: "rinch-navlink__right" } };

            let glyph = if let Some(icon) = self.right_section {
                render_tabler_icon(__scope, icon, TablerIconStyle::Outline)
            } else if let Some(icon) = self.chevron_icon {
                let svg = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
                svg.add_class("rinch-navlink__chevron");
                svg
            } else {
                let svg = rinch_macros::rsx! {
                    svg {
                        viewBox: "0 0 24 24",
                        fill: "none",
                        stroke: "currentColor"
                    }
                };
                svg.set_attribute("class", "rinch-navlink__chevron");
                svg.set_attribute("stroke-width", "2");

                let polyline = rinch_macros::rsx! { polyline { points: "9 18 15 12 9 6" } };
                svg.append_child(&polyline);
                svg
            };
            if !self.disable_right_section_rotation {
                if initial_open {
                    glyph.add_class(CHEVRON_OPENED_CLASS);
                }
                rotating = Some(glyph.clone());
            }

            right_span.append_child(&glyph);
            inner.append_child(&right_span);
        } else if let Some(icon) = self.right_section {
            let right_span = rinch_macros::rsx! { span { class: "rinch-navlink__right" } };
            let icon_el = render_tabler_icon(__scope, icon, TablerIconStyle::Outline);
            right_span.append_child(&icon_el);
            inner.append_child(&right_span);
        }

        // Build the link or button element
        let link_or_button = if !self.href.is_empty() {
            let href = &self.href;
            let a = rinch_macros::rsx! { a { class: "rinch-navlink" } };
            a.set_attribute("href", href);

            // Set initial class
            let class = if initial_active {
                format!("{} rinch-navlink--active", base_class)
            } else {
                base_class.clone()
            };
            let final_class = if self.disabled {
                format!("{} disabled", class)
            } else {
                class
            };
            a.set_attribute("class", &final_class);

            // Set up reactive effect for active state if active_fn is provided
            // Adding and removing the one class, never rewriting the attribute
            // (issue #717). `disabled` is not reactive and is already in the
            // class string above, so the effect has exactly one class to own —
            // and a rewrite would drop whatever a parent patched onto the
            // anchor, which is how every #474 category C prop travels.
            if let Some(ref active_fn) = self.active_fn {
                let active_fn = active_fn.clone();
                let a_handle = a.clone();
                __scope.create_effect(move || {
                    if active_fn() {
                        a_handle.add_class(ACTIVE_CLASS);
                    } else {
                        a_handle.remove_class(ACTIVE_CLASS);
                    }
                });
            }

            // A disabled anchor is not clickable from the keyboard either
            // (#2192): `pointer-events: none` stopped only the mouse, so no
            // handler is registered, as for a disabled `<button>`.
            if self.disabled {
                a.set_attribute("aria-disabled", "true");
                a.set_attribute("tabindex", "-1");
            }

            a.append_child(&inner);
            a
        } else {
            let btn = rinch_macros::rsx! { button { class: "rinch-navlink" } };

            // Set initial class
            let class = if initial_active {
                format!("{} rinch-navlink--active", base_class)
            } else {
                base_class.clone()
            };
            btn.set_attribute("class", &class);

            // Set up reactive effect for active state if active_fn is provided
            // Adding and removing the one class, never rewriting the
            // attribute — see the note on the anchor branch above (issue #717).
            if let Some(ref active_fn) = self.active_fn {
                let active_fn = active_fn.clone();
                let btn_handle = btn.clone();
                __scope.create_effect(move || {
                    if active_fn() {
                        btn_handle.add_class(ACTIVE_CLASS);
                    } else {
                        btn_handle.remove_class(ACTIVE_CLASS);
                    }
                });
            }

            if self.disabled {
                btn.set_attribute("disabled", "");
            }

            btn.append_child(&inner);
            btn
        };

        wrapper.append_child(&link_or_button);

        // Render nested children
        let mut toggle: Option<Rc<dyn Fn()>> = None;
        if has_children {
            let children_container =
                rinch_macros::rsx! { div { class: "rinch-navlink__children" } };
            if !initial_open {
                children_container.add_class(CHILDREN_COLLAPSED_CLASS);
                children_container.set_style("display", "none");
            }
            link_or_button
                .set_attribute("aria-expanded", if initial_open { "true" } else { "false" });

            for child in children {
                children_container.append_child(child);
            }

            wrapper.append_child(&children_container);

            // The open state: the navlink's own signal, or the app's getter.
            let own = Signal::new(initial_open);
            let opened_fn = self.opened_fn.clone();
            let is_open: Rc<dyn Fn() -> bool> = match opened_fn {
                Some(f) => f,
                None => Rc::new(move || own.get()),
            };

            {
                let is_open = is_open.clone();
                let container = children_container.clone();
                let control = link_or_button.clone();
                let rotating = rotating.clone();
                __scope.create_effect(move || {
                    let open = is_open();
                    // Closed children are not displayed, as a closed
                    // `<details>` and Accordion's panel are, so Tab skips them.
                    if open {
                        container.remove_class(CHILDREN_COLLAPSED_CLASS);
                        container.set_style("display", "");
                    } else {
                        container.add_class(CHILDREN_COLLAPSED_CLASS);
                        container.set_style("display", "none");
                    }
                    if let Some(glyph) = &rotating {
                        if open {
                            glyph.add_class(CHEVRON_OPENED_CLASS);
                        } else {
                            glyph.remove_class(CHEVRON_OPENED_CLASS);
                        }
                    }
                    control.set_attribute("aria-expanded", if open { "true" } else { "false" });
                });
            }

            let controlled = self.opened_fn.is_some();
            let on_change = self.on_opened_change.clone();
            toggle = Some(Rc::new(move || {
                let next = !is_open();
                if !controlled {
                    own.set(next);
                }
                if let Some(cb) = &on_change {
                    cb.invoke(next);
                }
            }));
        }

        // Click handler: the app's `onclick`, then the toggle. None at all on a
        // disabled navlink (#2192).
        if !self.disabled && (self.onclick.is_some() || toggle.is_some()) {
            let onclick = self.onclick.clone();
            let handler_id = __scope.register_handler(move || {
                if let Some(cb) = &onclick {
                    cb.invoke();
                }
                if let Some(toggle) = &toggle {
                    toggle();
                }
            });
            link_or_button.set_attribute("data-rid", &handler_id.0.to_string());
        }

        wrapper
    }
}
