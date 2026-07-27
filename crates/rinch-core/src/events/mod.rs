//! Event handling infrastructure for rinch.
//!
//! This module provides the event handler registry that maps element IDs
//! to Rust callbacks, enabling reactive event handling in the UI.

mod drag;
mod drag_context;
mod handlers;
mod keyboard;
mod modifier;
mod selection;

// Re-export all public items so external code continues to work.
pub use drag::*;
pub use drag_context::*;
pub use handlers::*;
pub use keyboard::*;
pub use modifier::*;
pub use selection::*;

use std::cell::RefCell;

/// Text hit testing result from the layout engine.
///
/// When a click lands on text content, rinch-dom can resolve the exact
/// block and byte offset using parley's text layout data.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextHitInfo {
    /// The editor block index (from `data-block-index` attribute).
    pub block_index: usize,
    /// Byte offset within the block's text content.
    pub byte_offset: usize,
    /// The DOM node ID of the inline root element.
    pub inline_root_node_id: usize,
    /// Whether this hit info is valid (was actually resolved from layout).
    pub valid: bool,
}

/// Which mouse button triggered a pointer event.
///
/// Defaults to [`MouseButton::Left`] so existing click/hover/drag contexts
/// (which don't track a button) are unaffected. This is a `rinch-core`-local
/// type so the events layer stays free of any platform dependency; backends map
/// their native button type onto it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseButton {
    /// Primary button (usually left).
    #[default]
    Left,
    /// Middle button (wheel press).
    Middle,
    /// Secondary button (usually right).
    Right,
}

/// Click event context with mouse position and element bounds.
///
/// This data is available to event handlers during callback execution.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClickContext {
    /// Mouse X position relative to viewport.
    pub mouse_x: f32,
    /// Mouse Y position relative to viewport.
    pub mouse_y: f32,
    /// Clicked element's X position.
    pub element_x: f32,
    /// Clicked element's Y position.
    pub element_y: f32,
    /// Clicked element's width.
    pub element_width: f32,
    /// Clicked element's height.
    pub element_height: f32,
    /// Text hit info from the layout engine (resolved via find_text_position).
    pub text_hit: TextHitInfo,
    /// Viewport width in logical pixels.
    pub viewport_width: f32,
    /// Viewport height in logical pixels.
    pub viewport_height: f32,
    /// Which mouse button triggered the event.
    ///
    /// Meaningful for `onmousedown`/`onmouseup` (and `oncontextmenu`, which is
    /// always [`MouseButton::Right`]). Defaults to [`MouseButton::Left`] for
    /// `onclick`/`onmousemove`/hover/drag contexts that don't carry a button.
    pub button: MouseButton,
    /// Keyboard modifier state (Shift/Ctrl/Alt/Meta) at the time of the event.
    ///
    /// Populated for mouse events on both backends. The global
    /// [`get_modifier_state`] is not used for this — it is filled directly from
    /// the live platform/browser modifier state when the context is set.
    pub modifiers: ModifierState,
}

impl ClickContext {
    /// Get the mouse X position relative to the clicked element (0.0 to element_width).
    pub fn relative_x(&self) -> f32 {
        self.mouse_x - self.element_x
    }

    /// Get the mouse Y position relative to the clicked element (0.0 to element_height).
    pub fn relative_y(&self) -> f32 {
        self.mouse_y - self.element_y
    }

    /// Get the click position as a percentage of element width (0.0 to 1.0).
    pub fn percent_x(&self) -> f32 {
        if self.element_width > 0.0 {
            ((self.mouse_x - self.element_x) / self.element_width).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Get the click position as a percentage of element height (0.0 to 1.0).
    pub fn percent_y(&self) -> f32 {
        if self.element_height > 0.0 {
            ((self.mouse_y - self.element_y) / self.element_height).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

// Thread-local storage for the current click context.
// Used by handlers.rs via `super::CLICK_CONTEXT`.
thread_local! {
    pub(crate) static CLICK_CONTEXT: RefCell<ClickContext> = RefCell::new(ClickContext::default());
}

/// Bounding box of an ancestor of the clicked element, captured at dispatch time.
///
/// Returned by [`click_ancestors`] / [`find_click_ancestor`] so handlers can do
/// math in a *specific parent's* coordinate space — e.g. converting a click on a
/// timeline block into "fraction of the strip's width" without depending on how
/// many wrapper divs sit between them.
///
/// Positions are absolute (logical pixels relative to the viewport), like
/// [`ClickContext::element_x`] / [`ClickContext::element_y`].
#[derive(Debug, Clone, Default)]
pub struct AncestorBounds {
    /// Tag name (e.g. `"div"`).
    pub tag: String,
    /// `id` attribute; empty if unset.
    pub id: String,
    /// `class` attribute; empty if unset.
    pub class: String,
    /// Absolute X position of the element's top-left corner, in logical pixels.
    pub x: f32,
    /// Absolute Y position of the element's top-left corner.
    pub y: f32,
    /// Element width in logical pixels.
    pub width: f32,
    /// Element height in logical pixels.
    pub height: f32,
}

impl AncestorBounds {
    /// Whether `name` appears in the space-separated `class` attribute.
    pub fn has_class(&self, name: &str) -> bool {
        self.class.split_whitespace().any(|c| c == name)
    }
}

// Thread-local storage for the ancestor chain of the current click target.
// Kept separate from `CLICK_CONTEXT` so that struct can stay `Copy`.
thread_local! {
    pub(crate) static CLICK_ANCESTORS: RefCell<Vec<AncestorBounds>> =
        const { RefCell::new(Vec::new()) };
}

/// Ancestors of the most recently dispatched click target, ordered from
/// **immediate parent (index 0)** outward to the document root.
///
/// Returns an empty vector when no click is being dispatched, or when the
/// dispatcher did not populate the chain (e.g. synthetic events). The chain
/// is recomputed each click, so values from a previous click are *not* visible.
pub fn click_ancestors() -> Vec<AncestorBounds> {
    CLICK_ANCESTORS.with(|c| c.borrow().clone())
}

/// First ancestor of the click target for which `filter` returns `true`,
/// starting from the immediate parent and walking outward.
///
/// ```ignore
/// // Convert click X into a fraction of an arbitrary parent strip's width,
/// // regardless of how many wrappers are between block and strip.
/// if let Some(strip) = find_click_ancestor(|a| a.has_class("timeline-strip")) {
///     let ctx = get_click_context();
///     let frac = (ctx.mouse_x - strip.x) / strip.width.max(1.0);
/// }
/// ```
pub fn find_click_ancestor(filter: impl Fn(&AncestorBounds) -> bool) -> Option<AncestorBounds> {
    CLICK_ANCESTORS.with(|c| c.borrow().iter().find(|a| filter(a)).cloned())
}

/// Escape HTML special characters in a string.
///
/// This is used at runtime for dynamic content in RSX.
pub fn html_escape_string(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn test_register_and_dispatch() {
        clear_handlers();

        let called = Rc::new(Cell::new(false));
        let called_clone = called.clone();

        let id = register_handler(std::rc::Rc::new(move || {
            called_clone.set(true);
        }));

        assert!(!called.get());
        assert!(dispatch_event(id));
        assert!(called.get());
    }

    #[test]
    fn test_dispatch_unknown_id() {
        clear_handlers();

        let unknown_id = EventHandlerId(99999);
        assert!(!dispatch_event(unknown_id));
    }

    #[test]
    fn test_key_event_data_construction() {
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert_eq!(data.key, "a");
        assert_eq!(data.code, "KeyA");
        assert!(!data.ctrl);
    }

    #[test]
    fn test_keyboard_interceptor_set_and_dispatch() {
        clear_keyboard_interceptor();
        let called = Rc::new(Cell::new(false));
        let called_clone = called.clone();
        set_keyboard_interceptor(move |_data| {
            called_clone.set(true);
            true
        });
        let data = KeyEventData {
            key: "Enter".into(),
            code: "Enter".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert!(dispatch_keyboard_event(&data));
        assert!(called.get());
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_keyboard_interceptor_cleared() {
        clear_keyboard_interceptor();
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert!(!dispatch_keyboard_event(&data));
    }

    #[test]
    fn test_keyboard_interceptor_returns_false() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| false);
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert!(!dispatch_keyboard_event(&data));
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_keyboard_interceptor_replaced() {
        clear_keyboard_interceptor();
        set_keyboard_interceptor(|_| false);
        set_keyboard_interceptor(|_| true);
        let data = KeyEventData {
            key: "a".into(),
            code: "KeyA".into(),
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
        };
        assert!(dispatch_keyboard_event(&data));
        clear_keyboard_interceptor();
    }

    #[test]
    fn test_clear_handlers() {
        clear_handlers();

        let id = register_handler(std::rc::Rc::new(|| {}));
        assert_eq!(handler_count(), 1);

        clear_handlers();
        assert_eq!(handler_count(), 0);
        assert!(!dispatch_event(id));
    }

    /// An input handler must be able to touch the input registry while it runs
    /// (issue #141).
    ///
    /// This is not exotic: an `oninput` writes a signal, `Signal::set` flushes
    /// effects synchronously outside `batch()`, and an effect that flips an `if`
    /// renders new content which registers new handlers. `dispatch_input_event`
    /// used to hold `INPUT_REGISTRY.borrow()` across the callback, so that path
    /// was a BorrowMutError on the first such keystroke. Pre-fix this test
    /// panics with "already borrowed".
    #[test]
    fn input_handler_may_register_another_handler_while_running() {
        clear_handlers();

        let registered: std::rc::Rc<std::cell::Cell<bool>> =
            std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = registered.clone();

        let id = register_input_handler(InputCallback::new(move |_value: String| {
            // Re-enters INPUT_REGISTRY while the dispatch is on the stack.
            register_input_handler(InputCallback::new(|_| {}));
            flag.set(true);
        }));

        assert!(dispatch_input_event(id, "hello".to_string()));
        assert!(registered.get(), "the nested registration must have run");

        clear_handlers();
    }

    /// The same re-entrancy reached the way production actually reaches it: the
    /// handler writes a signal, `Signal::set` flushes effects synchronously, and
    /// an effect renders a branch containing an input — which registers a
    /// handler while the outer dispatch is still on the stack.
    ///
    /// This is the exact shape of "typing in a box that reveals another box".
    /// Pre-fix it panics with "already borrowed: BorrowMutError".
    #[test]
    fn input_handler_writing_a_signal_may_trigger_handler_registration() {
        use crate::reactive::{Effect, Signal};
        use std::cell::Cell;
        use std::rc::Rc;

        clear_handlers();

        let show_second = Signal::new(false);
        let registrations = Rc::new(Cell::new(0));
        let count = registrations.clone();

        // Stands in for an `if show_second { TextInput {} }` branch.
        let _effect = Effect::new(move || {
            if show_second.get() {
                register_input_handler(InputCallback::new(|_| {}));
                count.set(count.get() + 1);
            }
        });
        assert_eq!(registrations.get(), 0, "branch starts hidden");

        let first = register_input_handler(InputCallback::new(move |_| show_second.set(true)));

        assert!(dispatch_input_event(first, "hello".to_string()));
        assert_eq!(
            registrations.get(),
            1,
            "the revealed branch must have registered its handler"
        );

        clear_handlers();
    }
}
