//! #1865: the split painter's real output on whole pages, rendered and
//! composited by the window's own `SplitFrame` (slices rendered where the
//! window draws them, the split compositor), against the in-scene render of
//! the same page. Built by the #1894 review (its curated pages and random
//! page generator, `split_pages_1865_tests.rs`), adopted so the coverage
//! stays in CI. Pages are 200x120 at scale 1, painted straight from a
//! `RinchDocument` (no runtime): the runtime's own steps are
//! `split_runtime_1865_tests.rs`'s.
use std::collections::HashMap;
use std::sync::Arc;

use super::split_frame::{SplitChange, SplitFrame};
use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;
use rinch_dom::paint::painter::Painter;
use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
use rinch_dom::paint::vello_painter::VelloPainter;
use vello::kurbo::Rect;
use vello::peniko::{Blob, Color, ImageAlphaType, ImageData, ImageFormat};
use vello::{AaConfig, RenderParams, Renderer, Scene};
use wgpu::{Device, Queue, Texture, TextureFormat, TextureUsages};

use super::split_compositor::SplitCompositor;
use super::split_slices_1865_tests::{device, renderer};

pub(crate) const W: u32 = 200;
pub(crate) const H: u32 = 120;

fn texture(device: &Device, (w, h): (u32, u32), usage: TextureUsages) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split_pages_1865"),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
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

fn render(
    device: &Device,
    queue: &Queue,
    r: &mut Renderer,
    scene: &Scene,
    size: (u32, u32),
    base: Color,
) -> Texture {
    let out = texture(
        device,
        size,
        TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
    );
    r.render_to_texture(
        device,
        queue,
        scene,
        &out.create_view(&Default::default()),
        &RenderParams {
            base_color: base,
            width: size.0.max(1),
            height: size.1.max(1),
            antialiasing_method: AaConfig::Msaa16,
        },
    )
    .expect("render");
    out
}

fn read(device: &Device, queue: &Queue, texture: &Texture) -> Vec<[u8; 4]> {
    let (w, h) = (texture.width(), texture.height());
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * h) as u64,
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
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("the buffer is mapped");
    (0..h)
        .flat_map(|y| {
            let data = &data;
            (0..w).map(move |x| {
                let i = (y * row + x * 4) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            })
        })
        .collect()
}

fn upload(device: &Device, queue: &Queue, size: (u32, u32), px: &[u8]) -> Texture {
    let t = texture(
        device,
        size,
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::COPY_DST,
    );
    queue.write_texture(
        t.as_image_copy(),
        px,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.0 * 4),
            rows_per_image: Some(size.1),
        },
        wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
    );
    t
}

/// A surface frame's texels: a varied, opaque (or translucent) pattern.
pub(crate) fn frame_pixels(w: u32, h: u32, translucent: bool, seed: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let a = if translucent {
                ((x * 37 + y * 11 + seed) % 200 + 40) as u8
            } else {
                255
            };
            // Premultiplied, as a registered GPU frame is (`SurfaceImages`).
            let m = |c: u32| ((c % 256) * a as u32 / 255) as u8;
            v.extend_from_slice(&[
                m(x * 13 + seed * 7),
                m(y * 29 + seed),
                m(x * y + seed * 3),
                a,
            ]);
        }
    }
    v
}

pub(crate) struct Frame {
    pub id: usize,
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

fn image(f: &Frame) -> ImageData {
    ImageData {
        data: Blob::new(Arc::new(f.px.clone())),
        format: ImageFormat::Rgba8,
        alpha_type: ImageAlphaType::AlphaPremultiplied,
        width: f.w,
        height: f.h,
    }
}

pub(crate) fn doc_from_html(html: &str) -> RinchDocument {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    doc.set_attribute(body, "style", "margin: 0");
    doc.set_inner_html(body, html);
    doc.resolve_layout(W as f32, H as f32);
    doc
}

pub(crate) fn paint(
    doc: &mut RinchDocument,
    frames: &[Frame],
    slicing: bool,
) -> (Scene, Vec<SurfaceSlice>) {
    rinch_dom::paint::set_surface_gpu_images(Some(
        frames
            .iter()
            .map(|f| (f.id, image(f)))
            .collect::<HashMap<_, _>>(),
    ));
    let mut painter = VelloPainter::new();
    painter.reset();
    if slicing {
        painter.begin_slicing(Rect::new(0.0, 0.0, W as f64, H as f64));
    }
    let mut layout_cx: parley::LayoutContext<vello::peniko::Brush> = parley::LayoutContext::new();
    rinch_dom::paint::paint_document(
        &doc.tree,
        &mut painter,
        1.0,
        (W as f32, H as f32),
        &mut doc.font_cx,
        &mut layout_cx,
    );
    let slices = painter.take_slices();
    rinch_dom::paint::set_surface_gpu_images(None);
    (painter.scene().clone(), slices)
}

/// A pixel off: `(x, y, split, in_scene, difference)`.
pub(crate) type Off = (u32, u32, [u8; 4], [u8; 4], i32);

pub(crate) struct Outcome {
    pub allow: Vec<i32>,
    pub in_scene: Vec<[u8; 4]>,
    pub split: Vec<[u8; 4]>,
    /// Each split surface's region, and whether it was tainted.
    pub regions: Vec<(u32, u32, u32, u32)>,
    pub tainted: Vec<bool>,
}

impl Outcome {
    /// `(x, y, split, in_scene, maxdiff)` for every pixel over `tol`.
    pub fn over(&self, tol: i32) -> Vec<Off> {
        self.over_by(|i| tol.max(if tol > 0 { self.allow[i] } else { 0 }))
    }
    /// Every pixel past its D10 classes' allowance (`allow`), with no floor.
    pub fn over_classes(&self) -> Vec<Off> {
        self.over_by(|i| self.allow[i])
    }

    fn over_by(&self, allowed: impl Fn(usize) -> i32) -> Vec<Off> {
        let mut out = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) as usize;
                let (a, b) = (self.split[i], self.in_scene[i]);
                let d = (0..4)
                    .map(|c| (a[c] as i32 - b[c] as i32).abs())
                    .max()
                    .unwrap();
                if d > allowed(i) {
                    out.push((x, y, a, b, d));
                }
            }
        }
        out
    }

    pub fn summary(&self, name: &str) -> String {
        let o = self.over(1);
        let max = self.over(0).iter().map(|v| v.4).max().unwrap_or(0);
        let regions: Vec<_> = self
            .regions
            .iter()
            .zip(&self.tainted)
            .map(|(r, t)| format!("R{r:?} taint={t}"))
            .collect();
        let mut s = format!(
            "{name}: slices={} {:?} max={max} over1={} ",
            self.regions.len(),
            regions,
            o.len()
        );
        for v in o.iter().take(6) {
            s += &format!(
                "[({},{}) split {:?} scene {:?} d{}] ",
                v.0, v.1, v.2, v.3, v.4
            );
        }
        s
    }
}

/// Paint in-scene and split, composite the split, read both.
pub(crate) fn oracle(html: &str, frames: &[Frame]) -> Outcome {
    let (device, queue, errors) = device();
    let mut vello = renderer(&device);
    let mut doc = doc_from_html(html);
    let (in_scene_scene, refused) = paint(&mut doc, frames, false);
    assert!(refused.is_empty());
    let in_scene = read(
        &device,
        &queue,
        &render(
            &device,
            &queue,
            &mut vello,
            &in_scene_scene,
            (W, H),
            Color::WHITE,
        ),
    );
    let (below, slices) = paint(&mut doc, frames, true);
    let below_t = render(&device, &queue, &mut vello, &below, (W, H), Color::WHITE);
    // The surfaces' textures, by id, as the window's registry holds them.
    let textures: HashMap<usize, Texture> = frames
        .iter()
        .map(|f| (f.id, upload(&device, &queue, (f.w, f.h), &f.px)))
        .collect();
    // The ancestors' masks, read back for D8's allowance below.
    let anc_masks: Vec<Option<Vec<[u8; 4]>>> = slices
        .iter()
        .map(|s| match &s.ancestors {
            SliceCoverage::Mask(m) if s.region.2 > 0 && s.region.3 > 0 => {
                let (rx, ry, rw, rh) = s.region;
                let full = read(
                    &device,
                    &queue,
                    &render(
                        &device,
                        &queue,
                        &mut vello,
                        m,
                        (rx + rw, ry + rh),
                        Color::TRANSPARENT,
                    ),
                );
                Some(
                    (0..rh)
                        .flat_map(|y| {
                            let full = &full;
                            (0..rw).map(move |x| full[((ry + y) * (rx + rw) + rx + x) as usize])
                        })
                        .collect(),
                )
            }
            _ => None,
        })
        .collect();
    let regions: Vec<_> = slices.iter().map(|s| s.region).collect();
    let tainted: Vec<bool> = slices.iter().map(|s| s.tainted).collect();
    // A frame not at its own size on whole pixels is resampled: D10's
    // resampled class (the compositor's bilinear tap against Vello's).
    let resampled: HashMap<usize, bool> = slices
        .iter()
        .map(|s| {
            let [a, b, c, d, e, f] = s.device_from_image;
            let whole = a == 1.0
                && d == 1.0
                && b == 0.0
                && c == 0.0
                && (e - 0.5).fract() == 0.0
                && (f - 0.5).fract() == 0.0;
            (s.surface_id, !whole)
        })
        .collect();
    let mut frame = SplitFrame::default();
    frame
        .apply(
            &device,
            &queue,
            &mut vello,
            SplitChange {
                drop: Vec::new(),
                all: true,
                add: slices,
            },
        )
        .expect("apply");
    let target = texture(
        &device,
        (W, H),
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
    );
    let compositor = SplitCompositor::new(&device, TextureFormat::Rgba8Unorm);
    let mut encoder = device.create_command_encoder(&Default::default());
    frame.composite(
        &compositor,
        &device,
        &mut encoder,
        &below_t.create_view(&Default::default()),
        &target.create_view(&Default::default()),
        (W, H),
        &textures,
    );
    queue.submit(Some(encoder.finish()));
    let split = read(&device, &queue, &target);
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );
    // D10's classes, from this page's own slices, one level each and summed
    // where two meet (a translucent frame under an antialiased above
    // slice). Inside R: a slice texture's fractional coverage, a translucent
    // frame, a resampled one, and one level at least (below where the frame
    // does not cover). Within a tile outside R: two (below's rounding where
    // the inverse clip's edge crosses, an ancestor clip's antialiased edge).
    // Nothing anywhere else (#1899 review: a blanket two levels let a surface
    // dimmed by two pass). Then D8's halo on an ancestor's ring.
    let translucent: HashMap<usize, bool> = frames
        .iter()
        .map(|f| (f.id, f.px.chunks(4).any(|p| p[3] != 255)))
        .collect();
    let mut allow = vec![0; (W * H) as usize];
    let kept_ids: Vec<usize> = frame.surface_ids().collect();
    for (((rx, ry, rw, rh), textures), id) in frame.slices().into_iter().zip(&kept_ids) {
        let read: Vec<Vec<[u8; 4]>> = textures
            .iter()
            .flatten()
            .map(|t| read(&device, &queue, t))
            .collect();
        let frame_class = i32::from(translucent.get(id).copied().unwrap_or(false))
            + i32::from(resampled.get(id).copied().unwrap_or(false));
        for y in 0..H {
            for x in 0..W {
                let i = (y * W + x) as usize;
                if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
                    let local = ((y - ry) * rw + (x - rx)) as usize;
                    let coverage = read
                        .iter()
                        .any(|px| px[local][3] != 0 && px[local][3] != 255);
                    // At least one level: below shows inside R wherever the
                    // frame does not cover it (a letterbox, a rounded
                    // corner), with the inverse clip's rounding.
                    allow[i] = allow[i].max((i32::from(coverage) + frame_class).max(1));
                } else if x + 16 >= rx && x < rx + rw + 16 && y + 16 >= ry && y < ry + rh + 16 {
                    // Below's rounding in a tile the inverse clip's edge
                    // crosses, and an ancestor clip's antialiased edge there.
                    allow[i] = allow[i].max(2);
                }
            }
        }
    }
    for (region, mask) in regions.iter().zip(&anc_masks) {
        let (rx, ry, rw, rh) = *region;
        let Some(px) = mask else { continue };
        for y in 0..rh {
            for x in 0..rw {
                let a = px[(y * rw + x) as usize][3];
                if a != 0 && a != 255 {
                    let c = a as f64 / 255.0;
                    let i = ((ry + y) * W + rx + x) as usize;
                    allow[i] = allow[i].max((c * (1.0 - c) * 255.0).ceil() as i32 + 1);
                }
            }
        }
    }
    Outcome {
        allow,
        in_scene,
        split,
        regions,
        tainted,
    }
}

pub(crate) fn opaque(id: usize, w: u32, h: u32) -> Frame {
    Frame {
        id,
        w,
        h,
        px: frame_pixels(w, h, false, id as u32),
    }
}
