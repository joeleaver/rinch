//! Rinch - A lightweight cross-platform GUI library for Rust.
//!
//! Rinch provides a reactive GUI framework built on top of blitz,
//! using HTML/CSS for layout and Vello for rendering.
//!
//! # Quick Start
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app() -> Element {
//!     rsx! {
//!         Window { title: "Hello Rinch", width: 800, height: 600,
//!             h1 { "Hello, World!" }
//!         }
//!     }
//! }
//!
//! fn main() {
//!     rinch::run(app);
//! }
//! ```
//!
//! # State Management with Hooks
//!
//! Rinch provides React-style hooks for managing state across renders.
//! See the [`rinch_core::hooks`] module for comprehensive documentation.
//!
//! ## Available Hooks
//!
//! | Hook | Purpose |
//! |------|---------|
//! | [`use_signal`] | Reactive state that triggers re-renders |
//! | [`use_state`] | Simple state with `(value, setter)` tuple |
//! | [`use_ref`] | Mutable reference (doesn't trigger re-renders) |
//! | [`use_effect`] | Side effects when dependencies change |
//! | [`use_effect_cleanup`] | Effects with cleanup functions |
//! | [`use_mount`] | One-time effect on first render |
//! | [`use_memo`] | Memoized expensive computations |
//! | [`use_callback`] | Memoized callbacks |
//!
//! ## Example with State
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app() -> Element {
//!     // Create reactive state with use_signal
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::from("World"));
//!
//!     // Clone for use in event handlers
//!     let count_inc = count.clone();
//!
//!     rsx! {
//!         Window { title: "Counter", width: 800, height: 600,
//!             div {
//!                 h1 { "Hello, " {name.get()} "!" }
//!                 p { "Count: " {count.get()} }
//!                 button {
//!                     onclick: move || count_inc.update(|n| *n += 1),
//!                     "Increment"
//!                 }
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! ## Rules of Hooks
//!
//! Hooks must be called in the **same order** on every render:
//!
//! - ✅ Call hooks at the top level of your app function
//! - ❌ Don't call hooks inside conditionals (`if`/`match`)
//! - ❌ Don't call hooks inside loops
//! - ❌ Don't call hooks after early returns
//! - ❌ Don't call hooks in event handlers
//!
//! See [`rinch_core::hooks`] for detailed documentation and examples.
//!
//! [`use_signal`]: prelude::use_signal
//! [`use_state`]: prelude::use_state
//! [`use_ref`]: prelude::use_ref
//! [`use_effect`]: prelude::use_effect
//! [`use_effect_cleanup`]: prelude::use_effect_cleanup
//! [`use_mount`]: prelude::use_mount
//! [`use_memo`]: prelude::use_memo
//! [`use_callback`]: prelude::use_callback

pub mod app;
pub mod menu;
pub mod shell;
pub mod window;
pub mod windows;

#[cfg(feature = "file-dialogs")]
pub mod dialogs;

#[cfg(feature = "clipboard")]
pub mod clipboard;

#[cfg(feature = "system-tray")]
pub mod tray;

pub mod prelude {
    //! Common imports for rinch applications.
    pub use crate::shell::run;
    pub use rinch_core::element::*;
    pub use rinch_core::{batch, derived, untracked, Effect, Memo, Scope, Signal};
    // Hooks for ergonomic state management
    pub use rinch_core::{
        create_context, use_callback, use_context, use_derived, use_effect, use_effect_cleanup,
        use_memo, use_mount, use_ref, use_signal, use_state, RefHandle,
    };
    pub use rinch_macros::rsx;
    // Window control functions
    pub use crate::windows::{
        close_current_window, minimize_current_window, toggle_maximize_current_window,
    };
}

// Re-export core types at crate root
pub use rinch_core::element::{
    AppMenuProps, Children, Element, MenuItemProps, MenuProps, WindowProps,
};
pub use rinch_core::{batch, derived, untracked, Effect, Memo, Scope, Signal};
pub use rinch_macros::rsx;
pub use shell::run;
#[cfg(feature = "hot-reload")]
pub use shell::run_with_hot_reload;

pub use rinch_core as core;
pub use rinch_renderer as renderer;
