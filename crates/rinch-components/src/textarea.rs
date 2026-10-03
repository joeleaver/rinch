//! Textarea component.
//!
//! Multi-line text input with label and description support.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback};
use std::rc::Rc;

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// A multi-line text input field.
#[derive(Default)]
pub struct Textarea {
    /// Label displayed above the textarea.
    pub label: String,
    /// Description displayed below the textarea.
    pub description: String,
    /// Error message to display.
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Whether the textarea is disabled.
    pub disabled: bool,
    /// Whether the textarea is required.
    pub required: bool,
    /// Whether the textarea should grow with its content, up to `max_rows`.
    ///
    /// Chrome's own `<textarea>` never does this — its height is always
    /// exactly `rows` lines, content or no content (autosize is a component
    /// convention, not an HTML one; see [`Self::max_rows`]). With this on, the
    /// control's `rows` attribute is recomputed on every value change from the
    /// number of `\n`-separated lines in the (controlled) value, clamped to
    /// `min_rows` (default 2) and `max_rows` (default unbounded) — a `rows`
    /// change re-measures the control exactly as a user-written one would
    /// (#297). This counts *explicit* lines, not wrapped ones: a long
    /// unbroken line that wraps visually does not grow the control, which is
    /// the one way this is not a faithful stand-in for a browser's own
    /// autosize library (those measure `scrollHeight` in pixels; rinch has no
    /// such signal for an attribute-valued control to read). Reactive only
    /// through [`Self::value_fn`] — an uncontrolled `value` sets the initial
    /// `rows` once, at mount, same as it always has.
    pub autosize: bool,
    /// Minimum number of visible text rows.
    ///
    /// Renders as the `rows` attribute, which sizes the control to that many
    /// lines (plus padding and border). Defaults to 2 rows when unset, matching
    /// HTML. A larger CSS `min-height` still wins. With [`Self::autosize`] on,
    /// this is the floor `rows` never shrinks below rather than the fixed
    /// value.
    pub min_rows: Option<u32>,
    /// Maximum number of visible text rows.
    ///
    /// Sets `max-height` to `rows` lines (at the control's own `normal` —
    /// 1.2× font-size — line height, plus its padding and border), which binds
    /// because the control's content height is now a Taffy *measure*
    /// (#297/#1152) rather than a `min-height` rinch wrote — before that fix,
    /// one `min-height` always beat the other in CSS, so nothing here could
    /// ever cap anything (issue #715). The stylesheet's own 60–120px
    /// `min-height` floor still wins when it is taller than this cap, exactly
    /// as an author's `min-height` beats `max-height` on any element — so a
    /// `max_rows` under about 3 lines is unreachable at every size step,
    /// which is a CSS fact about the two declarations, not a bug in this
    /// prop.
    pub max_rows: Option<u32>,
    /// Current value.
    pub value: String,
    /// Reactive value getter for fine-grained updates.
    pub value_fn: Option<ReactiveString>,
    /// Callback when textarea content changes.
    pub oninput: Option<InputCallback>,
    /// Callback when the typed gesture commits (focus leaves the textarea
    /// after a modification) — HTML `change` semantics, receives the final
    /// value. Fires only if the value changed since focus (issue #226).
    pub onchange: Option<InputCallback>,
}

impl std::fmt::Debug for Textarea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Textarea")
            .field("label", &self.label)
            .field("description", &self.description)
            .field("error", &self.error)
            .field("placeholder", &self.placeholder)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("required", &self.required)
            .field("autosize", &self.autosize)
            .field("min_rows", &self.min_rows)
            .field("max_rows", &self.max_rows)
            .field("value", &self.value)
            .field("value_fn", &self.value_fn.as_ref().map(|_| "<reactive>"))
            .field("oninput", &self.oninput.as_ref().map(|_| "<callback>"))
            .field("onchange", &self.onchange.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Textarea {
    /// Generate the CSS class string for this textarea.
    pub fn class_string(&self) -> String {
        let mut classes = vec!["rinch-textarea"];

        // Size
        if !self.size.is_empty() {
            match self.size.as_str() {
                "xs" => classes.push("rinch-textarea--xs"),
                "sm" => classes.push("rinch-textarea--sm"),
                "md" => classes.push("rinch-textarea--md"),
                "lg" => classes.push("rinch-textarea--lg"),
                "xl" => classes.push("rinch-textarea--xl"),
                _ => {}
            }
        }

        // Error state
        if !self.error.is_empty() {
            classes.push("rinch-textarea--error");
        }

        // Autosize
        if self.autosize {
            classes.push("rinch-textarea--autosize");
        }

        classes.join(" ")
    }
}

impl Component for Textarea {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let container = rinch_macros::rsx! { div { class: "rinch-textarea" } };
        container.set_attribute("class", &self.class_string());

        // Label
        if !self.label.is_empty() {
            let label_text = &self.label;
            let label =
                rinch_macros::rsx! { label { class: "rinch-textarea__label", {label_text} } };

            if self.required {
                let required_span =
                    rinch_macros::rsx! { span { class: "rinch-textarea__required", "*" } };
                label.append_child(&required_span);
            }

            container.append_child(&label);
        }

        // Textarea element
        let textarea = rinch_macros::rsx! { textarea { class: "rinch-textarea__input" } };

        if !self.placeholder.is_empty() {
            textarea.set_attribute("placeholder", &self.placeholder);
        }
        if self.disabled {
            textarea.set_attribute("disabled", "");
        }
        if self.required {
            textarea.set_attribute("required", "");
        }

        // `max_rows` caps the control's height at that many lines of its own
        // `normal` (1.2× font-size) line height, plus its padding and border
        // — the same terms the stylesheet's own box uses, so this tracks the
        // control's actual font-size and size step rather than a baked-in
        // pixel value (issue #715).
        if let Some(rows) = self.max_rows {
            textarea.set_style(
                "max-height",
                &format!("calc(1.2em * {rows} + 2 * var(--rinch-spacing-sm) + 2px)"),
            );
        }

        let min_rows = self.min_rows.unwrap_or(2).max(1);
        if self.autosize {
            let max_rows = self.max_rows;
            let rows_for = move |value: &str| -> u32 {
                let lines = (value.lines().count().max(1) as u32).max(min_rows);
                match max_rows {
                    Some(max) => lines.min(max.max(min_rows)),
                    None => lines,
                }
            };
            if let Some(ref value_fn) = self.value_fn {
                let value_fn = value_fn.clone();
                textarea.set_attribute("rows", &rows_for(&value_fn()).to_string());
                let field = textarea.clone();
                __scope.create_effect(move || {
                    field.set_attribute("rows", &rows_for(&value_fn()).to_string());
                });
            } else {
                textarea.set_attribute("rows", &rows_for(&self.value).to_string());
            }
        } else if let Some(rows) = self.min_rows {
            textarea.set_attribute("rows", &rows.to_string());
        }

        // Reactive value binding
        if let Some(ref value_fn) = self.value_fn {
            // Written now and on every change — minus the echo of the user's
            // own keystroke, which the field already shows (#238).
            crate::value_binding::bind_value_fn(__scope, &textarea, value_fn);
        } else if !self.value.is_empty() {
            textarea.set_attribute("value", &self.value);
        }

        // Input handler — always register so the runtime can route text input
        // to this element, even without an explicit oninput callback.
        {
            let callback = self.oninput.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                if let Some(cb) = &callback {
                    cb.invoke(value);
                }
            });
            textarea.set_attribute("data-oninput", &handler_id.to_string());
        }

        // Change handler — the commit boundary (issue #226)
        if let Some(callback) = &self.onchange {
            let callback = callback.clone();
            let handler_id = __scope.register_input_handler(move |value| {
                callback.invoke(value);
            });
            textarea.set_attribute("data-onchange", &handler_id.to_string());
        }

        container.append_child(&textarea);

        // Description
        if !self.description.is_empty() {
            let desc = &self.description;
            let desc_div =
                rinch_macros::rsx! { div { class: "rinch-textarea__description", {desc} } };
            container.append_child(&desc_div);
        }

        // Error
        if !self.error.is_empty() {
            let err = &self.error;
            let err_div = rinch_macros::rsx! { div { class: "rinch-textarea__error", {err} } };
            container.append_child(&err_div);
        }

        container
    }
}
