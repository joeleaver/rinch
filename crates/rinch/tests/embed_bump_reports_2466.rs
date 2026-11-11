//! #2466: the embed overlay hands on Vello's bump reports, what each frame's
//! bump buffers needed beside the rules' first sizes, which
//! `examples/bump-calibration` reads. Off until asked, and drained by a take.
//! Needs an adapter, so it is `#[ignore]`d:
//!     cargo test -p rinch --features gpu --test embed_bump_reports_2466 -- --ignored

#![cfg(feature = "gpu")]

use rinch::embed::{RinchContext, RinchContextConfig, RinchOverlayRenderer};
use rinch::prelude::*;
use rinch::wgpu;

const W: u32 = 640;
const H: u32 = 360;

fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a GPU adapter");
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap()
}

/// A page of a few boxes and a line of text.
fn context() -> RinchContext {
    RinchContext::new(
        RinchContextConfig {
            width: W,
            height: H,
            scale_factor: 1.0,
            theme: None,
            fonts: Vec::new(),
        },
        |scope: &mut RenderScope| {
            let root = scope.create_element("div");
            root.set_inner_html(
                "<div style=\"padding: 20px; background: rgb(30,30,40); color: white\">\
                 <div style=\"width: 200px; height: 80px; border-radius: 12px; \
                 background: rgb(60,70,90)\"></div>\
                 <p>Some text, to have glyphs in the scene.</p></div>",
            );
            root
        },
    )
}

/// Draw a frame, and wait for its counters.
fn frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    overlay: &mut RinchOverlayRenderer,
    ctx: &mut RinchContext,
) {
    ctx.update(&[]);
    let _ = overlay.render(device, queue, ctx);
    while overlay.check_frames(device).check_again_at.is_some() {
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }
}

#[test]
#[ignore = "needs a GPU adapter"]
fn gpu_the_overlay_reports_what_its_frames_needed_once_asked_2466() {
    let (device, queue) = gpu();
    let mut ctx = context();
    let mut overlay = RinchOverlayRenderer::new(&device, W, H, wgpu::TextureFormat::Rgba8Unorm);

    // Not asked: nothing kept.
    frame(&device, &queue, &mut overlay, &mut ctx);
    assert!(overlay.take_bump_reports().is_empty(), "off by default");

    overlay.record_bump_reports(8);
    frame(&device, &queue, &mut overlay, &mut ctx);
    let reports = overlay.take_bump_reports();
    let last = reports.last().expect("a report of the frame drawn");
    assert_eq!(last.counters.failed, 0, "the frame fitted");
    assert_eq!(
        (last.config.target_width, last.config.target_height),
        (W, H)
    );
    let needed = last.needed();
    assert!(needed.tiles > 0 && needed.lines > 0, "{needed:?}");
    // A frame that fitted needed no more than it was given, and the rules'
    // first sizes are the least it was given.
    let (given, initial) = (last.given, last.initial());
    assert!(
        needed.tiles <= given.tiles && needed.ptcl <= given.ptcl,
        "{needed:?} {given:?}"
    );
    assert!(
        initial.tiles <= given.tiles && initial.ptcl <= given.ptcl,
        "{initial:?} {given:?}"
    );
    assert!(overlay.take_bump_reports().is_empty(), "a take drains");

    overlay.record_bump_reports(0);
    frame(&device, &queue, &mut overlay, &mut ctx);
    assert!(overlay.take_bump_reports().is_empty(), "stopped");
}
