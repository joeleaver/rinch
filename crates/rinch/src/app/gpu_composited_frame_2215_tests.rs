//! #2215: `on_frame_composited` hands the app the whole presented frame, also
//! after a partial frame redrew only a region.
//!
//! Needs an adapter, so `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib gpu_composited_frame -- --ignored`
//! (CI's `test-gpu` job, on lavapipe).
use super::*;
use crate::window::{CompositedFrame, FrameAlpha, on_frame_composited};
use std::cell::RefCell;
use std::rc::Rc;

const W: u32 = 256;
const H: u32 = 128;

/// Two boxes side by side, red and blue, on white; the right one's colour
/// is the app's to change.
fn app(right: Rc<RefCell<Option<NodeHandle>>>) -> RinchApp {
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "display: flex; width: 256px; height: 128px");
        let left = scope.create_element("div");
        left.set_attribute("style", "width: 96px; height: 128px; background: #ff0000");
        let r = scope.create_element("div");
        r.set_attribute(
            "style",
            "width: 96px; height: 128px; margin-left: 32px; background: #0000ff",
        );
        root.append_child(&left);
        root.append_child(&r);
        *right.borrow_mut() = Some(r);
        root
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    app
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

/// What a handler saw: the frame's facts, and its pixels as the handler's
/// own copy read them back.
#[derive(Default)]
struct Seen {
    calls: usize,
    facts: Option<((u32, u32), wgpu::TextureFormat, FrameAlpha)>,
    buffer: Option<wgpu::Buffer>,
}

const BPR: u32 = (W * 4).div_ceil(256) * 256;

/// What a recorder does: copy the frame into a buffer of its own through the
/// frame's encoder, to map once the GPU is done.
fn record(seen: &Rc<RefCell<Seen>>) -> impl FnMut(&mut CompositedFrame<'_>) + 'static {
    let seen = seen.clone();
    move |frame: &mut CompositedFrame<'_>| {
        let mut s = seen.borrow_mut();
        s.calls += 1;
        s.facts = Some((frame.size, frame.format, frame.alpha));
        let buffer = frame.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("recorder"),
            size: (BPR * frame.size.1) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        frame.encoder.copy_texture_to_buffer(
            frame.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(BPR),
                    rows_per_image: Some(frame.size.1),
                },
            },
            wgpu::Extent3d {
                width: frame.size.0,
                height: frame.size.1,
                depth_or_array_layers: 1,
            },
        );
        s.buffer = Some(buffer);
    }
}

fn read(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Vec<u8> {
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.slice(..).get_mapped_range().expect("mapped");
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        out.extend_from_slice(&data[(y * BPR) as usize..(y * BPR + W * 4) as usize]);
    }
    out
}

/// After a full frame and then a partial one that redrew only the right box,
/// the texture the window presents from — what the shell hands over — holds
/// the whole frame: the left box from the full frame, the right one in its
/// new colour, and a handler's copy recorded into the frame's encoder reads
/// exactly that. A cancelled handler is not called again.
#[test]
#[ignore]
fn gpu_the_handed_over_frame_is_whole_after_a_partial_frame_2215() {
    use crate::shell::desktop::{PartialFrame, presented_frame, render_partial};
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut renderer = vello::Renderer::new(
        &device,
        vello::RendererOptions {
            antialiasing_support: vello::AaSupport::all(),
            use_cpu: false,
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .unwrap();
    // The UI texture as the shell makes it.
    let ui = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let right = Rc::new(RefCell::new(None));
    let mut app = app(right.clone());
    let GpuFrame::Full(scene) = app.build_gpu_frame(
        1.0,
        (W, H),
        Some(rinch_dom::perf::FullRepaintReason::FirstFrame),
    ) else {
        panic!("the first frame is full")
    };
    renderer
        .render_to_texture(
            &device,
            &queue,
            scene,
            &ui.create_view(&Default::default()),
            &vello::RenderParams {
                base_color: vello::peniko::Color::WHITE,
                width: W,
                height: H,
                antialiasing_method: vello::AaConfig::Msaa16,
            },
        )
        .unwrap();

    // The right box turns green: a partial frame, its damage only.
    right.borrow().as_ref().unwrap().set_attribute(
        "style",
        "width: 96px; height: 128px; margin-left: 32px; background: #00ff00",
    );
    app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(W as f32, H as f32);
    }
    let mut target = None;
    match app.build_gpu_frame(1.0, (W, H), None) {
        GpuFrame::Partial { scene, size, rects } => {
            assert!(
                rects.iter().all(|&(x, _, w, _)| x >= 96 && x + w <= W),
                "the damage is the right box's, not the left: {rects:?}"
            );
            render_partial(
                &device,
                &queue,
                &mut renderer,
                &ui,
                &mut target,
                PartialFrame {
                    scene,
                    size,
                    rects: &rects,
                    base: vello::peniko::Color::WHITE,
                },
            )
            .unwrap();
        }
        GpuFrame::Full(_) => panic!("expected a partial frame, got a full one"),
        GpuFrame::Unchanged => panic!("expected a partial frame, got none"),
    }

    let seen = Rc::new(RefCell::new(Seen::default()));
    let sub = on_frame_composited(record(&seen));
    // What `present` hands over with no layers and no split surfaces.
    let (texture, alpha) = presented_frame(None, &ui, false);
    crate::composited_frame::hand_over_frame(&device, &queue, texture, alpha);

    let s = seen.borrow();
    assert_eq!(s.calls, 1, "one call per presented frame");
    assert_eq!(
        s.facts,
        Some((
            (W, H),
            wgpu::TextureFormat::Rgba8Unorm,
            FrameAlpha::Straight
        ))
    );
    let rgba = read(&device, s.buffer.as_ref().unwrap());
    drop(s);
    assert_eq!(
        pixel(&rgba, 40, 64),
        [255, 0, 0, 255],
        "left box, full frame"
    );
    assert_eq!(
        pixel(&rgba, 170, 64),
        [0, 255, 0, 255],
        "right box, new colour"
    );
    assert_eq!(pixel(&rgba, 112, 64), [255, 255, 255, 255], "the gap");

    sub.cancel();
    crate::composited_frame::hand_over_frame(&device, &queue, texture, alpha);
    assert_eq!(seen.borrow().calls, 1, "cancelled: not called again");
}
