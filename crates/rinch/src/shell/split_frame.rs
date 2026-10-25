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
    let step = |v: u32| v.div_ceil(SCRATCH_STEP).max(1) * SCRATCH_STEP;
    let (want_w, want_h) = (step(x + w), step(y + h));
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
        if self.kept.is_empty() {
            // Nothing split: the scratch target (up to the window's size) goes.
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
