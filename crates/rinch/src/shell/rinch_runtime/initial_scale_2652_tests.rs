//! The scale a new Wayland surface starts at (#2652).
//!
//! A compositor sends a surface its scale only once the surface is mapped,
//! which takes a first buffer (headless sway: `preferred_scale` right after
//! the first `attach`), so the window is created at a scale rinch guesses
//! (`initial_surface_scale`, given to the winit fork's
//! `WindowAttributesWayland::with_initial_scale_factor`) and its first
//! frame is painted at it. The live half, the frames themselves on sway, is
//! `tests/first_frame_scale_2652.rs`.

use super::*;
use crate::outputs::{Output, OutputId, OutputTracking};
use rinch_core::element::{LayerSurface, WindowProps};

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

fn output(id: u64, name: &str, scale: f64) -> Output {
    Output {
        id: OutputId::from_raw(id),
        name: Some(name.into()),
        position: (0.0, 0.0),
        size: (1280.0, 720.0),
        scale,
    }
}

/// Two outputs: the first listed at 2, the second at 1.25. Off 1, so a
/// guess left at winit's default is told apart.
fn publish_two() {
    crate::outputs::publish(
        vec![output(7, "eDP-1", 2.0), output(9, "HDMI-A-1", 1.25)],
        OutputTracking::Live,
    );
}

fn runtime(props: WindowProps) -> RinchRuntime {
    let mut rt = build_runtime(component, &props, None, &[]);
    rt.main.app.set_device_pixel_ratio(1.0);
    rt
}

#[test]
fn a_first_appearance_starts_at_the_first_outputs_scale() {
    publish_two();
    let rt = runtime(WindowProps::default());
    assert_eq!(rt.main.initial_surface_scale(true), 2.0);
}

#[test]
fn the_output_the_window_asks_for_wins() {
    publish_two();
    let rt = runtime(WindowProps {
        output: Some("HDMI-A-1".into()),
        ..Default::default()
    });
    assert_eq!(rt.main.initial_surface_scale(true), 1.25);
    assert_eq!(
        rt.main.initial_surface_scale(false),
        1.25,
        "on a later show too"
    );
    // A layer surface's output by id (#2658): exact, whatever its name.
    let rt = runtime(WindowProps {
        output: Some("eDP-1".into()),
        wayland_layer: Some(LayerSurface {
            output_id: Some(OutputId::from_raw(9)),
            ..Default::default()
        }),
        ..Default::default()
    });
    assert_eq!(rt.main.initial_surface_scale(true), 1.25);
}

/// A later show starts at the scale the window had when last on screen
/// (its document's), not the first output's.
#[test]
fn a_later_show_starts_at_the_scale_it_had() {
    publish_two();
    let mut rt = runtime(WindowProps::default());
    rt.main.app.set_device_pixel_ratio(1.75);
    assert_eq!(rt.main.initial_surface_scale(false), 1.75);
}

/// A window prepared while hidden (#2633) starts at the scale it was
/// prepared at, so its prepared frame is the one shown.
#[test]
fn a_prepared_window_starts_at_its_preparations_scale() {
    publish_two();
    let mut rt = runtime(WindowProps::default());
    rt.main.prepared = Some((1.5, 0));
    assert_eq!(rt.main.initial_surface_scale(true), 1.5);
    assert_eq!(rt.main.initial_surface_scale(false), 1.5);
}

#[test]
fn with_no_outputs_listed_it_starts_at_its_documents_scale() {
    let mut rt = runtime(WindowProps::default());
    rt.main.app.set_device_pixel_ratio(1.5);
    assert_eq!(rt.main.initial_surface_scale(true), 1.5);
}

/// The guess reaches winit on Wayland, in the window's Wayland attributes,
/// and is not set anywhere else.
#[cfg(target_os = "linux")]
#[test]
fn the_wayland_attributes_carry_the_initial_scale() {
    use super::overlay_window::Backend;
    publish_two();
    let mut rt = runtime(WindowProps::default());
    let platform = |rt: &mut RinchRuntime, backend: Backend| {
        let steps = rt.main.main_window_steps(backend, true);
        let attrs = rt.main.main_window_attributes(steps, backend, true, true);
        format!("{:?}", attrs.platform)
    };
    let wayland = platform(&mut rt, Backend::Wayland);
    assert!(
        wayland.contains("initial_scale_factor: Some(2.0)"),
        "{wayland}"
    );
    let x11 = platform(&mut rt, Backend::X11);
    assert!(!x11.contains("initial_scale_factor"), "{x11}");
}
