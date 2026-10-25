//! The split-layer painter's bookkeeping (#1865,
//! `docs/design/split-layer-gpu-surfaces.md`, *Painter split*).
//!
//! On a frame the runtime opens for slicing ([`super::vello_painter::VelloPainter::begin_slicing`]),
//! the Vello painter mirrors every open clip and layer. When a GPU surface is
//! reached ([`super::painter::Painter::split_surface`]) and nothing open
//! around it is a group effect, the surface leaves the scene and gets a
//! **region** R (its content box's bounding box, snapped out to whole device
//! pixels) and an **above** scene in R's own coordinates. From then on every
//! draw is routed by its device bounds:
//!
//! - disjoint from every R: the window's scene (**below**) only, unclipped;
//! - inside one R: that surface's above scene only;
//! - across an edge (or bounds unknown): every above scene it meets, and
//!   below inside the **inverse clip** (the window with every R cut out, the
//!   holes wound backwards and filled non-zero — even-odd nested in a clip
//!   drops coverage on lavapipe). The inverse clip is open only around such
//!   draws and closed before any push or pop, so it never wraps anything else
//!   and never splits a layer.
//!
//! Each surface has two above scenes, both in R's coordinates:
//! - **`above_in`**, what is painted while every clip and layer open at the
//!   split is still open, **without** those (the compositor applies their
//!   coverage once, to the surface and `above_in` together, as the scene
//!   does; applying it to each separately conflates on an antialiased ring);
//! - **`above_out`**, what is painted after the first of them closes, with
//!   the ones still open replayed when it starts. If one still open is not a
//!   whole-pixel rect clip and a draw lands over R, the slice is tainted
//!   (D5a's repaint): its antialiased ring would need a level of its own.
//!
//! Pushes and pops after the split are mirrored into whichever scene is
//! current, so pops balance everywhere.
use peniko::kurbo::{Affine, BezPath, Rect, Shape};
use peniko::{Color, Fill};
use vello::Scene;

use super::painter::{BlendMode, PaintShape, Refusal, SplitVerdict, SurfaceSplit};
use super::vello_painter::centre_sampling;

/// A surface the painter took out of the scene, and what the compositor
/// needs to draw it (`rinch::shell::split_compositor`).
pub struct SurfaceSlice {
    pub surface_id: usize,
    /// R: `(x, y, width, height)` in device pixels, inside the window.
    /// Empty when the surface is wholly off-screen.
    pub region: (u32, u32, u32, u32),
    /// What was painted over the surface while the clips and layers open
    /// around it were still open, without them, in R's coordinates (R's
    /// origin at `(0, 0)`). `None` when nothing was: the common case.
    pub above_in: Option<Scene>,
    /// What was painted over it after they closed, likewise.
    pub above_out: Option<Scene>,
    /// The surface's own coverage: its image rect under its inner radius and
    /// `object-fit` clip.
    pub own: SliceCoverage,
    /// The coverage of the clips open around it.
    pub ancestors: SliceCoverage,
    /// The image-to-device transform the in-scene fill encodes (the fill's
    /// transform times its centre-sampling brush transform).
    pub device_from_image: [f64; 6],
    pub extents: (u32, u32),
    /// Something painted over the surface reads the backdrop it lands on
    /// (`filter: grayscale`'s saturation layer): the frame must be painted
    /// again with this surface in the scene (D5a).
    pub tainted: bool,
}

/// The surface's coverage of R's pixels.
pub enum SliceCoverage {
    /// All of each pixel inside `[x0, y0, x1, y1]` (device pixels), none
    /// outside: the image rect and every clip on it are whole-pixel rects.
    Rect([u32; 4]),
    /// Vello's coverage: a scene in R's coordinates, the clips replayed and a
    /// rect filled white.
    Mask(Box<Scene>),
}

/// A clip or layer open in the scene, mirrored to replay it.
#[derive(Clone)]
pub(crate) enum Open {
    Clip {
        fill: Fill,
        transform: Affine,
        shape: PaintShape,
    },
    Layer {
        blend: BlendMode,
        opacity: f32,
        transform: Affine,
        bounds: PaintShape,
    },
    Isolated {
        opacity: f32,
        transform: Affine,
        bounds: PaintShape,
    },
}

pub(crate) struct Slice {
    pub(crate) out: SurfaceSlice,
    /// R as a rect (empty when off-screen).
    pub(crate) r: Rect,
    /// Device → R's coordinates.
    pub(crate) to_r: Affine,
    pub(crate) above_in: Scene,
    pub(crate) above_in_draws: usize,
    /// Started when the first entry open at the split closes.
    pub(crate) above_out: Option<Scene>,
    pub(crate) above_out_draws: usize,
    /// How many entries were open at the split.
    pub(crate) depth: usize,
    /// How many of those are still open (`depth` until the first closes).
    pub(crate) remaining: usize,
}

impl Slice {
    /// The scene a draw or a push lands in now.
    fn current(&mut self) -> &mut Scene {
        match &mut self.above_out {
            Some(out) => out,
            None => &mut self.above_in,
        }
    }
}

pub(crate) struct Slicing {
    pub(crate) window: Rect,
    pub(crate) open: Vec<Open>,
    pub(crate) slices: Vec<Slice>,
    /// Below is inside the inverse clip.
    pub(crate) inverse_open: bool,
}

/// Whether two rects share any area.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

fn contains(outer: Rect, inner: Rect) -> bool {
    inner.x0 >= outer.x0 && inner.y0 >= outer.y0 && inner.x1 <= outer.x1 && inner.y1 <= outer.y1
}

/// Rotation- and skew-free, with positive scales.
fn axis_aligned(t: Affine) -> bool {
    let [a, b, c, d, e, f] = t.as_coeffs();
    b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 && e.is_finite() && f.is_finite()
}

fn rect_u32(r: Rect) -> [u32; 4] {
    if r.width() > 0.0 && r.height() > 0.0 {
        [r.x0 as u32, r.y0 as u32, r.x1 as u32, r.y1 as u32]
    } else {
        [0; 4]
    }
}

/// A clip that is a rect on whole pixels: its device rect.
fn whole_pixel_clip_rect(o: &Open) -> Option<Rect> {
    match o {
        Open::Clip {
            transform,
            shape: PaintShape::Rect(rect),
            ..
        } => whole_pixel_rect(*transform, *rect),
        _ => None,
    }
}

/// An open entry whose coverage is 0 or 1 everywhere: a whole-pixel rect
/// clip, or a layer that is not a clip at all (a normal layer at opacity 1,
/// the only kind a split allows open).
fn whole_pixel_clip(o: &Open) -> bool {
    match o {
        Open::Clip { .. } => whole_pixel_clip_rect(o).is_some(),
        _ => true,
    }
}

fn whole(v: f64) -> bool {
    v.fract() == 0.0
}

/// `r` under an axis-aligned `t`, if all four edges land on whole pixels.
fn whole_pixel_rect(t: Affine, r: Rect) -> Option<Rect> {
    if !axis_aligned(t) {
        return None;
    }
    let m = t.transform_rect_bbox(r);
    (whole(m.x0) && whole(m.y0) && whole(m.x1) && whole(m.y1)).then_some(m)
}

/// `t` under the slice's pre-transform: the identity leaves `t` exactly as it
/// was, so the window's scene encodes what it always has.
pub(crate) fn pre(pre: Affine, t: Affine) -> Affine {
    if pre == Affine::IDENTITY { t } else { pre * t }
}

pub(crate) fn push_open(scene: &mut Scene, entry: &Open, at: Affine) {
    use super::vello_painter::{push_clip_into, push_layer_into};
    match entry {
        Open::Clip {
            fill,
            transform,
            shape,
        } => push_clip_into(scene, *fill, pre(at, *transform), shape),
        Open::Layer {
            blend,
            opacity,
            transform,
            bounds,
        } => push_layer_into(scene, *blend, *opacity, pre(at, *transform), bounds),
        Open::Isolated {
            opacity,
            transform,
            bounds,
        } => push_layer_into(
            scene,
            BlendMode::Normal,
            *opacity,
            pre(at, *transform),
            bounds,
        ),
    }
}

impl Slicing {
    pub(crate) fn new(window: Rect) -> Self {
        Self {
            window,
            open: Vec::new(),
            slices: Vec::new(),
            inverse_open: false,
        }
    }

    /// The window with every non-empty R cut out, holes wound backwards.
    fn inverse_path(&self) -> BezPath {
        let mut path = self.window.to_path(0.1);
        for s in &self.slices {
            let r = s.r;
            if r.is_zero_area() {
                continue;
            }
            path.move_to((r.x0, r.y0));
            path.line_to((r.x0, r.y1));
            path.line_to((r.x1, r.y1));
            path.line_to((r.x1, r.y0));
            path.close_path();
        }
        path
    }

    pub(crate) fn open_inverse(&mut self, below: &mut Scene) {
        if !self.inverse_open {
            below.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &self.inverse_path());
            self.inverse_open = true;
        }
    }

    pub(crate) fn close_inverse(&mut self, below: &mut Scene) {
        if self.inverse_open {
            below.pop_layer();
            self.inverse_open = false;
        }
    }

    /// Route one draw whose device bounds are `bounds` (`None`: unknown).
    pub(crate) fn draw(
        &mut self,
        below: &mut Scene,
        bounds: Option<Rect>,
        op: &dyn Fn(&mut Scene, Affine),
    ) {
        let mut to_below = true;
        let mut across = false;
        for s in &mut self.slices {
            if s.r.is_zero_area() {
                continue;
            }
            let meets = bounds.is_none_or(|b| overlaps(b, s.r));
            if !meets {
                continue;
            }
            if s.above_out.is_some() {
                if self.open[..s.remaining]
                    .iter()
                    .any(|o| !whole_pixel_clip(o))
                {
                    // A ring still open over R: its coverage would be applied
                    // to this draw separately from the surface's.
                    s.out.tainted = true;
                }
                s.above_out_draws += 1;
            } else {
                s.above_in_draws += 1;
            }
            let to_r = s.to_r;
            op(s.current(), to_r);
            match bounds {
                Some(b) if contains(s.r, b) => to_below = false,
                _ => across = true,
            }
        }
        if to_below {
            if across {
                self.open_inverse(below);
            } else {
                self.close_inverse(below);
            }
            op(below, Affine::IDENTITY);
        }
    }

    /// Mirror a push into every above scene and record it.
    pub(crate) fn push(&mut self, below: &mut Scene, entry: Open) {
        self.close_inverse(below);
        push_open(below, &entry, Affine::IDENTITY);
        if let Open::Layer {
            blend,
            bounds,
            transform,
            ..
        } = &entry
            && !matches!(blend, BlendMode::Normal)
        {
            // A layer that blends with its backdrop over a surface reads a
            // backdrop without the surface in it (D5): unless an isolated
            // layer opened after the split holds it, the frame must be
            // painted in-scene.
            let b = transform.transform_rect_bbox(bounds.bounding_box());
            for s in &mut self.slices {
                let isolated = self.open[s.depth.min(self.open.len())..]
                    .iter()
                    .any(|o| matches!(o, Open::Isolated { .. }));
                if !isolated && !s.r.is_zero_area() && overlaps(b, s.r) {
                    s.out.tainted = true;
                }
            }
        }
        for s in &mut self.slices {
            let to_r = s.to_r;
            push_open(s.current(), &entry, to_r);
        }
        self.open.push(entry);
    }

    pub(crate) fn pop(&mut self, below: &mut Scene) {
        self.close_inverse(below);
        below.pop_layer();
        let index = self.open.len().saturating_sub(1);
        for s in &mut self.slices {
            if index >= s.remaining {
                // Pushed after the split (or after the last close): mirrored.
                s.current().pop_layer();
            } else {
                // An entry open at the split closes: `above_out` starts with
                // the ones still open replayed.
                s.remaining = index;
                match &mut s.above_out {
                    None => {
                        let mut out = Scene::new();
                        for o in &self.open[..index] {
                            push_open(&mut out, o, s.to_r);
                        }
                        s.above_out = Some(out);
                    }
                    // A later one: the top of what was replayed.
                    Some(out) => out.pop_layer(),
                }
            }
        }
        self.open.pop();
    }

    /// Take `split` out of the scene, or say why not.
    pub(crate) fn split(&mut self, split: &SurfaceSplit) -> SplitVerdict {
        if self.open.iter().any(|o| match o {
            Open::Clip { .. } => false,
            Open::Layer { blend, opacity, .. } => {
                !matches!(blend, BlendMode::Normal) || *opacity < 1.0
            }
            Open::Isolated { .. } => true,
        }) {
            return SplitVerdict::InScene(Refusal::GroupEffect);
        }
        if !axis_aligned(split.node_transform) || !axis_aligned(split.image_transform) {
            return SplitVerdict::InScene(Refusal::Transform);
        }
        let b = split.node_transform.transform_rect_bbox(split.content);
        let snapped = Rect::new(b.x0.floor(), b.y0.floor(), b.x1.ceil(), b.y1.ceil());
        let r = snapped.intersect(self.window);
        let r = if r.width() > 0.0 && r.height() > 0.0 {
            r
        } else {
            Rect::ZERO
        };
        if !r.is_zero_area()
            && self
                .slices
                .iter()
                .any(|s| !s.r.is_zero_area() && overlaps(s.r, r))
        {
            return SplitVerdict::InScene(Refusal::Overlap);
        }
        let to_r = Affine::translate((-r.x0, -r.y0));
        let image_rect = Rect::new(0.0, 0.0, split.extents.0 as f64, split.extents.1 as f64);

        // The surface's own coverage: its image rect under its own clips.
        let mut own_rect = whole_pixel_rect(split.image_transform, image_rect);
        if split.inner.is_some() {
            own_rect = None;
        }
        if split.clip_to_content {
            own_rect = own_rect.and_then(|c| {
                whole_pixel_rect(split.node_transform, split.content).map(|k| c.intersect(k))
            });
        }
        let own = match own_rect {
            Some(c) => SliceCoverage::Rect(rect_u32(c.intersect(r))),
            None => {
                let mut mask = Scene::new();
                let mut clips = 0;
                if let Some(inner) = &split.inner {
                    super::vello_painter::push_clip_into(
                        &mut mask,
                        Fill::NonZero,
                        pre(to_r, split.node_transform),
                        inner,
                    );
                    clips += 1;
                }
                if split.clip_to_content {
                    mask.push_clip_layer(
                        Fill::NonZero,
                        pre(to_r, split.node_transform),
                        &split.content,
                    );
                    clips += 1;
                }
                mask.fill(
                    Fill::NonZero,
                    pre(to_r, split.image_transform),
                    Color::WHITE,
                    None,
                    &image_rect,
                );
                for _ in 0..clips {
                    mask.pop_layer();
                }
                SliceCoverage::Mask(Box::new(mask))
            }
        };

        // The ancestors' coverage: the clips open around it.
        let clips: Vec<&Open> = self
            .open
            .iter()
            .filter(|o| matches!(o, Open::Clip { .. }))
            .collect();
        let mut ancestors_rect = Some(r);
        for c in &clips {
            ancestors_rect =
                ancestors_rect.and_then(|k| whole_pixel_clip_rect(c).map(|w| k.intersect(w)));
        }
        let ancestors = match ancestors_rect {
            Some(c) => SliceCoverage::Rect(rect_u32(c)),
            None => {
                let mut mask = Scene::new();
                for c in &clips {
                    push_open(&mut mask, c, to_r);
                }
                mask.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    Color::WHITE,
                    None,
                    &Rect::new(0.0, 0.0, r.width(), r.height()),
                );
                for _ in 0..clips.len() {
                    mask.pop_layer();
                }
                SliceCoverage::Mask(Box::new(mask))
            }
        };

        let t = split.image_transform;
        self.slices.push(Slice {
            out: SurfaceSlice {
                surface_id: split.surface_id,
                region: (
                    r.x0 as u32,
                    r.y0 as u32,
                    r.width() as u32,
                    r.height() as u32,
                ),
                above_in: None,
                above_out: None,
                own,
                ancestors,
                device_from_image: (t * centre_sampling(t, Affine::IDENTITY)).as_coeffs(),
                extents: split.extents,
                tainted: false,
            },
            r,
            to_r,
            above_in: Scene::new(),
            above_in_draws: 0,
            above_out: None,
            above_out_draws: 0,
            depth: self.open.len(),
            remaining: self.open.len(),
        });
        SplitVerdict::Split
    }

    /// The frame's slices; below's inverse clip closed.
    pub(crate) fn finish(mut self, below: &mut Scene) -> Vec<SurfaceSlice> {
        self.close_inverse(below);
        self.slices
            .into_iter()
            .map(|s| {
                let mut out = s.out;
                out.above_in = (s.above_in_draws > 0).then_some(s.above_in);
                out.above_out = s.above_out.filter(|_| s.above_out_draws > 0);
                out
            })
            .collect()
    }
}
