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

/// Properties for the Window component.
#[derive(Clone)]
pub struct WindowProps {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub borderless: bool,
    pub resizable: bool,
    pub transparent: bool,
    pub always_on_top: bool,
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
    /// On X11, this sets the `WM_CLASS`. Defaults to `"rinch-app"` if not set.
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
}

impl Default for WindowProps {
    fn default() -> Self {
        Self {
            title: String::from("Rinch Window"),
            width: 800,
            height: 600,
            x: None,
            y: None,
            borderless: false,
            resizable: true,
            transparent: false,
            always_on_top: false,
            visible: true,
            resize_inset: None,
            icon: None,
            app_id: None,
            on_close_requested: None,
            menu_in_titlebar: false,
        }
    }
}

impl std::fmt::Debug for WindowProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowProps")
            .field("title", &self.title)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("borderless", &self.borderless)
            .field("transparent", &self.transparent)
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
    pub fn invoke(&self) {
        (self.0)()
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
    pub fn invoke(&self, value: T) {
        (self.0)(value)
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
