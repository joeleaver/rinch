//! Context system for sharing state across components.
//!
//! Context provides a way to share values across your component tree without
//! explicitly passing them through props.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;

// Thread-local context store for sharing state across components
thread_local! {
    static CONTEXT_STORE: RefCell<HashMap<TypeId, Box<dyn Any>>> = RefCell::new(HashMap::new());
}

/// Create a context value accessible by any component.
///
/// Context provides a way to share values across your component tree without
/// explicitly passing them through props. This is useful for global state like
/// themes, user preferences, or authentication data.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// #[derive(Clone)]
/// struct Theme {
///     primary_color: String,
///     font_size: u32,
/// }
///
/// #[component]
/// fn app() -> NodeHandle {
///     // Create the context at the top of your app
///     let theme = create_context(Theme {
///         primary_color: "#007bff".into(),
///         font_size: 16,
///     });
///
///     rsx! {
///         div {
///             // Child components can access the theme via use_context
///         }
///     }
/// }
///
/// #[component]
/// fn themed_button() -> NodeHandle {
///     // Access the theme from anywhere in the component tree
///     let theme = use_context::<Theme>().expect("Theme context not found");
///
///     rsx! {
///         button { style: {|| format!("color: {}", theme.primary_color)},
///             "Click me"
///         }
///     }
/// }
/// ```
pub fn create_context<T: Clone + 'static>(value: T) -> T {
    CONTEXT_STORE.with(|store| {
        store
            .borrow_mut()
            .insert(TypeId::of::<T>(), Box::new(value.clone()));
    });
    value
}

/// Retrieve a context value by type.
///
/// Returns `Some(value)` if a context of the given type has been created,
/// or `None` if no such context exists.
///
/// # Example
///
/// ```ignore
/// #[derive(Clone)]
/// struct UserContext {
///     username: String,
///     is_admin: bool,
/// }
///
/// #[component]
/// fn user_info() -> NodeHandle {
///     let user = use_context::<UserContext>();
///
///     match user {
///         Some(u) => rsx! { p { "Welcome, " {u.username} } },
///         None => rsx! { p { "Not logged in" } },
///     }
/// }
/// ```
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    CONTEXT_STORE.with(|store| {
        store
            .borrow()
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
    })
}

/// Clear all context (called internally during app reset).
pub(crate) fn clear_context() {
    CONTEXT_STORE.with(|store| store.borrow_mut().clear());
}
