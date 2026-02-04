//! Rinch - A lightweight cross-platform GUI library for Rust.
//!
//! Rinch provides a reactive GUI framework using HTML/CSS for layout
//! and Vello for rendering. It uses fine-grained reactive rendering -
//! only the affected DOM nodes are updated when signals change.
//!
//! # Quick Start
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn app(__scope: &mut RenderScope) -> NodeHandle {
//!     let count = use_signal(|| 0);
//!     let count_inc = count.clone();
//!
//!     rsx! {
//!         div {
//!             // Closure syntax {|| ...} creates reactive Effects
//!             p { "Count: " {|| count.get().to_string()} }
//!             button {
//!                 onclick: move || count_inc.update(|n| *n += 1),
//!                 "Increment"
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # State Management with Hooks
//!
//! Rinch provides React-style hooks for managing state.
//! See the [`rinch_core::hooks`] module for comprehensive documentation.
//!
//! ## Available Hooks
//!
//! | Hook | Purpose |
//! |------|---------|
//! | [`use_signal`] | Reactive state that triggers effects |
//! | [`use_state`] | Simple state with `(value, setter)` tuple |
//! | [`use_ref`] | Mutable reference (doesn't trigger effects) |
//! | [`use_effect`] | Side effects when dependencies change |
//! | [`use_effect_cleanup`] | Effects with cleanup functions |
//! | [`use_mount`] | One-time effect on first render |
//! | [`use_memo`] | Memoized expensive computations |
//! | [`use_callback`] | Memoized callbacks |
//! | [`use_derived`] | Auto-tracking computed values |
//!
//! ## Example with State
//!
//! ```ignore
//! use rinch::prelude::*;
//!
//! fn counter(__scope: &mut RenderScope) -> NodeHandle {
//!     let count = use_signal(|| 0);
//!     let name = use_signal(|| String::from("World"));
//!     let count_inc = count.clone();
//!
//!     rsx! {
//!         div {
//!             // Reactive text - updates when name changes
//!             h1 { "Hello, " {|| name.get()} "!" }
//!             // Reactive formatted text
//!             p { {|| format!("Count: {}", count.get())} }
//!             button {
//!                 onclick: move || count_inc.update(|n| *n += 1),
//!                 "Increment"
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
//! - Call hooks at the top level of your component function
//! - Don't call hooks inside conditionals (`if`/`match`)
//! - Don't call hooks inside loops
//! - Don't call hooks after early returns
//! - Don't call hooks in event handlers
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
//! [`use_derived`]: prelude::use_derived

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

/// Memory profiling utilities (enable with `memory-profile` feature).
pub mod memory {
    pub use crate::shell::memory_profile::*;
}

/// Fine-grained reactive rendering.
///
/// This module provides primitives for surgical DOM updates where signal
/// changes only affect the specific nodes that depend on them.
pub mod fine_grained {
    pub use crate::update_theme;
    pub use rinch_core::dom::{
        DomDocument, DomUpdate, NodeHandle, NodeId, RenderScope, UpdateBatch, clear_render_scope,
        has_render_scope, set_render_scope, try_with_render_scope, with_render_scope,
    };
}

/// Theme system (enable with `theme` feature).
#[cfg(feature = "theme")]
pub mod theme {
    pub use rinch_theme::*;
}

/// Widget library (enable with `widgets` feature).
#[cfg(feature = "widgets")]
pub mod widgets {
    pub use rinch_widgets::*;
}

pub mod prelude {
    //! Common imports for rinch applications.
    pub use crate::shell::{run, run_with_theme};
    pub use rinch_core::element::*;
    pub use rinch_core::{Effect, Memo, Scope, Signal, batch, derived, untracked};
    // Hooks for ergonomic state management
    pub use rinch_core::{
        RefHandle, create_context, use_callback, use_context, use_derived, use_effect,
        use_effect_cleanup, use_memo, use_mount, use_ref, use_signal, use_state,
    };
    // Event handling - click context, drag support, and input callbacks
    pub use rinch_core::{ClickContext, InputCallback, get_click_context, start_drag};
    // Icon enum for type-safe icons
    pub use rinch_core::Icon;
    pub use rinch_macros::rsx;
    // Window control functions
    pub use crate::windows::{
        close_current_window, minimize_current_window, toggle_maximize_current_window,
    };
    // Fine-grained rendering types
    pub use rinch_core::dom::{
        NodeHandle, RenderScope, has_render_scope, try_with_render_scope, with_render_scope,
    };
    // Widget trait for fine-grained widgets
    pub use rinch_core::Widget;
    // Show for reactive conditional rendering
    pub use rinch_core::{FineShowBuilder, show_dom};
    // For for reactive list rendering
    pub use rinch_core::{FineForBuilder, ForItem, for_each_dom, to_for_items};

    // Re-export theme types when the theme feature is enabled
    #[cfg(feature = "theme")]
    pub use rinch_theme::*;

    // Dynamic theme updates
    #[cfg(feature = "theme")]
    pub use crate::update_theme;

    // Re-export widgets when the widgets feature is enabled
    #[cfg(feature = "widgets")]
    pub use rinch_widgets::*;
}

// Re-export core types at crate root
pub use rinch_core::element::{
    AppMenuProps, Children, Element, MenuItemProps, MenuProps, ThemeProviderProps, WindowProps,
};
pub use rinch_core::{Effect, Memo, Scope, Signal, batch, derived, untracked};
pub use rinch_macros::rsx;
pub use shell::{
    run, run_rinch, run_rinch_with_window_props, run_with_theme, run_with_window_props,
};

pub use rinch_core as core;
pub use rinch_renderer as renderer;

/// Dynamically update theme CSS at runtime.
#[cfg(feature = "theme")]
pub fn update_theme(props: &ThemeProviderProps) {
    use rinch_theme::{ColorName, RadiusSize, Theme, generate_theme_css};

    let mut builder = Theme::builder();

    if let Some(ref color) = props.primary_color
        && let Ok(name) = color.parse::<ColorName>()
    {
        builder = builder.primary_color(name);
    }

    if let Some(ref radius) = props.default_radius
        && let Ok(size) = radius.parse::<RadiusSize>()
    {
        builder = builder.default_radius(size);
    }

    if let Some(ref family) = props.font_family {
        builder = builder.font_family(family.clone());
    }

    if let Some(ref family) = props.font_family_monospace {
        builder = builder.font_family_monospace(family.clone());
    }

    if props.dark_mode {
        builder = builder.dark_mode(true);
    }

    if let Some(shade) = props.primary_shade {
        builder = builder.primary_shade(shade as usize);
    }

    let theme = builder.build();
    let mut css = generate_theme_css(&theme);

    #[cfg(feature = "widgets")]
    {
        css.push_str(&rinch_widgets::generate_widget_css());
    }

    rinch_core::set_current_theme_css(Some(css));
}

/// No-op when theme feature is disabled.
#[cfg(not(feature = "theme"))]
pub fn update_theme(_props: &ThemeProviderProps) {}

/// Set up theme CSS from ThemeProviderProps.
///
/// This is called by the rsx! macro when processing ThemeProvider elements.
/// It ensures the theme CSS is available before Window children build their DOM.
#[cfg(feature = "theme")]
pub fn setup_theme_css(props: &ThemeProviderProps) {
    update_theme(props);
}

/// No-op when theme feature is disabled.
#[cfg(not(feature = "theme"))]
pub fn setup_theme_css(_props: &ThemeProviderProps) {}
