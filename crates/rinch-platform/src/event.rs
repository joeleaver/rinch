//! Platform-agnostic event types.

/// A platform-agnostic input event.
///
/// Platform backends translate their native events into these variants.
/// The rinch runtime processes these without any platform-specific knowledge.
#[derive(Debug, Clone)]
pub enum PlatformEvent {
    /// The application has been resumed (window ready for rendering).
    Resumed,
    /// The window close button was pressed.
    CloseRequested,
    /// The window was resized.
    Resized { width: u32, height: u32 },
    /// A redraw was requested.
    RedrawRequested,
    /// Mouse cursor moved.
    MouseMove { x: f32, y: f32 },
    /// Mouse button pressed.
    MouseDown { x: f32, y: f32, button: MouseButton },
    /// Mouse button released.
    MouseUp { x: f32, y: f32, button: MouseButton },
    /// Mouse wheel scrolled.
    MouseWheel {
        x: f32,
        y: f32,
        delta_x: f64,
        delta_y: f64,
    },
    /// Key pressed.
    KeyDown {
        key: KeyCode,
        text: Option<String>,
        modifiers: Modifiers,
    },
    /// Modifier keys changed.
    ModifiersChanged(Modifiers),
    /// Display scale factor changed.
    ScaleFactorChanged(f64),
    /// A user-defined event from the application.
    UserEvent(UserEvent),
    /// The event loop is about to wait for new events.
    AboutToWait,
}

/// Application-level events sent to the event loop.
#[derive(Debug, Clone)]
pub enum UserEvent {
    /// A signal changed -- re-resolve layout and repaint.
    ReRender,
    /// Minimize the window.
    MinimizeWindow,
    /// Toggle maximize state.
    ToggleMaximizeWindow,
    /// Close the window.
    CloseWindow,
    /// A debug command is ready (debug feature only).
    DebugCommand,
}

/// Actions the runtime requests from the platform backend.
#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    /// Request a window redraw.
    RequestRedraw,
    /// Exit the application.
    Exit,
    /// Set minimized state.
    SetMinimized(bool),
    /// Set maximized state.
    SetMaximized(bool),
    /// Initiate a window drag (for custom titlebars).
    DragWindow,
}

/// Platform-agnostic mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Platform-agnostic modifier key state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Command on macOS, Windows key on Windows/Linux.
    pub meta: bool,
}

impl Modifiers {
    /// Returns the "primary" modifier for keyboard shortcuts.
    ///
    /// On macOS this is `meta` (Cmd), on all other platforms this is `ctrl`.
    pub fn primary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.meta
        } else {
            self.ctrl
        }
    }
}

/// Platform-agnostic key codes.
///
/// This is a subset of physical key codes sufficient for rinch's needs.
/// Platform backends translate their native key codes to these variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    // Arrow keys
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,

    // Navigation
    Home,
    End,
    PageUp,
    PageDown,

    // Editing
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    Space,

    // Letters
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,

    // Digits
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Symbols
    Equal,
    Minus,

    /// A key code not covered by the common set.
    /// The platform backend may include additional info in the event's `text` field.
    Other,
}
