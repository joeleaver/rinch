//! #2821: `set_current_window_layer`'s configuration stored by
//! `ShellWindow::store_layer_surface`, the namespace of a live layer
//! surface kept, without an OS window: a live layer surface is one with
//! layer-shell state (`ShellWindow::layer`), which `take_surface_kind` sets
//! when the surface is made and `hide_window` clears. The same on a
//! compositor is pinned live by `tests/layer_namespace_2821.rs`.

use super::*;
use rinch_core::element::{LayerExclusiveZone, LayerSurface, WindowProps};

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

fn surface(namespace: &str, zone: u32) -> LayerSurface {
    LayerSurface {
        namespace: Some(namespace.into()),
        exclusive_zone: LayerExclusiveZone::Reserve(zone),
        ..Default::default()
    }
}

/// A runtime whose main window has `props`, a layer surface (`live`) or
/// not.
fn window(props: WindowProps, live: bool) -> RinchRuntime {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.window_props = Some(props);
    if live {
        runtime.main.layer = Some(LayerState::app_placed());
    }
    runtime
}

fn stored_namespace(runtime: &RinchRuntime) -> Option<String> {
    runtime
        .main
        .app
        .window_props
        .as_ref()
        .and_then(|props| props.wayland_layer.as_ref())
        .and_then(|surface| surface.namespace.clone())
}

#[test]
fn a_live_layer_surface_keeps_its_namespace_and_takes_the_rest() {
    let mut runtime = window(
        WindowProps {
            wayland_layer: Some(surface("dock", 32)),
            ..Default::default()
        },
        true,
    );
    let stored = runtime
        .main
        .store_layer_surface(surface("desktop", 48), Backend::Wayland);
    assert_eq!(stored.surface.namespace.as_deref(), Some("dock"));
    assert_eq!(
        stored.surface.exclusive_zone,
        LayerExclusiveZone::Reserve(48)
    );
    assert_eq!(stored.was.namespace.as_deref(), Some("dock"));
    assert_eq!(
        stored_namespace(&runtime).as_deref(),
        Some("dock"),
        "what a remake (a new output) starts from keeps the name"
    );
}

#[test]
fn a_toplevel_made_a_layer_surface_takes_the_namespace_given() {
    let mut runtime = window(WindowProps::default(), false);
    let stored = runtime
        .main
        .store_layer_surface(surface("dock", 32), Backend::Wayland);
    assert!(!stored.was_layer, "it was a toplevel");
    assert_eq!(stored.surface.namespace.as_deref(), Some("dock"));
    assert_eq!(stored_namespace(&runtime).as_deref(), Some("dock"));
}

#[test]
fn a_hidden_layer_window_takes_the_namespace_given() {
    // Hidden: its configuration is a layer surface's, but it has none live.
    let mut runtime = window(
        WindowProps {
            wayland_layer: Some(surface("dock", 32)),
            ..Default::default()
        },
        false,
    );
    let stored = runtime
        .main
        .store_layer_surface(surface("desktop", 32), Backend::Wayland);
    assert!(stored.was_layer);
    assert_eq!(stored.surface.namespace.as_deref(), Some("desktop"));
    assert_eq!(stored_namespace(&runtime).as_deref(), Some("desktop"));
}

/// A layer surface the compositor closed with no output left is hidden
/// (`layer_surface_closed`) until one appears: it has no live surface, and
/// takes the namespace given then, as a hidden window does.
#[test]
fn a_window_waiting_for_an_output_takes_the_namespace_given() {
    let mut runtime = window(
        WindowProps {
            wayland_layer: Some(surface("dock", 32)),
            ..Default::default()
        },
        true,
    );
    // What `layer_surface_closed` does with no output left.
    runtime.main.hide_window(None);
    runtime.main.layer_awaiting_output = true;
    let stored = runtime
        .main
        .store_layer_surface(surface("desktop", 32), Backend::Wayland);
    assert_eq!(stored.surface.namespace.as_deref(), Some("desktop"));
}
