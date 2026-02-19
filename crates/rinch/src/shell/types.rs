//! Shared types for the rinch shell module.

use rinch_core::events::{ClickContext, EventHandlerId};
use winit::window::WindowId;

/// Events used internally by rinch.
#[derive(Debug, Clone)]
pub enum RinchEvent {
    /// Poll a window for document updates.
    Poll { window_id: WindowId },
    /// A menu item was activated.
    MenuEvent(muda::MenuId),
    /// Request a re-render of all windows (full app re-execution).
    ReRender,
    /// Sync DOM updates to windows (Effects have already updated the adapter).
    /// This is much faster than ReRender because it doesn't re-run app().
    SyncDom,
    /// Fine-grained reactive text update - directly update the DOM Document.
    /// This is the fastest path: no app() re-execution, no HTML regeneration.
    UpdateReactiveText { reactive_id: usize, text: String },
    /// An element was clicked (with handler ID, source window, and click context).
    ElementClicked {
        handler_id: EventHandlerId,
        window_id: WindowId,
        click_context: ClickContext,
    },
    /// Toggle the DevTools window.
    ToggleDevTools { source_window: WindowId },
    /// Update DevTools with hovered element info.
    UpdateDevToolsHover {
        element_info: Option<HoveredElementInfo>,
    },
    /// A keyboard shortcut was pressed - check against menu shortcuts.
    KeyboardShortcut {
        ctrl: bool,
        meta: bool,
        alt: bool,
        shift: bool,
        key: winit::keyboard::KeyCode,
    },
    /// Process pending window requests (open/close).
    ProcessWindowRequests,
    /// Minimize a window.
    MinimizeWindow { window_id: WindowId },
    /// Toggle maximize state of a window.
    ToggleMaximizeWindow { window_id: WindowId },
    /// Close a window (from window controls).
    CloseWindowControl { window_id: WindowId },
    /// Show a window (restore from hidden/tray).
    ShowWindow { window_id: WindowId },
    /// Hide a window (minimize to tray).
    HideWindow { window_id: WindowId },
    /// Refresh the DevTools window content.
    RefreshDevTools,
}

/// Information about a hovered element for DevTools display.
#[derive(Debug, Clone)]
pub struct HoveredElementInfo {
    /// The element's tag name (e.g., "div", "button").
    pub tag_name: String,
    /// The element's id attribute, if any.
    pub id: Option<String>,
    /// The element's class attribute, if any.
    pub classes: Option<String>,
    /// Key style properties.
    pub styles: Vec<(String, String)>,
    /// Layout information.
    pub layout: ElementLayout,
}

/// Layout information for an element.
#[derive(Debug, Clone)]
pub struct ElementLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
