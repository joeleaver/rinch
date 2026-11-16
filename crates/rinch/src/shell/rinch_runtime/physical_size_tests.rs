//! `rinch::window::window_physical_size`: what a window's surface was
//! configured at, published from the shell's `SurfaceResized`, not the
//! logical size times the scale recomputed. Driven through the runtime
//! without an OS window, as the shell's event path handles the event.

use super::*;
use crate::window::{window_physical_size, window_size};
use winit::dpi::PhysicalSize;

fn component(scope: &mut RenderScope) -> NodeHandle {
    scope.create_element("div")
}

/// A runtime with its main window mounted at scale `scale`.
fn runtime(scale: f64) -> RinchRuntime {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.set_device_pixel_ratio(scale);
    runtime.main.app.mount_component(800.0, 600.0);
    runtime
}

/// The shell's handling of a `SurfaceResized` to `physical` at `scale`: the
/// window's part, then the `Resized` it becomes, handed to the document.
fn surface_resized(runtime: &mut RinchRuntime, physical: (u32, u32), scale: f64) {
    let event = runtime
        .main
        .surface_resized(PhysicalSize::new(physical.0, physical.1), scale);
    let _ = runtime.main.app.handle_event(event, physical, scale);
}

/// At 1.25 a surface 1282 pixels wide is laid out 1026 logical pixels wide
/// (1282 / 1.25 = 1025.6), and 1026 times 1.25 is 1282.5, which rounds to
/// 1283: a recomputation from the logical size is a pixel wider than the
/// surface. The getter answers the surface's own 1282.
#[test]
fn at_a_fractional_scale_it_is_the_surface_and_not_a_recomputation() {
    let scale = 1.25;
    let mut runtime = runtime(scale);
    surface_resized(&mut runtime, (1282, 721), scale);

    assert_eq!(
        window_size(),
        (1026.0, 577.0),
        "laid out at the logical size"
    );
    let recomputed = (window_size().0 * scale).round() as u32;
    assert_eq!(recomputed, 1283, "the recomputation the getter must not be");
    assert_eq!(
        window_physical_size(),
        (1282, 721),
        "the surface's own size"
    );
}

/// A reader of the physical size runs again when the surface is resized,
/// once per resize, with the logical size it is laid out at in the same run;
/// an equal size wakes nothing.
#[test]
fn a_resize_reruns_a_reader() {
    let mut runtime = runtime(1.0);
    surface_resized(&mut runtime, (800, 600), 1.0);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _effect = {
        let seen = seen.clone();
        rinch_core::Effect::new(move || {
            seen.borrow_mut()
                .push((window_physical_size(), window_size()));
        })
    };
    surface_resized(&mut runtime, (1600, 900), 1.0);
    surface_resized(&mut runtime, (1600, 900), 1.0);
    runtime.main.app.set_device_pixel_ratio(2.0);
    surface_resized(&mut runtime, (3200, 1800), 2.0);
    assert_eq!(
        *seen.borrow(),
        [
            ((800, 600), (800.0, 600.0)),
            ((1600, 900), (1600.0, 900.0)),
            ((3200, 1800), (1600.0, 900.0)),
        ],
        "one run per change, none for an equal size; a new scale's surface \
         is followed though the logical size stays"
    );
}

/// The size is the mounted window's from its creation: what the shell set
/// before the mount is published with it.
#[test]
fn the_mount_publishes_the_surface_size_set_before_it() {
    let mut runtime = RinchRuntime::new("t", 800, 600, component, &[]);
    runtime.main.app.set_device_pixel_ratio(1.5);
    runtime.main.app.set_surface_size((1201, 900));
    runtime.main.app.mount_component(801.0, 600.0);
    assert_eq!(window_physical_size(), (1201, 900));
}
