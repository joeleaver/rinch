//! ColorInput component.
//!
//! A text input with an inline color preview swatch and a dropdown ColorPicker.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{Component, InputCallback, Signal};

use crate::color_picker::ColorPicker;
use crate::color_swatch::ColorSwatch;
use crate::color_utils::{ColorFormat, denotes_same, format_color, parse_color};

/// Reactive callback type for string state.
pub type ReactiveString = Rc<dyn Fn() -> String>;

/// A text input with color preview swatch and dropdown ColorPicker.
#[derive(Default)]
pub struct ColorInput {
    /// Input label.
    pub label: String,
    /// Description text below the input.
    pub description: String,
    /// Error message (shows error styling when present).
    pub error: String,
    /// Placeholder text.
    pub placeholder: String,
    /// Input size (xs, sm, md, lg, xl).
    pub size: String,
    /// Border radius.
    pub radius: String,
    /// Whether the input is disabled.
    pub disabled: bool,
    /// Current color value.
    pub value: String,
    /// Reactive value binding.
    pub value_fn: Option<ReactiveString>,
    /// Fires the formatted color string when a color change commits: a pick in
    /// the dropdown picker, or a typed edit at its commit boundary (focus
    /// leaves the field, or Enter — issue #226). Typing previews live in the
    /// swatch but reports here only on commit.
    pub onchange: Option<InputCallback>,
    /// Output format: hex, hexa, rgb, rgba, hsl, hsla.
    pub format: String,
    /// Show alpha slider in picker. Off unless set (`#[derive(Default)]`: false).
    pub alpha: bool,
    /// Preset swatch colors for the picker.
    pub swatches: Vec<String>,
    /// Swatches per row in the picker.
    pub swatches_per_row: Option<usize>,
    /// Close dropdown when clicking outside.
    pub close_on_click_outside: bool,
    /// Disallow typing into the input (only use picker).
    pub disallow_input: bool,
}

impl std::fmt::Debug for ColorInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColorInput")
            .field("label", &self.label)
            .field("value", &self.value)
            .field("format", &self.format)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl Component for ColorInput {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let color_format = ColorFormat::parse(&self.format).unwrap_or(ColorFormat::Hex);
        let show_alpha = self.alpha;
        let disallow_input = self.disallow_input;

        // Internal state
        let opened = Signal::new(false);
        let current_value = Signal::new(if self.value.is_empty() {
            "#000000".to_string()
        } else {
            self.value.clone()
        });

        // Root container
        let mut root_class = String::from("rinch-color-input");
        if !self.error.is_empty() {
            root_class.push_str(" rinch-color-input--error");
        }
        if self.disabled {
            root_class.push_str(" rinch-color-input--disabled");
        }

        let root = rinch_macros::rsx! { div {} };
        root.set_attribute("class", &root_class);

        // Label
        if !self.label.is_empty() {
            let label_el = rinch_macros::rsx! { label { class: "rinch-color-input__label", {self.label.clone()} } };
            root.append_child(&label_el);
        }

        // Wrapper (position: relative for dropdown)
        let wrapper = rinch_macros::rsx! { div { class: "rinch-color-input__wrapper" } };

        // Input group (swatch + text input)
        let input_group = rinch_macros::rsx! { div { class: "rinch-color-input__input-group" } };

        // Toggle open on click
        {
            let handler_id = __scope.register_handler(move || {
                opened.update(|v| *v = !*v);
            });
            input_group.set_attribute("data-rid", &handler_id.to_string());
        }

        // Preview swatch
        let initial_color = if self.value.is_empty() {
            "#000000"
        } else {
            &self.value
        };
        let preview = ColorSwatch {
            color: initial_color.to_string(),
            size: "22px".into(),
            radius: "sm".into(),
            ..Default::default()
        };
        let preview_node = preview.render(__scope, &[]);
        preview_node.set_attribute(
            "class",
            "rinch-color-swatch rinch-color-input__swatch-preview",
        );
        input_group.append_child(&preview_node);

        // Text input
        let text_input = rinch_macros::rsx! { input { class: "rinch-color-input__input" } };
        text_input.set_attribute("value", initial_color);
        if !self.placeholder.is_empty() {
            text_input.set_attribute("placeholder", &self.placeholder);
        }
        if disallow_input {
            text_input.set_attribute("readonly", "");
        }

        // The field's live text, as this component last heard it — same role
        // as ColorPicker's record (GH #231). Every `oninput` records here,
        // parseable or not: a parse-gated record would survive an edit it no
        // longer describes and later veto a legitimate rewrite. The display
        // effect prefers this record over the `value` attribute, which on web
        // holds only what was last written programmatically. Cleared whenever
        // the effect rewrites the field.
        let typed: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        // The last color the app actually holds: the mount value, then every
        // commit point — a reported typed commit, a dropdown pick, an external
        // `value_fn` write. `current_value` cannot play this role: parseable
        // keystrokes preview through it without reporting, so it can hold a
        // color the app has never heard of.
        let last_committed: Rc<RefCell<String>> = Rc::new(RefCell::new(initial_color.to_string()));

        // Handle text input changes. Per-keystroke this is internal only —
        // record the live text, parse, and preview through `current_value`
        // (swatch, dropdown picker). `self.onchange` fires from the commit
        // handler below, not here: the prop promises a committed color, not a
        // live preview per parseable keystroke (issue #226).
        if !disallow_input {
            let typed_in = typed.clone();
            let handler_id = __scope.register_input_handler(move |value: String| {
                let parsed = parse_color(&value);
                *typed_in.borrow_mut() = Some(value);
                if let Some(parsed) = parsed {
                    current_value.set(format_color(parsed, color_format));
                }
            });
            text_input.set_attribute("data-oninput", &handler_id.to_string());

            // The commit boundary (issue #226): the typed gesture ended, so the
            // author's mid-typing claim to the field (GH #231) lapses. A
            // parseable commit normalizes the field to the canonical form
            // ("336" → "#333366") and reports through `onchange` — but only
            // when the color differs from the last one the app holds
            // (`last_committed`): re-spelling the same color in another
            // notation is not a color change. The report is deliberately the
            // LAST step, with no field write after it, so a controlled
            // handler's synchronous rewrite (signal → value_fn effect →
            // display effect) lands after ours and wins. An unparseable
            // commit reverts the field — and any leaked preview in
            // `current_value`, swatch included — to the last committed color,
            // never to the live preview: the preview was never reported, so
            // keeping it would durably display a color the app never
            // received. Either rewrite clears the typed record, like any
            // effect rewrite.
            let onchange = self.onchange.clone();
            let typed_commit = typed.clone();
            let text_input_commit = text_input.clone();
            let last_committed_commit = last_committed.clone();
            let handler_id =
                __scope.register_input_handler(move |value: String| match parse_color(&value) {
                    Some(parsed) => {
                        let formatted = format_color(parsed, color_format);
                        let changed = *last_committed_commit.borrow() != formatted;
                        *typed_commit.borrow_mut() = None;
                        text_input_commit.set_attribute("value", &formatted);
                        current_value.set_if_changed(formatted.clone());
                        *last_committed_commit.borrow_mut() = formatted.clone();
                        if changed {
                            if let Some(ref cb) = onchange {
                                cb.invoke(formatted);
                            }
                        }
                    }
                    None => {
                        let committed = last_committed_commit.borrow().clone();
                        *typed_commit.borrow_mut() = None;
                        current_value.set_if_changed(committed.clone());
                        text_input_commit.set_attribute("value", &committed);
                    }
                });
            text_input.set_attribute("data-onchange", &handler_id.to_string());
        }

        input_group.append_child(&text_input);
        wrapper.append_child(&input_group);

        // Dropdown containing the ColorPicker
        let dropdown = rinch_macros::rsx! { div { class: "rinch-color-input__dropdown" } };

        let picker = ColorPicker {
            format: self.format.clone(),
            value: self.value.clone(),
            alpha: show_alpha,
            swatches: self.swatches.clone(),
            swatches_per_row: self.swatches_per_row,
            with_input: false, // The outer input serves as the text input
            onchange: Some(InputCallback::new({
                let onchange = self.onchange.clone();
                let last_committed = last_committed.clone();
                move |value: String| {
                    // A pick is a commit in its own right: record it as the
                    // last color the app holds before reporting it.
                    *last_committed.borrow_mut() = value.clone();
                    current_value.set(value.clone());
                    if let Some(ref cb) = onchange {
                        cb.invoke(value);
                    }
                }
            })),
            ..Default::default()
        };
        let picker_node = picker.render(__scope, &[]);
        dropdown.append_child(&picker_node);

        wrapper.append_child(&dropdown);
        root.append_child(&wrapper);

        // Reactive: toggle the opened class. Deliberately its own effect: the
        // field-display effect below must NOT re-run on dropdown toggles —
        // clicking the input group (the text field included) toggles
        // `opened`, and a coupled effect would rewrite the field while the
        // author's mid-typing text is still unparseable ("#33"), destroying
        // it without any colour change (GH #231).
        {
            let root_clone = root.clone();
            let base_class = root_class.clone();
            __scope.create_effect(move || {
                let mut cls = base_class.clone();
                if opened.get() {
                    cls.push_str(" rinch-color-input--opened");
                }
                root_clone.set_attribute("class", &cls);
            });
        }

        // Reactive: update preview/input from current_value
        {
            let preview_node = preview_node.clone();
            let text_input = text_input.clone();
            __scope.create_effect(move || {
                let val = current_value.get();

                // Update preview swatch
                let children = preview_node.children();
                if let Some(overlay_child) = children.first() {
                    overlay_child.set_attribute(
                        "style",
                        &format!(
                            "background-color: {}; border-radius: var(--rinch-radius-sm)",
                            val
                        ),
                    );
                }

                // Update text input — unless its text already denotes this
                // colour: a parseable prefix mid-typing lands here through the
                // input handler, and writing the normalized form back would
                // replace the text under the author's caret (GH #231). The
                // field is rewritten only when the colour moves away from it —
                // the dropdown picker, an external `value_fn` change. "The
                // field's text" is the `typed` record when one exists (the
                // live text on both backends), else the `value` attribute
                // (which on web is honest only until the first keystroke).
                let field_text = typed
                    .borrow()
                    .clone()
                    .or_else(|| text_input.get_attribute("value"));
                let field_agrees =
                    field_text.is_some_and(|text| denotes_same(&text, &val, color_format));
                if !field_agrees {
                    *typed.borrow_mut() = None;
                    text_input.set_attribute("value", &val);
                }
            });
        }

        // value_fn binding.
        //
        // Intentionally does NOT read `current_value` — we only mirror from
        // external to internal, never compare. Reading it would (a) subscribe
        // this effect to its own write and cause a re-entrant `run_effect`
        // panic (RefCell double borrow at effect.rs:144), and (b) silently
        // undo user edits: between the picker's `current_value.set(new)` and
        // the caller's signal catching up, the effect would see stale
        // `external(old) != current(new)` and write the old value back.
        // `set_if_changed` avoids notifying the display effect when the
        // value is already up-to-date (e.g. after a user edit has already
        // propagated current_value ← picker ← cb ← external).
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            let last_committed = last_committed.clone();
            __scope.create_effect(move || {
                let external = value_fn();
                // An external write is by definition app-held.
                *last_committed.borrow_mut() = external.clone();
                current_value.set_if_changed(external);
            });
        }

        // Description
        if !self.description.is_empty() {
            let desc = rinch_macros::rsx! { div { class: "rinch-color-input__description", {self.description.clone()} } };
            root.append_child(&desc);
        }

        // Error
        if !self.error.is_empty() {
            let err = rinch_macros::rsx! { div { class: "rinch-color-input__error", {self.error.clone()} } };
            root.append_child(&err);
        }

        root
    }
}
