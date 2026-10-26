//! The split surfaces a window composites (#1865,
//! `docs/design/split-layer-gpu-surfaces.md`, *Damage per slice*).
//!
//! Paint hands the window, frame by frame, a [`SplitChange`]: the surfaces
//! whose regions the frame repainted (their slices are replaced) and the
//! slices the painter produced (`rinch_dom::paint::slicing::SurfaceSlice`).
//! [`SplitFrame`] renders each new slice's scenes (the above slices and the
//! coverage masks) into region-sized textures once, and keeps them: a frame
//! in which only a surface drew renders nothing with Vello and composites
//! from what is kept.
use std::collections::HashMap;

use rinch_dom::paint::slicing::{SliceCoverage, SurfaceSlice};
use vello::peniko::Color;
use vello::{AaConfig, RenderParams, Renderer, Scene};
use wgpu::{
    CommandEncoder, Device, Queue, Texture, TextureFormat, TextureUsages, TextureView,
    TextureViewDescriptor,
};

use super::split_compositor::{Coverage, SplitCompositor, SurfaceDraw};

/// What a painted frame changes in the window's split surfaces.
#[derive(Default)]
pub struct SplitChange {
    /// The surfaces whose kept slices this frame replaces (its damage met
    /// their regions, or the frame was full). Every kept slice when `all`.
    pub drop: Vec<usize>,
    pub all: bool,
    /// The slices this frame produced, in paint order.
    pub add: Vec<SurfaceSlice>,
}

/// A kept slice as the runtime oracle reads it: region, then own mask,
/// ancestors' mask, `above_in`, `above_out`.
#[cfg(test)]
pub(crate) type KeptSlice<'a> = ((u32, u32, u32, u32), [Option<&'a Texture>; 4]);

enum KeptCoverage {
    Rect([u32; 4]),
    Mask(Texture),
}

struct Kept {
    surface_id: usize,
    region: (u32, u32, u32, u32),
    device_from_image: [f64; 6],
    extents: (u32, u32),
    straight_alpha: bool,
    own: KeptCoverage,
    ancestors: KeptCoverage,
    above_in: Option<Texture>,
    above_out: Option<Texture>,
}

impl Kept {
    fn cost(&self) -> SliceCost {
        let mask = |c: &KeptCoverage| u32::from(matches!(c, KeptCoverage::Mask(_)));
        SliceCost {
            surface_id: self.surface_id,
            region: self.region,
            textures: mask(&self.own)
                + mask(&self.ancestors)
                + u32::from(self.above_in.is_some())
                + u32::from(self.above_out.is_some()),
            upload: 0,
        }
    }
}

/// A window's split surfaces, their slices rendered.
#[derive(Default)]
pub struct SplitFrame {
    kept: Vec<Kept>,
    /// Where slices are rendered before their region is kept.
    scratch: Option<Texture>,
    /// CPU frames uploaded for split viewports (video), by id: their
    /// textures, as a registered GPU frame's texture is for a surface.
    pixels: HashMap<usize, Texture>,
}

/// The scratch target slices are rendered into grows in steps of this, as
/// the partial-frame target does (`desktop::PARTIAL_TARGET_STEP`).
const SCRATCH_STEP: u32 = 128;

/// The window's split memory may be this many times its own texture bytes
/// (`width * height * 4`) (#1865 D6). Counted: what splitting adds over
/// drawing in the scene, the scratch target and every kept slice's rendered
/// textures (above slices and coverage masks). Not counted: the window's own
/// textures, a `RenderSurface`'s texture (the app's), and the CPU frames
/// uploaded for split video ([`COUNT_UPLOADS`]).
pub const SPLIT_MEMORY_CAP_WINDOWS: u64 = 4;

/// Whether the uploaded video frames count against the cap. They do not
/// (D6, second amendment, Joe 2026-10-25): in the scene the same frame sits
/// in Vello's image atlas, so splitting a video adds no memory for it. They
/// are still allocated, and freed, by [`SplitFrame::upload_split_frames`].
pub const COUNT_UPLOADS: bool = false;

/// What one split surface holds in GPU memory, as [`split_bytes`] counts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SliceCost {
    pub surface_id: usize,
    /// R, `(x, y, width, height)`.
    pub region: (u32, u32, u32, u32),
    /// The R-sized textures rendered and kept for it: each coverage mask
    /// and each above slice.
    pub textures: u32,
    /// The bytes of its uploaded CPU frame (video), or 0.
    pub upload: u64,
}

impl SliceCost {
    pub fn of(s: &SurfaceSlice) -> Self {
        let mask = |c: &SliceCoverage| u32::from(matches!(c, SliceCoverage::Mask(_)));
        Self {
            surface_id: s.surface_id,
            region: s.region,
            textures: mask(&s.own)
                + mask(&s.ancestors)
                + u32::from(s.above_in.is_some())
                + u32::from(s.above_out.is_some()),
            // A CPU frame is uploaded at its own size (`upload_split_frames`);
            // a registered GPU frame is the app's texture.
            upload: if COUNT_UPLOADS && s.straight_alpha {
                u64::from(s.extents.0) * u64::from(s.extents.1) * 4
            } else {
                0
            },
        }
    }

    fn rendered(&self) -> bool {
        self.textures > 0 && self.region.2 > 0 && self.region.3 > 0
    }
}

fn scratch_step(v: u32) -> u32 {
    v.div_ceil(SCRATCH_STEP).max(1) * SCRATCH_STEP
}

/// The scratch target `costs` need, `(width, height)`: reaching the far
/// corner of every R rendered into it, in [`SCRATCH_STEP`]s; `(0, 0)` when
/// nothing is rendered.
fn scratch_size<'a>(costs: impl IntoIterator<Item = &'a SliceCost>) -> (u32, u32) {
    costs
        .into_iter()
        .filter(|c| c.rendered())
        .fold((0, 0), |(w, h), c| {
            let (x, y, cw, ch) = c.region;
            (w.max(scratch_step(x + cw)), h.max(scratch_step(y + ch)))
        })
}

/// The GPU memory `costs` hold together (#1865 D6): each slice's textures
/// and upload, and the one scratch target they share.
pub fn split_bytes<'a>(costs: impl IntoIterator<Item = &'a SliceCost> + Clone) -> u64 {
    let (sw, sh) = scratch_size(costs.clone());
    let slices: u64 = costs
        .into_iter()
        .map(|c| {
            u64::from(c.region.2) * u64::from(c.region.3) * 4 * u64::from(c.textures) + c.upload
        })
        .sum();
    slices + u64::from(sw) * u64::from(sh) * 4
}

/// The cap for a window of `size` device pixels.
pub fn split_memory_cap(size: (u32, u32)) -> u64 {
    #[cfg(test)]
    if let Some(cap) = CAP_OVERRIDE.with(|c| c.get()) {
        return cap;
    }
    SPLIT_MEMORY_CAP_WINDOWS * u64::from(size.0) * u64::from(size.1) * 4
}

#[cfg(test)]
thread_local! {
    /// A cap the GPU oracles set, so a 400x300 window's slices can pass it.
    pub(crate) static CAP_OVERRIDE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Whether refusals made under the cap for `old` window size are tried
/// again at `new`: only if the cap grew. A resize storm that shrinks or
/// keeps the window keeps them, rather than painting each frame twice to
/// refuse the same surfaces again (#1911 review).
pub fn cap_grew(old: (u32, u32), new: (u32, u32)) -> bool {
    split_memory_cap(new) > split_memory_cap(old)
}

/// Which of `new` (this frame's slices, in paint order) do not fit the cap
/// beside `kept` (the slices kept from earlier frames that this frame did not
/// replace): the first that would pass it, and every one after it (D6: split
/// in paint order until the next one does not fit).
pub fn over_cap(kept: &[SliceCost], new: &[SliceCost], cap: u64) -> Vec<usize> {
    let mut held: Vec<SliceCost> = kept.to_vec();
    for (i, c) in new.iter().enumerate() {
        held.push(*c);
        if split_bytes(&held) > cap {
            return new[i..].iter().map(|c| c.surface_id).collect();
        }
    }
    Vec::new()
}

/// Render `scene` (encoded in window coordinates) where the window draws it,
/// into `scratch` (grown to reach `region`'s far corner), over transparent,
/// with Msaa16 as the window's scene is, and keep `region` of it as a new
/// region-sized texture. Rendering it moved to R's origin instead would not
/// give the same pixels: an MSAA sample on a curve's edge flips under a
/// translation.
fn render_slice(
    device: &Device,
    queue: &Queue,
    renderer: &mut Renderer,
    scratch: &mut Option<Texture>,
    scene: &Scene,
    (x, y, w, h): (u32, u32, u32, u32),
) -> Result<Texture, String> {
    let (want_w, want_h) = (scratch_step(x + w), scratch_step(y + h));
    let (have_w, have_h) = scratch.as_ref().map_or((0, 0), |t| (t.width(), t.height()));
    if have_w < want_w || have_h < want_h {
        *scratch = Some(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("split slice scratch"),
            size: wgpu::Extent3d {
                width: want_w.max(have_w),
                height: want_h.max(have_h),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
    }
    let target = scratch.as_ref().expect("created above");
    renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &target.create_view(&TextureViewDescriptor::default()),
            &RenderParams {
                base_color: Color::TRANSPARENT,
                width: want_w,
                height: want_h,
                antialiasing_method: AaConfig::Msaa16,
            },
        )
        .map_err(|e| format!("split slice render: {e:?}"))?;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("split slice"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        // COPY_SRC: the runtime oracle reads the slices back for its classes.
        usage: TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("split slice copy"),
    });
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        texture.as_image_copy(),
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    Ok(texture)
}

/// A kept coverage as the compositor takes it; a mask not viewed is empty.
fn coverage<'a>(c: &KeptCoverage, m: Option<&'a TextureView>) -> Coverage<'a> {
    match (c, m) {
        (KeptCoverage::Rect(r), _) => Coverage::Rect(*r),
        (KeptCoverage::Mask(_), Some(m)) => Coverage::Mask(m),
        (KeptCoverage::Mask(_), None) => Coverage::Rect([0; 4]),
    }
}

impl SplitFrame {
    /// Whether anything is split.
    pub fn is_empty(&self) -> bool {
        self.kept.is_empty()
    }

    /// The split surfaces, by id.
    pub fn surface_ids(&self) -> impl Iterator<Item = usize> + '_ {
        self.kept.iter().map(|k| k.surface_id)
    }

    /// Each kept slice's region and its rendered textures (own mask,
    /// ancestors' mask, `above_in`, `above_out`), for the runtime oracle.
    #[cfg(test)]
    pub(crate) fn slices(&self) -> Vec<KeptSlice<'_>> {
        fn mask(c: &KeptCoverage) -> Option<&Texture> {
            match c {
                KeptCoverage::Mask(t) => Some(t),
                KeptCoverage::Rect(_) => None,
            }
        }
        self.kept
            .iter()
            .map(|k| {
                (
                    k.region,
                    [
                        mask(&k.own),
                        mask(&k.ancestors),
                        k.above_in.as_ref(),
                        k.above_out.as_ref(),
                    ],
                )
            })
            .collect()
    }

    /// Upload the CPU frames of the videos this window keeps split, after
    /// the frame's change is applied (`pending`, from
    /// [`prepare_split_viewports`]), one texture per id, and say how many
    /// were uploaded. A video not split (hidden, culled, refused) uploads
    /// nothing and loses its texture. A frame is uploaded when it is new or
    /// its texture is missing or of another size: the old layer path's
    /// upload, once per frame.
    pub fn upload_split_frames(
        &mut self,
        device: &Device,
        queue: &Queue,
        pending: Vec<PendingFrame>,
    ) -> u64 {
        let mut uploads = 0;
        let mut now = HashMap::new();
        for (id, px, fresh) in pending {
            if !self.kept.iter().any(|k| k.surface_id == id) {
                continue;
            }
            let expected = (px.width as usize) * (px.height as usize) * 4;
            if px.width == 0 || px.height == 0 || px.data.len() < expected {
                continue;
            }
            let kept = self
                .pixels
                .remove(&id)
                .filter(|t| t.width() == px.width && t.height() == px.height);
            let upload = fresh || kept.is_none();
            let texture = kept.unwrap_or_else(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("split viewport frame"),
                    size: wgpu::Extent3d {
                        width: px.width,
                        height: px.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: TextureFormat::Rgba8Unorm,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            });
            if upload {
                uploads += 1;
                queue.write_texture(
                    texture.as_image_copy(),
                    &px.data[..expected],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(px.width * 4),
                        rows_per_image: Some(px.height),
                    },
                    wgpu::Extent3d {
                        width: px.width,
                        height: px.height,
                        depth_or_array_layers: 1,
                    },
                );
            }
            now.insert(id, texture);
        }
        self.pixels = now;
        uploads
    }

    /// The GPU memory this window's split surfaces hold now (#1865 D6): the
    /// kept slices' textures, the uploaded frames and the scratch target, as
    /// allocated.
    #[cfg(test)]
    pub(crate) fn resident_bytes(&self) -> u64 {
        fn mask(c: &KeptCoverage) -> Option<&Texture> {
            match c {
                KeptCoverage::Mask(t) => Some(t),
                KeptCoverage::Rect(_) => None,
            }
        }
        let bytes = |t: &Texture| u64::from(t.width()) * u64::from(t.height()) * 4;
        let kept: u64 = self
            .kept
            .iter()
            .flat_map(|k| {
                [
                    mask(&k.own),
                    mask(&k.ancestors),
                    k.above_in.as_ref(),
                    k.above_out.as_ref(),
                ]
            })
            .flatten()
            .map(bytes)
            .sum();
        kept + self.pixels.values().map(bytes).sum::<u64>() + self.scratch.as_ref().map_or(0, bytes)
    }

    /// The bytes of the uploaded video frames this window holds.
    #[cfg(test)]
    pub(crate) fn upload_bytes(&self) -> u64 {
        self.pixels
            .values()
            .map(|t| u64::from(t.width()) * u64::from(t.height()) * 4)
            .sum()
    }

    /// How many uploaded video frames this window holds.
    #[cfg(test)]
    pub(crate) fn pixel_textures(&self) -> usize {
        self.pixels.len()
    }

    /// Drop every kept slice, uploaded frame and the scratch target (the
    /// window stopped splitting).
    pub fn clear(&mut self) {
        self.kept.clear();
        self.pixels.clear();
        self.scratch = None;
    }

    /// The kept `RenderSurface`s whose texture is no longer the size the
    /// slice was made for (the image atlas's share moved, #1768): their
    /// `extents` and image transform are stale, so the runtime marks them
    /// damaged and the frame splits them again (#1899 review). A video's
    /// new frame of another size is caught where it arrives, in
    /// [`prepare_split_viewports`] (#1906 review).
    pub fn stale_ids(&self, textures: &HashMap<usize, Texture>) -> Vec<usize> {
        self.kept
            .iter()
            .filter(|k| {
                textures
                    .get(&k.surface_id)
                    .is_some_and(|t| (t.width(), t.height()) != k.extents)
            })
            .map(|k| k.surface_id)
            .collect()
    }

    /// Apply `change`: drop what it replaces, render what it adds.
    pub fn apply(
        &mut self,
        device: &Device,
        queue: &Queue,
        renderer: &mut Renderer,
        change: SplitChange,
    ) -> Result<(), String> {
        if change.all {
            self.kept.clear();
        } else {
            self.kept.retain(|k| !change.drop.contains(&k.surface_id));
        }
        for s in change.add {
            let size = (s.region.2, s.region.3);
            if size.0 == 0 || size.1 == 0 {
                continue;
            }
            let mut coverage = |c: SliceCoverage| -> Result<KeptCoverage, String> {
                Ok(match c {
                    SliceCoverage::Rect(r) => KeptCoverage::Rect(r),
                    SliceCoverage::Mask(scene) => KeptCoverage::Mask(render_slice(
                        device,
                        queue,
                        renderer,
                        &mut self.scratch,
                        &scene,
                        s.region,
                    )?),
                })
            };
            let own = coverage(s.own)?;
            let ancestors = coverage(s.ancestors)?;
            let mut above = |scene: Option<Scene>| {
                scene
                    .map(|scene| {
                        render_slice(device, queue, renderer, &mut self.scratch, &scene, s.region)
                    })
                    .transpose()
            };
            let above_in = above(s.above_in)?;
            let above_out = above(s.above_out)?;
            self.kept.retain(|k| k.surface_id != s.surface_id);
            self.kept.push(Kept {
                surface_id: s.surface_id,
                region: s.region,
                device_from_image: s.device_from_image,
                extents: s.extents,
                straight_alpha: s.straight_alpha,
                own,
                ancestors,
                above_in,
                above_out,
            });
        }
        // The scratch target is kept only at the size the kept slices need
        // (D6 counts that size): nothing split, or a larger target left by a
        // slice that has gone, and it goes.
        let (need_w, need_h) = scratch_size(&self.kept.iter().map(Kept::cost).collect::<Vec<_>>());
        if self
            .scratch
            .as_ref()
            .is_some_and(|t| t.width() > need_w || t.height() > need_h)
        {
            self.scratch = None;
        }
        Ok(())
    }

    /// Composite `below` (the UI texture) and every kept slice into `target`
    /// (`size`), each surface sampled from `textures` (its registered texture,
    /// premultiplied). A surface whose texture is gone is composited without
    /// its frame, as in-scene would draw none.
    #[allow(clippy::too_many_arguments)]
    pub fn composite(
        &self,
        compositor: &SplitCompositor,
        device: &Device,
        encoder: &mut CommandEncoder,
        below: &TextureView,
        target: &TextureView,
        size: (u32, u32),
        textures: &HashMap<usize, Texture>,
    ) {
        // The plain `Rgba8Unorm` view: a `RenderSurface` texture also carries
        // an `Rgba8UnormSrgb` view format, which would decode on load.
        let plain = |t: &Texture| {
            t.create_view(&TextureViewDescriptor {
                format: Some(TextureFormat::Rgba8Unorm),
                ..Default::default()
            })
        };
        let views: Vec<_> = self
            .kept
            .iter()
            .map(|k| {
                let view = |c: &KeptCoverage| match c {
                    KeptCoverage::Mask(t) => Some(plain(t)),
                    KeptCoverage::Rect(_) => None,
                };
                (
                    textures
                        .get(&k.surface_id)
                        .or_else(|| self.pixels.get(&k.surface_id))
                        .map(plain),
                    view(&k.own),
                    view(&k.ancestors),
                    k.above_in.as_ref().map(plain),
                    k.above_out.as_ref().map(plain),
                )
            })
            .collect();
        let draws: Vec<SurfaceDraw<'_>> = self
            .kept
            .iter()
            .zip(&views)
            .map(|(k, (texture, own, ancestors, above_in, above_out))| {
                // A surface without a texture: nothing of it covers R.
                let (texture, own) = match texture {
                    Some(t) => (t, coverage(&k.own, own.as_ref())),
                    None => (
                        views.iter().find_map(|v| v.0.as_ref()).unwrap_or(below),
                        Coverage::Rect([0; 4]),
                    ),
                };
                SurfaceDraw {
                    texture,
                    extents: k.extents,
                    // A registered GPU frame is premultiplied
                    // (`SurfaceImages::sync`); a CPU frame straight.
                    straight_alpha: k.straight_alpha,
                    device_from_image: k.device_from_image,
                    own,
                    ancestors: coverage(&k.ancestors, ancestors.as_ref()),
                    region: k.region,
                    above_in: above_in.as_ref(),
                    above_out: above_out.as_ref(),
                }
            })
            .collect();
        compositor.composite(device, encoder, below, target, size, &draws);
    }
}

/// A split-able video's frame this paint, uploaded after paint if the
/// window splits it ([`SplitFrame::upload_split_frames`]): its surface id,
/// its pixels (shared, not copied), and whether they are new since the last
/// paint.
pub type PendingFrame = (usize, rinch_dom::paint::SurfacePixelData, bool);

/// The video half of a frame on the split path (#1865 PR 4), shared by the
/// desktop runtime and its oracle, before paint. With splitting `enabled`,
/// the painter may split each video viewport out of the scene
/// (`set_viewport_split_ids`), and a new frame of one the window composites
/// split is not damage (taken out of `frames.fresh`) unless its size is not
/// the slice's: then its image transform is stale, and the frame splits it
/// again (#1906 review: adaptive streaming changes the size mid-stream).
/// Returns the frames to upload after paint, for the videos the frame keeps
/// split. Without `enabled`, no viewport splits and the uploads are dropped.
pub fn prepare_split_viewports(
    frames: &mut crate::render_surface::ViewportFrames,
    split: &mut SplitFrame,
    enabled: bool,
) -> Vec<PendingFrame> {
    if !enabled {
        rinch_dom::paint::set_viewport_split_ids(None);
        split.pixels.clear();
        return Vec::new();
    }
    rinch_dom::paint::set_viewport_split_ids(Some(frames.ids.clone()));
    let pending: Vec<PendingFrame> = frames
        .ids
        .iter()
        .filter_map(|(name, id)| {
            let px = frames.frames.get(name)?;
            Some((*id, px.clone(), frames.fresh.contains(name)))
        })
        .collect();
    frames.fresh.retain(|name| {
        let Some(id) = frames.ids.get(name) else {
            return true;
        };
        let Some(kept) = split.kept.iter().find(|k| k.surface_id == *id) else {
            return true;
        };
        frames
            .frames
            .get(name)
            .is_some_and(|px| (px.width, px.height) != kept.extents)
    });
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::Scene;

    fn cost(id: usize, region: (u32, u32, u32, u32), textures: u32, upload: u64) -> SliceCost {
        SliceCost {
            surface_id: id,
            region,
            textures,
            upload,
        }
    }

    /// The record's worked case (D6): a full-window video with controls over
    /// it holds the scratch target (rounded up to 128 px) and `above_in`,
    /// about 2.07 windows; the cap is 4. Its uploaded frame is not counted
    /// (second amendment), so a 4K frame in a 1080p window splits too.
    #[test]
    fn full_window_video_under_controls_fits_the_cap() {
        for (w, h) in [(1920u32, 1080u32), (3840, 2160)] {
            let window = u64::from(w) * u64::from(h) * 4;
            let mut s = slice(1, (0, 0, w, h), (w, h));
            s.above_in = Some(Scene::new());
            let c = SliceCost::of(&s);
            let scratch = u64::from(scratch_step(w)) * u64::from(scratch_step(h)) * 4;
            assert_eq!(c.upload, 0, "the frame is not counted");
            assert_eq!(split_bytes(&[c]), window + scratch);
            assert!(split_bytes(&[c]) <= split_memory_cap((w, h)));
        }
        // A 4K frame under its controls in a 1080p window.
        let mut s = slice(1, (0, 0, 1920, 1080), (3840, 2160));
        s.above_in = Some(Scene::new());
        let c = SliceCost::of(&s);
        assert_eq!(split_bytes(&[c]), 8_294_400 + 8_847_360);
        assert!(over_cap(&[], &[c], split_memory_cap((1920, 1080))).is_empty());
    }

    /// A video slice as paint makes one: straight alpha, rect coverage.
    fn slice(id: usize, region: (u32, u32, u32, u32), extents: (u32, u32)) -> SurfaceSlice {
        SurfaceSlice {
            surface_id: id,
            region,
            above_in: None,
            above_out: None,
            own: SliceCoverage::Rect([0; 4]),
            ancestors: SliceCoverage::Rect([0; 4]),
            device_from_image: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            extents,
            straight_alpha: true,
            tainted: false,
        }
    }

    /// Only a bigger window retries the cap's refusals.
    #[test]
    fn only_a_bigger_cap_retries_refusals() {
        assert!(cap_grew((800, 600), (801, 600)));
        assert!(!cap_grew((800, 600), (800, 600)));
        assert!(!cap_grew((800, 600), (799, 600)));
        assert!(!cap_grew((800, 600), (1600, 300)), "the same area");
    }

    /// The scratch target is counted once, reaching the far corner of every
    /// rendered region; a slice with no rendered texture needs none.
    #[test]
    fn scratch_counted_once_to_the_far_corner() {
        let a = cost(1, (0, 0, 10, 10), 1, 0);
        let b = cost(2, (200, 0, 10, 300), 2, 0);
        let rect_only = cost(3, (900, 900, 10, 10), 0, 0);
        assert_eq!(split_bytes(&[a]), 400 + 128 * 128 * 4);
        assert_eq!(
            split_bytes(&[a, b, rect_only]),
            400 + 2 * 12_000 + 256 * 384 * 4
        );
        assert_eq!(split_bytes(&[rect_only]), 0);
        assert_eq!(split_bytes(&[cost(4, (0, 0, 0, 0), 0, 77)]), 77);
    }

    /// In paint order until one does not fit: it and every one after it,
    /// even one that would fit alone.
    #[test]
    fn over_cap_refuses_the_first_that_does_not_fit_and_the_rest() {
        let up = |id, bytes| cost(id, (0, 0, 0, 0), 0, bytes);
        assert_eq!(
            over_cap(&[], &[up(1, 40), up(2, 61), up(3, 1)], 100),
            vec![2, 3]
        );
        assert_eq!(
            over_cap(&[], &[up(1, 40), up(2, 60)], 100),
            Vec::<usize>::new()
        );
        // What stays split from earlier frames comes first.
        assert_eq!(over_cap(&[up(9, 70)], &[up(1, 40)], 100), vec![1]);
        assert_eq!(over_cap(&[], &[up(1, 101)], 100), vec![1]);
    }
}
