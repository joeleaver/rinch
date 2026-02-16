//! Keyboard modifier state tracking.

use std::cell::RefCell;

thread_local! {
    static MODIFIER_STATE: RefCell<ModifierState> = const { RefCell::new(ModifierState {
        shift: false,
        ctrl: false,
        alt: false,
        meta: false,
    }) };
}

/// Current keyboard modifier state (Shift, Ctrl, Alt, Meta).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModifierState {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Set the current modifier state.
///
/// This should be called by the shell whenever modifier keys change
/// (typically in the ModifiersChanged event handler).
pub fn set_modifier_state(state: ModifierState) {
    MODIFIER_STATE.with(|ms| {
        *ms.borrow_mut() = state;
    });
}

/// Get the current modifier state.
///
/// This can be called from event handlers to check if modifier keys
/// are currently pressed.
///
/// # Example
///
/// ```ignore
/// // In a click handler:
/// let mods = get_modifier_state();
/// if mods.shift {
///     // Extend selection
/// } else {
///     // Set cursor
/// }
/// ```
pub fn get_modifier_state() -> ModifierState {
    MODIFIER_STATE.with(|ms| *ms.borrow())
}
