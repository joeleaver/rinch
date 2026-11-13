//! #1788: what a surface keeps when the shared device is replaced. Its GPU
//! render callback's target and its registered texture were made on the old
//! device; `forget_gpu_device` drops both, and the next paint hands the GPU
//! callback a fresh target, and a frame, though nothing asked for one.
//! `#[ignore]`d: needs an adapter
//! (`cargo test -p rinch --features gpu --lib -- --ignored gpu_device_loss`).
use std::cell::RefCell;
use std::rc::Rc;

use super::compositor_routing_tests::{
    paint_like_paint_gpu, presenting_lock, surface_app, test_gpu,
};
use super::*;

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
    let registered = gpu.device.create_texture(&wgpu::TextureDescriptor {
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
    let view = registered.create_view(&Default::default());
    surface
        .gpu_registrar()
        .set_texture_source(registered, view, 4, 4);

    paint_like_paint_gpu(&mut app, 1.0);
    // On demand and unasked: a quiet paint draws nothing.
    paint_like_paint_gpu(&mut app, 1.0);
    let drawn_before = generations.borrow().len();

    forget_gpu_device();
    let target_gone = surface.gpu_target.borrow().is_none();
    let texture_gone = surface.texture_source.lock().unwrap().is_none();
    let surfaces = paint_like_paint_gpu(&mut app, 1.0);
    let frames = collect_gpu_surface_frames(&surfaces);
    let drawn = generations.borrow().clone();

    crate::shell::renderer::set_gpu_presenting(false);
    unregister_render_surface(surface.id());

    assert_eq!(drawn_before, 1, "one frame before the loss");
    assert!(target_gone, "the old device's target is dropped");
    assert!(
        texture_gone,
        "the texture registered on the old device is dropped"
    );
    assert_eq!(drawn.len(), 2, "a frame on the new target, unasked");
    assert_eq!(
        drawn[1],
        gpu.generation(),
        "the frame names the device's generation"
    );
    assert!(frames[0].2, "the new frame is fresh: the paint draws it");
}
