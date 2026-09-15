//! Checkbox component.
//!
//! Checkbox input with label support.
//!
//! # Fine-Grained Reactivity
//!
//! For reactive checked state without re-rendering, use the `checked_fn` prop:
//!
//! ```ignore
//! let is_checked = Signal::new(false);
//!
//! rsx! {
//!     Checkbox {
//!         checked_fn: Some(Rc::new(move || is_checked.get())),
//!         onchange: move || is_checked.update(|v| *v = !*v),
//!         label: "Accept terms"
//!     }
//! }
//! ```

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Callback, Component};
use std::rc::Rc;

/// Reactive callback type for boolean state.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// The class the checked state adds to a checkbox's root.
const CHECKED_CLASS: &str = "rinch-checkbox--checked";

/// A checkbox input with optional label.
#[derive(Default)]
pub struct Checkbox {
    /// Label displayed next to the checkbox.
    pub label: String,
    /// Description displayed below the label.
    ///
    /// Renders as `.rinch-checkbox__description` inside a `.rinch-checkbox__body`
    /// column beside the box, the layout `Radio` already uses for the same pair.
    /// A checkbox that carries one also takes `--with-description`, which aligns
    /// the box to the first line instead of centring it on a two-line body.
    pub description: String,
    /// Size (xs, sm, md, lg).
    pub size: String,
    /// Whether the checkbox is disabled.
    pub disabled: bool,
    /// Whether the checkbox is checked (static, for initial render or non-reactive use).
    pub checked: bool,
    /// Reactive checked getter - use this for fine-grained updates.
    /// When provided, the checkbox class updates automatically when the signal changes.
    pub checked_fn: Option<ReactiveBool>,
    /// Whether the checkbox is in indeterminate state.
    pub indeterminate: bool,
    /// Callback when checkbox is toggled.
    pub onchange: Option<Callback>,
}

impl std::fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checkbox")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("checked", &self.checked)
            .field(
                "checked_fn",
                &self.checked_fn.as_ref().map(|_| "<reactive>"),
            )
            .field("indeterminate", &self.indeterminate)
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Checkbox {
    /// Generate the base CSS class string for this checkbox (without checked state).
    fn base_class_string(&self) -> String {
        let mut classes = vec!["rinch-checkbox"];

        // Size
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-checkbox--xs"),
                "sm" => classes.push("rinch-checkbox--sm"),
                "md" => classes.push("rinch-checkbox--md"),
                "lg" => classes.push("rinch-checkbox--lg"),
                _ => {}
            }
        }

        // Disabled
        if self.disabled {
            classes.push("rinch-checkbox--disabled");
        }

        // A two-line body wants the box aligned to its first line. It is
        // derived from a non-reactive prop, so it belongs in the *base* class
        // string: nothing has to put it back later. (Until #717 there was a
        // second, sharper reason — the reactive-checked effect rebuilt the whole
        // attribute from this method and dropped anything added outside it.
        // That effect now owns only `--checked`, so a class added after render
        // survives; this one still belongs here.)
        if !self.description.is_empty() {
            classes.push("rinch-checkbox--with-description");
        }

        classes.join(" ")
    }

    /// Generate the CSS class string for this checkbox (static version).
    pub fn class_string(&self) -> String {
        let mut class = self.base_class_string();
        if self.checked {
            class.push(' ');
            class.push_str(CHECKED_CLASS);
        }
        class
    }
}

impl Component for Checkbox {
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

        // Register handler
        if let Some(cb) = &self.onchange {
            let handler_id = __scope.register_handler({
                let cb = cb.clone();
                move || cb.invoke()
            });
            label_node.set_attribute("data-rid", &handler_id.to_string());
        }

        // Input element (hidden, used for accessibility)
        let input = rinch_macros::rsx! { input {} };
        input.set_attribute("type", "checkbox");
        input.set_attribute("class", "rinch-checkbox__input");

        if self.disabled {
            input.set_attribute("disabled", "");
        }
        if is_checked {
            input.set_attribute("checked", "");
        }

        label_node.append_child(&input);

        // Checkbox icon (checkmark or indeterminate line)
        let icon = if self.indeterminate {
            crate::icons::indeterminate_dom(__scope)
        } else {
            crate::icons::checkmark_dom(__scope)
        };

        // Box with icon
        let box_node = rinch_macros::rsx! { span { class: "rinch-checkbox__box" } };

        let icon_span = rinch_macros::rsx! { span { class: "rinch-checkbox__icon" } };
        icon_span.append_child(&icon);

        box_node.append_child(&icon_span);
        label_node.append_child(&box_node);

        // Label and description
        if !self.label.is_empty() || !self.description.is_empty() {
            let body = rinch_macros::rsx! { div { class: "rinch-checkbox__body" } };

            if !self.label.is_empty() {
                let label_text = &self.label;
                let label_span =
                    rinch_macros::rsx! { span { class: "rinch-checkbox__label", {label_text} } };
                body.append_child(&label_span);
            }

            if !self.description.is_empty() {
                let desc = &self.description;
                let desc_span =
                    rinch_macros::rsx! { span { class: "rinch-checkbox__description", {desc} } };
                body.append_child(&desc_span);
            }

            label_node.append_child(&body);
        }

        // If reactive checked_fn is provided, create an Effect that toggles both
        // the label's checked class AND the native <input>'s `checked` state.
        //
        // It adds and removes the one class rather than rewriting the whole
        // `class` attribute from `base_class_string()` (issue #717). A rewrite
        // drops every class put on the node *after* render — above all the
        // universal `class:` prop, which the rsx macro merges onto the handle a
        // component *returned* (issue #647) — so the first toggle would silently
        // undo them. The input is toggled by *presence* (set "" / remove) so it
        // stays correct on both backends: on web `set_attribute` mirrors it onto
        // the live `.checked` property (issue #100) — without this, the
        // accessible/native checked state (and `:checked`, native form
        // submission) goes stale on a programmatic update; on desktop the
        // attribute drives `:checked`.
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
