//! What every text-entry component writes on its field the same way: the
//! browser's entry hints (`autocomplete`, `inputmode`) and the Enter-to-submit
//! handler.
//!
//! `TextInput`, `Textarea`, `PasswordInput`, `NumberInput` and `ColorInput`
//! each call these on their `<input>`/`<textarea>`, so a hint or `onsubmit`
//! means the same thing on every one of them.

use rinch_core::Callback;
use rinch_core::dom::{NodeHandle, RenderScope};

/// Write the field's `autocomplete` (#3028) and `inputmode` (#3048)
/// attributes. An empty value writes no attribute, which leaves the browser's
/// default; any other value is written as given.
///
/// On web the browser reads both: autofill and password managers read
/// `autocomplete`, and `inputmode` picks the on-screen keyboard (`numeric`
/// brings up a digit pad). On desktop the attributes are kept on the element
/// for the DOM and change nothing else.
pub(crate) fn write_entry_hints(field: &NodeHandle, autocomplete: &str, inputmode: &str) {
    if !autocomplete.is_empty() {
        field.set_attribute("autocomplete", autocomplete);
    }
    if !inputmode.is_empty() {
        field.set_attribute("inputmode", inputmode);
    }
}

/// Wire `onsubmit` to run when Enter is pressed in `field`: the callback is
/// registered as a click-style handler and its id written as `data-onsubmit`,
/// which both runtimes read on Enter (desktop's key path and rinch-web's
/// delegation), after the field's `data-onchange` commit. `None` writes
/// nothing, so Enter does not submit.
pub(crate) fn bind_submit(
    scope: &mut RenderScope,
    field: &NodeHandle,
    onsubmit: &Option<Callback>,
) {
    if let Some(callback) = onsubmit {
        let callback = callback.clone();
        let handler_id = scope.register_handler(move || {
            callback.invoke();
        });
        field.set_attribute("data-onsubmit", &handler_id.0.to_string());
    }
}
