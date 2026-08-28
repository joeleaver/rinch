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

#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
pub mod app;
/// The ProseMirror-style desktop rich-text editor view. Part of the `desktop`
/// feature; implements `rinch_editor_core::EditorView` over rinch-dom primitives.
#[cfg(feature = "desktop")]
pub mod editor;
#[cfg(any(feature = "gpu", feature = "embed"))]
pub mod embed;
/// Focus-target registration for custom keyboard-owning components (issue #147).
/// Gated with [`app`], whose focus arbiter it serves.
#[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
pub mod focus_registry;
#[cfg(feature = "desktop")]
pub mod menu;
pub mod render_surface;
#[cfg(any(feature = "desktop", feature = "android"))]
pub mod shell;
pub mod window;
#[cfg(feature = "desktop")]
pub mod windows;
#[cfg(all(feature = "android", not(feature = "desktop")))]
pub mod windows_stub;

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
#[cfg(any(feature = "desktop", feature = "android"))]
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
    #[cfg(all(feature = "android", target_os = "android"))]
    pub use crate::shell::android_runtime::{
        run as run_android, run_with_theme as run_android_with_theme,
    };
    #[cfg(feature = "desktop")]
    pub use crate::shell::{run, run_with_theme};
    #[cfg(all(feature = "android", target_os = "android"))]
    pub use android_activity::AndroidApp;
    pub use rinch_core::element::*;
    pub use rinch_core::{Memo, Scope, Signal, batch, derived, untracked};
    // Cross-platform one-shot timers (debounce / throttle / delayed work) and the
    // helper to run a closure on the main (UI) thread from a background thread.
    pub use rinch_core::{TimeoutHandle, clear_timeout, run_on_main_thread, set_timeout};
    // Context and stores for sharing state across components
    pub use rinch_core::{
        create_context, create_store, try_use_context, try_use_store, use_context, use_store,
    };
    // Event handling - click context, drag support, input callbacks, file-drop callbacks
    pub use rinch_core::{
        ClickContext, Drag, DragContext, FileDropCallback, InputCallback, ModifierState,
        MouseButton, ScrollEvent, get_click_context, restore_drag_ghost, suppress_drag_ghost,
    };
    // Paste notification (issue #150): on the web this is the only place app paste
    // logic can see content copied outside the app, because the browser's `paste`
    // event carries it and arrives *after* the Ctrl+V keydown.
    pub use rinch_core::{PasteEventData, clear_paste_interceptor, set_paste_interceptor};
    pub use rinch_macros::{component, rsx};
    // Window control functions
    #[cfg(feature = "desktop")]
    pub use crate::windows::{
        close_current_window, hide_current_window, minimize_current_window, show_current_window,
        toggle_maximize_current_window,
    };
    #[cfg(all(feature = "android", not(feature = "desktop")))]
    pub use crate::windows_stub::{
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
    #[cfg(any(feature = "gpu", feature = "embed"))]
    pub use crate::embed::{
        GameViewport, LayoutRect, RinchContext, RinchContextConfig, RinchOverlayRenderer,
    };

    // Focus lifecycle for custom keyboard-owning components (issue #147): a
    // `tabindex` node registers here to hear focus / blur / keys — and, if it
    // registers `on_ime`, IME composition (issue #176), whose events are the
    // portable `ImeEvent` every text target consumes.
    #[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
    pub use crate::focus_registry::{FocusEntry, register_focus_target};
    #[cfg(any(feature = "desktop", feature = "android", feature = "embed"))]
    pub use rinch_platform::ImeEvent;

    // Render surface API for embedding external renderers
    pub use crate::render_surface::{
        RenderSurface, RenderSurfaceHandle, SurfaceEvent, SurfaceKeyData, SurfaceMouseButton,
        SurfaceWriter, create_render_surface, invoke_render_callbacks,
    };

    // New ProseMirror-style rich-text editor (M5+)
    #[cfg(feature = "desktop")]
    pub use crate::editor::{Editor, EditorHandle, create_editor};

    // Collaborative editing (M9): the editor's collab error type + the thread-safe
    // inbound route a network transport posts received deltas through.
    #[cfg(feature = "collaboration")]
    pub use crate::editor::{CollabError, collab_receive_for, post_remote_delta};

    // Reconciliation for poll/reconnect transports (`EditorHandle::collab_state_vector`
    // / `collab_sync_diff`) needs no extra imports: state vectors and diffs are opaque
    // `Vec<u8>`, and a diff is applied through the same `collab_receive` as a delta.

    // Desktop-only GPU types for render surfaces
    #[cfg(feature = "gpu")]
    pub use crate::render_surface::{GpuTextureRegistrar, TextureSource};

    // Shared GPU handle + device-sharing entry points for zero-copy compositing
    #[cfg(feature = "gpu")]
    pub use crate::shell::desktop::{ExternalGpu, GpuHandle, RinchGpuConfig, gpu_handle};
    #[cfg(feature = "gpu")]
    pub use crate::shell::{run_with_external_device, run_with_gpu_config};

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
// `Owner` / `current_owner` / `unowned` sit at the crate root rather than in the
// prelude, alongside `Effect`, for the same reason: they are advanced tools, and
// `unowned` in particular is an attractive nuisance — it turns a lifetime bug
// into a permanent leak. Reach for `Signal::try_get` / `is_alive` first.
pub use rinch_core::{
    Effect, Memo, Owner, Scope, Signal, batch, current_owner, derived, unowned, untracked,
};
pub use rinch_macros::{component, rsx};
#[cfg(feature = "desktop")]
#[allow(deprecated)]
pub use shell::{
    inject_platform_event, run, run_on_main_thread, run_rinch, run_rinch_with_window_props,
    run_with_menu, run_with_theme, run_with_window_props, run_with_window_props_and_menu,
};
#[cfg(feature = "gpu")]
pub use shell::{run_with_external_device, run_with_gpu_config};

/// Re-export of the exact `wgpu` rinch links against.
///
/// Embedders that share a GPU device with rinch (see [`run_with_gpu_config`],
/// [`run_with_external_device`], [`gpu_handle`], and
/// [`render_surface::GpuTextureRegistrar`]) must construct `wgpu` types from
/// **this** `wgpu` so the types match rinch's (rinch pins a patched fork). Use
/// `rinch::wgpu::…` rather than a separate `wgpu` dependency.
#[cfg(any(feature = "gpu", feature = "embed"))]
pub use wgpu;

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

/// Generate the theme (+ component) CSS for the given props.
///
/// Pure: does **not** touch the thread-local theme slot. The shell paths use
/// [`update_theme`] (which writes the slot); an embed `RinchContext` owns its
/// CSS per document instead (issue #138).
#[cfg(feature = "theme")]
pub fn generate_theme_css_string(props: &ThemeProviderProps) -> String {
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

    css
}

/// Dynamically update theme CSS at runtime.
///
/// Writes the **thread-global** theme slot, which single-root shell/web/android
/// apps follow. Embed contexts ignore the slot — use
/// `embed::RinchContext::set_theme` there instead (issue #138).
#[cfg(feature = "theme")]
pub fn update_theme(props: &ThemeProviderProps) {
    rinch_core::set_current_theme_css(Some(generate_theme_css_string(props)));
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
