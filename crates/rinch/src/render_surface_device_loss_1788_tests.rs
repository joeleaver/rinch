//! #1788: what a surface keeps when the shared device is replaced. Its GPU
//! render callback's target and its registered texture were made on the old
//! device; `forget_gpu_device` drops both on the mounted surfaces, the
//! generation each was made on drops them everywhere else (an unmounted
//! surface, a texture a render thread registers late), and the next paint
//! hands the GPU callback a fresh target, and a frame, though nothing asked
//! for one.
//!
//! The replacement is the event loop's, on the test device itself: retired,
//! the surfaces forgotten, and published again, so its generation moves on
//! by two as a real replacement's does, while the device stays usable for
//! the other GPU tests (under `presenting_lock`).
//! `#[ignore]`d: needs an adapter
//! (`cargo test -p rinch --features gpu --lib -- --ignored gpu_device_loss`).
use std::cell::RefCell;
use std::rc::Rc;

use super::compositor_routing_tests::{
    paint_like_paint_gpu, presenting_lock, surface_app, test_gpu,
};
use super::*;
use crate::shell::desktop::gpu_generation;
use crate::shell::gpu_device::GPU;

/// What `RinchRuntime::replace_gpu_device` does to the device and the
/// surfaces, with the test device coming back as the new one. Answers the
/// new generation.
fn replace_test_device() -> u64 {
    let (instance, handle) = GPU.stack().expect("the test device is published");
    GPU.retire();
    forget_gpu_device();
    GPU.publish(instance, handle).expect("published again")
}

fn app_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("app texture"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_loss_gives_surfaces_a_fresh_target_and_a_frame_1788() {
    let _p = presenting_lock();
    let gpu = test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let surface = create_render_surface();
    let mut app = surface_app(
        &surface,
        "position: absolute; left: 0; top: 0; width: 30px; height: 20px;",
    );
    let generations: Rc<RefCell<Vec<u64>>> = Rc::default();
    let g = generations.clone();
    surface
        .set_gpu_render_callback(move |f: &GpuSurfaceFrame<'_>| g.borrow_mut().push(f.generation));

    // A texture the app registered on the device that is going.
    let (registered, view) = app_texture(&gpu.device);
    surface
        .gpu_registrar()
        .set_texture_source(registered, view, 4, 4);

    paint_like_paint_gpu(&mut app, 1.0);
    // On demand and unasked: a quiet paint draws nothing.
    paint_like_paint_gpu(&mut app, 1.0);
    let drawn_before = generations.borrow().clone();
    let before = gpu_generation();

    let after = replace_test_device();
    let target_gone = surface.gpu_target.borrow().is_none();
    let texture_gone = surface.texture_source.lock().unwrap().is_none();
    let surfaces = paint_like_paint_gpu(&mut app, 1.0);
    let frames = collect_gpu_surface_frames(&surfaces);
    let drawn = generations.borrow().clone();

    crate::shell::renderer::set_gpu_presenting(false);
    unregister_render_surface(surface.id());

    assert_eq!(drawn_before, [before], "one frame before the loss");
    assert!(target_gone, "the old device's target is dropped");
    assert!(
        texture_gone,
        "the texture registered on the old device is dropped"
    );
    assert_eq!(after, before + 2, "a retirement and a publish later");
    assert_eq!(
        drawn,
        [before, after],
        "a frame on the new target, unasked, naming the new device's generation"
    );
    assert!(frames[0].2, "the new frame is fresh: the paint draws it");
}

/// #1788 review: a surface unmounted while the device is replaced (a
/// GameViewport in a hidden tab) is not in the registry `forget_gpu_device`
/// walks, so it comes back with its target of the old device. A target of
/// the same size used to be reused, handing the new device a texture of the
/// dead one; it is made again, on the new device, and drawn into.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_loss_a_surface_unmounted_meanwhile_remounts_with_a_fresh_target_1788() {
    let _p = presenting_lock();
    let _gpu = test_gpu();
    crate::shell::renderer::set_gpu_presenting(true);
    let surface = create_render_surface();
    let mut app = surface_app(
        &surface,
        "position: absolute; left: 0; top: 0; width: 30px; height: 20px;",
    );
    let generations: Rc<RefCell<Vec<u64>>> = Rc::default();
    let g = generations.clone();
    surface
        .set_gpu_render_callback(move |f: &GpuSurfaceFrame<'_>| g.borrow_mut().push(f.generation));
    paint_like_paint_gpu(&mut app, 1.0);
    let old = surface
        .gpu_target
        .borrow()
        .as_ref()
        .map(|t| t.texture.clone())
        .expect("a target");

    // Unmounted (the registry no longer has it) across the replacement.
    unregister_render_surface(surface.id());
    let after = replace_test_device();
    let kept = surface.gpu_target.borrow().is_some();
    mount_render_surface(&surface);
    let surfaces = paint_like_paint_gpu(&mut app, 1.0);
    let frames = collect_gpu_surface_frames(&surfaces);
    let new = surface
        .gpu_target
        .borrow()
        .as_ref()
        .map(|t| (t.texture.clone(), t.generation));

    crate::shell::renderer::set_gpu_presenting(false);
    unregister_render_surface(surface.id());

    assert!(
        kept,
        "the unmounted surface kept its target: nothing forgot it"
    );
    let (new, stamp) = new.expect("a target after the remount");
    assert!(new != old, "the old device's target is not reused");
    assert_eq!(stamp, after, "it is made on the new device");
    assert_eq!(
        generations.borrow().last(),
        Some(&after),
        "and drawn into, unasked"
    );
    assert!(frames[0].2, "the new frame is fresh: the paint draws it");
}

/// #1788 review: a render thread that made a texture on the old device
/// registers it after the replacement: dropped, so the new device never
/// draws it. One made on the new device is registered.
#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_device_loss_a_texture_of_the_old_device_registered_late_is_dropped_1788() {
    let _p = presenting_lock();
    let gpu = test_gpu();
    let surface = create_render_surface();
    let registrar = surface.gpu_registrar();
    let held = gpu_generation();
    let (late, late_view) = app_texture(&gpu.device);

    let after = replace_test_device();
    // The slot itself, not `has_texture_source` (which drops a stale texture
    // too): the registration's own check refuses it.
    let accepted = registrar.set_texture_source_on(held, late, late_view, 4, 4);
    let refused = !accepted && surface.texture_source.lock().unwrap().is_none();

    // One that slipped in before the check is dropped where it is used.
    let (slipped, slipped_view) = app_texture(&gpu.device);
    *surface.texture_source.lock().unwrap() = Some(TextureSource {
        texture: slipped,
        view: slipped_view,
        width: 4,
        height: 4,
        generation: held,
    });
    let dropped_at_use = live_texture_source(&surface.texture_source).is_none();

    let (fresh, fresh_view) = app_texture(&gpu.device);
    let registered = registrar.set_texture_source_on(after, fresh, fresh_view, 4, 4)
        && surface.texture_source.lock().unwrap().is_some();
    // Without a generation: the device `gpu_handle` has now.
    let (plain, plain_view) = app_texture(&gpu.device);
    registrar.set_texture_source(plain, plain_view, 4, 4);
    let plain_registered = surface.has_texture_source();

    unregister_render_surface(surface.id());
    assert!(
        refused,
        "a texture of the replaced device is not registered"
    );
    assert!(dropped_at_use, "nor drawn, when it got in first");
    assert!(registered, "one made on the new device is");
    assert!(plain_registered, "and one registered without a generation");
}

/// The generation rule, with no GPU: a resource is usable on the device it
/// was made on, and one made before rinch published any (an app's own
/// device at startup, `0`) on the first device only.
#[test]
fn made_on_current_device_follows_the_generation() {
    let current = crate::shell::desktop::gpu_generation();
    assert!(made_on_current_device(current));
    assert!(!made_on_current_device(current + 1));
    assert_eq!(made_on_current_device(0), current <= 1);
}
