//! Radio component.
//!
//! A radio button input for single selection from a group.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive checked state without re-rendering, use the `checked_fn` prop:
//!
//! ```ignore
//! let selected = Signal::new("option1".to_string());
//!
//! rsx! {
//!     Radio {
//!         checked_fn: Some(Rc::new(move || selected.get() == "option1")),
//!         onchange: move || selected.set("option1".to_string()),
//!         label: "Option 1"
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use std::rc::Rc;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Radio size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RadioSize {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
    Xl,
}

impl RadioSize {
    /// Get the CSS class name for this size.
    pub fn class_name(&self) -> &'static str {
        match self {
            RadioSize::Xs => "rinch-radio--xs",
            RadioSize::Sm => "rinch-radio--sm",
            RadioSize::Md => "rinch-radio--md",
            RadioSize::Lg => "rinch-radio--lg",
            RadioSize::Xl => "rinch-radio--xl",
        }
    }
}

impl std::str::FromStr for RadioSize {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xs" => Ok(RadioSize::Xs),
            "sm" => Ok(RadioSize::Sm),
            "md" => Ok(RadioSize::Md),
            "lg" => Ok(RadioSize::Lg),
            "xl" => Ok(RadioSize::Xl),
            _ => Err(()),
        }
    }
}

/// A radio button input component.
///
/// Used for selecting a single option from a group.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     Radio { name: "color", value: "red", label: "Red" }
///     Radio { name: "color", value: "blue", label: "Blue", checked: true }
///     Radio { name: "color", value: "green", label: "Green" }
/// }
/// ```
#[derive(Default)]
pub struct Radio {
    /// Input name (for grouping).
    pub name: String,
    /// Input value.
    pub value: String,
    /// Label text.
    pub label: String,
    /// Description text.
    pub description: String,
    /// Whether the radio is checked (static, for initial render or non-reactive use).
    pub checked: bool,
    /// Reactive checked getter - use this for fine-grained updates.
    /// When provided, the radio class updates automatically when the signal changes.
    pub checked_fn: Option<ReactiveBool>,
    /// Whether the radio is disabled.
    pub disabled: bool,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Color override.
    pub color: String,
    /// Error state.
    pub error: bool,
    /// Callback when radio is selected.
    pub onchange: Option<Callback>,
}

impl std::fmt::Debug for Radio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Radio")
            .field("name", &self.name)
            .field("value", &self.value)
            .field("label", &self.label)
            .field("description", &self.description)
            .field("checked", &self.checked)
            .field(
                "checked_fn",
                &self.checked_fn.as_ref().map(|_| "<reactive>"),
            )
            .field("disabled", &self.disabled)
            .field("size", &self.size)
            .field("color", &self.color)
            .field("error", &self.error)
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

/// The class the checked state adds to a radio's root.
const CHECKED_CLASS: &str = "rinch-radio--checked";

impl Radio {
    /// Generate the base CSS class string for this radio (without checked state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-radio"];

        // Size class
        let size: RadioSize = if self.size.is_empty() {
            RadioSize::default()
        } else {
            self.size.parse().unwrap_or_default()
        };
        classes.push(size.class_name());

        if self.disabled {
            classes.push("rinch-radio--disabled");
        }
        if self.error {
            classes.push("rinch-radio--error");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this radio (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.checked {
            class.push(' ');
            class.push_str(CHECKED_CLASS);
        }
        class
    }
}

impl Component for Radio {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let base_class = self.base_class_string();

        // Determine if we have a reactive checked state
        let is_checked = if let Some(ref checked_fn) = self.checked_fn {
            checked_fn()
        } else {
            self.checked
        };

        // Build the class string
        let class = if is_checked {
            format!("{base_class} {CHECKED_CLASS}")
        } else {
            base_class
        };

        // Create label container
        let label_node = rinch_macros::rsx! { label {} };
        label_node.set_attribute("class", &class);

        // A size of this radio's own. The class alone cannot say so — the
        // default is pushed unconditionally, so `rinch-radio--md` is what both
        // `size: "md"` and an unset `size` produce. `RadioGroup` needs the
        // difference to know whether its own `size` is free to apply, so the
        // radio records the ask rather than only its outcome.
        if !self.size.is_empty() {
            label_node.set_attribute("data-size", &self.size);
        }

        // Color style
        if !self.color.is_empty() {
            let c = &self.color;
            let style = if c.starts_with('#') || c.starts_with("rgb") || c.starts_with("hsl") {
                format!("--rinch-radio-color: {}", c)
            } else {
                format!("--rinch-radio-color: var(--rinch-color-{}-6)", c)
            };
            label_node.set_attribute("style", &style);
        }

        // Register handler
        if let Some(cb) = &self.onchange {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            label_node.set_attribute("data-rid", &handler_id.to_string());
        }

        // Hidden native radio input
        // Always generate a name attribute - the DOM crashes without it
        let name = if self.name.is_empty() {
            "radio-group"
        } else {
            &self.name
        };
        let input = rinch_macros::rsx! { input { class: "rinch-radio__input" } };
        input.set_attribute("type", "radio");
        input.set_attribute("name", name);

        if !self.value.is_empty() {
            input.set_attribute("value", &self.value);
        }
        if is_checked {
            input.set_attribute("checked", "");
        }
        if self.disabled {
            input.set_attribute("disabled", "");
        }

        label_node.append_child(&input);

        // Custom radio indicator
        let indicator = rinch_macros::rsx! { span { class: "rinch-radio__indicator" } };

        let dot = rinch_macros::rsx! { span { class: "rinch-radio__dot" } };
        indicator.append_child(&dot);

        label_node.append_child(&indicator);

        // Label and description
        if !self.label.is_empty() || !self.description.is_empty() {
            let body = rinch_macros::rsx! { div { class: "rinch-radio__body" } };

            if !self.label.is_empty() {
                let label_text = &self.label;
                let label_span =
                    rinch_macros::rsx! { span { class: "rinch-radio__label", {label_text} } };
                body.append_child(&label_span);
            }

            if !self.description.is_empty() {
                let desc = &self.description;
                let desc_span =
                    rinch_macros::rsx! { span { class: "rinch-radio__description", {desc} } };
                body.append_child(&desc_span);
            }

            label_node.append_child(&body);
        }

        // If reactive checked_fn is provided, create an Effect that toggles both
        // the label's checked class AND the native <input>'s `checked` state.
        //
        // It adds and removes the one class rather than rewriting the whole
        // `class` attribute from `base_class_string()`. A rewrite drops every
        // class put on the node *after* render — `RadioGroup`'s size class, and
        // the universal `class:` prop the rsx macro merges onto a component's
        // root — so the first toggle would silently undo them. The
        // input is toggled by *presence* (set "" / remove) so it stays correct on
        // both backends: on web `set_attribute` mirrors it onto the live `.checked`
        // property (issue #100) — without this, the accessible/native checked
        // state goes stale on a programmatic update (and, for a radio group,
        // unchecking must clear the property so native state stays exclusive); on
        // desktop the attribute drives `:checked`.
        if let Some(ref checked_fn) = self.checked_fn {
            let checked_fn = checked_fn.clone();
            let label_clone = label_node.clone();
            let input_clone = input.clone();

            __scope.create_effect(move || {
                let is_checked = checked_fn();
                if is_checked {
                    label_clone.add_class(CHECKED_CLASS);
                    input_clone.set_attribute("checked", "");
                } else {
                    label_clone.remove_class(CHECKED_CLASS);
                    input_clone.remove_attribute("checked");
                }
            });
        }

        label_node
    }
}

/// A radio group container component.
///
/// Wraps multiple Radio components for styling and accessibility.
///
/// # Example
///
/// ```ignore
/// rsx! {
///     RadioGroup { label: "Choose a color",
///         Radio { name: "color", value: "red", label: "Red" }
///         Radio { name: "color", value: "blue", label: "Blue" }
///     }
/// }
/// ```
#[derive(Debug, Default)]
pub struct RadioGroup {
    /// Group label.
    pub label: String,
    /// Description text.
    pub description: String,
    /// Error message.
    pub error: String,
    /// Default size for the radios in this group.
    ///
    /// Applied to every [`Radio`] in this group that sets no `size` of its own —
    /// the radio's size wins. A radio records its own ask as `data-size`, which
    /// is what tells an unset `size` from an explicit `"md"`. A radio that
    /// arrives **later**, by a `for` reconcile, a `show_dom` branch or a
    /// hand-rolled `append_child`, is resized as it lands (issue #716).
    pub size: String,
    /// Orientation (horizontal or vertical).
    pub orientation: String,
}

impl RadioGroup {
    /// Generate the CSS class string for this radio group.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-radio-group"];

        if self.orientation == "horizontal" {
            classes.push("rinch-radio-group--horizontal");
        }

        if !self.error.is_empty() {
            classes.push("rinch-radio-group--error");
        }

        classes.join(" ")
    }
}

impl Component for RadioGroup {
    fn render(&self, __scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
        let class = self.class_string();

        // Create container
        let container = rinch_macros::rsx! { div {} };
        container.set_attribute("class", &class);
        container.set_attribute("role", "radiogroup");

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let label_div =
                rinch_macros::rsx! { div { class: "rinch-radio-group__label", {label_text} } };
            container.append_child(&label_div);
        }

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-radio-group__description", {desc} } };
            container.append_child(&desc_div);
        }

        // Radios wrapper
        let radios = rinch_macros::rsx! { div { class: "rinch-radio-group__radios" } };

        // Append children
        for child in children {
            radios.append_child(child);
        }

        if !self.size.is_empty()
            && let Ok(size) = self.size.parse::<RadioSize>()
        {
            give_radios_a_default_size(&radios, size);
            // And again for every radio that lands later (issue #716). Not
            // for one that leaves: a group's size is positionless, so nothing
            // is re-derived by a removal (issue #745).
            crate::late_children::adopt_late_children(
                __scope,
                &radios,
                &["rinch-radio"],
                move |inserted, _scope| give_radios_a_default_size(inserted, size),
            );
        }

        container.append_child(&radios);

        // Error message
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-radio-group__error", {err} } };
            container.append_child(&err_div);
        }

        container
    }
}

/// Give every radio in `node`'s subtree that asked for no size of its own the
/// group's `size`.
///
/// A radio always carries exactly one size class, so applying the group's means
/// removing whichever one the radio defaulted to. The other modifier classes —
/// `--checked`, `--disabled`, `--error` — are left alone.
///
/// Called on the group's radios wrapper at render, and again on every subtree
/// that lands in it afterwards (issue #716). Idempotent: a radio already at the
/// group's size is rewritten to the same class.
fn give_radios_a_default_size(node: &NodeHandle, size: RadioSize) {
    let classes = node.get_attribute("class").unwrap_or_default();
    if classes.split_whitespace().any(|c| c == "rinch-radio") {
        if node.get_attribute("data-size").is_none() {
            for step in [
                RadioSize::Xs,
                RadioSize::Sm,
                RadioSize::Md,
                RadioSize::Lg,
                RadioSize::Xl,
            ] {
                node.remove_class(step.class_name());
            }
            node.add_class(size.class_name());
        }
        return;
    }

    for child in node.children() {
        give_radios_a_default_size(&child, size);
    }
}
