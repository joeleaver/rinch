//! Platform-agnostic event types.

use std::path::PathBuf;

/// A platform-agnostic input event.
///
/// Platform backends translate their native events into these variants.
/// The rinch runtime processes these without any platform-specific knowledge.
///
/// # Coordinate space
///
/// **Every pointer coordinate on this enum is in logical (CSS) pixels, on every
/// host** — `MouseMove`/`MouseDown`/`MouseUp`/`MouseWheel`, the `position` on
/// the `File*` variants, and `MouseWheel`'s `delta_x`/`delta_y`. That is the
/// space the document is laid out in and the space `hit_test` probes, so a
/// backend whose windowing system reports physical pixels must divide before
/// constructing the event — [`crate::to_logical_point`] is the shared
/// conversion. (`Resized` carries the logical viewport for the same reason;
/// `RinchApp::handle_event`'s separate `window_size` argument is the one
/// genuinely physical quantity, and it exists for the shell's own surface
/// arithmetic.)
///
/// Forwarding physical coordinates instead displaces every click, hover,
/// drag and scroll by the scale factor times its distance from the window
/// origin — which is exactly what issue #299 was.
///
/// `#[non_exhaustive]`: a new variant can be added in a minor release without
/// that being a breaking change for downstream code. Any `match` on this enum
/// outside `rinch-platform` must carry a wildcard (`_`) arm.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum PlatformEvent {
    /// The application has been resumed (window ready for rendering).
    Resumed,
    /// The window close button was pressed.
    CloseRequested,
    /// The window was resized. `width`/`height` are the new **logical**
    /// (CSS-pixel) viewport — the size the document is laid out at — not the
    /// physical surface size. See [`crate::to_logical`].
    Resized { width: u32, height: u32 },
    /// A redraw was requested.
    RedrawRequested,
    /// Mouse cursor moved. `x`/`y` are **logical** pixels (see the type's
    /// *Coordinate space* note).
    MouseMove { x: f32, y: f32 },
    /// Mouse button pressed. `x`/`y` are **logical** pixels.
    MouseDown { x: f32, y: f32, button: MouseButton },
    /// Mouse button released. `x`/`y` are **logical** pixels.
    MouseUp { x: f32, y: f32, button: MouseButton },
    /// Mouse wheel scrolled. `x`/`y` are the **logical** pointer position and
    /// `delta_x`/`delta_y` are a **logical**-pixel scroll distance — a backend
    /// reporting a physical pixel delta must divide it too, or a HiDPI wheel
    /// scrolls `scale` times too far.
    MouseWheel {
        x: f32,
        y: f32,
        delta_x: f64,
        delta_y: f64,
    },
    /// The pointer interaction in flight was taken away, and must not be
    /// completed.
    ///
    /// The native twin of the `pointercancel` the web backend already listens
    /// for (`rinch-web`'s `event_delegation`), and the event `Drag::cancel` was
    /// written for. Everything a press is holding — a pending click, an element
    /// drag, a pointer-capture drag, a scrollbar or text-selection drag, the
    /// `:active` style — is released *without* the click, drop or `on_end`
    /// commit that a [`MouseUp`](Self::MouseUp) would carry.
    ///
    /// Android's touch recogniser sends it when a press it had not yet resolved
    /// becomes a scroll: the finger is still down and still moving, but whatever
    /// the document thought was being pressed is not being pressed any more.
    ///
    /// It carries no position on purpose. A cancel is not a place — nothing it
    /// tears down is hit-tested, and the coordinates a browser puts on
    /// `pointercancel` would only invite treating it as a release.
    PointerCancel,
    /// Key pressed.
    KeyDown {
        /// The **physical** key (layout-independent position).
        key: KeyCode,
        /// The **logical** key value this key produces in the active layout,
        /// spelled exactly as a browser spells [`KeyboardEvent.key`] — so the
        /// desktop shell reports the same strings `rinch-web` forwards from the
        /// browser, and a press and its release spell alike (issue #337):
        ///
        /// - a printable key is the string it produces, **case-accurate**
        ///   (`"a"`, `"A"` under Shift, `"!"` where the layout puts one, `"é"`)
        ///   and layout-mapped (`"b"` for the Dvorak/AZERTY key labelled B) —
        ///   unlike [`text`](Self::KeyDown::text), it survives a `Ctrl`/`Cmd`
        ///   chord, which is why `Mod`+letter shortcuts read it;
        /// - a named key is its W3C [key-values] name (`"Enter"`, `"ArrowLeft"`,
        ///   `"Shift"`, `"Meta"` for the OS key) — bar the spacebar, which the
        ///   spec deliberately spells as its character, `" "`;
        /// - a dead key is `"Dead"`, as in a browser;
        /// - `None` when the backend cannot tell (the debug channel's named
        ///   keys, Android's hardware-key path outside what its character map
        ///   resolves, a key winit itself reports as unidentified). The
        ///   consumer then falls back to `key`.
        ///
        /// **Case is identity here**: consumers wanting a case-insensitive
        /// match (`Mod+B` on `"B"` or `"b"`) must fold at the comparison site,
        /// as `editor_key_binding` does — a lowercased source made a `Shift`
        /// chord's release disagree with its own press, which defeats pairing
        /// them, the thing `KeyUp` exists for.
        ///
        /// [`KeyboardEvent.key`]: https://developer.mozilla.org/docs/Web/API/KeyboardEvent/key
        /// [key-values]: https://w3c.github.io/uievents-key/
        logical_key: Option<String>,
        /// The text this keypress would insert (suppressed under Ctrl/Cmd), or `None`.
        text: Option<String>,
        modifiers: Modifiers,
    },
    /// Key released.
    ///
    /// Carries `logical_key` for the same reason [`KeyDown`](Self::KeyDown)
    /// does, and it is load-bearing rather than cosmetic (issue #337): a
    /// consumer pairs a press with its release by comparing the key string, so
    /// if a release resolved the *physical* letter while its press resolved the
    /// *layout* letter, the pairing would silently never match — on AZERTY a
    /// press of `"a"` would release as `"q"`, and the key would look held for
    /// ever. winit's `KeyEvent` is one struct for both states and populates
    /// `logical_key` on each; only `text` is press-gated.
    KeyUp {
        /// The **physical** key (layout-independent position).
        key: KeyCode,
        /// The **logical** key value this key produces in the active layout,
        /// spelled like the browser's `KeyboardEvent.key` — see
        /// [`KeyDown::logical_key`](Self::KeyDown).
        logical_key: Option<String>,
        modifiers: Modifiers,
    },
    /// Modifier keys changed.
    ModifiersChanged(Modifiers),
    /// The window gained (`true`) or lost (`false`) **OS** focus.
    ///
    /// Distinct from the in-document focus arbiter: the focused widget **keeps**
    /// its claim across a window blur (browser semantics — alt-tabbing away and
    /// back must not fire `data-onchange` on every field, issue #226). It is
    /// only *notified*, and re-notified when the window comes back. While the
    /// window is blurred the runtime reports IME disabled, so the OS candidate
    /// window follows the window that actually has the keyboard.
    ///
    /// Backends: winit's `WindowEvent::Focused` on desktop, android-activity's
    /// `MainEvent::GainedFocus`/`LostFocus` on Android. The browser backend
    /// (`rinch-web`) has no `PlatformEvent` pump at all — the browser is the
    /// arbiter there and fires its own `focus`/`blur` — so nothing translates
    /// into this variant on web.
    WindowFocus(bool),
    /// Display scale factor changed.
    ScaleFactorChanged(f64),
    /// A user-defined event from the application.
    UserEvent(UserEvent),
    /// The event loop is about to wait for new events.
    AboutToWait,
    /// A file drag entered the window from the OS. `position` is in **logical**
    /// pixels, like every other pointer coordinate here.
    /// In winit 0.31, all paths arrive in a single DragEntered event.
    FileHoverEnter { path: PathBuf, position: (f64, f64) },
    /// A file drag is moving over the window. `position` is **logical**.
    FileDragMoved { position: (f64, f64) },
    /// The OS file drag left the window without dropping.
    FileHoverCancelled,
    /// Files were dropped onto the window from the OS. `position` is
    /// **logical**.
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
/// - **Android:** the `InputConnection` call stream, through
///   `rinch::shell::android_ime`: `setComposingText` → [`ImeEvent::Preedit`],
///   `deleteSurroundingText` → [`ImeEvent::DeleteSurrounding`]. The two calls
///   that *end* a composition (`commitText`, `finishComposingText`) clear the
///   preedit through [`ImeEvent::Preedit`] and then apply their text as key
///   input rather than as [`ImeEvent::Commit`] — see that module for why.
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
    ///
    /// **Android does not convert yet**: `deleteSurroundingText` counts UTF-16
    /// code units and the shell passes them through, so an astral character in
    /// the deleted run costs one deletion too many. The conversion needs the
    /// field's text, which the `InputConnection` does not report.
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
