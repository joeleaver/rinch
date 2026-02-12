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
    AppMenuProps, Children, Element, ForItem, HandlerId, IntoDom, IntoElement, MenuItemCallback,
    MenuItemProps, MenuProps, Reactive, ThemeProviderProps, ValueCallback, Widget, WidgetCallback,
    WindowProps,
};

// Re-export Show and For functions (fine-grained DOM-based)
pub use for_loop::{FineForBuilder, for_each_dom, to_for_items};
pub use show::{FineShowBuilder, show_dom};

// Re-export reconciliation types
pub use reconcile::{ListOp, diff_keyed};

// Re-export reactive types for convenience
pub use reactive::{
    Effect, Memo, Scope, Signal, batch, clear_on_signal_change, clear_signals_changed, derived,
    set_on_signal_change, signals_changed, untracked,
};

// Re-export hooks for ergonomic state management
pub use hooks::{
    HookEntry, HookMeta, RefHandle, begin_render, clear_hooks, create_context, end_render,
    get_hooks_debug_info, pop_hook_scope, push_hook_scope, use_callback, use_context, use_derived,
    use_effect, use_effect_cleanup, use_memo, use_mount, use_ref, use_signal, use_state,
    with_render_context,
};

// Re-export event handling types
pub use events::{
    ClickContext, EventCallback, EventHandlerId, InputCallback, InputContext, KeyEventData,
    ModifierState, SelectionAction, TextHitInfo, check_and_clear_input_handled, clear_handlers,
    clear_keyboard_interceptor, clear_selection_callback, clear_selection_snapshot,
    clear_selection_sync_callback, dispatch_event, dispatch_input_event, dispatch_keyboard_event,
    dispatch_selection, fire_selection_sync, get_click_context, get_input_context,
    get_modifier_state, get_saved_selection, is_dragging, query_selection_ranges, register_handler,
    register_input_handler, request_focus, request_selection_clear, save_selection_snapshot,
    set_click_context, set_input_context, set_keyboard_interceptor, set_modifier_state,
    set_selection_callback, set_selection_sync_callback, start_drag, start_drag_absolute,
    stop_drag, take_pending_focus_request, take_pending_selection_clear, update_drag,
};

// Re-export DOM types for fine-grained rendering
pub use dom::{
    DomDocument, DomUpdate, IntoNode, NodeHandle, NodeId, RenderScope, UpdateBatch,
    clear_render_scope, has_render_scope, reactive_widget_dom, set_render_scope,
    try_with_render_scope, with_render_scope,
};

// Re-export icon types
pub use icon::Icon;

use std::cell::RefCell;

/// Resolve spacing scale values to CSS custom properties.
///
/// Maps `xs`, `sm`, `md`, `lg`, `xl` to `var(--rinch-spacing-{value})`.
/// Other values are passed through unchanged.
///
/// Used by macro-generated code for style shorthand props (e.g., `p: "md"`, `m: "xl"`).
pub fn resolve_spacing(value: &str) -> String {
    match value {
        "xs" | "sm" | "md" | "lg" | "xl" => format!("var(--rinch-spacing-{})", value),
        _ => value.to_string(),
    }
}

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
