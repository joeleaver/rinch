//! Select component.
//!
//! Custom dropdown select with label, description, error, sizes, and reactive value binding.
//! Renders a styled trigger button with a dropdown overlay — no native `<select>` element.
//!
//! ## Keyboard (issues #251, #434)
//!
//! The trigger is a WAI-ARIA *select-only combobox*: it takes the focus
//! (`tabindex="0"`) and **keeps** it while the list is open, and the list is
//! driven through `aria-activedescendant` rather than by moving focus into it.
//! Enter/Space on the closed trigger opens the list (its `data-rid`, the same
//! toggle a click runs). While the list is open:
//!
//! | Key | Does |
//! |---|---|
//! | ArrowDown / ArrowUp | move the highlight one option, wrapping at the ends |
//! | Home / End | highlight the first / last option |
//! | Enter | commit the highlighted option and close |
//! | Space | the same — unless a type-ahead prefix is live, which it extends ("new y") |
//! | Escape | close without committing |
//! | Tab | close without committing, and let focus move on |
//! | a printable character | type-ahead: highlight the first option whose label starts with what was typed |
//!
//! Opening highlights the selected option (or the first, with none selected).
//! The keys reach the component through [`rinch_core::push_key_handler`],
//! pushed when the list opens and released when it closes, and offered a key
//! only while the focus is inside the `Select` — so a closed `Select`
//! consumes nothing, a field focused while the list is open keeps its own
//! keys, and focus leaving the `Select` closes the list; Escape is a dismiss-stack entry pushed at open
//! (the #465 policy), so a `Select` open inside a `Modal` closes before the
//! modal does. Both are dispatched from `dispatch_keyboard_event`, which the
//! desktop runtime and rinch-web both call, so the behaviour is one code path
//! on both backends. A key with Ctrl, Alt or Meta held is left to the app.
//!
//! The option list carries `data-nofocus`, so a pointer pick leaves the
//! keyboard on the trigger as well.

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::{
    Callback, Component, InputCallback, KeyEventData, Signal, events::get_click_context,
};
use rinch_tabler_icons::{TablerIcon, TablerIconStyle, render_tabler_icon};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use web_time::{Duration, Instant};

pub type ReactiveString = Rc<dyn Fn() -> String>;

/// The class an unset value adds to the trigger's display span.
const PLACEHOLDER_CLASS: &str = "rinch-select__display--placeholder";
/// The class the selected option carries in the dropdown list.
const SELECTED_CLASS: &str = "rinch-select__option--selected";
/// The class the keyboard-highlighted option carries while the list is open.
const HIGHLIGHTED_CLASS: &str = "rinch-select__option--highlighted";
/// Type-ahead keystrokes further apart than this start a new prefix — the
/// native `<select>` popup's figure (`TYPEAHEAD_RESET_MS` in the runtime).
const TYPEAHEAD_RESET: Duration = Duration::from_millis(900);

/// Mints the per-instance prefix for the listbox's and options' `id`s, which
/// `aria-controls` and `aria-activedescendant` refer to document-wide.
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// Where the highlight goes for `key` on a list of `n` options whose highlight
/// is at `from`, or `None` if `key` is not a movement key. ArrowDown/ArrowUp
/// wrap at the ends, as Mantine's `Select` does (`loop`), Home/End jump.
fn step_highlight(key: &str, from: Option<usize>, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match key {
        "ArrowDown" => Some(from.map_or(0, |i| (i + 1) % n)),
        "ArrowUp" => Some(from.map_or(n - 1, |i| (i + n - 1) % n)),
        "Home" => Some(0),
        "End" => Some(n - 1),
        _ => None,
    }
}

/// Type-ahead: the first option at or after `from` whose label starts with
/// `query` (case-insensitive), searching round the end. A query of one letter
/// repeated (`"bb"`) steps to the next option starting with that letter, as the
/// native `<select>` popup does.
fn typeahead_match(labels: &[String], query: &str, from: usize) -> Option<usize> {
    let n = labels.len();
    if n == 0 || query.is_empty() {
        return None;
    }
    let first = query.chars().next().unwrap();
    let single_repeat = query.chars().count() >= 2 && query.chars().all(|c| c == first);
    let (needle, start) = if single_repeat {
        (first.to_string(), (from + 1) % n)
    } else {
        (query.to_string(), from % n)
    };
    (0..n)
        .map(|k| (start + k) % n)
        .find(|&i| labels[i].to_lowercase().starts_with(&needle))
}

/// An option in a Select dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption {
    /// The value submitted when this option is selected.
    pub value: String,
    /// The display label. If empty, `value` is used.
    pub label: String,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
        }
    }

    /// Display text: label if non-empty, otherwise value.
    pub fn display(&self) -> &str {
        if self.label.is_empty() {
            &self.value
        } else {
            &self.label
        }
    }
}

/// A dropdown select input.
#[derive(Default)]
pub struct Select {
    /// Label displayed above the select.
    pub label: String,
    /// Description displayed below the select.
    pub description: String,
    /// Error message to display.
    pub error: String,
    /// Placeholder text when no value is selected.
    pub placeholder: String,
    /// Size (xs, sm, md, lg, xl).
    pub size: String,
    /// Whether the select is disabled.
    pub disabled: bool,
    /// Whether the select is required.
    pub required: bool,
    /// Currently selected value.
    pub value: String,
    /// Reactive value getter for fine-grained updates.
    pub value_fn: Option<ReactiveString>,
    /// Callback when selection changes (receives selected value).
    pub onchange: Option<InputCallback>,
    /// The list of selectable options.
    pub data: Vec<SelectOption>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("label", &self.label)
            .field("placeholder", &self.placeholder)
            .field("size", &self.size)
            .field("disabled", &self.disabled)
            .field("value", &self.value)
            .field("data", &self.data)
            .finish()
    }
}

impl Component for Select {
    fn render(&self, __scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        let opened = Signal::new(false);
        // `flip_above` flips the dropdown to open upward when there isn't enough
        // room below the trigger. Decided at click time using the cursor's y vs
        // viewport height (the actual dropdown height isn't known until after
        // layout). The estimate matches the dropdown's CSS `max-height: 200px`
        // plus margin/border slop. Conservative — biases toward flipping rather
        // than clipping.
        let flip_above = Signal::new(false);
        const DROPDOWN_RESERVE_PX: f32 = 220.0;

        // Determine the current value
        let current_value = if let Some(ref vf) = self.value_fn {
            vf()
        } else {
            self.value.clone()
        };
        let selected_value = Signal::new(current_value);
        // The keyboard highlight (#434): which option Enter would commit.
        // Distinct from the selection — moving it commits nothing — and `None`
        // whenever the list is closed.
        let highlighted: Signal<Option<usize>> = Signal::new(None);
        let instance = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
        let listbox_id = format!("rinch-select-{instance}-listbox");
        let option_id = move |i: usize| format!("rinch-select-{instance}-option-{i}");

        // If value_fn is provided, sync from it.
        // `set_if_changed` no-ops on the initial run (selected_value was just
        // seeded from the same value_fn() above), which avoids re-entering
        // `flush_effects` when mounted during a parent flush. See GH #24.
        if let Some(ref value_fn) = self.value_fn {
            let value_fn = value_fn.clone();
            __scope.create_effect(move || {
                selected_value.set_if_changed(value_fn());
            });
        }

        let options: Rc<Vec<SelectOption>> = Rc::new(self.data.clone());

        // Opening puts the highlight on the selection (or the first option);
        // closing clears it, whatever closed the list.
        let opts_for_highlight = options.clone();
        __scope.create_effect(move || {
            let open = opened.get();
            let next = if open {
                let val = rinch_core::untracked(|| selected_value.get());
                opts_for_highlight.iter().position(|o| o.value == val).or(
                    if opts_for_highlight.is_empty() {
                        None
                    } else {
                        Some(0)
                    },
                )
            } else {
                None
            };
            highlighted.set_if_changed(next);
        });

        // Classes
        let size_class = match self.size.as_str() {
            "xs" => " rinch-select--xs",
            "sm" => " rinch-select--sm",
            "lg" => " rinch-select--lg",
            "xl" => " rinch-select--xl",
            _ => "",
        };
        let error_class = if !self.error.is_empty() {
            " rinch-select--error"
        } else {
            ""
        };
        let disabled_class = if self.disabled {
            " rinch-select--disabled"
        } else {
            ""
        };
        let wrapper_class =
            format!("rinch-select{size_class}{error_class}{disabled_class}").to_string();
        let container = __scope.create_element("div");
        container.set_attribute("class", &wrapper_class);

        // Label
        if !self.label.is_empty() {
            let label = __scope.create_element("label");
            label.set_attribute("class", "rinch-select__label");
            let label_text = __scope.create_text(&self.label);
            label.append_child(&label_text);
            if self.required {
                let star = __scope.create_element("span");
                star.set_attribute("class", "rinch-select__required");
                let star_text = __scope.create_text(" *");
                star.append_child(&star_text);
                label.append_child(&star);
            }
            container.append_child(&label);
        }

        // Trigger wrapper (position: relative for dropdown positioning)
        let trigger_wrapper = __scope.create_element("div");
        trigger_wrapper.set_attribute("class", "rinch-select__wrapper");

        // Trigger button
        let trigger = __scope.create_element("div");
        if self.disabled {
            trigger.set_attribute("class", "rinch-select__input rinch-select__input--disabled");
            trigger.set_attribute("disabled", "");
        } else {
            trigger.set_attribute("class", "rinch-select__input");
            // Keyboard-reachable (issue #251). The trigger is a `<div>`, so it
            // has no implicit focusability to inherit — without this the whole
            // control was unreachable by Tab on both backends. Enter/Space then
            // dispatch its `data-rid` below, which is the same toggle the
            // pointer path runs.
            trigger.set_attribute("tabindex", "0");
        }
        trigger.set_attribute("role", "combobox");
        trigger.set_attribute("aria-haspopup", "listbox");
        trigger.set_attribute("aria-expanded", "false");
        trigger.set_attribute("aria-controls", &listbox_id);

        let trigger_aria = trigger.clone();
        __scope.create_effect(move || {
            trigger_aria
                .set_attribute("aria-expanded", if opened.get() { "true" } else { "false" });
        });

        let trigger_active = trigger.clone();
        __scope.create_effect(move || match highlighted.get() {
            Some(i) => trigger_active.set_attribute("aria-activedescendant", &option_id(i)),
            None => trigger_active.remove_attribute("aria-activedescendant"),
        });

        // Display text
        let display_span = __scope.create_element("span");
        display_span.set_attribute("class", "rinch-select__display");
        let display_text = __scope.create_text("");
        display_span.append_child(&display_text);

        let placeholder = self.placeholder.clone();
        let opts_for_display = options.clone();
        let placeholder2 = placeholder.clone();
        let display_text_c = display_text.clone();
        __scope.create_effect(move || {
            let val = selected_value.get();
            let text = if val.is_empty() {
                placeholder2.clone()
            } else {
                opts_for_display
                    .iter()
                    .find(|o| o.value == val)
                    .map(|o| o.display().to_string())
                    .unwrap_or(val)
            };
            display_text_c.set_text(&text);
        });

        // Placeholder styling
        let placeholder_empty = placeholder.is_empty();
        let display_span_c = display_span.clone();
        // Adding and removing the one class, never rewriting the attribute
        // (issue #717) — a rewrite drops whatever a parent patched onto the
        // span, which is how every #474 category C prop travels.
        __scope.create_effect(move || {
            let val = selected_value.get();
            if val.is_empty() && !placeholder_empty {
                display_span_c.add_class(PLACEHOLDER_CLASS);
            } else {
                display_span_c.remove_class(PLACEHOLDER_CLASS);
            }
        });

        trigger.append_child(&display_span);

        // Chevron icon
        let chevron =
            render_tabler_icon(__scope, TablerIcon::ChevronDown, TablerIconStyle::Outline);
        chevron.set_attribute("class", "rinch-select__chevron");
        let chevron_c = chevron.clone();
        __scope.create_effect(move || {
            if opened.get() {
                chevron_c.set_style("transform", "rotate(180deg)");
            } else {
                // `none`, not `rotate(0deg)`: an identity transform is still a
                // stacking context (#415); `none` ↔ `rotate(180deg)` turns the same.
                chevron_c.set_style("transform", "none");
            }
        });
        trigger.append_child(&chevron);

        // Click handler on trigger
        if !self.disabled {
            let handler_id = __scope.register_handler(move || {
                // Decide flip direction at toggle time using viewport bounds
                // (we don't know the dropdown's actual size until after layout).
                if !opened.get() {
                    let ctx = get_click_context();
                    let space_below = ctx.viewport_height - ctx.element_y - ctx.element_height;
                    flip_above.set(
                        space_below < DROPDOWN_RESERVE_PX && ctx.element_y > DROPDOWN_RESERVE_PX,
                    );
                }
                opened.update(|v| *v = !*v);
            });
            trigger.set_attribute("data-rid", &handler_id.0.to_string());
        }

        trigger_wrapper.append_child(&trigger);

        // Dropdown options list
        let dropdown = __scope.create_element("div");
        dropdown.set_attribute("class", "rinch-select__dropdown");
        dropdown.set_attribute("role", "listbox");
        dropdown.set_attribute("id", &listbox_id);
        // A pointer pick takes the click without the keyboard (#312), so the
        // trigger keeps the focus a combobox keeps — and Enter reopens the
        // list afterwards instead of reaching nothing.
        dropdown.set_attribute("data-nofocus", "");
        dropdown.set_style("display", "none");

        let dropdown_c = dropdown.clone();
        __scope.create_effect(move || {
            if opened.get() {
                dropdown_c.set_style("display", "flex");
            } else {
                dropdown_c.set_style("display", "none");
            }
        });

        let dropdown_flip = dropdown.clone();
        __scope.create_effect(move || {
            if flip_above.get() {
                dropdown_flip.add_class("rinch-select__dropdown--above");
            } else {
                dropdown_flip.remove_class("rinch-select__dropdown--above");
            }
        });

        // Commit option `i`: what a click on it and Enter on it both do.
        let onchange = self.onchange.clone();
        let opts_for_pick = options.clone();
        let pick: Rc<dyn Fn(usize)> = Rc::new(move |i: usize| {
            let Some(opt) = opts_for_pick.get(i) else {
                return;
            };
            let value = opt.value.clone();
            selected_value.set(value.clone());
            opened.set(false);
            if let Some(ref cb) = onchange {
                cb.invoke(value);
            }
        });

        // Render option items
        for (i, opt) in self.data.iter().enumerate() {
            let item = __scope.create_element("div");
            item.set_attribute("class", "rinch-select__option");
            item.set_attribute("role", "option");
            item.set_attribute("id", &option_id(i));
            let item_text = __scope.create_text(opt.display());
            item.append_child(&item_text);

            // Highlight selected option
            let opt_value2 = opt.value.clone();
            let item_c = item.clone();
            // Adding and removing the one class, never rewriting the attribute
            // — see the note on the display span above (issue #717).
            __scope.create_effect(move || {
                let val = selected_value.get();
                if val == opt_value2 {
                    item_c.add_class(SELECTED_CLASS);
                    item_c.set_attribute("aria-selected", "true");
                } else {
                    item_c.remove_class(SELECTED_CLASS);
                    item_c.set_attribute("aria-selected", "false");
                }
            });

            // The keyboard highlight, kept in view in a list taller than its
            // `max-height`.
            let item_h = item.clone();
            __scope.create_effect(move || {
                if highlighted.get() == Some(i) {
                    item_h.add_class(HIGHLIGHTED_CLASS);
                    item_h.scroll_into_view();
                } else {
                    item_h.remove_class(HIGHLIGHTED_CLASS);
                }
            });

            // Click handler for this option
            let pick_c = pick.clone();
            let handler_id = __scope.register_handler(move || pick_c(i));
            item.set_attribute("data-rid", &handler_id.0.to_string());

            dropdown.append_child(&item);
        }

        // The open list owns the keyboard (#434); see the module docs.
        let labels: Vec<String> = self
            .data
            .iter()
            .map(|o| o.display().to_lowercase())
            .collect();
        let typed: Rc<RefCell<(String, Option<Instant>)>> =
            Rc::new(RefCell::new((String::new(), None)));
        let pick_k = pick.clone();
        let on_key: Rc<dyn Fn(&KeyEventData) -> bool> = Rc::new(move |k: &KeyEventData| {
            if k.ctrl || k.alt || k.meta {
                return false;
            }
            let n = labels.len();
            let current = highlighted.get();
            if let Some(next) = step_highlight(&k.key, current, n) {
                highlighted.set(Some(next));
                return true;
            }
            // The spacebar is `" "` on both backends (#1161).
            let key = if k.is_space() { " " } else { k.key.as_str() };
            // A space typed while a type-ahead prefix is live extends it
            // ("new y" → New York), as the native popup does; otherwise Space
            // commits like Enter.
            let prefix_live = {
                let t = typed.borrow();
                !t.0.is_empty()
                    && t.1
                        .is_some_and(|at| Instant::now().duration_since(at) <= TYPEAHEAD_RESET)
            };
            let key = if key == " " && !prefix_live {
                "Enter"
            } else {
                key
            };
            match key {
                "Enter" => {
                    match current {
                        Some(i) => pick_k(i),
                        None => opened.set(false),
                    }
                    true
                }
                // Tab closes and moves on: it is not consumed.
                "Tab" => {
                    opened.set(false);
                    false
                }
                key if key.chars().count() == 1 && key.chars().all(|c| !c.is_control()) => {
                    let now = Instant::now();
                    let query = {
                        let mut t = typed.borrow_mut();
                        if t.1
                            .is_none_or(|at| now.duration_since(at) > TYPEAHEAD_RESET)
                        {
                            t.0.clear();
                        }
                        t.0.push_str(&key.to_lowercase());
                        t.1 = Some(now);
                        t.0.clone()
                    };
                    if let Some(i) = typeahead_match(&labels, &query, current.unwrap_or(0)) {
                        highlighted.set(Some(i));
                    }
                    true
                }
                // Escape is the dismiss entry's; everything else is the page's.
                _ => false,
            }
        });
        // Keys are the list's only while the focus is in this `Select` — the
        // trigger, in practice — and focus leaving it closes the list, as it
        // closes a native `<select>`'s popup (review of #1165).
        crate::overlay_dismiss::arm_keys_while_open(
            __scope,
            &container,
            Rc::new(move || opened.get()),
            on_key,
            Rc::new(move || opened.set(false)),
        );
        crate::overlay_dismiss::arm_close_on_escape_while_open(
            __scope,
            &trigger,
            Rc::new(move || opened.get()),
            Callback::new(move || opened.set(false)),
        );

        trigger_wrapper.append_child(&dropdown);

        // Backdrop to close on click outside
        let backdrop = __scope.create_element("div");
        backdrop.set_attribute("class", "rinch-select__backdrop");
        backdrop.set_style("display", "none");

        let backdrop_c = backdrop.clone();
        __scope.create_effect(move || {
            if opened.get() {
                backdrop_c.set_style("display", "block");
            } else {
                backdrop_c.set_style("display", "none");
            }
        });
        let backdrop_handler = __scope.register_handler(move || {
            opened.set(false);
        });
        backdrop.set_attribute("data-rid", &backdrop_handler.0.to_string());
        // A press of any button outside dismisses, not only a left one (#1093).
        backdrop.set_attribute(rinch_core::events::BACKDROP_ATTRIBUTE, "");
        trigger_wrapper.append_child(&backdrop);

        container.append_child(&trigger_wrapper);

        // Description
        if !self.description.is_empty() {
            let desc_div = __scope.create_element("div");
            desc_div.set_attribute("class", "rinch-select__description");
            let desc_text = __scope.create_text(&self.description);
            desc_div.append_child(&desc_text);
            container.append_child(&desc_div);
        }

        // Error
        if !self.error.is_empty() {
            let err_div = __scope.create_element("div");
            err_div.set_attribute("class", "rinch-select__error");
            let err_text = __scope.create_text(&self.error);
            err_div.append_child(&err_text);
            container.append_child(&err_div);
        }

        container
    }
}
