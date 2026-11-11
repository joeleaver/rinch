//! The desktop entry points that build the runtime and run the event loop.

use super::*;

// ── Public entry points ──────────────────────────────────────────────────────

/// Run a rinch application using the rinch-dom rendering pipeline.
///
/// This is an alternative to [`run`](crate::shell::run) that uses rinch-dom
/// (Taffy + Parley + Vello) for layout and rendering.
///
/// # Example
///
/// ```ignore
/// use rinch::prelude::*;
///
/// fn app(__scope: &mut RenderScope) -> NodeHandle {
///     let count = Signal::new(0);
///     let count_inc = count.clone();
///     rsx! {
///         div { style: "display: flex; flex-direction: column; padding: 20px; gap: 10px;",
///             p { "Count: " {|| count.get().to_string()} }
///             button {
///                 onclick: move || count_inc.update(|n| *n += 1),
///                 style: "padding: 8px 16px; background-color: #007bff; color: white;",
///                 "Increment"
///             }
///         }
///     }
/// }
///
/// fn main() {
///     rinch::run_rinch("Counter", 800, 600, app);
/// }
/// ```
#[deprecated(
    since = "0.2.0",
    note = "use `App::new(component).title(title).size(width, height).run()`"
)]
pub fn run_rinch<F>(title: &str, width: u32, height: u32, component: F)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::logging::install_shell_subscriber();

    // Clear stale state
    events::clear_handlers();
    clear_context();

    let event_loop = overlay_window::new_event_loop();

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    // Dispatcher first: a worker sending between the two would otherwise be
    // classified "not the main thread" with no dispatcher to take its write,
    // and panic. In this order it is still unregistered there, and its write
    // is queued through the dispatcher (#1541).
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::register_main_thread();

    // Set up signal change notification
    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    let mut runtime = RinchRuntime::new(title, width, height, component, &[]);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    install_global_proxy(&proxy);

    // Start debug IPC server if feature is enabled (disable with RINCH_DEBUG=0)
    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.main.app._debug_server = Some(debug_server);
                    runtime.main.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}

/// Run a rinch-dom application with full window configuration.
#[deprecated(
    since = "0.2.0",
    note = "use `App::new(component).window_props(props).run()`"
)]
pub fn run_rinch_with_window_props<F>(component: F, props: rinch_core::element::WindowProps)
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    run_rinch_with_window_props_and_menu(component, props, None, &[]);
}

/// Assemble the runtime from everything that is **not** the event loop.
///
/// Extracted so this assembly can be unit-tested: [`RinchRuntime::new`] opens
/// no window, so window props, the native menu and the bundled faces can all be
/// pinned without a display. Its caller cannot be tested at all — it creates the
/// winit event loop and then never returns — which is exactly why as little as
/// possible happens there.
///
/// What that leaves untested is the caller forwarding its own four arguments
/// into this call. That residue is structural and shared with every other
/// terminal path in the crate (issue #493): a *new* call site cannot silently
/// drop the fonts, because the parameter is required and has no default.
pub(super) fn build_runtime<F>(
    component: F,
    props: &rinch_core::element::WindowProps,
    native_menu: Option<muda::Menu>,
    fonts: &[crate::font::AppFont],
) -> RinchRuntime
where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    let mut runtime = RinchRuntime::new(&props.title, props.width, props.height, component, fonts);
    runtime.main.app.set_window_props(props.clone());
    runtime.native_menu = native_menu;
    runtime
}

/// Run a rinch-dom application with full window configuration, an optional
/// native menu, and the app's own bundled typefaces.
///
/// The single desktop dispatch target: [`App::run`](crate::App::run) resolves
/// every builder option and lands here. `fonts` is a parameter rather than
/// something registered beforehand because the faces have to reach the
/// `RinchApp` at construction — ahead of the window, and therefore ahead of
/// the first layout pass. Pass `&[]` for a build that carries none.
pub fn run_rinch_with_window_props_and_menu<F>(
    component: F,
    props: rinch_core::element::WindowProps,
    native_menu: Option<muda::Menu>,
    fonts: &[crate::font::AppFont],
) where
    F: FnOnce(&mut RenderScope) -> NodeHandle + 'static,
{
    crate::logging::install_shell_subscriber();

    events::clear_handlers();
    clear_context();

    let event_loop = overlay_window::new_event_loop();

    let proxy = event_loop.create_proxy();

    // Register main thread and cross-thread dispatcher for Signal::send()
    // Dispatcher first: a worker sending between the two would otherwise be
    // classified "not the main thread" with no dispatcher to take its write,
    // and panic. In this order it is still unregistered there, and its write
    // is queued through the dispatcher (#1541).
    rinch_core::set_cross_thread_dispatcher(dispatch_to_main_thread);
    rinch_core::register_main_thread();

    rinch_core::set_on_signal_change(move || {
        send_native_event(RinchNativeEvent::ReRender);
    });

    #[cfg(all(target_os = "linux", feature = "system-tray"))]
    if let Some(id) = props.app_id.as_deref() {
        let _ = DECLARED_APP_ID.set(id.to_string());
    }
    let mut runtime = build_runtime(component, &props, native_menu, fonts);
    runtime.proxy = Some(proxy.clone());

    // Install push-based menu event handler (covers native + tray menus)
    crate::menu::install_menu_event_handler();

    // Set native proxy for window control functions
    NATIVE_PROXY.with(|p| *p.borrow_mut() = Some(proxy.clone()));

    // Set global proxy so run_on_main_thread() works from any thread
    install_global_proxy(&proxy);

    #[cfg(feature = "debug")]
    {
        if std::env::var("RINCH_DEBUG").map_or(true, |v| v != "0") {
            match rinch_debug::attach(&props.title, move || {
                send_native_event(RinchNativeEvent::DebugCommand);
            }) {
                Ok((debug_server, cmd_rx)) => {
                    tracing::info!("rinch-debug listening on port {}", debug_server.port());
                    runtime.main.app._debug_server = Some(debug_server);
                    runtime.main.app.debug_cmd_rx = Some(cmd_rx);
                }
                Err(e) => {
                    tracing::warn!("Failed to start rinch-debug server: {}", e);
                }
            }
        }
    }

    event_loop.run_app(runtime).expect("Event loop error");
}
