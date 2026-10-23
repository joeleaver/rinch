//! #1866 on the desktop GPU window: a `GameViewport` is the base layer.
//!
//! The window's present path, step by step, minus the window: the app's
//! scene ([`RinchApp::install_viewport_holes`] with the viewport's name, then
//! [`RinchApp::build_scene`]) rendered by Vello over the base colour
//! `WgpuRenderer` picks with a layer present (`ui_base_color`), the game's
//! layer blitted at [`RinchApp::viewport_rect`] over the window's white base,
//! and the UI blended on top (`LayerCompositor`). The pixel tests need an
//! adapter, so they are `#[ignore]`d:
//! `cargo test -p rinch --features gpu --lib -- --ignored gpu_game_viewport_base_layer`.
//!
//! The warning tests only build the scene, so they run everywhere (debug
//! builds). The software painter's pixels for the same rule are
//! `rinch-dom/tests/game_viewport_base_layer_1866_tests.rs`, and the embed
//! host's `tests/embed_game_viewport_base_layer_1866.rs`.

use super::*;

const W: u32 = 400;
const H: u32 = 300;

/// The issue's cases on one page, inside a green ancestor that does not
/// clip: a red earlier sibling under the viewport's top-left, the viewport
/// (`border-radius: 40px`), a blue later sibling over its bottom edge, and a
/// yellow popup (`z-index: 5`, earlier in the DOM) over its top-right.
fn page(name: &str) -> String {
    format!(
        "<div style=\"position: absolute; left: 0; top: 0; width: 400px; height: 300px; \
         background: rgb(0,128,0)\">\
         <div style=\"position: absolute; z-index: 5; left: 250px; top: 40px; width: 40px; \
         height: 40px; background: rgb(255,255,0)\"></div>\
         <div style=\"position: absolute; left: 0; top: 0; width: 120px; height: 120px; \
         background: rgb(255,0,0)\"></div>\
         <div data-viewport=\"{name}\" style=\"position: absolute; left: 50px; top: 50px; \
         width: 300px; height: 200px; border-radius: 40px\"></div>\
         <div style=\"position: absolute; left: 100px; top: 200px; width: 60px; height: 30px; \
         background: rgb(0,0,255)\"></div>\
         </div>"
    )
}

fn app_with(html: String) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_inner_html(&html);
        root
    });
    app.mount_component(W as f32, H as f32);
    app
}

/// Build the scene the GPU window builds for a frame in which `name` has a
/// compositor layer.
fn build(app: &mut RinchApp, name: &str) {
    app.install_viewport_holes(Some([name.to_string()].into_iter().collect()));
    app.build_scene(1.0, (W, H));
    rinch_dom::paint::set_active_viewports(None);
}

#[cfg(debug_assertions)]
#[test]
fn an_opacity_ancestor_warns_on_the_desktop_scene_and_a_plain_one_does_not() {
    let _ = rinch_dom::paint::take_viewport_warnings();
    let mut plain = app_with(page("desk-warn-plain"));
    build(&mut plain, "desk-warn-plain");
    assert_eq!(
        rinch_dom::paint::take_viewport_warnings(),
        Vec::<String>::new()
    );

    let mut faded = app_with(format!(
        "<div class=\"hud\" style=\"opacity: 0.5\">{}</div>",
        page("desk-warn-faded")
    ));
    build(&mut faded, "desk-warn-faded");
    let warnings = rinch_dom::paint::take_viewport_warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("\"desk-warn-faded\"") && warnings[0].contains("div.hud"),
        "{}",
        warnings[0]
    );
}

mod gpu {
    use super::*;
    use crate::shell::compositor::LayerCompositor;
    use wgpu::{Device, Queue, Texture, TextureFormat, TextureViewDescriptor};

    const MAGENTA: [u8; 4] = [255, 0, 255, 255];

    fn device() -> (Device, Queue) {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .expect("a GPU adapter (these fixtures are #[ignore]d for hosts without one)");
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device")
    }

    fn texture(device: &Device, usage: wgpu::TextureUsages) -> Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("game_viewport_base_layer_1866"),
            size: wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage,
            view_formats: &[],
        })
    }

    fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
        let row = (W * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * H) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d {
                width: W,
                height: H,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range();
        (0..H)
            .flat_map(|y| {
                let data = &data;
                (0..W).map(move |x| {
                    let i = (y * row + x * 4) as usize;
                    [data[i], data[i + 1], data[i + 2], data[i + 3]]
                })
            })
            .collect()
    }

    /// The window's pixels: the UI over a magenta game layer at the
    /// viewport's rect, over the window's white base.
    fn composite(app: &mut RinchApp, name: &str) -> Vec<[u8; 4]> {
        let (device, queue) = device();
        build(app, name);
        let usage = wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        let ui = texture(&device, usage);
        let mut renderer = vello::Renderer::new(
            &device,
            vello::RendererOptions {
                antialiasing_support: vello::AaSupport::all(),
                use_cpu: false,
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .expect("vello");
        let base = crate::shell::desktop::ui_base_color(vello::peniko::Color::WHITE, false, true);
        renderer
            .render_to_texture(
                &device,
                &queue,
                app.painter.scene(),
                &ui.create_view(&TextureViewDescriptor::default()),
                &vello::RenderParams {
                    base_color: base,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .expect("render");

        let layer = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let layer_view = layer.create_view(&TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &layer_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        }
        let ((x, y, w, h), radius) = app.viewport_rect_with_radius(name).expect("the viewport");
        let compositor = LayerCompositor::new(&device, TextureFormat::Rgba8Unorm);
        let target = texture(
            &device,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let view = target.create_view(&TextureViewDescriptor::default());
        compositor.blit_layer_view(
            &device,
            &queue,
            &mut encoder,
            &layer_view,
            &view,
            (x, y, w, h),
            (W, H),
            Some(wgpu::Color::WHITE),
            radius,
            None,
        );
        compositor.overlay_ui(&device, &mut encoder, &ui, &view);
        queue.submit(Some(encoder.finish()));
        read(&device, &queue, &target)
    }

    fn near(a: [u8; 4], b: [u8; 4]) -> bool {
        a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    /// The issue's tests on the desktop GPU window, one page: the game shows
    /// through the non-clipping ancestor's background and the earlier
    /// sibling; the later sibling and the popup cover it; its own corners keep
    /// what is beneath them.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn gpu_game_viewport_base_layer_on_the_desktop_window() {
        let name = "desk-base";
        let mut app = app_with(page(name));
        let px = composite(&mut app, name);
        let at = |x: u32, y: u32| px[(y * W + x) as usize];
        let want: &[((u32, u32), [u8; 4], &str)] = &[
            ((200, 150), MAGENTA, "the game, through the ancestor"),
            ((100, 100), MAGENTA, "the game, through the earlier sibling"),
            ((25, 25), [255, 0, 0, 255], "the earlier sibling beside it"),
            ((52, 52), [255, 0, 0, 255], "outside the top-left arc"),
            ((130, 215), [0, 0, 255, 255], "the later sibling over it"),
            ((270, 60), [255, 255, 0, 255], "the popup over it"),
            ((375, 150), [0, 128, 0, 255], "the ancestor beside it"),
            ((347, 247), [0, 128, 0, 255], "outside the bottom-right arc"),
        ];
        let bad: Vec<_> = want
            .iter()
            .filter(|&&((x, y), w, _)| !near(at(x, y), w))
            .map(|&((x, y), w, what)| (what, (x, y), at(x, y), w))
            .collect();
        assert!(bad.is_empty(), "(what, point, got, want): {bad:?}");
    }
}
