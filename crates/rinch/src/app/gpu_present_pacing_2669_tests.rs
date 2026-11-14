//! #2669 on a real adapter (CI's `test-gpu (lavapipe)`): a Wayland surface
//! whose frame callback is outstanding is not presented to, and a resize
//! while it waits (a configure the compositor sent a surface it does not
//! show) is drawn at the new size once the callback arrives.
//!
//! The swapchain is the renderer's test stand-in (`WgpuRenderer::stand_in`):
//! each present composes into a new texture of the surface's configured size.

use super::*;
use crate::shell::desktop::WgpuRenderer;
use rinch_dom::perf::FullRepaintReason;
use rinch_platform::PlatformRenderer;
use std::sync::Arc;

/// An app whose root fills the window with one colour, at any size.
fn filled_app(w: u32, h: u32) -> RinchApp {
    let mut app = super::perf_expect::new_app(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: fixed; inset: 0; background: rgb(0, 0, 255)",
        );
        root
    });
    app.mount_component(w as f32, h as f32);
    app.resolve_and_repaint(w as f32, h as f32);
    app
}

fn gpu() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    (Arc::new(device), Arc::new(queue))
}

/// A redraw at `size`: the app laid out and painted at it, and the frame
/// handed to the renderer (which presents it, or holds it).
fn redraw(app: &mut RinchApp, r: &mut WgpuRenderer, size: (u32, u32), why: FullRepaintReason) {
    app.resolve_and_repaint(size.0 as f32, size.1 as f32);
    let frame = app.build_gpu_frame(1.0, size, Some(why));
    r.paint_frame(frame, false).unwrap();
}

/// The RGBA pixel at (`x`, `y`) of `texture`.
fn pixel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    x: u32,
    y: u32,
) -> [u8; 4] {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(1),
            },
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    [data[0], data[1], data[2], data[3]]
}

/// deskrinch's acceptance case: a surface the compositor is not showing is
/// resized meanwhile (a configure arrives while its redraw is held), and when
/// it is shown (its frame callback arrives) the frame presented is at the new
/// size, filled to its far corner.
#[test]
#[ignore]
fn gpu_a_held_surface_resized_meanwhile_presents_at_its_new_size_2669() {
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    let (device, queue) = gpu();
    let (old, new) = ((200, 100), (300, 150));
    let mut r = WgpuRenderer::offscreen(
        device.clone(),
        queue.clone(),
        old.0,
        old.1,
        wgpu::TextureFormat::Rgba8Unorm,
    );
    r.stand_in = Some(Vec::new());
    // A Wayland surface presenting in Fifo (the offscreen renderer's mode).
    r.pacing.gate.by_callbacks = true;
    let mut app = filled_app(old.0, old.1);

    // The first frame goes out and asks for a frame callback.
    redraw(&mut app, &mut r, old, FullRepaintReason::FirstFrame);
    let presented = |r: &WgpuRenderer| {
        r.stand_in
            .as_ref()
            .unwrap()
            .iter()
            .map(|t| (t.width(), t.height()))
            .collect::<Vec<_>>()
    };
    assert_eq!(presented(&r), [old]);

    // The compositor does not show the surface, and configures it to a new
    // size: the renderer is resized, and the redraw that follows is held, not
    // presented (at either size).
    r.resize(new.0, new.1);
    app.handle_event(
        PlatformEvent::Resized {
            width: new.0,
            height: new.1,
        },
        new,
        1.0,
    );
    redraw(&mut app, &mut r, new, FullRepaintReason::Resize);
    assert_eq!(
        presented(&r),
        [old],
        "a present while the callback is outstanding"
    );

    // The surface is shown: the callback arrives, winit delivers the held
    // `RedrawRequested`, and that redraw presents at the new size.
    r.pacing.gate.callback_arrived();
    redraw(&mut app, &mut r, new, FullRepaintReason::Resize);
    assert_eq!(presented(&r), [old, new]);
    let frame = r.stand_in.as_ref().unwrap().last().unwrap();
    for (x, y) in [(0, 0), (new.0 - 1, new.1 - 1), (old.0 + 10, old.1 + 10)] {
        assert_eq!(
            pixel(&device, &queue, frame, x, y),
            BLUE,
            "({x}, {y}) of the new frame"
        );
    }
    // And it asked for the next callback.
    assert!(r.pacing.gate.awaiting);
}
