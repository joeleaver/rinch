//! Core types and traits for rinch.

pub mod dom;
pub mod element;
pub mod event;
pub mod events;
pub mod for_loop;
pub mod hooks;
pub mod icon;
pub mod reactive;
pub mod reconcile;
pub mod show;

// Re-export element types for convenience
pub use element::{
    AppMenuProps, Children, Element, ForItem, HandlerId, IntoDom, IntoElement,
    MenuItemCallback, MenuItemProps, MenuProps, Reactive, ThemeProviderProps,
    ValueCallback, Widget, WidgetCallback, WindowProps,
};

// Re-export Show and For functions (fine-grained DOM-based)
pub use for_loop::{for_each_dom, to_for_items, FineForBuilder};
pub use show::{show_dom, FineShowBuilder};

// Re-export reconciliation types
pub use reconcile::{diff_keyed, ListOp};

// Re-export reactive types for convenience
pub use reactive::{
    batch, clear_on_signal_change, clear_signals_changed, derived, set_on_signal_change,
    signals_changed, untracked, Effect, Memo, Scope, Signal,
};

// Re-export hooks for ergonomic state management
pub use hooks::{
    begin_render, clear_hooks, create_context, end_render, get_hooks_debug_info, use_callback,
    use_context, use_derived, use_effect, use_effect_cleanup, use_memo, use_mount, use_ref,
    use_signal, use_state, HookMeta, RefHandle,
};

// Re-export event handling types
pub use events::{
    check_and_clear_input_handled, clear_handlers, dispatch_event, dispatch_input_event,
    get_click_context, get_input_context, is_dragging, register_handler, register_input_handler,
    set_click_context, set_input_context, start_drag, stop_drag, update_drag,
    clear_keyboard_interceptor, dispatch_keyboard_event, set_keyboard_interceptor,
    get_modifier_state, set_modifier_state,
    dispatch_selection, set_selection_callback, clear_selection_callback, query_selection_ranges,
    save_selection_snapshot, clear_selection_snapshot, get_saved_selection,
    set_selection_sync_callback, clear_selection_sync_callback, fire_selection_sync,
    request_selection_clear, take_pending_selection_clear,
    request_focus, take_pending_focus_request,
    ClickContext, EventCallback, EventHandlerId, InputCallback, InputContext, KeyEventData,
    ModifierState, TextHitInfo, SelectionAction,
};

// Re-export DOM types for fine-grained rendering
pub use dom::{
    clear_render_scope, has_render_scope, set_render_scope, try_with_render_scope,
    with_render_scope, DomDocument, DomUpdate, IntoNode, NodeHandle, NodeId, RenderScope, UpdateBatch,
};


// Re-export icon types
pub use icon::Icon;

use std::cell::RefCell;

// ============================================================================
// Thread-Local Effect Storage
// ============================================================================

/// Stored Effects that need to live for the app lifetime.
/// Used by Show/For components to keep their condition-watching Effects alive.
struct StoredEffect {
    #[allow(dead_code)]
    effect: Effect,
}

thread_local! {
    /// Effects stored globally - kept alive for the app lifetime.
    static STORED_EFFECTS: RefCell<Vec<StoredEffect>> = const { RefCell::new(Vec::new()) };
}

/// Store an effect so it lives for the app lifetime.
///
/// Used by fine-grained Show/For components to keep their condition-watching
/// Effects alive even though they're not attached to a specific RenderScope.
#[deprecated(note = "Attach effects to a RenderScope instead of storing globally")]
pub fn store_effect(effect: Effect) {
    STORED_EFFECTS.with(|e| {
        e.borrow_mut().push(StoredEffect { effect });
    });
}

/// Clear all stored effects.
///
/// Called on app shutdown or when resetting the application state.
pub fn clear_stored_effects() {
    STORED_EFFECTS.with(|e| e.borrow_mut().clear());
}

/// Get the number of stored effects (for debugging).
pub fn stored_effect_count() -> usize {
    STORED_EFFECTS.with(|e| e.borrow().len())
}

// Thread-local storage for the current theme CSS (for snapshot creation)
#[cfg(feature = "theme")]
thread_local! {
    static CURRENT_THEME_CSS: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the current theme CSS for snapshot creation.
#[cfg(feature = "theme")]
pub fn set_current_theme_css(css: Option<String>) {
    CURRENT_THEME_CSS.with(|theme_css| {
        *theme_css.borrow_mut() = css;
    });
}

/// Get the current theme CSS for snapshot creation.
#[cfg(feature = "theme")]
pub fn get_current_theme_css() -> Option<String> {
    CURRENT_THEME_CSS.with(|theme_css| theme_css.borrow().clone())
}

