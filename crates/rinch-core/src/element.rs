//! Element types and component traits.

use std::any::Any;
use std::rc::Rc;

/// Trait for extensible widgets.
///
/// Widgets render directly to DOM nodes via RenderScope for fine-grained updates.
///
/// # Example
///
/// ```ignore
/// use rinch_core::{Widget, WidgetCallback};
/// use rinch_core::dom::{RenderScope, NodeHandle};
///
/// #[derive(Debug, Default)]
/// pub struct MyButton {
///     pub label: Option<String>,
///     pub onclick: Option<WidgetCallback>,
/// }
///
/// impl Widget for MyButton {
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
pub trait Widget: std::fmt::Debug + 'static {
    /// Render this widget directly to DOM nodes.
    ///
    /// Creates DOM structure via RenderScope and returns the root NodeHandle.
    /// This is called once per widget instance - use Effects for reactive updates.
    ///
    /// # Arguments
    /// * `scope` - The render scope for creating DOM nodes and effects
    /// * `children` - Child nodes already rendered to NodeHandles
    ///
    /// # Returns
    /// A handle to the root DOM node created by this widget
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
    /// An extensible widget from any crate implementing the Widget trait.
    Widget(Rc<dyn Widget>, Children),
}

impl Element {
    /// Get a short string describing the type of this element.
    pub fn type_name(&self) -> &'static str {
        match self {
            Element::Html(_) => "Html",
            Element::Fragment(_) => "Fragment",
            Element::Widget(_, _) => "Widget",
        }
    }
}

/// A reactive value that can be either static or dynamic.
///
/// Use this for widget props that should support fine-grained reactivity.
/// When a `Dynamic` value is used, widgets should create Effects to track changes.
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
        Element::Widget(widget, children) => {
            // Render children to NodeHandles first
            let mut child_handles = Vec::with_capacity(children.len());
            let temp_container = scope.create_element("template");
            for child in children {
                render_element_to_dom(child, scope, &temp_container);
            }
            child_handles.extend(temp_container.children());

            // Render widget directly to DOM
            let handle = widget.render(scope, &child_handles);
            parent.append_child(&handle);
        }
    }
}

/// Properties for the Window component.
#[derive(Debug, Clone)]
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
        }
    }
}

/// Properties for the AppMenu component.
#[derive(Debug, Clone)]
pub struct AppMenuProps {
    /// If true, render as native OS menu. If false, render as HTML.
    pub native: bool,
}

impl Default for AppMenuProps {
    fn default() -> Self {
        Self { native: true }
    }
}

/// Properties for a Menu (dropdown) within AppMenu.
#[derive(Debug, Clone)]
pub struct MenuProps {
    pub label: String,
}

/// Callback type for menu items.
///
/// Uses `Rc` for `Clone` support, allowing callbacks to be stored and invoked.
#[derive(Clone)]
pub struct MenuItemCallback(pub Rc<dyn Fn()>);

impl MenuItemCallback {
    /// Create a new menu item callback from a function.
    pub fn new<F: Fn() + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback.
    pub fn invoke(&self) {
        (self.0)()
    }
}

impl std::fmt::Debug for MenuItemCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MenuItemCallback(...)")
    }
}

/// Properties for a MenuItem.
#[derive(Debug, Clone)]
pub struct MenuItemProps {
    pub label: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// Callback to invoke when the menu item is activated.
    pub onclick: Option<MenuItemCallback>,
}

impl Default for MenuItemProps {
    fn default() -> Self {
        Self {
            label: String::new(),
            shortcut: None,
            enabled: true,
            checked: None,
            onclick: None,
        }
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
}

impl std::fmt::Debug for ForItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForItem").field("key", &self.key).finish()
    }
}

/// Callback type for widget events.
///
/// Uses `Rc` for `Clone` support, allowing callbacks to be stored and invoked.
#[derive(Clone)]
pub struct WidgetCallback(pub Rc<dyn Fn()>);

impl WidgetCallback {
    /// Create a new widget callback from a function.
    pub fn new<F: Fn() + 'static>(f: F) -> Self {
        Self(Rc::new(f))
    }

    /// Invoke the callback.
    pub fn invoke(&self) {
        (self.0)()
    }
}

impl<F: Fn() + 'static> From<F> for WidgetCallback {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl std::fmt::Debug for WidgetCallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WidgetCallback(...)")
    }
}

/// Callback type for widgets that pass a value (like sliders, number inputs).
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
