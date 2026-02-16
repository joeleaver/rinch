//! Event handling infrastructure for rinch.
//!
//! This module provides the event handler registry that maps element IDs
//! to Rust callbacks, enabling reactive event handling in the UI.

mod drag;
mod handlers;
mod keyboard;
mod modifier;
mod selection;

// Re-export all public items so external code continues to work.
pub use drag::*;
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
}
