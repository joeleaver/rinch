//! Platform-agnostic event types.

use std::path::PathBuf;

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
        /// The **physical** key (layout-independent position).
        key: KeyCode,
        /// The **logical** letter this key produces in the active layout, lowercased
        /// (`Some('b')` for the Dvorak/AZERTY key labelled B), when it is a single
        /// ASCII letter — used so `Mod`+letter shortcuts follow the layout label rather
        /// than the physical position. `None` for non-letters or when unknown; the
        /// consumer then falls back to `key`. Distinct from `text` (which is the text to
        /// insert, and is suppressed by modifiers).
        logical_key: Option<char>,
        /// The text this keypress would insert (suppressed under Ctrl/Cmd), or `None`.
        text: Option<String>,
        modifiers: Modifiers,
    },
    /// Key released.
    KeyUp { key: KeyCode, modifiers: Modifiers },
    /// Modifier keys changed.
    ModifiersChanged(Modifiers),
    /// Display scale factor changed.
    ScaleFactorChanged(f64),
    /// A user-defined event from the application.
    UserEvent(UserEvent),
    /// The event loop is about to wait for new events.
    AboutToWait,
    /// A file drag entered the window from the OS.
    /// In winit 0.31, all paths arrive in a single DragEntered event.
    FileHoverEnter { path: PathBuf, position: (f64, f64) },
    /// A file drag is moving over the window.
    FileDragMoved { position: (f64, f64) },
    /// The OS file drag left the window without dropping.
    FileHoverCancelled,
    /// Files were dropped onto the window from the OS.
    FileDropped {
        paths: Vec<PathBuf>,
        position: (f64, f64),
    },
    /// An IME (input method editor) composition event for the focused text
    /// target. Desktop (winit `WindowEvent::Ime`) and Android (the
    /// `InputConnection` drain loop) both translate their native composition
    /// events into this single portable variant, so IME rides the same
    /// focus-arbiter routing as [`PlatformEvent::KeyDown`] — it is not an
    /// editor-specific path.
    Ime(ImeEvent),
}

/// A platform-agnostic IME (input method editor) composition event.
///
/// This is the **portable IME contract**: every text target (the rich-text
/// editor, a single-line `<input>`, …) consumes the same five variants through
/// the runtime's [`FocusTarget`](https://docs.rs) routing, so IME behaves
/// uniformly everywhere rather than being re-implemented per widget.
///
/// Backends translate into this:
/// - **Desktop:** winit `WindowEvent::Ime(Ime)` → one of these variants.
/// - **Android:** `drain_committed_text()` → [`ImeEvent::Commit`],
///   `drain_deletions()` → [`ImeEvent::DeleteSurrounding`].
#[derive(Debug, Clone, PartialEq)]
pub enum ImeEvent {
    /// Composition became available for the focused target. The target may
    /// start receiving [`Preedit`](ImeEvent::Preedit)/[`Commit`](ImeEvent::Commit)
    /// events.
    Enabled,
    /// Set (or clear) the current composition string shown inline at the caret.
    ///
    /// `text` is the in-progress composition (an empty string clears it).
    /// `cursor` is an optional `(begin, end)` **byte** range within `text` for
    /// candidate-box placement; `None` hides the candidate cursor. The preedit
    /// is **never** part of the document — it is rendered as a transient overlay
    /// and discarded on the next [`Commit`](ImeEvent::Commit) or clear.
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
    /// Commit composed text into the focused target. The target clears any
    /// pending preedit and inserts `text` at the selection in one edit.
    Commit(String),
    /// Delete `before`/`after` units around the cursor (surrounding-text edit
    /// some IMEs use to recompose). Units are characters in rinch's char-based
    /// model; the platform boundary converts as needed. Only delivered once a
    /// backend advertises surrounding-text support.
    DeleteSurrounding { before: usize, after: usize },
    /// Composition ended for the focused target; any pending preedit is cleared.
    Disabled,
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
    /// Show the window.
    ShowWindow,
    /// Hide the window.
    HideWindow,
    /// A debug command is ready (debug feature only).
    DebugCommand,
}

/// Direction for window resize drag operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeDirection {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
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
    /// Set window visibility.
    SetVisible(bool),
    /// Initiate a window drag (for custom titlebars).
    DragWindow,
    /// Initiate a window resize drag from an edge or corner.
    DragResizeWindow(ResizeDirection),
    /// Set the mouse cursor icon. Values match CSS cursor keywords.
    SetCursor(CursorStyle),
    /// Toggle the DevTools window.
    ToggleDevTools,
    /// Toggle inspect mode (hover highlight).
    ToggleInspectMode,
}

/// Platform-agnostic cursor style matching CSS cursor property values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorStyle {
    Auto,
    Default,
    Pointer,
    Text,
    Move,
    NotAllowed,
    Crosshair,
    Grab,
    Grabbing,
    ColResize,
    RowResize,
    NResize,
    SResize,
    EResize,
    WResize,
    NeResize,
    NwResize,
    SeResize,
    SwResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ZoomIn,
    ZoomOut,
    Wait,
    Progress,
    Help,
    ContextMenu,
    Cell,
    Copy,
    Alias,
    NoDrop,
    AllScroll,
    None,
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

    // Modifier keys (as physical keys, not just modifier state)
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,

    /// A key code not covered by the common set.
    /// The platform backend may include additional info in the event's `text` field.
    Other,
}
