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
//! **The other half (issue #695).** Containment alone leaves an overlay opening
//! with the keyboard still on whatever opened it, and closing with the claim
//! pointing into a subtree that no longer has a box. [`arm_overlay_focus`] is
//! the move-in and the restore, and it needed portable API that did not exist:
//! [`DomDocument::active_element`](rinch_core::dom::DomDocument::active_element)
//! to read the current focus,
//! [`focus_into`](rinch_core::dom::DomDocument::focus_into) to move it in
//! (because "the first focusable" is each backend's own computation and a third
//! answer written here would be wrong on both), and
//! [`restore_focus`](rinch_core::dom::DomDocument::restore_focus) to give it
//! back or let it go.
//!
//! **Why the close is one call and not a decision made here.** Whether the
//! keyboard is even this overlay's to return, and whether the remembered opener
//! can still take it, are both questions about *boxes* — and the boxes are a
//! layout out of date at the moment this effect runs, because the close is a
//! class change in this very flush. A `blur()` verb would not have helped: a
//! caller that could only blur would still have to decide, and a caller that
//! guessed and then corrected itself would lose one of its two writes to the
//! single request slot.

use std::cell::RefCell;
use std::rc::Rc;

use rinch_core::dom::{FocusIntoPolicy, NodeHandle, RenderScope};

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

/// Move focus **into** an overlay when it opens and give it **back** when it
/// closes (issue #695) — the half of `showModal()`/`close()` that #474 left out.
///
/// A no-op when `trap_focus` is false. That tie is deliberate and it is the
/// prop's whole meaning here: `trap_focus` is rinch's spelling of "this overlay
/// is modal", and a browser moves focus for `showModal()` and not for `show()`.
/// An overlay that wants containment without the move, or the move without
/// containment, has no way to ask for it — say so rather than invent a second
/// prop.
///
/// # What happens, and in what order
///
/// **On the false→true edge:** remember whatever holds the keyboard right now
/// (the opener — usually the button that was clicked), then
/// [`NodeHandle::focus_into`] the overlay's root. `policy` picks the browser
/// rule: a dialog focuses its first stop, a popover only an `autofocus` one.
///
/// **On the true→false edge, and on unmount while open:** hand the keyboard
/// back to the remembered node — but only if it is **still connected**. An
/// opener the dialog itself deleted (a row's Edit button, a list that
/// re-rendered) must not be focused back into a detached subtree. When it
/// cannot be restored, the claim is released instead, and only if it is still
/// *inside* the overlay: a user who moved focus elsewhere before the close
/// keeps it.
///
/// Exactly **one** of focus / blur is issued per close, never both. Desktop
/// parks focus requests in a single slot (`rinch_core::FocusRequest`), so a
/// speculative focus followed by a corrective blur would silently discard the
/// first — the bug this ordering is written to avoid.
///
/// # Nesting falls out of it
///
/// An inner overlay opening remembers whatever the outer one focused, so
/// closing the inner restores *into* the outer, and closing the outer restores
/// to the page. Nothing here knows about nesting; the memory being per-overlay
/// is the whole mechanism.
///
/// # Not covered
///
/// A screen reader is not told anything extra. Desktop's AccessKit surface
/// (`crates/rinch/src/editor/a11y.rs`) is the rich-text editor's, so a focus
/// move outside the editor pushes no accessibility event on either backend;
/// giving overlays an announced focus change is separate work.
pub fn arm_overlay_focus(
    scope: &mut RenderScope,
    root: &NodeHandle,
    trap_focus: bool,
    opened: bool,
    opened_fn: Option<&ReactiveBool>,
    policy: FocusIntoPolicy,
) {
    if !trap_focus {
        return;
    }
    // Without an `opened_fn` the static prop is the only truth there is, and it
    // cannot change — the effect runs once and the edge it sees is the right
    // one.
    let is_open: ReactiveBool = match opened_fn {
        Some(f) => f.clone(),
        None => Rc::new(move || opened),
    };

    // `None` = this overlay is not currently holding focus captive. The edge
    // guard, exactly as `overlay_scroll_lock` needs one: an effect re-runs for
    // anything its getter read, not only for a change of `opened`, and a re-run
    // must not re-capture (which would forget the real opener) or re-restore.
    let held: Rc<RefCell<Option<Capture>>> = Rc::new(RefCell::new(None));

    let effect_root = root.clone();
    let effect_held = held.clone();
    scope.create_effect(move || {
        let open = is_open();
        let currently_held = effect_held.borrow().is_some();
        if open && !currently_held {
            // The opener, read *before* anything moves. `None` is a real
            // answer — nothing was focused — and is remembered as such, so the
            // close path releases rather than restoring blind.
            *effect_held.borrow_mut() = Some(Capture {
                opener: effect_root.active_element(),
            });
            effect_root.focus_into(policy);
        } else if !open && currently_held {
            let captured = effect_held.borrow_mut().take();
            if let Some(captured) = captured {
                restore(&effect_root, captured);
            }
        }
    });

    let cleanup_root = root.clone();
    scope.on_cleanup(move || {
        // An overlay can leave the tree while still open — the ordinary shape
        // of `if show { Modal { … } }` — and then the effect that would have
        // restored focus never runs again. Without this the keyboard stays
        // claimed by a node that no longer exists: state armed by one event and
        // cleared only by a second that may never arrive.
        let captured = held.borrow_mut().take();
        if let Some(captured) = captured {
            restore(&cleanup_root, captured);
        }
    });
}

/// What one open overlay remembers: who had the keyboard before it took it.
struct Capture {
    opener: Option<NodeHandle>,
}

/// Give the keyboard back, or let it go — see [`arm_overlay_focus`].
///
/// One call, and the backend decides. It has to: whether the opener can still
/// take the keyboard, and whether the keyboard is even this overlay's to return,
/// are both questions about boxes, and the boxes here are a layout out of date —
/// the close is a class change in this very effect flush. See
/// [`DomDocument::restore_focus`](rinch_core::dom::DomDocument::restore_focus).
fn restore(root: &NodeHandle, captured: Capture) {
    root.restore_focus(captured.opener.as_ref());
}
