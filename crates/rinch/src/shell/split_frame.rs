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

    /// Drop every kept slice (the window stopped splitting).
    pub fn clear(&mut self) {
        self.kept.clear();
        self.scratch = None;
    }

    /// The kept surfaces whose texture is no longer the size the slice was
    /// made for (the image atlas's share moved, #1768): their `extents` and
    /// image transform are stale, so the runtime marks them damaged and the
    /// frame splits them again (#1899 review).
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
                    textures.get(&k.surface_id).map(plain),
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
                    // Registered premultiplied (`SurfaceImages::sync`).
                    straight_alpha: false,
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
