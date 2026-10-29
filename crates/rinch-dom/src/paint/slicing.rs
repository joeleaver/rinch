//! The split-layer painter's bookkeeping (#1865,
//! `docs/design/split-layer-gpu-surfaces.md`, *Painter split*).
//!
//! On a frame the runtime opens for slicing ([`super::vello_painter::VelloPainter::begin_slicing`]),
//! the Vello painter mirrors every open clip and layer. When a GPU surface is
//! reached ([`super::painter::Painter::split_surface`]) and nothing open
//! around it is a group effect, the surface leaves the scene and gets a
//! **region** R (its content box's bounding box, snapped out to whole device
//! pixels, and one pixel further left: Vello's antialiasing reaches a pixel
//! left of a fractional edge) and **above** scenes. From then on every draw is
//! routed by its device bounds (grown a pixel left, for the same reason):
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
//! Each surface has two above scenes, both in window coordinates (encoded
//! where the window's scene draws them; the renderer keeps R of each):
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

/// The coverage of `shape` (drawn with `transform`) over R: a rect when the
/// shape is a rect on whole device pixels, else a mask scene with the shape
/// filled white.
fn shape_coverage(transform: Affine, shape: &PaintShape, r: Rect) -> SliceCoverage {
    let rect = match shape {
        PaintShape::Rect(rect) => whole_pixel_rect(transform, *rect),
        _ => None,
    };
    match rect {
        Some(a) => SliceCoverage::Rect(rect_u32(a.intersect(r))),
        None => {
            let mut mask = Scene::new();
            let white = peniko::Brush::Solid(Color::WHITE);
            super::vello_painter::fill_into(&mut mask, Fill::NonZero, transform, &white, shape);
            SliceCoverage::Mask(Box::new(mask))
        }
    }
}
use super::vello_painter::centre_sampling;

/// A surface the painter took out of the scene, and what the compositor
/// needs to draw it (`rinch::shell::split_compositor`).
pub struct SurfaceSlice {
    pub surface_id: usize,
    /// R: `(x, y, width, height)` in device pixels, inside the window.
    /// Empty when the surface is wholly off-screen.
    pub region: (u32, u32, u32, u32),
    /// What was painted over the surface while the clips and layers open
    /// around it were still open, without them, in window coordinates (to
    /// be rendered where the window draws it, and R kept). `None` when
    /// nothing was: the common case.
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
    /// Whether the frame's texels are straight alpha.
    pub straight_alpha: bool,
    /// Something painted over the surface reads the backdrop it lands on
    /// (`filter: grayscale`'s saturation layer): the frame must be painted
    /// again with this surface in the scene (D5a).
    pub tainted: bool,
    /// The element's ink in device pixels (`SurfaceSplit::ink`): damage that
    /// meets it repaints the surface, so its region must be repainted whole.
    pub ink: Rect,
    /// Set for a `GameViewport`, the base layer (#1866): the compositor
    /// replaces below with the game inside `clear` rather than drawing the
    /// game over it.
    pub base: Option<BaseSlice>,
}

/// A base layer's slice (#1866): what the game replaces.
pub struct BaseSlice {
    /// The viewport's box under its own radius: where below is replaced.
    /// `own` (the image) lies inside it.
    pub clear: SliceCoverage,
    /// Opaque black under the image inside `clear` (a CPU frame's letterbox).
    pub black_backdrop: bool,
}

/// The surface's coverage of R's pixels.
pub enum SliceCoverage {
    /// All of each pixel inside `[x0, y0, x1, y1]` (device pixels), none
    /// outside: the image rect and every clip on it are whole-pixel rects.
    Rect([u32; 4]),
    /// Vello's coverage: a scene in window coordinates, the clips replayed and
    /// a rect filled white.
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
    pub(crate) above_in: Scene,
    pub(crate) above_in_draws: usize,
    /// Started when the first entry open at the split closes.
    pub(crate) above_out: Option<Scene>,
    pub(crate) above_out_draws: usize,
    /// How many of the entries open at the split are still open.
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
    /// The surfaces the runtime refuses this frame: they did not fit the
    /// window's split memory cap (D6).
    pub(crate) refused: Vec<usize>,
    /// Watched nodes' starts against the splits (S4): see
    /// `VelloPainter::node_starts`.
    pub(crate) starts: std::collections::HashMap<crate::node::RawNodeId, usize>,
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

/// `b` grown one pixel left when its left edge is fractional: Vello's
/// antialiasing of such an edge may reach the pixel left of the shape's
/// bounds (#1894 review), so routing treats that pixel as touched. A
/// whole-pixel left edge is not grown: two surfaces side by side at whole
/// pixels then touch without overlapping (#1899 review: the growth refused
/// the second and made every frame a partial repaint).
fn spill(b: Rect) -> Rect {
    if b.x0.fract() == 0.0 {
        b
    } else {
        Rect::new(b.x0 - 1.0, b.y0, b.x1, b.y1)
    }
}

pub(crate) fn push_open(scene: &mut Scene, entry: &Open) {
    use super::vello_painter::{push_clip_into, push_layer_into};
    match entry {
        Open::Clip {
            fill,
            transform,
            shape,
        } => push_clip_into(scene, *fill, *transform, shape),
        Open::Layer {
            blend,
            opacity,
            transform,
            bounds,
        } => push_layer_into(scene, *blend, *opacity, *transform, bounds),
        Open::Isolated {
            opacity,
            transform,
            bounds,
        } => push_layer_into(scene, BlendMode::Normal, *opacity, *transform, bounds),
    }
}

impl Slicing {
    pub(crate) fn new(window: Rect) -> Self {
        Self {
            window,
            open: Vec::new(),
            slices: Vec::new(),
            inverse_open: false,
            refused: Vec::new(),
            starts: Default::default(),
        }
    }

    /// The window with every non-empty R cut out, holes wound backwards. The
    /// outer edge lies well outside the window: where a clip's edge runs
    /// through a Vello tile the tile's content goes through the 8-bit blend
    /// stack, and an outer edge on the window's own edge did that to every
    /// tile along it (#1899 review's random sequences: a sidebar's shadow at
    /// the window's bottom, one level off, far from any region).
    fn inverse_path(&self) -> BezPath {
        let mut path = self.window.inflate(64.0, 64.0).to_path(0.1);
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
        op: &dyn Fn(&mut Scene),
    ) {
        let bounds = bounds.map(spill);
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
            op(s.current());
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
            op(below);
        }
    }

    /// Mirror a push into every above scene and record it.
    pub(crate) fn push(&mut self, below: &mut Scene, entry: Open) {
        self.close_inverse(below);
        push_open(below, &entry);
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
            let b = spill(transform.transform_rect_bbox(bounds.bounding_box()));
            for s in &mut self.slices {
                // Opened after the split, or after the last of the entries
                // open at it closed: what `s` is drawn into now.
                let isolated = self.open[s.remaining.min(self.open.len())..]
                    .iter()
                    .any(|o| matches!(o, Open::Isolated { .. }));
                if !isolated && !s.r.is_zero_area() && overlaps(b, s.r) {
                    s.out.tainted = true;
                }
            }
        }
        for s in &mut self.slices {
            push_open(s.current(), &entry);
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
                            push_open(&mut out, o);
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

    /// Take `split` out of the scene, or say why not. Below's inverse clip is
    /// closed first: it is cut for the regions so far, and a draw after this
    /// split must open one with this region's hole too (#1894 review: a draw
    /// across the second region landed in below inside it).
    pub(crate) fn split(&mut self, below: &mut Scene, split: &SurfaceSplit) -> SplitVerdict {
        self.close_inverse(below);
        if self.refused.contains(&split.surface_id) {
            return SplitVerdict::InScene(Refusal::MemoryCap);
        }
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
        // Snapped out to whole pixels, and a pixel further left where an
        // edge in the first column is fractional (the box's, or the fitted
        // frame's), which Vello's antialiasing may spill left of (`spill`).
        let image = split.image_transform.transform_rect_bbox(Rect::new(
            0.0,
            0.0,
            split.extents.0 as f64,
            split.extents.1 as f64,
        ));
        let first = b.x0.floor();
        let fractional_left =
            b.x0 != first || (image.x0 != image.x0.floor() && image.x0 < first + 1.0);
        let left = if fractional_left { first - 1.0 } else { first };
        let snapped = Rect::new(left, b.y0.floor(), b.x1.ceil(), b.y1.ceil());
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
        // Slices are encoded where the window's scene draws them, not moved to
        // R's origin: Vello's output for a pixel depends on where the target
        // puts it (an MSAA sample on a curve's edge flips under a translation,
        // 16 levels; #1718's note on partial frames). The renderer draws each
        // into a target covering R and keeps R.
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
                        split.node_transform,
                        inner,
                    );
                    clips += 1;
                }
                if split.clip_to_content {
                    mask.push_clip_layer(Fill::NonZero, split.node_transform, &split.content);
                    clips += 1;
                }
                mask.fill(
                    Fill::NonZero,
                    split.image_transform,
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
                    push_open(&mut mask, c);
                }
                mask.fill(Fill::NonZero, Affine::IDENTITY, Color::WHITE, None, &r);
                for _ in 0..clips.len() {
                    mask.pop_layer();
                }
                SliceCoverage::Mask(Box::new(mask))
            }
        };

        // A base layer's clear: its box under its own radius (#1866).
        let base = split.base.as_ref().map(|b| BaseSlice {
            clear: shape_coverage(split.node_transform, &b.clear, r),
            black_backdrop: b.black_backdrop,
        });

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
                straight_alpha: split.straight_alpha,
                ink: split.ink,
                tainted: false,
                base,
            },
            r,
            above_in: Scene::new(),
            above_in_draws: 0,
            above_out: None,
            above_out_draws: 0,
            remaining: self.open.len(),
        });
        SplitVerdict::Split
    }

    /// Taint every split surface: something of unknown bounds that blends
    /// with its backdrop is painted (an appended scene, the drag ghost).
    pub(crate) fn taint_all(&mut self) {
        for s in &mut self.slices {
            s.out.tainted = true;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use peniko::kurbo::{ParamCurveArea, PathEl};

    fn split_at(slicing: &mut Slicing, below: &mut Scene, id: usize, content: Rect) {
        let verdict = slicing.split(
            below,
            &SurfaceSplit {
                surface_id: id,
                content,
                node_transform: Affine::IDENTITY,
                inner: None,
                clip_to_content: false,
                image_transform: Affine::translate((content.x0, content.y0)),
                extents: (content.width() as u32, content.height() as u32),
                straight_alpha: false,
                ink: content,
                base: None,
            },
        );
        assert_eq!(verdict, SplitVerdict::Split);
    }

    /// Each subpath's signed area.
    fn areas(path: &BezPath) -> Vec<f64> {
        let mut out = Vec::new();
        let mut sub = BezPath::new();
        for el in path.elements() {
            if matches!(el, PathEl::MoveTo(_)) && !sub.elements().is_empty() {
                out.push(sub.segments().map(|s| s.signed_area()).sum());
                sub = BezPath::new();
            }
            sub.push(*el);
        }
        if !sub.elements().is_empty() {
            out.push(sub.segments().map(|s| s.signed_area()).sum());
        }
        out
    }

    /// The inverse clip, filled non-zero: the window wound one way and every
    /// region's hole the other, so the holes are holes (#1894 review: wound
    /// the same way, a region kept its after-content beneath the surface,
    /// 47 levels). Even-odd would not need it, and misrenders on lavapipe.
    #[test]
    fn the_inverse_clip_winds_each_hole_against_the_window() {
        let mut slicing = Slicing::new(Rect::new(0.0, 0.0, 200.0, 120.0));
        let mut below = Scene::new();
        split_at(
            &mut slicing,
            &mut below,
            1,
            Rect::new(20.0, 12.0, 60.0, 38.0),
        );
        split_at(
            &mut slicing,
            &mut below,
            2,
            Rect::new(100.0, 60.0, 140.0, 86.0),
        );
        let a = areas(&slicing.inverse_path());
        assert_eq!(a.len(), 3, "the window and two holes");
        assert!(a[0].abs() > 0.0);
        for hole in &a[1..] {
            assert!(
                hole.signum() == -a[0].signum(),
                "a hole wound as the window is: {a:?}"
            );
        }
    }

    /// A second split closes below's inverse clip, so the next draw across a
    /// region opens one cut for every region so far.
    #[test]
    fn a_split_closes_the_inverse_clip() {
        let mut slicing = Slicing::new(Rect::new(0.0, 0.0, 200.0, 120.0));
        let mut below = Scene::new();
        split_at(
            &mut slicing,
            &mut below,
            1,
            Rect::new(20.0, 12.0, 60.0, 38.0),
        );
        slicing.open_inverse(&mut below);
        split_at(
            &mut slicing,
            &mut below,
            2,
            Rect::new(100.0, 60.0, 140.0, 86.0),
        );
        assert!(!slicing.inverse_open);
    }
}
