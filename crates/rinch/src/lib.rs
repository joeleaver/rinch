#![allow(clippy::collapsible_if)]
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
//! #[component]
//! fn app() -> NodeHandle {
//!     let count = Signal::new(0);
//!
//!     rsx! {
//!         div {
//!             // Closure syntax {|| ...} creates reactive Effects
//!             p { "Count: " {|| count.get().to_string()} }
//!             button {
//!                 onclick: move || count.update(|n| *n += 1),
//!                 "Increment"
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! # State Management
//!
//! Rinch uses fine-grained reactive primitives for state management.
//! Components run **once** to build the DOM, and reactive Effects
//! handle all subsequent updates surgically.
//!
//! | Primitive | Purpose |
//! |-----------|---------|
//! | [`Signal`] | Reactive state that triggers effects |
//! | [`Effect`] | Side effects that auto-track signal dependencies |
//! | [`Memo`] | Memoized computed values with auto-tracking |
//! | [`derived`] | Shorthand for creating a Memo |
//! | [`create_context`] | Share state across components |
//! | [`use_context`] | Access shared context values |

#[cfg(feature = "desktop")]
pub mod app;
#[cfg(feature = "desktop")]
pub mod ce_ops;
#[cfg(feature = "desktop")]
pub(crate) mod ce_render;
#[cfg(feature = "gpu")]
pub mod embed;
#[cfg(feature = "desktop")]
pub mod menu;
pub mod render_surface;
#[cfg(feature = "desktop")]
pub mod shell;
pub mod window;
#[cfg(feature = "desktop")]
pub mod windows;

#[cfg(feature = "image-network")]
pub mod image_loader;

/// Video playback (enable with `video` feature).
#[cfg(feature = "video")]
pub mod video {
    pub use rinch_video::*;
}

#[cfg(feature = "file-dialogs")]
pub mod dialogs;

#[cfg(feature = "clipboard")]
pub mod clipboard;

#[cfg(feature = "system-tray")]
pub mod tray;

/// Memory profiling utilities (enable with `memory-profile` feature).
#[cfg(feature = "desktop")]
pub mod memory {
    pub use crate::shell::memory_profile::*;
}

/// Fine-grained reactive rendering.
///
/// This module provides primitives for surgical DOM updates where signal
/// changes only affect the specific nodes that depend on them.
pub mod fine_grained {
    #[cfg(feature = "theme")]
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

/// Component library (enable with `components` feature).
#[cfg(feature = "components")]
pub mod components {
    pub use rinch_components::*;
}

pub mod prelude {
    //! Common imports for rinch applications.
    #[cfg(feature = "desktop")]
    pub use crate::shell::{run, run_with_theme};
    pub use rinch_core::element::*;
    pub use rinch_core::{Memo, Scope, Signal, batch, derived, untracked};
    // Context and stores for sharing state across components
    pub use rinch_core::{
        create_context, create_store, try_use_context, try_use_store, use_context, use_store,
    };
    // Event handling - click context, drag support, input callbacks, file-drop callbacks
    pub use rinch_core::{
        ClickContext, Drag, DragContext, FileDropCallback, InputCallback, get_click_context,
        restore_drag_ghost, suppress_drag_ghost,
    };
    pub use rinch_macros::{component, rsx};
    // Window control functions
    #[cfg(feature = "desktop")]
    pub use crate::windows::{
        close_current_window, hide_current_window, minimize_current_window, show_current_window,
        toggle_maximize_current_window,
    };
    // Fine-grained rendering types
    pub use rinch_core::dom::{
        NodeHandle, RenderScope, has_render_scope, try_with_render_scope, with_render_scope,
    };
    // Component trait for fine-grained components
    pub use rinch_core::Component;
    // Show for reactive conditional rendering
    pub use rinch_core::{FineShowBuilder, show_dom};
    // For for reactive list rendering
    pub use rinch_core::{FineForBuilder, ForItem, for_each_dom, to_for_items};
    // Virtual list for large datasets
    pub use rinch_core::virtual_list;

    // Embed API for game engine integration
    #[cfg(feature = "gpu")]
    pub use crate::embed::{
        GameViewport, LayoutRect, RinchContext, RinchContextConfig, RinchOverlayRenderer,
    };

    // Render surface API for embedding external renderers
    pub use crate::render_surface::{
        RenderSurface, RenderSurfaceHandle, SurfaceEvent, SurfaceKeyData, SurfaceMouseButton,
        SurfaceWriter, create_render_surface, invoke_render_callbacks,
    };

    // Desktop-only GPU types for render surfaces
    #[cfg(feature = "gpu")]
    pub use crate::render_surface::{GpuTextureRegistrar, TextureSource};

    // Shared GPU handle for zero-copy compositing
    #[cfg(feature = "gpu")]
    pub use crate::shell::desktop::{GpuHandle, gpu_handle};

    // Re-export theme types when the theme feature is enabled
    #[cfg(feature = "theme")]
    pub use rinch_theme::*;

    // Dynamic theme updates
    #[cfg(feature = "theme")]
    pub use crate::update_theme;

    // Re-export components when the components feature is enabled
    #[cfg(feature = "components")]
    pub use rinch_components::*;

    // Video playback
    #[cfg(feature = "video")]
    pub use rinch_video::{VideoControls, VideoPlayer, VideoViewport, use_video_player};
}

// Re-export core types at crate root
pub use rinch_core::element::{
    Children, CloseRequestCallback, Element, ThemeProviderProps, WindowProps,
};
pub use rinch_core::{Effect, Memo, Scope, Signal, batch, derived, untracked};
pub use rinch_macros::{component, rsx};
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub use shell::{
    inject_platform_event, run, run_on_main_thread, run_rinch, run_rinch_with_window_props,
    run_with_menu, run_with_theme, run_with_window_props, run_with_window_props_and_menu,
};

pub use rinch_core as core;
/// Reactive primitives (Signal, Effect, Memo, Scope).
///
/// Most reactive types are re-exported in the prelude. `Effect` is intentionally
/// excluded from the prelude — use `{|| expr}` in rsx for reactive DOM updates,
/// and store methods for side effects. Import `Effect` explicitly when needed:
///
/// ```ignore
/// use rinch::reactive::Effect;
/// ```
pub use rinch_core::reactive;
pub use rinch_platform as platform;
#[cfg(feature = "gpu")]
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

    #[cfg(feature = "components")]
    {
        css.push_str(&rinch_components::generate_component_css());
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
