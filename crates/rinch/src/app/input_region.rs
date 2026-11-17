//! A window's input region (#2741): the rectangles, in logical px, where a
//! window with [`InputRegion::Painted`] or [`InputRegion::Marked`] takes
//! pointer input.
//!
//! **The walk is the hit tester's, run forwards.** `hit_testing::hit_test_node`
//! moves a probe point *into* each box (inverting transforms, adding scroll
//! offsets, testing the clips each stacking entry was hoisted past); this moves
//! each box *out* to the viewport by the same steps and keeps the rects. So the
//! two agree on what reaches where: paint order does not matter for a union,
//! but the stacking sequence does decide which clips a hoisted box is under
//! (`PaintOrder::clips_for`), and a `position: fixed` box resolves against the
//! body's space, not its parent's. Where the hit tester tests a point against
//! a clip, this intersects the box with it. A transformed box contributes the
//! bounding box of its transformed border box, as `painted_border_box` does.
//!
//! What counts as painted is [`paints_own_box`]: a background, a visible
//! border, a replaced element or form control, a render surface. Text counts
//! line by line ([`text_lines`]), so a transparent paragraph holding a short
//! label puts the label's extent in the region, not the paragraph's width.
//! Box shadows and outlines are not in the region: they are decoration around
//! a box a click is meant for.
//!
//! **Corners.** Wayland and X11 regions are unions of rectangles, so a
//! `border-radius` corner is approximated by strips ([`rounded_strips`]): each
//! band of rows inset at the curve's depth at the band's middle row.
//!
//! **Cost.** One walk of the box tree per computation, reusing the hit cache's
//! stacking sequences. The shell computes only when the window's region mode
//! is `Painted` or `Marked`, and only when the hit cache's generation, the
//! tree's region generation or the viewport moved since the last computation
//! (`InputRegionState`), so a still window computes it once. The region
//! generation (`NodeTree::region_generation`) is what the hit cache's misses:
//! a transition or animation tick that writes a `transform` value (the hit
//! cache keys only whether it is `none`), or moves a background or border
//! colour across alpha 0. So a sliding popup recomputes the region on each of
//! its frames, and one fading in from `transparent` when it first shows.
//!
//! **Not counted** (documented on `InputRegion::Painted`): an inline
//! element's background padding above or below its line, and a scroller's
//! scrollbars, beyond the boxes and lines that hold them. Text whose colour is
//! transparent still counts: its lines are counted, not its pixels.

use peniko::kurbo::{Affine, Rect};
use rinch_core::element::{INPUT_REGION_ATTRIBUTE, InputRegion};
use rinch_dom::computed_style::{
    BackgroundValue, BorderStyleValue, DisplayValue, PointerEventsValue, PositionValue,
};
use rinch_dom::paint::radii::BorderRadii;

/// The most strips a rounded band is cut into, and the height each aims for
/// (logical px). A 12px radius is four 3px strips; a 40px one is six.
const MAX_STRIPS: usize = 6;
const STRIP_PX: f64 = 3.0;

/// The region's rectangles for `mode`, in logical px, in the viewport. Empty
/// for nothing painted or marked; `All` and `None` are the caller's (they need
/// no walk) and answer empty here.
pub(crate) fn compute(tree: &rinch_dom::NodeTree, mode: InputRegion) -> Vec<Rect> {
    if !matches!(mode, InputRegion::Painted | InputRegion::Marked) {
        return Vec::new();
    }
    tree.perf
        .bump(rinch_dom::perf::Counter::InputRegionComputes);
    let mut walk = Walk {
        tree,
        painted: mode == InputRegion::Painted,
        out: Vec::new(),
        excluded: std::collections::HashMap::new(),
    };
    walk.visit(
        tree.body_id,
        0.0,
        0.0,
        Affine::IDENTITY,
        Rect::new(f64::MIN, f64::MIN, f64::MAX, f64::MAX),
        Affine::IDENTITY,
    );
    dedupe(walk.out)
}

/// Whether the point `(x, y)` (logical px) is in `rects`.
pub(crate) fn contains(rects: &[Rect], x: f32, y: f32) -> bool {
    let (x, y) = (x as f64, y as f64);
    rects
        .iter()
        .any(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
}

/// Whether `(x, y)` (logical px) is in one of the whole-pixel rects
/// [`to_pixels`] made at scale 1: what the windowing system was sent.
pub(crate) fn contains_pixels(rects: &[(i32, i32, i32, i32)], x: f32, y: f32) -> bool {
    rects.iter().any(|&(rx, ry, w, h)| {
        x >= rx as f32 && x < (rx + w) as f32 && y >= ry as f32 && y < (ry + h) as f32
    })
}

/// `rects` as whole-pixel rectangles `(x, y, width, height)` that cover them,
/// at `scale` (1.0 for Wayland's surface-local logical px, the DPI scale for
/// X11's physical px). Each edge is rounded outwards, so the region never
/// loses a painted pixel; duplicates after rounding are dropped.
pub(crate) fn to_pixels(rects: &[Rect], scale: f64) -> Vec<(i32, i32, i32, i32)> {
    let mut out: Vec<(i32, i32, i32, i32)> = Vec::with_capacity(rects.len());
    for r in rects {
        let x0 = (r.x0 * scale).floor();
        let y0 = (r.y0 * scale).floor();
        let x1 = (r.x1 * scale).ceil();
        let y1 = (r.y1 * scale).ceil();
        let clamp = |v: f64| v.clamp(i32::MIN as f64 / 2.0, i32::MAX as f64 / 2.0) as i32;
        let (x0, y0, x1, y1) = (clamp(x0), clamp(y0), clamp(x1), clamp(y1));
        if x1 > x0 && y1 > y0 {
            let px = (x0, y0, x1 - x0, y1 - y0);
            if !out.contains(&px) {
                out.push(px);
            }
        }
    }
    out
}

/// How an element is marked with [`INPUT_REGION_ATTRIBUTE`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// `data-input-region="none"`: the subtree is out of a `Painted` region.
    Out,
    /// Any other value: the element's border box is in.
    In,
}

fn mark_of(node: &rinch_dom::Node) -> Option<Mark> {
    let value = node.attributes.get(INPUT_REGION_ATTRIBUTE)?;
    Some(if value.trim().eq_ignore_ascii_case("none") {
        Mark::Out
    } else {
        Mark::In
    })
}

struct Walk<'a> {
    tree: &'a rinch_dom::NodeTree,
    painted: bool,
    out: Vec<Rect>,
    /// Whether a node or a DOM ancestor is `data-input-region="none"`,
    /// memoised per node for this computation ([`Walk::excluded`]).
    excluded: std::collections::HashMap<u64, bool>,
}

impl Walk<'_> {
    /// Visit `node_id` entered at `offset` in the space `to_view` maps to the
    /// viewport, under the viewport-space `clip`. `body_view` is the body's
    /// own space (after its transform), where a `position: fixed` box is
    /// placed (`hit_testing::descend`).
    fn visit(
        &mut self,
        node_id: u64,
        offset_x: f64,
        offset_y: f64,
        to_view: Affine,
        clip: Rect,
        body_view: Affine,
    ) {
        let tree = self.tree;
        let Some(node) = tree.get_live(node_id) else {
            return;
        };
        tree.perf
            .bump(rinch_dom::perf::Counter::InputRegionNodesVisited);
        let cs = &node.computed_style;
        if cs.display == DisplayValue::None {
            return;
        }

        // `descend`, forwards: the fixed hoist, the IFC content-box offset,
        // the node's transform, the body's re-seed of viewport space.
        let is_fixed = node.box_position() == PositionValue::Fixed;
        let (to_view, (nx, ny)) = if is_fixed {
            (body_view, (node.layout.x as f64, node.layout.y as f64))
        } else {
            let (dx, dy) = rinch_dom::paint::ifc_content_box_offset(tree, node);
            (
                to_view,
                (
                    offset_x + (node.layout.x + dx) as f64,
                    offset_y + (node.layout.y + dy) as f64,
                ),
            )
        };
        let contents_box = cs.display == DisplayValue::Contents
            && (node.layout.width == 0.0 || node.layout.height == 0.0);
        let to_view = if node.has_applied_transform() && !contents_box {
            let t = rinch_dom::paint::compose_node_transform(node, nx, ny, 1.0, Affine::IDENTITY);
            let det = t.determinant();
            if !det.is_finite() || det.abs() < 1e-12 {
                // Paints to zero area (`local_point`'s `None`).
                return;
            }
            to_view * t
        } else {
            to_view
        };
        let body_view = if node_id == tree.body_id {
            to_view
        } else {
            body_view
        };

        let hidden = rinch_dom::visibility::hides_own(tree, node);
        if hidden && !rinch_dom::visibility::shows_below(tree, node_id) {
            return;
        }

        let (w, h) = (node.layout.width as f64, node.layout.height as f64);
        let own = Rect::new(nx, ny, nx + w, ny + h);
        let takes_pointer = !hidden && cs.pointer_events != PointerEventsValue::None;
        if takes_pointer && !contents_box && w > 0.0 && h > 0.0 {
            let own_in = mark_of(node) == Some(Mark::In);
            let out = self.painted && self.excluded(node_id);
            let whole = if own_in {
                true
            } else if !self.painted || out {
                false
            } else {
                paints_own_box(node)
            };
            if whole {
                let radii = rinch_dom::paint::border_radii(node, 1.0);
                self.push_box(own, &radii, to_view, clip);
            } else if self.painted
                && !out
                && node.is_text()
                && node.ifc_root.is_none()
                && node.text_content().is_some_and(|t| !t.trim().is_empty())
            {
                // Text laid out on its own (not by an IFC root).
                self.push(own, to_view, clip);
            }
        }

        // A `mask-image` clips everything the box paints to its border box.
        let clip = if !cs.mask_image.is_empty() && !contents_box {
            clip.intersect(to_view.transform_rect_bbox(own))
        } else {
            clip
        };

        // The clip the node's `overflow` puts on its content: the padding box,
        // per axis (`hit_test_node`'s `check_children`).
        let (clip_x, clip_y) = node.clip_axes();
        let content_clip = if !clip_x && !clip_y {
            clip
        } else {
            let (l, t, r, b) = rinch_dom::paint::padding_box_insets(node);
            let local = Rect::new(
                if clip_x { nx + l as f64 } else { -1e9 },
                if clip_y { ny + t as f64 } else { -1e9 },
                if clip_x { nx + w - r as f64 } else { 1e9 },
                if clip_y { ny + h - b as f64 } else { 1e9 },
            );
            clip.intersect(to_view.transform_rect_bbox(local))
        };

        let sx = node.scroll_offset.0;
        let sy = node.scroll_offset.1;

        // An IFC root's text, line by line.
        if self.painted && takes_pointer && node.text_layout.is_some() {
            if !self.excluded(node_id) {
                for line in text_lines(node, nx - sx, ny - sy) {
                    self.push(line, to_view, content_clip);
                }
            }
        }

        let is_stretched_ifc_text =
            |child: &rinch_dom::Node| child.is_text() && child.ifc_root.is_some();
        let kids = rinch_dom::RinchDocument::box_tree_children(&tree.nodes, node.id);
        if kids.is_empty() {
            return;
        }
        if node_id == tree.body_id || node.creates_stacking_context() {
            // The stacking sequence the hit tester probes, from the same cache.
            let order = super::hit_testing::plain_paint_order(tree, node_id, nx - sx, ny - sy);
            for entry in order.iter() {
                let Some(child) = tree.get_live(entry.node_id) else {
                    continue;
                };
                if is_stretched_ifc_text(child) {
                    continue;
                }
                // A fixed entry is not clipped by this root (#545).
                let mut entry_clip = if child.box_position() == PositionValue::Fixed {
                    clip
                } else {
                    content_clip
                };
                for c in order.clips_for(entry) {
                    entry_clip = entry_clip.intersect(to_view.transform_rect_bbox(c.rect));
                }
                if is_empty(entry_clip) {
                    continue;
                }
                self.visit(
                    entry.node_id,
                    entry.offset_x,
                    entry.offset_y,
                    to_view,
                    entry_clip,
                    body_view,
                );
            }
        } else {
            if is_empty(content_clip) {
                return;
            }
            for &child_id in kids.iter() {
                let Some(child) = tree.get_live(child_id) else {
                    continue;
                };
                if rinch_dom::stacking::paints_at_stacking_root(child)
                    || is_stretched_ifc_text(child)
                {
                    continue;
                }
                self.visit(child_id, nx - sx, ny - sy, to_view, content_clip, body_view);
            }
        }
    }

    /// Whether `node_id` or a DOM ancestor is marked `data-input-region=
    /// "none"`, which takes a subtree out of a `Painted` region (a node's own
    /// `In` mark still puts its box in). The DOM ancestors, not the walk's
    /// path: a hoisted box is visited from its stacking root, past the
    /// ancestors that mark it. Memoised per node, so the computation reads
    /// each node's attribute once.
    fn excluded(&mut self, node_id: u64) -> bool {
        if let Some(&e) = self.excluded.get(&node_id) {
            return e;
        }
        let Some(node) = self.tree.get_live(node_id) else {
            return false;
        };
        let e = if mark_of(node) == Some(Mark::Out) {
            true
        } else if node_id == self.tree.body_id {
            false
        } else {
            node.parent.is_some_and(|p| self.excluded(p))
        };
        self.excluded.insert(node_id, e);
        e
    }

    /// Push `local` (in the space `to_view` maps from) as its viewport
    /// bounding box, cut by `clip`.
    fn push(&mut self, local: Rect, to_view: Affine, clip: Rect) {
        let r = to_view.transform_rect_bbox(local).intersect(clip);
        if !is_empty(r) {
            self.out.push(r);
        }
    }

    /// Push a border box, its rounded corners stepped in where `to_view`
    /// keeps rectangles rectangles (no rotation or skew).
    fn push_box(&mut self, local: Rect, radii: &BorderRadii, to_view: Affine, clip: Rect) {
        let [_, b, c, _, _, _] = to_view.as_coeffs();
        if b != 0.0 || c != 0.0 {
            self.push(local, to_view, clip);
            return;
        }
        for strip in rounded_strips(local, radii) {
            self.push(strip, to_view, clip);
        }
    }
}

fn is_empty(r: Rect) -> bool {
    !(r.x1 > r.x0 && r.y1 > r.y0)
}

/// Whether an element's own box draws something a click is meant for: a
/// background colour or image, a visible border, a replaced element, a form
/// control, a render surface or viewport.
fn paints_own_box(node: &rinch_dom::Node) -> bool {
    if !node.is_element() {
        return false;
    }
    if let Some(tag) = node.tag()
        && matches!(
            tag,
            "img"
                | "svg"
                | "video"
                | "canvas"
                | "iframe"
                | "input"
                | "textarea"
                | "select"
                | "button"
                | "progress"
                | "meter"
        )
    {
        return true;
    }
    if node.attributes.contains_key("data-render-surface")
        || node.attributes.contains_key("data-viewport")
    {
        return true;
    }
    let cs = &node.computed_style;
    let opaque = |c: &peniko::Color| c.components[3] > 0.0;
    let background = match &cs.background {
        BackgroundValue::None => false,
        BackgroundValue::Color(c) => opaque(c),
        _ => true,
    };
    if background || cs.background_underlay.as_ref().is_some_and(opaque) {
        return true;
    }
    if !cs.background_layers.is_empty() {
        return true;
    }
    let side = |width: f32, style: BorderStyleValue, color: &Option<peniko::Color>| {
        width > 0.0
            && !matches!(style, BorderStyleValue::None | BorderStyleValue::Hidden)
            && color.as_ref().is_none_or(opaque)
    };
    side(
        cs.border_top_width.to_px(),
        cs.border_top_style,
        &cs.border_top_color,
    ) || side(
        cs.border_right_width.to_px(),
        cs.border_right_style,
        &cs.border_right_color,
    ) || side(
        cs.border_bottom_width.to_px(),
        cs.border_bottom_style,
        &cs.border_bottom_color,
    ) || side(
        cs.border_left_width.to_px(),
        cs.border_left_style,
        &cs.border_left_color,
    )
}

/// The lines of IFC root `node`'s text, each from the line's start to its end
/// and from its top to its bottom, relative to `(ox, oy)` (the root's border
/// box origin, its own scroll offset taken off). A line with no advance (an
/// empty one) is left out.
fn text_lines(node: &rinch_dom::Node, ox: f64, oy: f64) -> Vec<Rect> {
    let Some(inline) = node.text_layout.as_ref() else {
        return Vec::new();
    };
    if inline.text_content.trim().is_empty() {
        return Vec::new();
    }
    let (cx, cy) = rinch_dom::paint::ifc_root_content_origin(node);
    let (x, y) = (ox + cx as f64, oy + cy as f64);
    inline
        .layout
        .lines()
        .filter_map(|line| {
            let m = line.metrics();
            (m.advance > 0.0).then(|| {
                Rect::new(
                    x + m.offset as f64,
                    y + m.block_min_coord as f64,
                    x + (m.offset + m.advance) as f64,
                    y + m.block_max_coord as f64,
                )
            })
        })
        .collect()
}

/// `r` cut into rectangles that follow its rounded corners: a middle band at
/// full width, and above and below it strips each inset at the curve's depth
/// at the strip's middle row. Square corners give `r` alone.
pub(crate) fn rounded_strips(r: Rect, radii: &BorderRadii) -> Vec<Rect> {
    let (tl, tr, br, bl) = (
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    );
    let top = tl.y.max(tr.y);
    let bottom = bl.y.max(br.y);
    if top <= 0.0 && bottom <= 0.0 {
        return vec![r];
    }
    let h = r.height();
    // Radii are already scaled so that opposite corners fit the side.
    let top = top.min(h);
    let bottom = bottom.min(h - top);
    let mut out = Vec::new();
    // How far a corner of radii (rx, ry) is inset `d` rows from its edge.
    let inset = |rx: f64, ry: f64, d: f64| -> f64 {
        if rx <= 0.0 || ry <= 0.0 || d >= ry {
            return 0.0;
        }
        let t = (ry - d) / ry;
        rx - rx * (1.0 - t * t).max(0.0).sqrt()
    };
    let strips = |band: f64| ((band / STRIP_PX).ceil() as usize).clamp(1, MAX_STRIPS);
    if top > 0.0 {
        let n = strips(top);
        for i in 0..n {
            let a = top * i as f64 / n as f64;
            let b = top * (i + 1) as f64 / n as f64;
            let d = (a + b) / 2.0;
            let x0 = r.x0 + inset(tl.x, tl.y, d);
            let x1 = r.x1 - inset(tr.x, tr.y, d);
            if x1 > x0 {
                out.push(Rect::new(x0, r.y0 + a, x1, r.y0 + b));
            }
        }
    }
    let mid = Rect::new(r.x0, r.y0 + top, r.x1, r.y1 - bottom);
    if mid.height() > 0.0 {
        out.push(mid);
    }
    if bottom > 0.0 {
        let n = strips(bottom);
        for i in 0..n {
            let a = bottom * i as f64 / n as f64;
            let b = bottom * (i + 1) as f64 / n as f64;
            let d = (a + b) / 2.0;
            let x0 = r.x0 + inset(bl.x, bl.y, d);
            let x1 = r.x1 - inset(br.x, br.y, d);
            if x1 > x0 {
                out.push(Rect::new(x0, r.y1 - b, x1, r.y1 - a));
            }
        }
    }
    out
}

/// Drop every rect inside another (a painted child inside its painted card),
/// largest first. The union is unchanged; the region sent is smaller.
fn dedupe(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.sort_by(|a, b| b.area().total_cmp(&a.area()));
    let mut kept: Vec<Rect> = Vec::with_capacity(rects.len());
    for r in rects {
        if !kept
            .iter()
            .any(|k| k.x0 <= r.x0 && k.y0 <= r.y0 && k.x1 >= r.x1 && k.y1 >= r.y1)
        {
            kept.push(r);
        }
    }
    kept
}

/// What a window keeps of its input region between frames (#2741): the
/// mode, what the last computation was keyed on, the rects it found, and
/// what was last handed to the windowing system.
#[derive(Debug, Default)]
pub(crate) struct InputRegionState {
    /// The hit cache's generation, the tree's region generation and the
    /// viewport the rects were computed at; `None` until the first
    /// computation (or after a mode change).
    key: Option<(u64, u64, (u32, u32))>,
    /// The region in logical px, as last computed.
    rects: Vec<Rect>,
}

impl InputRegionState {
    /// The region for `mode`, recomputed only when the tree's hit-cache
    /// generation, its region generation or the viewport moved since the
    /// last call. `None` means
    /// the whole window ([`InputRegion::All`]); an empty slice, nothing.
    pub(crate) fn region(
        &mut self,
        tree: &rinch_dom::NodeTree,
        mode: InputRegion,
        viewport: (u32, u32),
    ) -> Option<&[Rect]> {
        match mode {
            InputRegion::All => {
                self.key = None;
                None
            }
            InputRegion::None => {
                self.key = None;
                self.rects.clear();
                Some(&self.rects)
            }
            InputRegion::Painted | InputRegion::Marked => {
                // The region generation: a transform or colour-alpha step of
                // a transition or animation, which the hit cache's does not
                // see (`NodeTree::region_generation`).
                let key = (
                    tree.hit_cache.generation(),
                    tree.region_generation,
                    viewport,
                );
                if self.key != Some(key) {
                    self.rects = compute(tree, mode);
                    // The walk can fill the hit cache, which leaves its
                    // generation alone: the key read before it still holds.
                    self.key = Some(key);
                }
                Some(&self.rects)
            }
        }
    }

    /// Forget the last computation, so the next [`Self::region`] computes
    /// again (a mode change: `Painted` and `Marked` share the key).
    pub(crate) fn invalidate(&mut self) {
        self.key = None;
    }

    /// The rects of the last computation.
    pub(crate) fn rects(&self) -> &[Rect] {
        &self.rects
    }

    /// Whether a press at `(x, y)` is in the region for `mode`, as last
    /// computed: what the windowing system was given with the frame on
    /// screen.
    pub(crate) fn takes_press_at(&self, mode: InputRegion, x: f32, y: f32) -> bool {
        match mode {
            InputRegion::All => true,
            InputRegion::None => false,
            InputRegion::Painted | InputRegion::Marked => contains(&self.rects, x, y),
        }
    }
}
