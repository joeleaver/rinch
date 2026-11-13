//! Element types and component traits.

use std::any::Any;
use std::rc::Rc;
use std::sync::Arc;

/// Trait for extensible components.
///
/// Components render directly to DOM nodes via RenderScope for fine-grained updates.
///
/// # Example
///
/// ```ignore
/// use rinch_core::{Component, Callback};
/// use rinch_core::dom::{RenderScope, NodeHandle};
///
/// #[derive(Debug, Default)]
/// pub struct MyButton {
///     pub label: Option<String>,
///     pub onclick: Option<Callback>,
/// }
///
/// impl Component for MyButton {
///     fn render(&self, scope: &mut RenderScope, children: &[NodeHandle]) -> NodeHandle {
///         let btn = scope.create_element("button");
///         btn.set_attribute("class", "my-button");
///
///         if let Some(cb) = &self.onclick {
///             let handler_id = scope.register_handler({
///                 let cb = cb.clone();
///                 move || cb.invoke()
///             });
///             btn.set_attribute("data-rid", &handler_id.0.to_string());
///         }
///
///         for child in children {
///             btn.append_child(child);
///         }
///
///         btn
///     }
/// }
/// ```
pub trait Component: std::fmt::Debug + 'static {
    /// Render this component directly to DOM nodes.
    ///
    /// Creates DOM structure via RenderScope and returns the root NodeHandle.
    /// This is called once per component instance - use Effects for reactive updates.
    ///
    /// # Arguments
    /// * `scope` - The render scope for creating DOM nodes and effects
    /// * `children` - Child nodes already rendered to NodeHandles
    ///
    /// # Returns
    /// A handle to the root DOM node created by this component
    fn render(
        &self,
        scope: &mut crate::dom::RenderScope,
        children: &[crate::dom::NodeHandle],
    ) -> crate::dom::NodeHandle;
}

/// A node in the UI tree.
///
/// Element is now minimal - just content types that can be rendered to DOM.
/// Shell constructs (windows, menus, themes) use their Props types directly.
#[derive(Clone)]
pub enum Element {
    /// Raw HTML content to be rendered by the DOM backend.
    Html(String),
    /// A fragment containing multiple children.
    Fragment(Children),
    /// An extensible component from any crate implementing the Component trait.
    Component(Rc<dyn Component>, Children),
}

impl Element {
    /// Get a short string describing the type of this element.
    pub fn type_name(&self) -> &'static str {
        match self {
            Element::Html(_) => "Html",
            Element::Fragment(_) => "Fragment",
            Element::Component(_, _) => "Component",
        }
    }
}

/// A reactive value that can be either static or dynamic.
///
/// Use this for component props that should support fine-grained reactivity.
/// When a `Dynamic` value is used, components should create Effects to track changes.
///
/// # Example
///
/// ```ignore
/// pub struct Checkbox {
///     pub checked: Reactive<bool>,
/// }
///
/// // Static value (captured once)
/// Checkbox { checked: Reactive::Static(true) }
///
/// // Dynamic value (tracks signal changes)
/// Checkbox { checked: Reactive::Dynamic(Rc::new(move || signal.get())) }
/// ```
#[derive(Clone)]
pub enum Reactive<T: Clone + 'static> {
    /// A static value that doesn't change.
    Static(T),
    /// A dynamic value computed from a closure (typically reading signals).
    Dynamic(Rc<dyn Fn() -> T>),
}

impl<T: Clone + 'static> Reactive<T> {
    /// Get the current value.
    pub fn get(&self) -> T {
        match self {
            Reactive::Static(v) => v.clone(),
            Reactive::Dynamic(f) => f(),
        }
    }

    /// Returns true if this is a dynamic reactive value.
    pub fn is_dynamic(&self) -> bool {
        matches!(self, Reactive::Dynamic(_))
    }

    /// Get the closure if this is dynamic, for creating Effects.
    pub fn as_dynamic(&self) -> Option<&Rc<dyn Fn() -> T>> {
        match self {
            Reactive::Dynamic(f) => Some(f),
            Reactive::Static(_) => None,
        }
    }
}

impl<T: Clone + 'static + std::fmt::Debug> std::fmt::Debug for Reactive<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reactive::Static(v) => write!(f, "Static({:?})", v),
            Reactive::Dynamic(_) => write!(f, "Dynamic(<fn>)"),
        }
    }
}

impl<T: Clone + 'static + Default> Default for Reactive<T> {
    fn default() -> Self {
        Reactive::Static(T::default())
    }
}

// Convenience conversions
impl<T: Clone + 'static> From<T> for Reactive<T> {
    fn from(value: T) -> Self {
        Reactive::Static(value)
    }
}

impl From<Rc<dyn Fn() -> bool>> for Reactive<bool> {
    fn from(f: Rc<dyn Fn() -> bool>) -> Self {
        Reactive::Dynamic(f)
    }
}

impl From<Rc<dyn Fn() -> String>> for Reactive<String> {
    fn from(f: Rc<dyn Fn() -> String>) -> Self {
        Reactive::Dynamic(f)
    }
}

/// Unique identifier for an event handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandlerId(pub u64);

pub type Children = Vec<Element>;

/// Trait for types that can be used as content in rsx expressions.
///
/// This allows both `Element` values and text types to be embedded in rsx:
/// ```ignore
/// let section = overview_section(); // Returns Element
/// let count = 42;
///
/// rsx! {
///     div {
///         {section}  // Element passed through directly
///         {count}    // Converted to Html via Display
///     }
/// }
/// ```
pub trait IntoElement {
    fn into_element(self) -> Element;
}

/// Trait for types that can be rendered into the DOM.
///
/// This trait enables both `Element` values and text-like types to be
/// rendered into the DOM adapter. It's the fine-grained equivalent of
/// `IntoElement`.
///
/// - For `Element`: Recursively renders the element tree to DOM nodes
/// - For `String`/`&str`/numbers: Creates a text node
pub trait IntoDom {
    /// Render this value into the DOM, appending to the given parent.
    fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle);
}

impl IntoElement for Element {
    #[inline]
    fn into_element(self) -> Element {
        self
    }
}

impl IntoElement for String {
    #[inline]
    fn into_element(self) -> Element {
        Element::Html(self)
    }
}

impl IntoElement for &str {
    #[inline]
    fn into_element(self) -> Element {
        Element::Html(self.to_string())
    }
}

impl IntoElement for std::borrow::Cow<'_, str> {
    #[inline]
    fn into_element(self) -> Element {
        Element::Html(self.into_owned())
    }
}

// Implement for common numeric types
macro_rules! impl_into_element_for_display {
    ($($ty:ty),*) => {
        $(
            impl IntoElement for $ty {
                #[inline]
                fn into_element(self) -> Element {
                    Element::Html(self.to_string())
                }
            }
        )*
    };
}

impl_into_element_for_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
);

// IntoDom implementations

impl IntoDom for Element {
    fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
        render_element_to_dom(self, scope, parent);
    }
}

impl IntoDom for String {
    #[inline]
    fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
        let text = scope.create_text(self);
        parent.append_child(&text);
    }
}

impl IntoDom for &str {
    #[inline]
    fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
        let text = scope.create_text(self);
        parent.append_child(&text);
    }
}

impl IntoDom for std::borrow::Cow<'_, str> {
    #[inline]
    fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
        let text = scope.create_text(self.as_ref());
        parent.append_child(&text);
    }
}

// Implement IntoDom for numeric types
macro_rules! impl_into_dom_for_display {
    ($($ty:ty),*) => {
        $(
            impl IntoDom for $ty {
                #[inline]
                fn render_to_dom(&self, scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
                    let text = scope.create_text(&self.to_string());
                    parent.append_child(&text);
                }
            }
        )*
    };
}

impl_into_dom_for_display!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64, bool, char
);

impl IntoDom for crate::dom::NodeHandle {
    #[inline]
    fn render_to_dom(&self, _scope: &mut crate::dom::RenderScope, parent: &crate::dom::NodeHandle) {
        parent.append_child(self);
    }
}

/// Recursively render an Element tree into the DOM.
fn render_element_to_dom(
    element: &Element,
    scope: &mut crate::dom::RenderScope,
    parent: &crate::dom::NodeHandle,
) {
    match element {
        Element::Html(html) => {
            // Parse HTML and insert into DOM
            // Try parse_html first, get the ID (drops borrow), then append
            let parsed_id = scope
                .doc_weak()
                .upgrade()
                .and_then(|doc| doc.borrow_mut().parse_html(html));
            if let Some(parsed_id) = parsed_id {
                let parsed_handle = crate::dom::NodeHandle::new(parsed_id, scope.doc_weak());
                parent.append_child(&parsed_handle);
                return;
            }
            // Fallback: create text node with the HTML content
            let span = scope.create_element("span");
            let text = scope.create_text(html);
            span.append_child(&text);
            parent.append_child(&span);
        }
        Element::Fragment(children) => {
            for child in children {
                render_element_to_dom(child, scope, parent);
            }
        }
        Element::Component(component, children) => {
            // Render children to NodeHandles first
            let mut child_handles = Vec::with_capacity(children.len());
            let temp_container = scope.create_element("template");
            for child in children {
                render_element_to_dom(child, scope, &temp_container);
            }
            child_handles.extend(temp_container.children());

            // Render component directly to DOM
            let handle = component.render(scope, &child_handles);
            // The scratch container is in no subtree, so nothing else can ever
            // reclaim it (issue #719). After the render, not before: the
            // children it adopted have been re-parented out by then.
            crate::dom::release_scratch_container(scope, &temp_container);
            parent.append_child(&handle);
        }
    }
}

/// Renderer that produces a menu bar DOM node.
///
/// Used by `MenuBarContext` to pass a menu bar renderer from the shell layer
/// (which owns `Menu` types) to components (which can't depend on `rinch`).
pub type MenuBarRenderer = Rc<dyn Fn(&mut crate::dom::RenderScope) -> crate::dom::NodeHandle>;

/// Layout mode for the in-app menu bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuBarLayout {
    /// Menu bar is a separate row below the titlebar (default).
    #[default]
    BelowTitlebar,
    /// Menu items render inline in the titlebar (VS Code style).
    InlineTitlebar,
}

/// Context for in-app menu bar rendering (Linux).
///
/// Set by the shell layer when a borderless window has menus.
/// Consumed by `BorderlessWindow` to render the menu bar between the
/// titlebar and content area.
#[derive(Clone)]
pub struct MenuBarContext {
    pub renderer: MenuBarRenderer,
    /// Height of the menu bar in pixels, used by BorderlessWindow to add
    /// padding-top on the content area.
    pub bar_height: u32,
    /// Layout mode for the menu bar.
    pub layout: MenuBarLayout,
    /// Renderer for just the menu items row (used by InlineTitlebar layout).
    pub items_renderer: Option<MenuBarRenderer>,
    /// Renderer for just the click-outside overlay (used by InlineTitlebar layout).
    pub overlay_renderer: Option<MenuBarRenderer>,
    /// First-frame estimate (px) of the inline menu row's width, used by
    /// BorderlessWindow to size the titlebar spacer that keeps the right
    /// section and window controls clear of the row. The shell makes it
    /// before any document exists, so it cannot measure text; once the row
    /// has been laid out, `BorderlessWindow` sizes the spacer from the row's
    /// measured box instead (issue #529).
    pub spacer_width: u32,
}

/// Callback invoked when the window close button is pressed.
///
/// Return `true` to proceed with closing (exit), or `false` to cancel
/// the close (e.g., to hide the window to the system tray instead).
///
/// # Example
///
/// ```ignore
/// let on_close: CloseRequestCallback = Arc::new(|| {
///     hide_current_window();
///     false // Don't exit, just hide
/// });
/// ```
pub type CloseRequestCallback = Arc<dyn Fn() -> bool + Send + Sync>;

/// What kind of surface a window got (issue #1523): see
/// [`WindowProps::wayland_layer_shell`] and `current_window_surface_kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowSurfaceKind {
    /// An ordinary window: a Windows, macOS or X11 window, or a Wayland
    /// xdg-toplevel.
    Toplevel,
    /// A Wayland `wlr-layer-shell` surface.
    LayerShell,
}

/// Properties for the Window component.
#[derive(Clone)]
pub struct WindowProps {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// The outer top-left on the desktop, logical pixels; the system places
    /// the window when `None`. Taken only where a grip of the window's top
    /// (its title bar, what moves it: 64 x 12 logical pixels, or all of a
    /// smaller window) would be on the connected monitors, so a position
    /// saved on a monitor since unplugged never opens the window out of
    /// reach. On a desktop whose monitors have different scales, the position
    /// is read in the scale of the monitor it falls on, as
    /// `current_window_position` gives it. Ignored on Wayland, except by a
    /// layer-shell window.
    pub x: Option<i32>,
    pub y: Option<i32>,
    /// The monitor `x`/`y` is on, by the platform's name for it (what
    /// `rinch::windows::current_window_restore` answers with the position).
    /// On a desktop whose monitors have different scales a logical position
    /// can name a point on two of them; this monitor is tried first, then
    /// every monitor as without it. Ignored when no connected monitor has
    /// the name.
    pub monitor: Option<String>,
    /// Whether the window is a `wlr-layer-shell` surface on native Wayland
    /// (issue #1523): `None` (the default) makes it one when it is
    /// [`non_activating`](Self::non_activating); `Some(true)` makes any window
    /// one (a focusable overlay: its keyboard is then on-demand);
    /// `Some(false)` never. A shorthand for an overlay's configuration;
    /// [`wayland_layer`](Self::wayland_layer) asks for any other, and wins
    /// when both are given.
    ///
    /// A layer surface is how a Wayland overlay refuses keyboard focus, stays
    /// off the taskbar and above other windows. It has no server decorations,
    /// title bar, minimize, maximize or compositor move/resize: a drag region
    /// (`data-drag-window`, `start_window_drag`) and resize edges
    /// (`resize_inset`, `start_window_resize`) move and resize it through its
    /// margins instead. Its position is relative to its output (see
    /// [`output`](Self::output)). KWin, wlroots compositors (sway, labwc),
    /// Hyprland, niri and COSMIC support it; **GNOME/Mutter does not**, and
    /// there the window is an ordinary xdg-toplevel as before (a warning is
    /// logged, and `current_window_surface_kind()` answers `Toplevel`).
    /// A layer surface has no taskbar entry, so it also does what
    /// [`skip_taskbar`](Self::skip_taskbar) asks, which an xdg-toplevel
    /// cannot. Ignored on X11 (`App::prefer_x11` wins), Windows and macOS.
    pub wayland_layer_shell: Option<bool>,
    /// The whole `wlr-layer-shell` role (issue #2413): on native Wayland,
    /// `Some` makes the window a layer surface with exactly this
    /// configuration — layer, anchors, margins, exclusive zone, keyboard,
    /// output and compositor-assigned size ([`LayerSurface`]) — for a panel,
    /// a dock, a wallpaper, a kiosk or signage surface.
    ///
    /// **It wins over the overlay shorthands.** With `Some`, the window is a
    /// layer surface even when [`wayland_layer_shell`](Self::wayland_layer_shell)
    /// is `Some(false)`; its keyboard is [`LayerSurface::keyboard`] whatever
    /// [`non_activating`](Self::non_activating) says (which still applies on
    /// X11, Windows and macOS); [`level`](Self::level) does not change its
    /// layer, `x`/`y` are not its margins, and [`output`](Self::output) is
    /// ignored for [`LayerSurface::output`]. `None` (the default) leaves the
    /// shorthands as they were.
    ///
    /// The app places such a surface itself, by its anchors and margins:
    /// rinch runs no move or resize gesture on it (`data-drag-window`,
    /// `resize_inset`, `start_window_drag`, `start_window_resize` do nothing)
    /// and `set_current_window_position` is ignored; change it at run time
    /// with `set_current_window_layer`. `set_current_window_size` and
    /// `width`/`height` set the size on the axes the compositor does not
    /// assign. Where the compositor has no layer shell (GNOME/Mutter) the
    /// window is an ordinary xdg-toplevel of `width` x `height`, with one
    /// warning, as for an overlay. Ignored on X11, Windows and macOS.
    pub wayland_layer: Option<LayerSurface>,
    /// The output (monitor) a Wayland layer-shell window is shown on, by its
    /// connector name (`DP-1`, `eDP-1`, `HDMI-A-1`). `None` lets the
    /// compositor choose (usually the focused output). Change it at run time
    /// with `set_current_window_output`. Ignored by every other kind of
    /// window (issue #1523), and by a window with
    /// [`wayland_layer`](Self::wayland_layer), whose surface names its own.
    pub output: Option<String>,
    pub borderless: bool,
    pub resizable: bool,
    pub transparent: bool,
    /// Keep the window above ordinary windows: [`WindowLevel::Floating`].
    /// [`WindowProps::level`] chooses a higher level; this never lowers one
    /// ([`WindowProps::effective_level`]).
    pub always_on_top: bool,
    /// Whether the window first appears maximized (default `false`), at
    /// `width` x `height` (and `x`/`y`) when it is restored. With
    /// `rinch::windows::current_window_restore` read at exit, it brings a
    /// window back as the person left it.
    pub maximized: bool,
    /// Whether the window is on screen when the app starts (default `true`).
    ///
    /// `false` starts the app with no window shown — a tray-only app whose
    /// window (settings, say) appears only on `show_current_window()`. The root
    /// component is mounted at startup either way, at the window's size, so
    /// its effects and stores run while it is hidden; showing it creates the
    /// OS window over that same document. Desktop only (issue #1520).
    pub visible: bool,
    /// The inset from window edges where resize handles are active (in logical pixels).
    ///
    /// For transparent/borderless windows, this should match the CSS padding/margin
    /// used for shadow effects. The resize hit area extends from the window edge
    /// inward by this amount, plus a small grab extension into visible content.
    ///
    /// Set to `None` to disable custom resize handles (use native window chrome).
    /// Default is `Some(8.0)` for borderless windows.
    pub resize_inset: Option<f32>,
    /// PNG data for the window icon (use `include_bytes!`).
    ///
    /// Sets the icon shown in the taskbar and title bar on Windows and Linux (X11).
    /// On Wayland, a `.desktop` file is auto-generated using the `app_id`.
    /// On macOS this is a no-op (use an app bundle with `.icns` instead).
    ///
    /// ```rust,ignore
    /// icon: Some(include_bytes!("../assets/icon.png")),
    /// ```
    pub icon: Option<&'static [u8]>,
    /// Application ID for desktop integration (Linux).
    ///
    /// On Wayland, this sets the `app_id` used by compositors to match `.desktop` files.
    /// On X11, this sets the `WM_CLASS` (instance and class). When not set,
    /// Wayland gets the executable's file name and X11 keeps winit's default
    /// `WM_CLASS` (`argv[0]`'s file name).
    pub app_id: Option<String>,
    /// Callback invoked when the window close button (X) is pressed.
    ///
    /// Return `true` to proceed with closing, or `false` to cancel.
    /// This enables "minimize to tray" patterns where clicking X hides
    /// the window instead of exiting.
    ///
    /// If `None`, the default behavior is to exit the application.
    pub on_close_requested: Option<CloseRequestCallback>,
    /// When true, menu items render inline in the titlebar (VS Code style)
    /// instead of on a separate row below the titlebar.
    pub menu_in_titlebar: bool,
    /// Never take keyboard focus or activation: not when the window is
    /// created, not when it is shown, and not when it is clicked (issue
    /// #1519). For an overlay that sits over another application while the
    /// user keeps typing there — a status pill, a HUD. Pointer clicks still
    /// reach the window's DOM, so it can have buttons; keystrokes do not, so
    /// a text field in it cannot be typed into.
    ///
    /// Windows: `WS_EX_NOACTIVATE` and `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`.
    /// macOS: a non-activating `NSPanel` that does not become key on a click.
    /// X11: `WM_HINTS.input = False`. Wayland: a `wlr-layer-shell` surface
    /// with no keyboard interactivity (issue #1523; see
    /// [`wayland_layer_shell`](Self::wayland_layer_shell)), on compositors
    /// that support it — not GNOME, where it has no effect (a warning is
    /// logged) and `App::prefer_x11(true)` (XWayland) is the way.
    pub non_activating: bool,
    /// Let pointer events pass through to the window underneath. Change it
    /// at run time with `set_click_through_current_window`; the state is kept
    /// across hide/show. Windows, macOS and Wayland through winit's hit test;
    /// on X11 rinch sets an empty input shape itself.
    pub click_through: bool,
    /// Keep the window off the taskbar (and, on Windows, out of Alt+Tab).
    /// Windows: `WS_EX_TOOLWINDOW`. X11: `_NET_WM_STATE_SKIP_TASKBAR` and
    /// `_SKIP_PAGER`. macOS: nothing to do — a window has no Dock entry of its
    /// own; since #1549 it keeps the window out of window cycling (Cmd-`)
    /// and Mission Control (`ignoresCycle`, `stationary`). **Wayland: no
    /// effect.**
    pub skip_taskbar: bool,
    /// How high the window sits among other windows (issue #1549). `Normal`
    /// (the default) is an ordinary window; `always_on_top: true` is
    /// [`WindowLevel::Floating`]. Only macOS tells the three raised levels
    /// apart; see [`WindowLevel`] for each platform.
    pub level: WindowLevel,
    /// Show the window on every workspace (macOS Space, X11 desktop),
    /// including the Space of another app's fullscreen window (issue #1549).
    /// macOS: `canJoinAllSpaces` and `fullScreenAuxiliary`. X11:
    /// `_NET_WM_DESKTOP = 0xFFFFFFFF` and `_NET_WM_STATE_STICKY`. **Windows
    /// and Wayland: no effect** (Windows has no public API that pins a
    /// window to every virtual desktop).
    pub visible_on_all_workspaces: bool,
}

impl WindowProps {
    /// The level the window is created at: [`WindowProps::level`], raised to
    /// [`WindowLevel::Floating`] by `always_on_top`.
    pub fn effective_level(&self) -> WindowLevel {
        if self.always_on_top && self.level == WindowLevel::Normal {
            WindowLevel::Floating
        } else {
            self.level
        }
    }
}

/// How high a window sits among other windows: [`WindowProps::level`]
/// (issue #1549).
///
/// The names are macOS's window levels, the one platform with several. On
/// the others every raised level is the platform's one "above" state:
///
/// | | `Floating` | `StatusBar` | `ScreenSaver` |
/// |---|---|---|---|
/// | macOS | `NSFloatingWindowLevel` (3) | `NSStatusWindowLevel` (25), above the menu bar | `NSScreenSaverWindowLevel` (1000), above nearly everything |
/// | Windows | `HWND_TOPMOST` | `HWND_TOPMOST` | `HWND_TOPMOST` |
/// | X11 | `_NET_WM_STATE_ABOVE` | `_NET_WM_STATE_ABOVE` | `_NET_WM_STATE_ABOVE` |
/// | Wayland (layer-shell window) | `top` layer | `overlay` layer | `overlay` layer |
/// | Wayland (xdg-toplevel) | no effect | no effect | no effect |
///
/// On native Wayland a level places a `wlr-layer-shell` window (#1523: a
/// `non_activating` one, or `WindowProps::wayland_layer_shell: Some(true)`):
/// `Normal` is in the `top` layer too. An xdg-toplevel cannot ask for any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WindowLevel {
    /// An ordinary window (the default).
    #[default]
    Normal,
    /// Above ordinary windows. What `always_on_top: true` asks for.
    Floating,
    /// Above floating windows and, on macOS, above the menu bar and the Dock.
    StatusBar,
    /// Above everything the system lets an app go above.
    ScreenSaver,
}

/// An output's (monitor's) identity (issue #2658): `rinch::outputs::Output::id`.
///
/// Every output has one, whether or not the platform names it, and no two
/// outputs connected at the same time share one, so it is the key for a
/// window per output. It stays the same for as long as the output stays
/// connected (through a change of mode, scale, position or rotation); an
/// output connected again may come back with another. It means nothing
/// across runs: an app that remembers an output between sessions keeps its
/// name.
///
/// It is the platform's own number for the output: the `wl_output` global's
/// name on Wayland, the RandR CRTC on X11, the `HMONITOR` on Windows, the
/// `CGDirectDisplayID` on macOS. Compare ids; do not read meaning into the
/// number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(u64);

impl OutputId {
    /// The id whose number is `raw`. **For tests**: an app reads real ids
    /// from `outputs()` and never makes one, since the number is the
    /// platform's and means nothing on its own. A test of a component that
    /// takes an output list builds its `Output`s with ids made here, one
    /// number per output:
    ///
    /// ```
    /// use rinch_core::element::OutputId;
    /// let (left, right) = (OutputId::from_raw(1), OutputId::from_raw(2));
    /// assert_ne!(left, right);
    /// assert_eq!(left, OutputId::from_raw(1));
    /// ```
    ///
    /// An id made up this way names no real output: a window placed by it
    /// is not made (see [`LayerSurface::output_id`]).
    pub const fn from_raw(raw: u64) -> Self {
        OutputId(raw)
    }

    /// The platform's number (the shell reads it to place a window).
    #[doc(hidden)]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// The platform's number, so an id can be a `key:` in rsx (which takes any
/// `ToString`) or appear in a log line.
impl std::fmt::Display for OutputId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A Wayland `wlr-layer-shell` surface's configuration (issue #2413):
/// [`WindowProps::wayland_layer`], and `set_current_window_layer` at run time.
///
/// The fields are the protocol's (`zwlr_layer_surface_v1`), in logical
/// pixels. A layer surface is placed by the compositor from its anchors and
/// margins, never by a position:
///
/// - Anchored to one edge, it sits against that edge, centred along it.
/// - Anchored to two adjacent edges, it sits in that corner.
/// - Anchored to both edges of an axis, it is centred on that axis, or — with
///   [`compositor_size`](Self::compositor_size) on that axis — stretched to
///   the output's extent less the margins.
/// - Anchored to none, it is centred on the output.
///
/// A full-width bar at the bottom of its output that keeps maximised windows
/// above it:
///
/// ```
/// use rinch_core::element::{LayerAnchor, LayerAxes, LayerExclusiveZone, LayerSurface};
/// let bar = LayerSurface {
///     anchor: LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT,
///     exclusive_zone: LayerExclusiveZone::Reserve(32),
///     compositor_size: LayerAxes::WIDTH,
///     ..Default::default()
/// };
/// // With `WindowProps { height: 32, wayland_layer: Some(bar), .. }`.
/// # let _ = bar;
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerSurface {
    /// The layer it is stacked in.
    pub layer: LayerShellLayer,
    /// The output edges it is anchored to.
    pub anchor: LayerAnchor,
    /// Distances from the anchored edges (a margin on an edge it is not
    /// anchored to has no effect).
    pub margins: LayerMargins,
    /// What it does with the edge it is anchored to, and with other
    /// surfaces' reserved space.
    pub exclusive_zone: LayerExclusiveZone,
    /// Whether it takes keyboard focus.
    pub keyboard: LayerKeyboard,
    /// The output, by connector name (`DP-1`, `eDP-1`, as `wlr-randr` or
    /// `kscreen-doctor -o` list them); `None` lets the compositor choose. An
    /// unknown name is the compositor's choice too. The protocol fixes a
    /// surface's output when it is made, so a change at run time makes the
    /// surface again on the new output.
    pub output: Option<String>,
    /// The output by its [`OutputId`] (`rinch::outputs::Output::id`), which
    /// every output has, named or not; it wins over [`output`](Self::output).
    /// It is how a surface goes on an output the platform gives no name (a
    /// Wayland `wl_output` older than version 4; issue #2658). A change at
    /// run time makes the surface again, as a new `output` does.
    ///
    /// **An id no output has is never the compositor's choice.** An id comes
    /// from `outputs()`, so an unknown one is an output that went away between
    /// the list and the surface (for a window per output, the compositor's
    /// choice would be a second wallpaper or panel on an output that has its
    /// own). Such a window is not made: its handle reports closed
    /// (`is_open()` false, gone from `windows()`), the `Window` element's
    /// `on_closed` is called once, `on_close_requested` is never asked, and a
    /// warning is logged. A window given one at run time
    /// (`set_current_window_layer`) closes the same way. An unknown *name*
    /// stays the compositor's choice.
    pub output_id: Option<OutputId>,
    /// The axes whose size the compositor assigns (the protocol's size 0):
    /// the width needs [`LayerAnchor::LEFT`] and [`LayerAnchor::RIGHT`], the
    /// height [`LayerAnchor::TOP`] and [`LayerAnchor::BOTTOM`]. The window's
    /// content lays out to the size the compositor gives, and again when it
    /// changes (a new output mode, other surfaces' exclusive zones).
    /// `WindowProps::width`/`height` is then only the first frame's size on
    /// that axis. Asked on an axis not anchored to both edges, where the
    /// protocol forbids 0, it is not sent and the window's own size is used
    /// (a warning is logged).
    pub compositor_size: LayerAxes,
}

/// The layer a [`LayerSurface`] is stacked in, bottom to top. Ordinary
/// windows are between `Bottom` and `Top`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerShellLayer {
    /// Below everything: a wallpaper.
    Background,
    /// Above the background, below ordinary windows: desktop widgets.
    Bottom,
    /// Above ordinary windows: panels, docks, overlays (the default).
    #[default]
    Top,
    /// Above everything, fullscreen windows included: lock screens,
    /// notifications.
    Overlay,
}

/// The output edges a [`LayerSurface`] is anchored to: any combination,
/// built with `|` (`LayerAnchor::BOTTOM | LayerAnchor::LEFT`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerAnchor {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl LayerAnchor {
    /// No edge: centred on the output.
    pub const NONE: Self = Self {
        top: false,
        bottom: false,
        left: false,
        right: false,
    };
    pub const TOP: Self = Self {
        top: true,
        ..Self::NONE
    };
    pub const BOTTOM: Self = Self {
        bottom: true,
        ..Self::NONE
    };
    pub const LEFT: Self = Self {
        left: true,
        ..Self::NONE
    };
    pub const RIGHT: Self = Self {
        right: true,
        ..Self::NONE
    };
    /// All four edges: a wallpaper or a kiosk surface, with
    /// [`LayerAxes::BOTH`].
    pub const ALL: Self = Self {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
}

impl std::ops::BitOr for LayerAnchor {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self {
            top: self.top || rhs.top,
            bottom: self.bottom || rhs.bottom,
            left: self.left || rhs.left,
            right: self.right || rhs.right,
        }
    }
}

/// A [`LayerSurface`]'s distances from the output edges it is anchored to,
/// logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerMargins {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

/// What a [`LayerSurface`] does with the space at the edge it is anchored to
/// (the protocol's exclusive zone).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerExclusiveZone {
    /// Reserve nothing, and keep clear of the space other surfaces reserve
    /// (the protocol's 0; the default).
    #[default]
    KeepClear,
    /// Reserve nothing, and extend under other surfaces' reserved space (the
    /// protocol's -1): a wallpaper, a full-output overlay.
    Ignore,
    /// Reserve this many logical pixels from the anchored edge, which
    /// ordinary windows (a maximised one) and other `KeepClear` surfaces
    /// then stay out of: a panel's thickness. The compositor honours it only
    /// for a surface anchored to one edge, or to one edge and both of its
    /// neighbours (a bar).
    Reserve(u32),
}

impl LayerExclusiveZone {
    /// The protocol's value.
    pub fn protocol_value(self) -> i32 {
        match self {
            Self::KeepClear => 0,
            Self::Ignore => -1,
            Self::Reserve(px) => i32::try_from(px).unwrap_or(i32::MAX),
        }
    }
}

/// Whether a [`LayerSurface`] takes keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LayerKeyboard {
    /// Never (the default): a panel of buttons, a wallpaper, an indicator.
    #[default]
    None,
    /// The compositor's usual way, a click. Needs `zwlr_layer_shell_v1`
    /// version 4; below it the surface takes no focus (a warning is logged).
    OnDemand,
    /// All keyboard input while it is mapped in the `Top` or `Overlay`
    /// layer: a launcher, a lock screen.
    Exclusive,
}

/// Axes of a [`LayerSurface`]: [`LayerSurface::compositor_size`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerAxes {
    pub width: bool,
    pub height: bool,
}

impl LayerAxes {
    pub const NONE: Self = Self {
        width: false,
        height: false,
    };
    pub const WIDTH: Self = Self {
        width: true,
        height: false,
    };
    pub const HEIGHT: Self = Self {
        width: false,
        height: true,
    };
    pub const BOTH: Self = Self {
        width: true,
        height: true,
    };
}

impl LayerSurface {
    /// The axes the compositor assigns: asked for in
    /// [`compositor_size`](Self::compositor_size) and anchored to both
    /// edges.
    pub fn assigned_axes(&self) -> LayerAxes {
        let a = self.anchor;
        LayerAxes {
            width: self.compositor_size.width && a.left && a.right,
            height: self.compositor_size.height && a.top && a.bottom,
        }
    }
}

impl Default for WindowProps {
    fn default() -> Self {
        Self {
            title: String::from("Rinch Window"),
            width: 800,
            height: 600,
            x: None,
            y: None,
            wayland_layer_shell: None,
            wayland_layer: None,
            output: None,
            borderless: false,
            resizable: true,
            transparent: false,
            always_on_top: false,
            maximized: false,
            monitor: None,
            visible: true,
            resize_inset: None,
            icon: None,
            app_id: None,
            on_close_requested: None,
            menu_in_titlebar: false,
            non_activating: false,
            click_through: false,
            skip_taskbar: false,
            level: WindowLevel::Normal,
            visible_on_all_workspaces: false,
        }
    }
}

impl std::fmt::Debug for WindowProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowProps")
            .field("title", &self.title)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("maximized", &self.maximized)
            .field("monitor", &self.monitor)
            .field("wayland_layer_shell", &self.wayland_layer_shell)
            .field("wayland_layer", &self.wayland_layer)
            .field("output", &self.output)
            .field("borderless", &self.borderless)
            .field("transparent", &self.transparent)
            .field("non_activating", &self.non_activating)
            .field("click_through", &self.click_through)
            .field("skip_taskbar", &self.skip_taskbar)
            .field("level", &self.level)
            .field("visible_on_all_workspaces", &self.visible_on_all_workspaces)
            .field(
                "on_close_requested",
                &self.on_close_requested.as_ref().map(|_| "Fn() -> bool"),
            )
            .finish()
    }
}

/// Properties for the ThemeProvider component.
#[derive(Debug, Clone, Default)]
pub struct ThemeProviderProps {
    /// The primary color name (e.g., "blue", "cyan", "red").
    pub primary_color: Option<String>,
    /// The default border radius size (e.g., "xs", "sm", "md", "lg", "xl").
    pub default_radius: Option<String>,
    /// The primary font family.
    pub font_family: Option<String>,
    /// The monospace font family.
    pub font_family_monospace: Option<String>,
    /// Whether to enable dark mode.
    pub dark_mode: bool,
    /// The primary shade index (0-9).
    pub primary_shade: Option<u8>,
}

/// An item in a For loop with its key and data.
#[derive(Clone)]
pub struct ForItem {
    /// Unique key for this item (used for reconciliation).
    pub key: String,
    /// The actual data for this item.
    pub data: Rc<dyn Any>,
}

impl ForItem {
    /// Create a new ForItem with the given key and data.
    pub fn new<T: Any + 'static>(key: impl Into<String>, data: T) -> Self {
        Self {
            key: key.into(),
            data: Rc::new(data),
        }
    }

    /// Try to downcast the data to a specific type.
    pub fn downcast<T: 'static>(&self) -> Option<&T> {
        self.data.downcast_ref::<T>()
    }

    /// Create ForItems from an iterator with a key function.
    ///
    /// This is the recommended way to build `Vec<ForItem>` for the `For` component.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let items = ForItem::from_iter(todos.get(), |t| t.id.to_string());
    /// ```
    pub fn from_iter<I, T, K>(iter: I, key_fn: K) -> Vec<ForItem>
    where
        I: IntoIterator<Item = T>,
        T: Any + 'static,
        K: Fn(&T) -> String,
    {
        iter.into_iter()
            .map(|item| {
                let key = key_fn(&item);
                ForItem::new(key, item)
            })
            .collect()
    }
}

impl std::fmt::Debug for ForItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForItem").field("key", &self.key).finish()
    }
}

/// Callback type for component events.
///
/// Uses `Rc` for `Clone` support, allowing callbacks to be stored and invoked.
#[derive(Clone)]
pub struct Callback(pub Rc<dyn Fn()>);

impl Callback {
    /// Create a new callback from a function.
    pub fn new<F: Fn() + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback.
    ///
    /// Runs untracked (issue #285): called from inside an effect, the
    /// handler's signal reads do not subscribe that effect.
    pub fn invoke(&self) {
        crate::reactive::untracked_handler(|| (self.0)())
    }
}

impl<F: Fn() + 'static> From<F> for Callback {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callback(...)")
    }
}

/// Callback type for components that pass a value (like sliders, number inputs).
///
/// Uses `Rc` for `Clone` support, allowing callbacks to be stored and invoked.
#[derive(Clone)]
pub struct ValueCallback<T>(pub Rc<dyn Fn(T)>);

impl<T> ValueCallback<T> {
    /// Create a new value callback from a function.
    pub fn new<F: Fn(T) + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback with a value.
    ///
    /// Runs untracked (issue #285): called from inside an effect, the
    /// handler's signal reads do not subscribe that effect.
    pub fn invoke(&self, value: T) {
        crate::reactive::untracked_handler(|| (self.0)(value))
    }
}

impl<T: 'static, F: Fn(T) + 'static> From<F> for ValueCallback<T> {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl<T> std::fmt::Debug for ValueCallback<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ValueCallback(...)")
    }
}

// ── IntoEventHandler trait ──────────────────────────────────────────────────

/// Trait for converting event handlers into component field values.
///
/// Used by the RSX macro for `on*` prop assignments. Supports both direct types
/// (e.g., `Callback`) and optional types (e.g., `Option<Callback>`) as field targets,
/// so custom components can use `on_toggle: Callback` while built-in components
/// continue to use `Option<Callback>`.
pub trait IntoEventHandler<T> {
    fn into_event_handler(self) -> T;
}

/// Blanket: any `IntoEventHandler<T>` also works for `Option<T>`.
impl<V, T> IntoEventHandler<Option<T>> for V
where
    V: IntoEventHandler<T>,
{
    fn into_event_handler(self) -> Option<T> {
        Some(IntoEventHandler::<T>::into_event_handler(self))
    }
}

// ── Callback impls ──

impl IntoEventHandler<Callback> for Callback {
    fn into_event_handler(self) -> Callback {
        self
    }
}

impl<F: Fn() + 'static> IntoEventHandler<Callback> for F {
    fn into_event_handler(self) -> Callback {
        Callback::from(self)
    }
}

// ── ValueCallback<T> impls ──

impl<T: 'static> IntoEventHandler<ValueCallback<T>> for ValueCallback<T> {
    fn into_event_handler(self) -> ValueCallback<T> {
        self
    }
}

impl<T: 'static, F: Fn(T) + 'static> IntoEventHandler<ValueCallback<T>> for F {
    fn into_event_handler(self) -> ValueCallback<T> {
        ValueCallback::from(self)
    }
}
