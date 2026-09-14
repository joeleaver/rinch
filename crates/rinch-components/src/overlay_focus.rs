//! Tab containment for `Modal`, `Drawer` and `Popover` (issue #474).
//!
//! `trap_focus` was declared on all three, documented, and read by nothing:
//! Tab walked straight out of an open dialog and into the page behind it, which
//! is the "backdrop modality" gap CLAUDE.md listed as *not yet*.
//!
//! The whole component side is **one attribute**, `data-trap-focus`, present on
//! the overlay's root while it is open and absent otherwise. The backends read
//! it: desktop's `RinchApp::tab_trap_root` starts the focusable collection at
//! the trap instead of at the document, and `rinch-web`'s keydown listener
//! computes the trap's focusable set and moves focus itself. That is the same
//! shape as `data-nofocus`, deliberately — a component that says what it wants
//! and two backends that each know how to grant it, rather than a component
//! that reaches into either.
//!
//! **Why not `register_focus_target`.** `FocusEntry::on_key` is the obvious
//! candidate and it silently does nothing here: the arbiter offers a key to a
//! registered target only while it holds `FocusTarget::Node`, and the focused
//! element inside a dialog is normally an `<input>`, i.e. `FocusTarget::Input`.
//! A trap built on it would work in a fixture with a `tabindex` div and fail
//! the moment the dialog contained a text field.
//!
//! **Absence, not `"false"`.** `data-trap-focus` is in
//! [`rinch_core::dom::is_boolean_attribute`], so
//! [`NodeHandle::write_attribute`](rinch_core::dom::NodeHandle::write_attribute)
//! writes presence for open and *removes* it for closed. Leaving
//! `data-trap-focus="false"` on a closed overlay would be read as a live trap
//! by anything testing presence, and Tab would circle inside an invisible
//! dialog forever. Both readers do honour rinch's `"false"` escape, so that
//! spelling is survivable — but it is the backstop, not the contract.
//!
//! **Not in scope (deferred, issue #695).** Moving focus *into* the overlay
//! when it opens, and restoring the previously focused element when it closes.
//! `NodeHandle::focus()` is portable but there is no portable way to *read* the
//! current focus or to blur, so restore needs new `DomDocument` surface.
//! Containment is what closes the prop.

use std::rc::Rc;

use rinch_core::dom::{NodeHandle, RenderScope};

/// A reactive `opened` getter, as the three overlay components spell it.
pub type ReactiveBool = Rc<dyn Fn() -> bool>;

/// Stamp `data-trap-focus` on an overlay's root for as long as it is open.
///
/// A no-op when `trap_focus` is false — the attribute is never written, so Tab
/// is left to the whole document, which is what "off" has to mean.
///
/// With an `opened_fn` the attribute tracks the signal through an effect; with
/// only the static `opened` prop it is written once, from the same truth the
/// rest of the component renders from.
pub fn arm_trap_focus(
    scope: &mut RenderScope,
    root: &NodeHandle,
    trap_focus: bool,
    opened: bool,
    opened_fn: Option<&ReactiveBool>,
) {
    if !trap_focus {
        return;
    }
    match opened_fn {
        Some(f) => {
            let f = f.clone();
            let root = root.clone();
            // `write_attribute`, not `set_attribute`: closing must *remove* the
            // attribute, and the effect re-runs on every toggle.
            scope.create_effect(move || {
                root.write_attribute("data-trap-focus", if f() { "" } else { "false" });
            });
        }
        None => {
            if opened {
                root.write_attribute("data-trap-focus", "");
            }
        }
    }
}
