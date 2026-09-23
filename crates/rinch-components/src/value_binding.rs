//! The `value_fn` binding every text component shares (issue #238).
//!
//! `TextInput`, `PasswordInput`, `Textarea` and `NumberInput` each used to
//! carry the same three lines — write the initial value, then an effect that
//! writes `value_fn()` into the field's `value` on every change — and each new
//! component that copied them copied the missing echo guard with them. This is
//! the one copy, so the next component inherits the guard rather than the gap.

use rinch_core::dom::{NodeHandle, RenderScope};
use std::rc::Rc;

/// Bind `field`'s value to `value_fn`: write it now, then again whenever a
/// signal it reads changes — **unless the field already shows it**.
///
/// The documented controlled pattern is `value_fn` + `oninput` on one signal,
/// so the common re-run is the echo of the user's own keystroke:
/// keystroke → `oninput` → signal → this effect, holding exactly the text the
/// field shows. That echo is skipped by asking the field for its
/// [`live_value`](NodeHandle::live_value) — the text the user sees, which on
/// the web is the `.value` property and not the `value` attribute. A value the
/// field does not show (a normalising `oninput`'s rewrite, a programmatic
/// change) is written as before, and the backend adopts a write to the focused
/// field without moving the caret (#287).
///
/// The comparison is against the **live** text, never the attribute: on the
/// web the attribute keeps the last programmatic write while the user types,
/// so a guard reading it would skip a change back to that stale value and
/// leave the field showing something else.
pub fn bind_value_fn(
    scope: &mut RenderScope,
    field: &NodeHandle,
    value_fn: &Rc<dyn Fn() -> String>,
) {
    field.set_attribute("value", &value_fn());

    let value_fn = value_fn.clone();
    let field = field.clone();
    scope.create_effect(move || {
        let value = value_fn();
        if field.live_value().as_deref() != Some(value.as_str()) {
            field.set_attribute("value", &value);
        }
    });
}
