//! Game engine integration API.
//!
//! This module provides [`RinchContext`] for embedding rinch UI into an
//! existing game engine or custom render loop. The game owns the wgpu
//! `Device`/`Queue`/`Surface` and frame loop; rinch provides UI as a Vello
//! scene that can be composited on top of game content.
//!
//! # Architecture
//!
//! ```text
//! Game loop:
//!   1. Collect platform events
//!   2. ctx.update(&events)        // rinch processes input, updates layout
//!   3. game.render()              // render game scene
//!   4. overlay.render(ctx.scene()) // render rinch UI to texture
//!   5. composite game + UI        // blit both to swapchain
//! ```
//!
//! # Two integration patterns
//!
//! - **Full overlay (HUD)**: Rinch covers the entire window. Use
//!   [`RinchContext::wants_mouse`] / [`wants_keyboard`](RinchContext::wants_keyboard)
//!   to decide whether input goes to the game or UI.
//!
//! - **Split layout**: Rinch renders toolbars/panels around a
//!   [`GameViewport`] hole. Query the viewport rect with
//!   [`RinchContext::viewport_rect`] and render the game into that region.
//!
//! # Example
//!
//! ```ignore
//! use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};
//! use rinch::prelude::*;
//!
//! let mut ctx = RinchContext::new(
//!     RinchContextConfig {
//!         width: 1280,
//!         height: 720,
//!         scale_factor: 1.0,
//!         theme: None,
//!     },
//!     game_ui,
//! );
//!
//! // In your game loop:
//! let actions = ctx.update(&platform_events);
//! let scene = ctx.scene();
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use peniko::Color;
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRenderer, RendererOptions, Scene};
use wgpu::{
    Device, Extent3d, Queue, Texture, TextureDescriptor, TextureDimension, TextureFormat,
    TextureUsages, TextureView, TextureViewDescriptor,
};

use rinch_core::dom::{NodeHandle, RenderScope};
use rinch_core::element::ThemeProviderProps;
use rinch_platform::{AppAction, PlatformEvent};

use crate::app::RinchApp;
use crate::app::hit_testing::hit_test;

// ── LayoutRect ───────────────────────────────────────────────────────────────

/// A layout rectangle in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// ── RinchContextConfig ───────────────────────────────────────────────────────

/// Configuration for creating a [`RinchContext`].
pub struct RinchContextConfig {
    /// Initial viewport width in physical pixels.
    pub width: u32,
    /// Initial viewport height in physical pixels.
    pub height: u32,
    /// Display scale factor (e.g. 2.0 for Retina).
    pub scale_factor: f64,
    /// Optional theme configuration. When `None`, the default theme is used
    /// (if the `theme` feature is enabled).
    pub theme: Option<ThemeProviderProps>,
    /// Fonts (TTF/OTF bytes) to register **before** the component mounts, so the
    /// very first layout/paint pass has glyphs.
    ///
    /// This is the correct hook for environments with no system fonts — most
    /// importantly **wasm32/WebGPU**, where the platform provides no fallback
    /// font and text would otherwise render as zero glyphs. (Registering a font
    /// *after* [`RinchContext::new`] via [`register_font`](RinchContext::register_font)
    /// is too late for the initial mount.)
    pub fonts: Vec<&'static [u8]>,
}

// ── RinchContext ──────────────────────────────────────────────────────────────

/// Handle to an embedded rinch UI.
///
/// Created once during game initialization. The game loop calls [`update`]
/// each frame with platform events, then reads the Vello [`Scene`] via
/// [`scene`] for compositing.
///
/// [`update`]: RinchContext::update
/// [`scene`]: RinchContext::scene
pub struct RinchContext {
    app: RinchApp,
    size: (u32, u32),
    scale_factor: f64,
    dirty: Arc<AtomicBool>,
    /// This context's signal-change subscription. Multi-subscriber: several
    /// concurrent contexts (plus a mounted shell/web root) each hold their own,
    /// and dropping this context detaches only its own callback — creating or
    /// dropping context B never silences context A (issue #134).
    _signal_change_sub: rinch_core::SignalChangeSubscription,
}

impl RinchContext {
    /// Create and mount a rinch UI.
    ///
    /// The component function runs immediately, building the initial DOM.
    /// Call this once during game initialization.
    pub fn new<F>(config: RinchContextConfig, component: F) -> Self
    where
        F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
    {
        // Set up theme CSS before mounting the component
        #[cfg(feature = "theme")]
        {
            if let Some(ref theme) = config.theme {
                crate::setup_theme_css(theme);
            } else {
                crate::setup_theme_css(&ThemeProviderProps::default());
            }
        }
        let _ = &config.theme; // suppress unused warning when theme feature is off

        let mut app = RinchApp::new(component);

        // Namespace this context's stores/contexts under its document's
        // doc_key: two contexts creating the same store type no longer
        // overwrite each other, and lookups fall back to the thread-global
        // root 0 for stores created outside any context (issue #136).
        app.scope_context_to_doc = true;

        // Register any pre-mount fonts BEFORE mounting so the initial layout/paint
        // pass has glyphs (essential on wasm, which has no system fonts). These are
        // drained onto the document's render font context during mount_component.
        for font in &config.fonts {
            app.register_font_data(font);
        }

        let width = config.width.max(1);
        let height = config.height.max(1);
        let scale_factor = config.scale_factor;

        // Mount the component (builds DOM, runs initial layout)
        let logical_w = width as f32 / scale_factor as f32;
        let logical_h = height as f32 / scale_factor as f32;
        app.mount_component(logical_w, logical_h);

        // Register main thread for cross-thread signal dispatch
        rinch_core::register_main_thread();

        // Subscribe to signal changes so the game loop can detect dirty state.
        // A guard-based subscription (not the legacy single slot): each context
        // observes independently, so N contexts can coexist (issue #134).
        let dirty = Arc::new(AtomicBool::new(false));
        let dirty_clone = dirty.clone();
        let signal_change_sub = rinch_core::subscribe_signal_change(move || {
            dirty_clone.store(true, Ordering::Release);
        });

        Self {
            app,
            size: (width, height),
            scale_factor,
            dirty,
            _signal_change_sub: signal_change_sub,
        }
    }

    /// Process input events and update layout. Call once per frame.
    ///
    /// Returns actions the game should handle (e.g. `SetCursor`, `Exit`).
    /// Input events that rinch doesn't consume are ignored.
    pub fn update(&mut self, events: &[PlatformEvent]) -> Vec<AppAction> {
        let mut all_actions = Vec::new();

        // Process each event
        for event in events {
            let actions = self
                .app
                .handle_event(event.clone(), self.size, self.scale_factor);
            all_actions.extend(actions);
        }

        // If signals changed, resolve layout
        if self.dirty.swap(false, Ordering::AcqRel) || self.app.has_dirty_nodes() {
            let (w, h) = self.logical_size();
            if self.app.resolve_and_repaint(w, h) {
                // Deduplicate RequestRedraw
                if !all_actions.contains(&AppAction::RequestRedraw) {
                    all_actions.push(AppAction::RequestRedraw);
                }
            }
        }

        // Process AboutToWait for transitions
        let transition_actions =
            self.app
                .handle_event(PlatformEvent::AboutToWait, self.size, self.scale_factor);
        all_actions.extend(transition_actions);

        all_actions
    }

    /// Get the current Vello scene. Rebuilds only if dirty.
    ///
    /// The returned scene contains the full rinch UI and can be rendered
    /// to a texture via [`RinchOverlayRenderer`] or your own Vello setup.
    pub fn scene(&mut self) -> &Scene {
        self.app.build_scene(self.scale_factor, self.size)
    }

    /// Notify rinch the window was resized.
    ///
    /// Call this when the game window size changes. Dimensions are in
    /// physical pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        self.size = (width, height);
        self.app.resize_layout(width, height);
    }

    /// Update the display scale factor (DPI).
    pub fn set_scale_factor(&mut self, scale: f64) {
        self.scale_factor = scale;
    }

    /// Query the layout rect of a [`GameViewport`] component by name.
    ///
    /// Returns the absolute position and size in logical pixels, or `None`
    /// if no viewport with that name exists.
    pub fn viewport_rect(&self, name: &str) -> Option<LayoutRect> {
        self.app.viewport_rect(name).map(|(x, y, w, h)| LayoutRect {
            x,
            y,
            width: w,
            height: h,
        })
    }

    /// Returns `true` if the point `(x, y)` (in logical pixels) hits a
    /// rinch UI element rather than a [`GameViewport`] hole.
    ///
    /// When this returns `true`, the game should let rinch handle the
    /// input. When `false`, the game should handle it (e.g. camera,
    /// gameplay).
    pub fn wants_mouse(&self, x: f32, y: f32) -> bool {
        let Some(doc) = self.app.doc() else {
            return false;
        };
        let d = doc.borrow();

        // Hit-test the rinch DOM
        let Some(hit_node_id) = hit_test(&d.tree, x, y) else {
            return false;
        };

        // Walk up from the hit node — if we find a data-viewport attribute,
        // the click is in the game area, not rinch UI.
        let mut current = Some(hit_node_id);
        while let Some(id) = current {
            if let Some(node) = d.tree.get(id) {
                if node.attributes.contains_key("data-viewport") {
                    return false;
                }
                current = node.parent;
            } else {
                break;
            }
        }

        true
    }

    /// Returns `true` if a text input or contenteditable is focused.
    ///
    /// When this returns `true`, the game should route keyboard events to
    /// rinch instead of handling them as game input.
    pub fn wants_keyboard(&self) -> bool {
        self.app.has_focused_input() || self.app.has_focused_contenteditable()
    }

    /// Register font data for text rendering **after** the initial mount.
    ///
    /// Useful in environments without system fonts (e.g. WASM, embedded).
    /// The font applies to subsequent layout/paint passes, not the mount that
    /// already happened inside [`new`](RinchContext::new). To have glyphs on the
    /// **first** frame — which is mandatory on wasm/WebGPU, where there is no
    /// system fallback font — supply the font via
    /// [`RinchContextConfig::fonts`] instead.
    ///
    /// The data should be a TrueType (`.ttf`) or OpenType (`.otf`) font file.
    pub fn register_font(&mut self, data: &'static [u8]) {
        self.app.register_font_data(data);
    }

    /// Whether the UI needs a repaint.
    ///
    /// Useful for game engines that want to skip rendering unchanged frames.
    pub fn needs_update(&self) -> bool {
        self.dirty.load(Ordering::Acquire) || self.app.has_dirty_nodes()
    }

    /// Access the underlying `RinchApp` for advanced use cases.
    pub fn app(&self) -> &RinchApp {
        &self.app
    }

    /// Mutable access to the underlying `RinchApp`.
    pub fn app_mut(&mut self) -> &mut RinchApp {
        &mut self.app
    }

    fn logical_size(&self) -> (f32, f32) {
        (
            self.size.0 as f32 / self.scale_factor as f32,
            self.size.1 as f32 / self.scale_factor as f32,
        )
    }
}

impl Drop for RinchContext {
    fn drop(&mut self) {
        // Clear this context's store/context namespace so its stores don't
        // outlive it (issue #136). The thread-global root 0 — and every other
        // context's namespace — is untouched.
        let key = self.app.doc_key();
        if key != 0 {
            rinch_core::clear_context_for_root(key);
        }
    }
}

// ── Debug integration ───────────────────────────────────────────────────────

/// Re-export screenshot utilities for game engine integration.
#[cfg(feature = "debug")]
pub use crate::shell::screenshot::{capture_texture_rgba, encode_png};

/// A pending screenshot request from the debug server.
///
/// Returned by [`RinchContext::process_debug_commands`]. Call
/// [`respond`](ScreenshotRequest::respond) with the captured pixel data.
#[cfg(feature = "debug")]
pub struct ScreenshotRequest {
    response_tx: std::sync::mpsc::Sender<rinch_debug::DebugResult>,
}

#[cfg(feature = "debug")]
impl ScreenshotRequest {
    /// Send a successful screenshot (raw RGBA pixels → PNG encoded internally).
    pub fn respond(self, width: u32, height: u32, rgba: Vec<u8>) {
        use base64::Engine;
        let png_bytes = crate::shell::screenshot::encode_png(&rgba, width, height);
        let _ = self.response_tx.send(rinch_debug::DebugResult::Bytes {
            data: base64::engine::general_purpose::STANDARD.encode(&png_bytes),
        });
    }

    /// Send an error response.
    pub fn fail(self, message: String) {
        let _ = self
            .response_tx
            .send(rinch_debug::DebugResult::Error { message });
    }
}

#[cfg(feature = "debug")]
impl RinchContext {
    /// Attach the debug server for MCP tool integration.
    ///
    /// The `notify` callback is invoked (from a background thread) when a
    /// debug command arrives. Use it to wake the event loop so commands are
    /// processed promptly. In polling game loops this can be a no-op.
    pub fn attach_debug(
        &mut self,
        app_name: &str,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<()> {
        let (server, rx) = rinch_debug::attach(app_name, notify)?;
        self.app._debug_server = Some(server);
        self.app.debug_cmd_rx = Some(rx);
        Ok(())
    }

    /// Process pending debug commands.
    ///
    /// Non-screenshot commands (DOM queries, clicks, typing, etc.) are
    /// handled internally. Screenshot requests are returned so the game
    /// can capture the composited frame and respond via
    /// [`ScreenshotRequest::respond`].
    ///
    /// Call this once per frame, after rendering but before presenting.
    pub fn process_debug_commands(&mut self) -> Vec<ScreenshotRequest> {
        use rinch_debug::DebugCommandKind;

        let Some(rx) = self.app.debug_cmd_rx.take() else {
            return Vec::new();
        };

        let mut pending = Vec::new();
        while let Ok(cmd) = rx.0.try_recv() {
            pending.push(cmd);
        }

        let mut screenshots = Vec::new();

        for cmd in pending {
            match cmd.kind {
                DebugCommandKind::Screenshot => {
                    screenshots.push(ScreenshotRequest {
                        response_tx: cmd.response_tx,
                    });
                }
                other => {
                    let mut actions = Vec::new();
                    let result = self.app.execute_debug_command(
                        other,
                        &mut actions,
                        self.scale_factor,
                        self.size,
                    );
                    if actions.contains(&AppAction::RequestRedraw) {
                        self.dirty.store(true, Ordering::Release);
                    }
                    let _ = cmd.response_tx.send(result);
                }
            }
        }

        self.app.debug_cmd_rx = Some(rx);
        screenshots
    }
}

// ── RinchOverlayRenderer ─────────────────────────────────────────────────────

/// Convenience helper that renders a Vello [`Scene`] to a GPU texture.
///
/// Game engines with their own Vello setup can skip this and render the
/// scene directly. This struct manages a [`vello::Renderer`] and an
/// intermediate render texture sized to the window.
///
/// # Usage
///
/// ```ignore
/// let mut overlay = RinchOverlayRenderer::new(&device, 1280, 720, TextureFormat::Rgba8Unorm);
///
/// // Each frame:
/// let view = overlay.render(&device, &queue, ctx.scene());
/// // composite `view` on top of your game scene
/// ```
pub struct RinchOverlayRenderer {
    renderer: VelloRenderer,
    render_texture: Texture,
    width: u32,
    height: u32,
    format: TextureFormat,
}

impl RinchOverlayRenderer {
    /// Create a new overlay renderer from the game engine's wgpu device.
    ///
    /// The `format` should match the texture format you'll composite with
    /// (typically `Rgba8Unorm` or `Bgra8Unorm`).
    pub fn new(device: &Device, width: u32, height: u32, format: TextureFormat) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let renderer = VelloRenderer::new(
            device,
            RendererOptions {
                antialiasing_support: AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("Failed to create Vello renderer");

        let render_texture = Self::create_texture(device, format, width, height);

        Self {
            renderer,
            render_texture,
            width,
            height,
            format,
        }
    }

    /// Render the rinch scene to a texture.
    ///
    /// Returns a [`TextureView`] that the game can sample/composite.
    /// The texture has a transparent background so it can be alpha-blended
    /// over the game scene.
    pub fn render(&mut self, device: &Device, queue: &Queue, scene: &Scene) -> TextureView {
        let view = self
            .render_texture
            .create_view(&TextureViewDescriptor::default());

        self.renderer
            .render_to_texture(
                device,
                queue,
                scene,
                &view,
                &RenderParams {
                    base_color: Color::TRANSPARENT,
                    width: self.width,
                    height: self.height,
                    antialiasing_method: AaConfig::Msaa16,
                },
            )
            .expect("Vello render_to_texture failed");

        view
    }

    /// Resize the render target. Call when the window size changes.
    pub fn resize(&mut self, device: &Device, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.render_texture = Self::create_texture(device, self.format, width, height);
    }

    /// Get the current render texture (e.g. for binding to a shader).
    pub fn texture(&self) -> &Texture {
        &self.render_texture
    }

    /// Current width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Current height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    fn create_texture(device: &Device, format: TextureFormat, width: u32, height: u32) -> Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("rinch overlay texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::STORAGE_BINDING
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}

// ── GameViewport component ───────────────────────────────────────────────────

/// A transparent placeholder that marks a region for game rendering.
///
/// Place this component in your rinch UI layout wherever you want the game
/// scene to show through. Use [`RinchContext::viewport_rect`] to query
/// the computed layout rect and render your game into that region.
///
/// The component renders as a transparent `div` with `pointer-events: none`
/// so mouse events pass through to the game.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
/// use rinch::embed::GameViewport;
///
/// #[component]
/// fn game_ui() -> NodeHandle {
///     rsx! {
///         div { style: "display: flex; flex-direction: column; height: 100%;",
///             div { class: "toolbar", "Tools here" }
///             div { style: "display: flex; flex: 1;",
///                 div { style: "width: 200px;", "Side panel" }
///                 GameViewport { name: "main", style: "flex: 1;" }
///             }
///             div { class: "status-bar", "Status" }
///         }
///     }
/// }
/// ```
pub fn game_viewport(__scope: &mut RenderScope, name: &str) -> NodeHandle {
    let div = __scope.create_element("div");
    div.set_attribute("class", "rinch-game-viewport");
    div.set_attribute("data-viewport", name);
    div.set_attribute("style", "pointer-events: none; background: transparent;");
    div
}

/// Component struct for `GameViewport` — use in RSX as `GameViewport { name: "main" }`.
#[derive(Debug, Default)]
pub struct GameViewport {
    /// Name used to query this viewport's rect via [`RinchContext::viewport_rect`].
    pub name: String,
}

impl rinch_core::Component for GameViewport {
    fn render(&self, scope: &mut RenderScope, _children: &[NodeHandle]) -> NodeHandle {
        game_viewport(scope, &self.name)
    }
}
