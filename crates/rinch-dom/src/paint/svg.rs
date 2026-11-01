//! SVG element painting.

use peniko::Fill;
use peniko::color::{AlphaColor, Srgb};
use peniko::kurbo::{
    Affine, BezPath, Cap, Circle, Ellipse, Join, Line, Rect, RoundedRect, Shape, Stroke,
};

use std::rc::Rc;

use super::painter::{BlendMode, PaintShape, Painter};
use crate::layout::parse_color_with_current;
use crate::node::{Node, NodeKind, NodeTree};

// =============================================================================
// SVG rendering
// =============================================================================

/// Paint an `<svg>` element and its shapes (`path`, `rect` with `rx`/`ry`,
/// `circle`, `ellipse`, `line`, `polyline`, `polygon`), at any depth inside
/// `<g>` groups, each under its own and its groups' `transform` (#1538,
/// #1539), with SVG 2's stroke properties (dashes, miter limit, opacities,
/// `paint-order`, `vector-effect: non-scaling-stroke`) and `opacity` on
/// shapes and groups.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_svg(
    tree: &NodeTree,
    node: &Node,
    painter: &mut dyn Painter,
    scale: f64,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    css_transform: Affine,
) {
    // Parse viewBox (default "0 0 24 24" for most icon SVGs)
    let viewbox = node
        .attributes
        .get("viewBox")
        .and_then(|v| parse_viewbox(v))
        .unwrap_or((0.0, 0.0, 24.0, 24.0));

    let (_, _, vb_w, vb_h) = viewbox;
    if vb_w <= 0.0 || vb_h <= 0.0 || w <= 0.0 || h <= 0.0 {
        return;
    }

    // Compute transform: CSS transform composed with viewBox-to-layout scaling.
    // Honors `preserveAspectRatio="none"` (stretch x and y independently);
    // any other value (including the SVG default `xMidYMid meet`) collapses
    // to uniform scaling centered in the container.
    let preserve_aspect = node
        .attributes
        .get("preserveAspectRatio")
        .map(|v| v.as_str());
    let (vb_sx, vb_sy, vb_tx, vb_ty) =
        viewbox_to_box_transform(viewbox, (x, y, w, h), preserve_aspect);
    let transform = css_transform * Affine::new([vb_sx, 0.0, 0.0, vb_sy, vb_tx, vb_ty]);

    // A percentage stroke length (`stroke-width`, `stroke-dasharray`,
    // `stroke-dashoffset`) is of the viewport's normalized diagonal,
    // `sqrt((w² + h²) / 2)`, in user units: the viewBox's size.
    let percent_base = ((vb_w * vb_w + vb_h * vb_h) / 2.0).sqrt() / 100.0;

    // What the `<svg>` hands its content: its own values over SVG's initial
    // ones. The `<svg>`'s own `transform` attribute is not read.
    let initial = SvgInherited {
        transform,
        fill: InheritedPaint::Fixed(Some(SVG_INITIAL_FILL)),
        stroke: InheritedPaint::Fixed(None),
        fill_opacity: 1.0,
        stroke_opacity: 1.0,
        stroke_width: 1.0,
        linecap: Cap::Butt,
        linejoin: Join::Miter,
        miter_limit: 4.0,
        dasharray: None,
        dashoffset: 0.0,
        stroke_first: false,
    };
    let root = initial.derive(node, resolve_current_color(tree, node), percent_base);

    // The `<svg>`'s own CSS `opacity` is its box's (`paint_node`'s layer);
    // an `opacity` *attribute* on it is the SVG presentation attribute, which
    // only this painter reads.
    let svg_box = PaintShape::Rect(Rect::new(x, y, x + w, y + h));
    let root_opacity = if node.computed_style.opacity < 1.0 || declares_in_style(node, "opacity") {
        1.0
    } else {
        node.attributes
            .get("opacity")
            .and_then(|v| parse_alpha(v.trim()))
            .unwrap_or(1.0)
    };
    if root_opacity <= 0.0 {
        return;
    }
    let root_layer = root_opacity < 1.0;
    if root_layer {
        painter.push_layer(BlendMode::Normal, root_opacity, css_transform, &svg_box);
    }

    // A depth-first walk in document order, so later shapes paint over
    // earlier ones. Iterative: a group's nesting is the document's, and paint
    // must not run out of stack on it. Each frame remembers whether it pushed
    // an opacity layer, popped with the frame.
    let mut stack: Vec<(&Node, SvgInherited, usize, bool)> = vec![(node, root, 0, root_layer)];
    while let Some((parent, inherited, next, _)) = stack.last_mut() {
        let Some(&child_id) = parent.children.get(*next) else {
            if let Some((_, _, _, true)) = stack.pop() {
                painter.pop_layer();
            }
            continue;
        };
        *next += 1;
        let inherited = inherited.clone();
        let Some(child) = tree.get_live(child_id) else {
            continue;
        };
        let NodeKind::Element(ref el) = child.kind else {
            continue;
        };
        let tag = el.tag.as_str();
        // `display: none` (the property or the presentation attribute) on a
        // group or a shape renders it and its content not at all.
        if child.computed_style.display == crate::computed_style::DisplayValue::None
            || child
                .attributes
                .get("display")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("none"))
        {
            continue;
        }
        // `opacity` does not inherit: a group's or a shape's applies to what
        // it paints as a whole (a stroke over its own fill is not seen
        // through), so it is a layer, not an alpha on each paint.
        let opacity = element_opacity(child);
        if opacity <= 0.0 {
            continue;
        }
        if tag == "g" {
            // A hidden group still paints a child that sets `visibility`
            // back to `visible`, so only shapes ask.
            let derived = inherited.derive(child, resolve_current_color(tree, child), percent_base);
            // The group's content is not measured, so its layer is the
            // `<svg>` box, which a browser clips SVG content to anyway.
            let layer = opacity < 1.0;
            if layer {
                painter.push_layer(BlendMode::Normal, opacity, css_transform, &svg_box);
            }
            stack.push((child, derived, 0, layer));
            continue;
        }
        if !matches!(
            tag,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        ) {
            continue; // Unsupported SVG elements (text, defs, use, …)
        }
        // `visibility` inherits into an SVG's shapes and each shape can set it
        // back to `visible`, so the question is the shape's own (#829).
        if super::text::is_hidden(child) {
            continue;
        }
        let Some((shape, fillable)) = shape_geometry(child, tag) else {
            continue;
        };
        // `currentcolor` is the shape's own cascaded `color` (a shape that
        // declares none inherits its parent's).
        let current_color = resolve_current_color(tree, child);
        let own = inherited.derive(child, current_color, percent_base);
        // `skewX(90)` (see `tan_degrees`): nothing a browser draws either.
        if !own.transform.is_finite() {
            continue;
        }
        let fill = if fillable {
            own.fill
                .for_child(current_color)
                .map(|c| c.multiply_alpha(own.fill_opacity))
        } else {
            None
        };
        // A zero width strokes nothing (tiny-skia would draw a hairline).
        let stroke_color = own
            .stroke
            .for_child(current_color)
            .filter(|_| own.stroke_width > 0.0)
            .map(|c| c.multiply_alpha(own.stroke_opacity));
        let stroke = stroke_color.map(|color| {
            // `vector-effect: non-scaling-stroke` strokes the shape's
            // outline in device space with lengths in CSS px (times the
            // device scale); otherwise the stroke is in user space and the
            // transform scales its width and dashes, as in a browser.
            if non_scaling_stroke(child) {
                let mut path = shape_path(&shape);
                path.apply_affine(own.transform);
                (
                    own.kurbo_stroke(scale),
                    Affine::IDENTITY,
                    PaintShape::BezPath(path),
                    color,
                )
            } else {
                (own.kurbo_stroke(1.0), own.transform, shape.clone(), color)
            }
        });
        if fill.is_none() && stroke.is_none() {
            continue;
        }
        let layer = opacity < 1.0;
        if layer {
            // The shape's own ink: its outline grown by the stroke's reach.
            let mut ink = own.transform.transform_rect_bbox(shape.bounding_box());
            if let Some((s, t, path, _)) = &stroke {
                let r = stroke_outset(s);
                ink = ink.union(t.transform_rect_bbox(path.bounding_box().inflate(r, r)));
            }
            painter.push_layer(
                BlendMode::Normal,
                opacity,
                Affine::IDENTITY,
                &PaintShape::Rect(ink.inflate(1.0, 1.0)),
            );
        }
        let paint_fill = |painter: &mut dyn Painter| {
            if let Some(fc) = fill {
                painter.fill_color(Fill::NonZero, own.transform, fc, &shape);
            }
        };
        let paint_stroke = |painter: &mut dyn Painter| {
            if let Some((s, t, path, sc)) = &stroke {
                painter.stroke_color(s, *t, *sc, path);
            }
        };
        if own.stroke_first {
            paint_stroke(painter);
            paint_fill(painter);
        } else {
            paint_fill(painter);
            paint_stroke(painter);
        }
        if layer {
            painter.pop_layer();
        }
    }
}

/// How far `stroke` can reach past its path, in the path's space: half its
/// width, times the miter limit where a miter join can spike, times `√2`
/// where a square cap's corner sticks out diagonally.
pub(super) fn stroke_outset(stroke: &Stroke) -> f64 {
    let mut factor: f64 = 1.0;
    if stroke.join == Join::Miter {
        factor = factor.max(stroke.miter_limit);
    }
    if stroke.start_cap == Cap::Square || stroke.end_cap == Cap::Square {
        factor = factor.max(std::f64::consts::SQRT_2);
    }
    stroke.width * 0.5 * factor
}

/// `shape` as a path, for transforming it into device space.
fn shape_path(shape: &PaintShape) -> BezPath {
    const TOLERANCE: f64 = 0.01;
    match shape {
        PaintShape::BezPath(p) => p.clone(),
        PaintShape::Rect(r) => r.to_path(TOLERANCE),
        PaintShape::RoundedRect(r) => r.to_path(TOLERANCE),
        PaintShape::Circle(c) => c.to_path(TOLERANCE),
        PaintShape::Line(l) => l.to_path(TOLERANCE),
    }
}

/// What an SVG element hands its content: the paint properties that inherit
/// and the transform from user space to the window. Lengths are user units,
/// percentages already resolved.
#[derive(Clone)]
struct SvgInherited {
    transform: Affine,
    fill: InheritedPaint,
    stroke: InheritedPaint,
    fill_opacity: f32,
    stroke_opacity: f32,
    stroke_width: f64,
    linecap: Cap,
    linejoin: Join,
    miter_limit: f64,
    /// `None` draws solid. Otherwise even in length (an odd list is
    /// repeated), no value negative and their sum positive.
    dasharray: Option<Rc<[f64]>>,
    dashoffset: f64,
    /// `paint-order` puts the stroke before the fill.
    stroke_first: bool,
}

impl SvgInherited {
    /// `el`'s own values: each one it declares (usably) replaces the
    /// inherited one, and its `transform` composes after its parent's.
    /// `current_color` is `el`'s own `color`; `percent_base` is one percent
    /// of the viewport's normalized diagonal.
    fn derive(&self, el: &Node, current_color: AlphaColor<Srgb>, percent_base: f64) -> Self {
        let transform = match el.attributes.get("transform") {
            // The `<svg>` itself reads no `transform` (the caller's is the
            // viewBox one), and an attribute that does not parse is ignored.
            Some(t) if el.tag() != Some("svg") => match parse_svg_transform(t) {
                Some(own) => self.transform * own,
                None => self.transform,
            },
            _ => self.transform,
        };
        let font_size = el.computed_style.font_size as f64;
        let length = |v: &str| parse_svg_length(v, percent_base, font_size);
        let dasharray = match declared(el, "stroke-dasharray", |v| {
            parse_dasharray(v, percent_base, font_size)
        }) {
            Some(d) => d,
            None => self.dasharray.clone(),
        };
        Self {
            transform,
            fill: InheritedPaint::derive(
                declared_paint(el, "fill").as_deref(),
                current_color,
                self.fill,
            ),
            stroke: InheritedPaint::derive(
                declared_paint(el, "stroke").as_deref(),
                current_color,
                self.stroke,
            ),
            fill_opacity: declared(el, "fill-opacity", parse_alpha).unwrap_or(self.fill_opacity),
            stroke_opacity: declared(el, "stroke-opacity", parse_alpha)
                .unwrap_or(self.stroke_opacity),
            // A negative width is an error, so the declaration is ignored.
            stroke_width: declared(el, "stroke-width", |v| length(v).filter(|w| *w >= 0.0))
                .unwrap_or(self.stroke_width),
            linecap: declared(el, "stroke-linecap", parse_linecap).unwrap_or(self.linecap),
            linejoin: declared(el, "stroke-linejoin", parse_linejoin).unwrap_or(self.linejoin),
            // A limit below 1 is an error.
            miter_limit: declared(el, "stroke-miterlimit", |v| {
                parse_svg_number_whole(v).filter(|m| *m >= 1.0)
            })
            .unwrap_or(self.miter_limit),
            dasharray,
            dashoffset: declared(el, "stroke-dashoffset", length).unwrap_or(self.dashoffset),
            stroke_first: declared(el, "paint-order", parse_paint_order)
                .unwrap_or(self.stroke_first),
        }
    }

    /// The `kurbo::Stroke` to draw with, its lengths times `unit` (`1` in
    /// user space, the device scale for a non-scaling stroke).
    fn kurbo_stroke(&self, unit: f64) -> Stroke {
        let stroke = Stroke::new(self.stroke_width * unit)
            .with_caps(self.linecap)
            .with_join(self.linejoin)
            .with_miter_limit(self.miter_limit);
        match &self.dasharray {
            Some(d) => {
                // The offset brought into the first period, so a negative
                // one means the same to both painters.
                let period: f64 = d.iter().sum();
                let offset = self.dashoffset.rem_euclid(period);
                stroke.with_dashes(offset * unit, d.iter().map(|v| v * unit))
            }
            None => stroke,
        }
    }
}

/// A property `el` declares, parsed by `parse`: its inline `style`
/// declaration, which beats the presentation attribute, else the attribute;
/// every `var()` substituted from the element's computed custom properties
/// (as `declared_paint`). `None` — take the parent's value — for no
/// declaration, for `inherit`/`unset`/`initial` (the last approximated as
/// `inherit`, as for paints), for a `var()` that resolves to nothing usable
/// (invalid at computed-value time), and for an attribute that does not
/// parse (ignored, as SVG ignores a bad presentation attribute). A literal
/// `style` value that does not parse is dropped as a browser drops it while
/// parsing, so the attribute applies.
///
/// rinch's Stylo build has none of these properties (they are Gecko-only
/// there), so a stylesheet rule is not read.
fn declared<T>(node: &Node, name: &str, parse: impl Fn(&str) -> Option<T>) -> Option<T> {
    if let Some(style) = node.attributes.get("style")
        && style.contains(name)
        && let Some(value) = rinch_core::dom::split_declarations(style)
            .into_iter()
            .find_map(|(k, v)| (k == name).then_some(v))
    {
        let value = strip_important(&value).trim();
        if has_var(value) {
            return substitute_vars(node, value, 0).and_then(|v| parse(v.trim()));
        }
        if is_css_wide_keyword(value) {
            return None;
        }
        if let Some(v) = parse(value) {
            return Some(v);
        }
    }
    let attr = node.attributes.get(name)?;
    if has_var(attr) {
        substitute_vars(node, attr, 0).and_then(|v| parse(v.trim()))
    } else {
        parse(attr.trim())
    }
}

/// Whether `node`'s inline `style` declares `name`.
fn declares_in_style(node: &Node, name: &str) -> bool {
    node.attributes.get("style").is_some_and(|style| {
        style.contains(name)
            && rinch_core::dom::split_declarations(style)
                .iter()
                .any(|(k, _)| k == name)
    })
}

fn is_css_wide_keyword(v: &str) -> bool {
    ["inherit", "unset", "initial", "revert", "revert-layer"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k))
}

/// A shape's or group's `opacity`. Stylo resolves the CSS property (a
/// stylesheet rule or the `style` attribute) into `computed_style.opacity`;
/// the presentation attribute applies under neither. A CSS declaration of
/// exactly `1` in a stylesheet cannot be told from none, so the attribute
/// wins there.
fn element_opacity(node: &Node) -> f32 {
    let css = node.computed_style.opacity;
    if css < 1.0 || declares_in_style(node, "opacity") {
        return css;
    }
    declared(node, "opacity", parse_alpha).unwrap_or(1.0)
}

/// Whether a shape declares `vector-effect: non-scaling-stroke` (the other
/// SVG 2 values, which Chrome does not implement either, are `none`). The
/// property does not inherit.
fn non_scaling_stroke(node: &Node) -> bool {
    declared(node, "vector-effect", |v| {
        if v.eq_ignore_ascii_case("non-scaling-stroke") {
            Some(true)
        } else if v.eq_ignore_ascii_case("none") {
            Some(false)
        } else {
            None
        }
    })
    .unwrap_or(false)
}

/// An `<alpha-value>` (`fill-opacity`, `stroke-opacity`, `opacity`): a
/// number or a percentage, clamped to `0..=1`.
fn parse_alpha(v: &str) -> Option<f32> {
    let (n, rest) = parse_svg_number(v)?;
    let n = match rest {
        "" => n,
        "%" => n / 100.0,
        _ => return None,
    };
    n.is_finite().then(|| n.clamp(0.0, 1.0) as f32)
}

/// A whole value that is one SVG number and nothing else.
fn parse_svg_number_whole(v: &str) -> Option<f64> {
    match parse_svg_number(v)? {
        (n, "") if n.is_finite() => Some(n),
        _ => None,
    }
}

/// A `<length-percentage>` in user units: a number (user units), a number
/// with an absolute unit (`px`, `in`, `cm`, `mm`, `q`, `pt`, `pc`) or `em`
/// (`font_size`, the element's), or a percentage of the viewport's
/// normalized diagonal (`percent_base` is one percent of it).
fn parse_svg_length(v: &str, percent_base: f64, font_size: f64) -> Option<f64> {
    let (n, unit) = parse_svg_number(v)?;
    let per = match unit.to_ascii_lowercase().as_str() {
        "" | "px" => 1.0,
        "%" => percent_base,
        "em" => font_size,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "q" => 96.0 / 101.6,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        _ => return None,
    };
    let len = n * per;
    len.is_finite().then_some(len)
}

/// `stroke-dasharray`: `none`, or a list of non-negative lengths separated by
/// commas and/or whitespace. `Some(None)` draws solid — `none`, and a list
/// whose values are all zero; `None` (the declaration is ignored) for a
/// negative value or one that is no length, as Chrome 152 does. An odd list
/// is repeated to make it even, as SVG says.
fn parse_dasharray(v: &str, percent_base: f64, font_size: f64) -> Option<Option<Rc<[f64]>>> {
    if v.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let mut dashes = Vec::new();
    for part in v
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
    {
        let d = parse_svg_length(part, percent_base, font_size)?;
        if d < 0.0 {
            return None;
        }
        dashes.push(d);
    }
    if dashes.is_empty() {
        return None;
    }
    let sum: f64 = dashes.iter().sum();
    if !(sum > 0.0 && sum.is_finite()) {
        return Some(None);
    }
    if dashes.len() % 2 == 1 {
        dashes.extend_from_within(..);
    }
    Some(Some(dashes.into()))
}

/// `paint-order`: whether the stroke paints before the fill. `normal` is
/// fill, stroke, markers; a list names some of the three once each, the
/// unnamed ones following in that order. `None` when it is no such list.
fn parse_paint_order(v: &str) -> Option<bool> {
    if v.eq_ignore_ascii_case("normal") {
        return Some(false);
    }
    let mut seen: Vec<&str> = Vec::new();
    for t in v.split_ascii_whitespace() {
        let t = ["fill", "stroke", "markers"]
            .into_iter()
            .find(|k| t.eq_ignore_ascii_case(k))?;
        if seen.contains(&t) {
            return None;
        }
        seen.push(t);
    }
    if seen.is_empty() {
        return None;
    }
    let at = |k| seen.iter().position(|t| *t == k);
    Some(match (at("fill"), at("stroke")) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(f), Some(s)) => s < f,
    })
}

/// A numeric geometry attribute (`x`, `cx`, `r`, …); `None` when absent or
/// not a number.
fn num_attr(node: &Node, name: &str) -> Option<f64> {
    node.attributes
        .get(name)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// A radius that may be `auto` (`rx`/`ry` of `<rect>` and `<ellipse>`):
/// `None` for `auto` — absent, the keyword, or negative (an error, which is
/// ignored, leaving the initial `auto`) — else its value. Any other value
/// that is not a number is `0`, as in Chrome 156 (which also reads the
/// attribute *keyword* `auto` as `0`, against SVG 2; its `style="rx: auto"`
/// is `auto`).
fn radius_attr(node: &Node, name: &str) -> Option<f64> {
    let v = node.attributes.get(name)?.trim();
    if v.eq_ignore_ascii_case("auto") {
        return None;
    }
    match v.parse::<f64>() {
        Ok(r) if r.is_finite() => (r >= 0.0).then_some(r),
        _ => Some(0.0),
    }
}

/// SVG 2's `auto` rule for a pair of radii: an `auto` one takes the other's
/// value; both `auto` is `0`.
fn resolve_radii(rx: Option<f64>, ry: Option<f64>) -> (f64, f64) {
    match (rx, ry) {
        (Some(rx), Some(ry)) => (rx, ry),
        (Some(r), None) | (None, Some(r)) => (r, r),
        (None, None) => (0.0, 0.0),
    }
}

/// A shape element's geometry in its user space, and whether it has an
/// interior to fill (a `<line>` has none). `None` when the element renders
/// nothing (a missing `d`, a zero size or radius).
fn shape_geometry(el: &Node, tag: &str) -> Option<(PaintShape, bool)> {
    let n = |name| num_attr(el, name).unwrap_or(0.0);
    match tag {
        "path" => {
            let path = BezPath::from_svg(el.attributes.get("d")?).ok()?;
            Some((PaintShape::BezPath(path), true))
        }
        "rect" => {
            let (x, y, w, h) = (n("x"), n("y"), n("width"), n("height"));
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            let (rx, ry) = resolve_radii(radius_attr(el, "rx"), radius_attr(el, "ry"));
            // Each radius is clamped to half its side, separately.
            let (rx, ry) = (rx.min(w / 2.0), ry.min(h / 2.0));
            let shape = if rx <= 0.0 || ry <= 0.0 {
                PaintShape::Rect(Rect::new(x, y, x + w, y + h))
            } else if rx == ry {
                PaintShape::RoundedRect(RoundedRect::new(x, y, x + w, y + h, rx))
            } else {
                PaintShape::BezPath(elliptical_rounded_rect(x, y, w, h, rx, ry))
            };
            Some((shape, true))
        }
        "circle" => {
            let r = n("r");
            if r <= 0.0 {
                return None;
            }
            Some((Circle::new((n("cx"), n("cy")), r).into(), true))
        }
        "ellipse" => {
            let (rx, ry) = resolve_radii(radius_attr(el, "rx"), radius_attr(el, "ry"));
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            let ellipse = Ellipse::new((n("cx"), n("cy")), (rx, ry), 0.0);
            Some((PaintShape::BezPath(ellipse.to_path(0.01)), true))
        }
        "line" => {
            let line = Line::new((n("x1"), n("y1")), (n("x2"), n("y2")));
            Some((line.into(), false))
        }
        "polyline" | "polygon" => {
            let path = parse_polyline_points(el.attributes.get("points")?, tag == "polygon")?;
            Some((PaintShape::BezPath(path), true))
        }
        _ => None,
    }
}

/// A rect whose corners are quarter ellipses of radii `rx` x `ry` (already
/// clamped), as SVG's `<rect>` path: clockwise from `(x + rx, y)`.
fn elliptical_rounded_rect(x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) -> BezPath {
    // The cubic control-point distance that approximates a quarter circle.
    const K: f64 = 0.552_284_749_830_793_4;
    let (kx, ky) = (rx * K, ry * K);
    let (x1, y1) = (x + w, y + h);
    let mut p = BezPath::new();
    p.move_to((x + rx, y));
    p.line_to((x1 - rx, y));
    p.curve_to((x1 - rx + kx, y), (x1, y + ry - ky), (x1, y + ry));
    p.line_to((x1, y1 - ry));
    p.curve_to((x1, y1 - ry + ky), (x1 - rx + kx, y1), (x1 - rx, y1));
    p.line_to((x + rx, y1));
    p.curve_to((x + rx - kx, y1), (x, y1 - ry + ky), (x, y1 - ry));
    p.line_to((x, y + ry));
    p.curve_to((x, y + ry - ky), (x + rx - kx, y), (x + rx, y));
    p.close_path();
    p
}

/// Parse an SVG `transform` attribute: a list of `matrix(a b c d e f)`,
/// `translate(x [y])`, `scale(x [y])`, `rotate(a [cx cy])`, `skewX(a)` and
/// `skewY(a)` (angles in degrees), separated by whitespace and/or a comma,
/// applied right to left. Arguments are SVG numbers separated by whitespace
/// and/or one comma, or by nothing where the next one starts with a sign or a
/// second `.` (`translate(10-5)`). `None` when any of it does not parse,
/// which makes the whole attribute ignored, as in Chrome.
pub(super) fn parse_svg_transform(s: &str) -> Option<Affine> {
    let mut out = Affine::IDENTITY;
    let mut rest = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    while !rest.is_empty() {
        let name_len = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        rest = rest[name_len..]
            .trim_start_matches(|c: char| c.is_ascii_whitespace())
            .strip_prefix('(')?;
        let mut args = [0.0f64; 6];
        let mut count = 0;
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
            if let Some(r) = rest.strip_prefix(')') {
                rest = r;
                break;
            }
            if count > 0
                && let Some(r) = rest.strip_prefix(',')
            {
                rest = r.trim_start_matches(|c: char| c.is_ascii_whitespace());
            }
            if count == args.len() {
                return None;
            }
            let (v, r) = parse_svg_number(rest)?;
            args[count] = v;
            count += 1;
            rest = r;
        }
        let a = &args[..count];
        let t = match (name, a) {
            ("matrix", &[a, b, c, d, e, f]) => Affine::new([a, b, c, d, e, f]),
            ("translate", &[tx]) => Affine::translate((tx, 0.0)),
            ("translate", &[tx, ty]) => Affine::translate((tx, ty)),
            ("scale", &[s]) => Affine::scale(s),
            ("scale", &[sx, sy]) => Affine::scale_non_uniform(sx, sy),
            ("rotate", &[deg]) => Affine::rotate(deg.to_radians()),
            ("rotate", &[deg, cx, cy]) => Affine::rotate_about(deg.to_radians(), (cx, cy)),
            ("skewX", &[deg]) => Affine::new([1.0, 0.0, tan_degrees(deg), 1.0, 0.0, 0.0]),
            ("skewY", &[deg]) => Affine::new([1.0, tan_degrees(deg), 0.0, 1.0, 0.0, 0.0]),
            _ => return None,
        };
        out *= t;
        // Between two functions: optional whitespace and one optional comma.
        let trimmed = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        rest = match trimmed.strip_prefix(',') {
            Some(r) => {
                let r = r.trim_start_matches(|c: char| c.is_ascii_whitespace());
                // A trailing comma is not a list.
                if r.is_empty() {
                    return None;
                }
                r
            }
            None => trimmed,
        };
    }
    Some(out)
}

/// `tan` of an angle in degrees, infinite at an odd multiple of 90° (where
/// `f64`'s radians land just off the pole and give about `1.6e16`): a skew of
/// 90° makes a transform no browser draws through (Chrome 156 draws nothing
/// for `skewX(90)`), and an infinite one is skipped by the walk.
fn tan_degrees(deg: f64) -> f64 {
    if (deg % 180.0).abs() == 90.0 {
        f64::INFINITY
    } else {
        deg.to_radians().tan()
    }
}

/// One SVG number at the start of `s` (`-1`, `.5`, `1e-3`, `+2.`), and the
/// rest of `s` after it. A second `.` or a sign ends it (`1.5.5` is `1.5`,
/// `.5`); an `e` not followed by digits is not part of it.
fn parse_svg_number(s: &str) -> Option<(f64, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - int_start;
    if b.get(i) == Some(&b'.') {
        let frac_start = i + 1;
        let mut j = frac_start;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        digits += j - frac_start;
        if digits > 0 {
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exp_start = j;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    let v: f64 = s[..i].parse().ok()?;
    Some((v, &s[i..]))
}

/// Parse SVG viewBox attribute: "minX minY width height"
pub(super) fn parse_viewbox(s: &str) -> Option<(f64, f64, f64, f64)> {
    let parts: Vec<f64> = s
        .split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() == 4 {
        Some((parts[0], parts[1], parts[2], parts[3]))
    } else {
        None
    }
}

/// Compute the (sx, sy, tx, ty) affine coefficients that map a viewBox of
/// (vb_x, vb_y, vb_w, vb_h) into a layout box of (x, y, w, h), honoring the
/// `preserveAspectRatio` attribute.
///
/// The full SVG spec enumerates nine alignment modes plus `meet` / `slice`
/// fitting. We support the two cases that matter in practice:
///
/// - `"none"` — stretch x and y independently to fill the layout box exactly.
/// - anything else (including the SVG default `xMidYMid meet`) — uniform
///   scale by `min(sx, sy)`, content centered in the layout box.
pub(super) fn viewbox_to_box_transform(
    viewbox: (f64, f64, f64, f64),
    layout: (f64, f64, f64, f64),
    preserve_aspect: Option<&str>,
) -> (f64, f64, f64, f64) {
    let (vb_x, vb_y, vb_w, vb_h) = viewbox;
    let (x, y, w, h) = layout;
    let sx = w / vb_w;
    let sy = h / vb_h;
    if preserve_aspect == Some("none") {
        (sx, sy, x - vb_x * sx, y - vb_y * sy)
    } else {
        let s = sx.min(sy);
        (
            s,
            s,
            x + (w - vb_w * s) * 0.5 - vb_x * s,
            y + (h - vb_h * s) * 0.5 - vb_y * s,
        )
    }
}

/// Resolve `currentColor` by walking up the DOM tree to find a CSS `color` property.
pub(super) fn resolve_current_color(tree: &NodeTree, node: &Node) -> AlphaColor<Srgb> {
    let mut current = Some(node.id);
    while let Some(id) = current {
        if let Some(n) = tree.get(id) {
            // Check computed_style.color
            if let Some(c) = n.computed_style.color {
                return c;
            }
            current = n.parent;
        } else {
            break;
        }
    }
    // Default: black
    AlphaColor::<Srgb>::from_rgba8(0, 0, 0, 255)
}

/// The `fill` or `stroke` (`name`) an element declares, as the text to parse:
/// its inline `style` declaration, which beats the presentation attribute,
/// else the attribute; every `var()` in it substituted from the element's own
/// computed custom properties (#1800). rinch's Stylo build has no `fill` or
/// `stroke` property (both are Gecko-only there), so nothing else resolves
/// either; a rule in a stylesheet is not read at all.
///
/// `None` for no declaration, and for one whose `var()` names a missing
/// property with no fallback: that is invalid at computed-value time, which
/// for an inherited property is `unset`, so the element takes its parent's
/// paint (Chrome 155: `stroke="var(--missing)"` in an `<svg stroke="green">`
/// is green). A literal `style` value that is no paint at all is dropped as
/// a browser drops it while parsing, so the attribute applies.
fn declared_paint<'a>(node: &'a Node, name: &str) -> Option<std::borrow::Cow<'a, str>> {
    use std::borrow::Cow;
    if let Some(style) = node.attributes.get("style")
        && style.contains(name)
        && let Some(value) = rinch_core::dom::split_declarations(style)
            .into_iter()
            .find_map(|(k, v)| (k == name).then_some(v))
    {
        let value = strip_important(&value);
        if has_var(value) {
            return substitute_vars(node, value, 0).map(Cow::Owned);
        }
        if !matches!(
            parse_svg_paint(Some(value), AlphaColor::BLACK),
            SvgPaint::Unset
        ) || value.eq_ignore_ascii_case("inherit")
            || value.eq_ignore_ascii_case("unset")
        {
            return Some(Cow::Owned(value.to_string()));
        }
    }
    let attr = node.attributes.get(name)?;
    if has_var(attr) {
        substitute_vars(node, attr, 0).map(Cow::Owned)
    } else {
        Some(Cow::Borrowed(attr.as_str()))
    }
}

/// A declaration's value without a trailing `!important`.
fn strip_important(value: &str) -> &str {
    let v = value.trim_end();
    match v.rfind('!') {
        Some(i) if v[i + 1..].trim().eq_ignore_ascii_case("important") => v[..i].trim_end(),
        _ => v,
    }
}

fn has_var(value: &str) -> bool {
    find_var(value).is_some()
}

/// The byte offset of the first `var(` in `value`, any case.
fn find_var(value: &str) -> Option<usize> {
    value
        .as_bytes()
        .windows(4)
        .position(|w| w.eq_ignore_ascii_case(b"var("))
}

/// How deep fallbacks may nest `var()`s: a bound on a crafted value, not a
/// limit anyone meets (a custom property's own value arrives substituted).
const MAX_VAR_DEPTH: u32 = 16;

/// `value` with every `var(--name[, fallback])` replaced by `node`'s computed
/// custom property `--name`, else the fallback (itself substituted). `None`
/// when one has neither, or the text is malformed.
fn substitute_vars(node: &Node, value: &str, depth: u32) -> Option<String> {
    if depth > MAX_VAR_DEPTH {
        return None;
    }
    let mut out = String::new();
    let mut rest = value;
    while let Some(at) = find_var(rest) {
        out.push_str(&rest[..at]);
        let args_start = at + 4;
        // The `)` that closes this `var(`, past any nested parentheses.
        let mut level = 0usize;
        let mut close = None;
        for (i, c) in rest[args_start..].char_indices() {
            match c {
                '(' => level += 1,
                ')' if level == 0 => {
                    close = Some(args_start + i);
                    break;
                }
                ')' => level -= 1,
                _ => {}
            }
        }
        let close = close?;
        let args = &rest[args_start..close];
        let (var_name, fallback) = match args.find(',') {
            Some(i) => (args[..i].trim(), Some(&args[i + 1..])),
            None => (args.trim(), None),
        };
        let prop = var_name.strip_prefix("--")?;
        match custom_property(node, prop) {
            Some(v) => out.push_str(&v),
            None => out.push_str(&substitute_vars(node, fallback?, depth + 1)?),
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// `node`'s computed value of the custom property `--name` (`name` without
/// the dashes), already substituted by Stylo; custom properties inherit, so
/// one declared on `:root` or the `<svg>` reaches every shape.
fn custom_property(node: &Node, name: &str) -> Option<String> {
    let data = node.stylo_element_data.borrow();
    let primary = data.as_ref().and_then(|d| d.styles.get_primary())?;
    let atom = style::Atom::from(name);
    primary
        .custom_properties()
        .get(
            style::properties_and_values::registry::PropertyRegistrationData::unregistered(),
            &atom,
        )
        .map(|v| v.to_variable_value().css.trim().to_string())
}

/// What an SVG `fill`/`stroke` attribute asked for.
///
/// Three states, not `Option`, because "paint nothing" and "I could not use
/// that" are different answers and the old `Option` conflated them (#258): a
/// shape with `fill="notacolour"` went unpainted exactly like `fill="none"`,
/// when SVG says an unusable presentation attribute is *ignored* — the shape
/// keeps whatever it would have had without it.
enum SvgPaint {
    /// `none`, or an unresolvable `url()` with no fallback. Paint nothing.
    None,
    /// A resolved colour.
    Color(AlphaColor<Srgb>),
    /// Absent, or present but unusable. Both mean the same thing to the
    /// caller: take the value that would have applied anyway — the parent's
    /// (`<svg>` or `<g>`) for its content, the property's initial value for
    /// the `<svg>` itself.
    Unset,
}

/// Resolve an SVG `fill`/`stroke` attribute against an element's own `color`.
///
/// Handled, in order: `none`; `currentcolor` (any casing — the spec spells it
/// lowercase, Tabler emits `currentColor`); `inherit`/`unset`, which for a
/// paint (an inherited property) means the parent's value, i.e. `Unset`;
/// `url(<ref>)` with an optional fallback; and anything `parse_color_with_current`
/// accepts, which since #256 includes `color-mix()` and relative colours over
/// `currentcolor`.
///
/// Two documented approximations. `url()` is *always* unresolvable here —
/// `paint_svg` renders no `<defs>`, so there are no paint servers to find — and
/// the reference is split at the first `)`, which a quoted URL containing one
/// would defeat. And `initial`/`revert` land in `Unset` with everything else
/// unusable, so they read as `inherit`; they differ only when the `<svg>`
/// itself declares a paint.
fn parse_svg_paint(attr: Option<&str>, current_color: AlphaColor<Srgb>) -> SvgPaint {
    let Some(value) = attr.map(str::trim) else {
        return SvgPaint::Unset;
    };
    if value.eq_ignore_ascii_case("none") {
        return SvgPaint::None;
    }
    if value.eq_ignore_ascii_case("currentcolor") {
        // Resolved here rather than parsed: every icon in the tree says this,
        // so it is worth skipping the parser for.
        return SvgPaint::Color(current_color);
    }
    if value.eq_ignore_ascii_case("inherit") || value.eq_ignore_ascii_case("unset") {
        return SvgPaint::Unset;
    }
    if value
        .get(..4)
        .is_some_and(|p| p.eq_ignore_ascii_case("url("))
    {
        let rest = &value[4..];
        // The paint server cannot be found, so SVG 2 says use the fallback;
        // with no fallback the element is not rendered (which is what Chrome
        // does too).
        return match rest.split_once(')') {
            Some((_, fallback)) => match fallback.trim() {
                "" => SvgPaint::None,
                f => parse_svg_paint(Some(f), current_color),
            },
            // Unterminated `url(` — not a paint at all.
            None => SvgPaint::Unset,
        };
    }
    match parse_color_with_current(value, current_color) {
        Some(c) => SvgPaint::Color(c),
        None => SvgPaint::Unset,
    }
}

/// SVG's initial `fill`: a shape with no `fill` anywhere paints black.
const SVG_INITIAL_FILL: AlphaColor<Srgb> = AlphaColor::<Srgb>::BLACK;

/// An inherited `fill`/`stroke` — the `<svg>`'s or a `<g>`'s — resolved once
/// so the shapes that inherit it don't re-parse it per paint. `currentcolor`
/// stays symbolic, as CSS inherits the keyword: it resolves against each
/// shape's own `color`.
#[derive(Clone, Copy)]
enum InheritedPaint {
    CurrentColor,
    Fixed(Option<AlphaColor<Srgb>>),
}

impl InheritedPaint {
    /// The paint an element with the declaration `attr` has, and hands on,
    /// when its parent's is `parent`. `current_color` is the element's own
    /// `color`. An absent *or unusable* value takes the parent's, which is
    /// what SVG means by ignoring a bad presentation attribute; `none` does
    /// not, it wins. (For the `<svg>`, `parent` is the property's initial
    /// value: black for `fill`, `none` for `stroke`.)
    fn derive(attr: Option<&str>, current_color: AlphaColor<Srgb>, parent: Self) -> Self {
        // `currentcolor` is checked before parsing so it stays *symbolic*: it
        // must resolve against each shape's own `color`, not this element's.
        // (A `currentcolor` reached any other way — as a `url()` fallback, or
        // inside a `color-mix()` — resolves eagerly against this element's.)
        if attr.is_some_and(|v| v.trim().eq_ignore_ascii_case("currentcolor")) {
            return Self::CurrentColor;
        }
        match parse_svg_paint(attr, current_color) {
            SvgPaint::Unset => parent,
            SvgPaint::None => Self::Fixed(None),
            SvgPaint::Color(c) => Self::Fixed(Some(c)),
        }
    }

    fn for_child(&self, child_current_color: AlphaColor<Srgb>) -> Option<AlphaColor<Srgb>> {
        match self {
            Self::CurrentColor => Some(child_current_color),
            Self::Fixed(color) => *color,
        }
    }
}

/// `stroke-linecap`; `None` for anything else (the declaration is ignored).
pub(super) fn parse_linecap(v: &str) -> Option<Cap> {
    [
        ("butt", Cap::Butt),
        ("round", Cap::Round),
        ("square", Cap::Square),
    ]
    .into_iter()
    .find_map(|(k, c)| v.eq_ignore_ascii_case(k).then_some(c))
}

/// `stroke-linejoin`; `None` for anything else (the declaration is ignored).
/// SVG 2's `miter-clip` and `arcs` are ignored too, as Chrome 152 ignores
/// them: neither has a join in kurbo or tiny-skia.
pub(super) fn parse_linejoin(v: &str) -> Option<Join> {
    [
        ("miter", Join::Miter),
        ("round", Join::Round),
        ("bevel", Join::Bevel),
    ]
    .into_iter()
    .find_map(|(k, j)| v.eq_ignore_ascii_case(k).then_some(j))
}

/// Parse SVG polyline/polygon points attribute into a BezPath.
pub(super) fn parse_polyline_points(points_str: &str, close: bool) -> Option<BezPath> {
    let nums: Vec<f64> = points_str
        .split_whitespace()
        .flat_map(|p| p.split(','))
        .filter_map(|p| p.parse().ok())
        .collect();
    if nums.len() < 4 || !nums.len().is_multiple_of(2) {
        return None;
    }
    let mut path = BezPath::new();
    path.move_to((nums[0], nums[1]));
    for i in (2..nums.len()).step_by(2) {
        path.line_to((nums[i], nums[i + 1]));
    }
    if close {
        path.close_path();
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {a} ≈ {b}");
    }

    fn coeffs(t: &str) -> [f64; 6] {
        parse_svg_transform(t)
            .unwrap_or_else(|| panic!("{t:?} should parse"))
            .as_coeffs()
    }

    fn assert_coeffs(t: &str, want: [f64; 6]) {
        let got = coeffs(t);
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-9, "{t:?}: got {got:?}, want {want:?}");
        }
    }

    #[test]
    fn transform_functions_map_to_svgs_matrices() {
        assert_coeffs("", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_coeffs("matrix(1 2 3 4 5 6)", [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_coeffs("translate(7)", [1.0, 0.0, 0.0, 1.0, 7.0, 0.0]);
        assert_coeffs("translate(7 -3)", [1.0, 0.0, 0.0, 1.0, 7.0, -3.0]);
        assert_coeffs("scale(2)", [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        assert_coeffs("scale(2, 3)", [2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
        let (s, c) = 30f64.to_radians().sin_cos();
        assert_coeffs("rotate(30)", [c, s, -s, c, 0.0, 0.0]);
        // About (10, 20): translate(10,20) rotate(30) translate(-10,-20).
        assert_coeffs(
            "rotate(30 10 20)",
            [
                c,
                s,
                -s,
                c,
                10.0 - c * 10.0 + s * 20.0,
                20.0 - s * 10.0 - c * 20.0,
            ],
        );
        let t = 20f64.to_radians().tan();
        assert_coeffs("skewX(20)", [1.0, 0.0, t, 1.0, 0.0, 0.0]);
        assert_coeffs("skewY(20)", [1.0, t, 0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn a_transform_list_multiplies_left_to_right() {
        // translate(5,0) scale(2): x' = 5 + 2x.
        assert_coeffs("translate(5,0) scale(2)", [2.0, 0.0, 0.0, 2.0, 5.0, 0.0]);
        // scale(2) translate(5,0): x' = 2(x + 5).
        assert_coeffs("scale(2),translate(5,0)", [2.0, 0.0, 0.0, 2.0, 10.0, 0.0]);
        assert_coeffs(" scale(2)translate(5) ", [2.0, 0.0, 0.0, 2.0, 10.0, 0.0]);
    }

    #[test]
    fn transform_numbers_follow_svgs_grammar() {
        assert_coeffs("translate(10-5)", [1.0, 0.0, 0.0, 1.0, 10.0, -5.0]);
        assert_coeffs("translate(.5.25)", [1.0, 0.0, 0.0, 1.0, 0.5, 0.25]);
        assert_coeffs("translate(1e1,+2.)", [1.0, 0.0, 0.0, 1.0, 10.0, 2.0]);
        assert_coeffs("translate(1E-1 , 3)", [1.0, 0.0, 0.0, 1.0, 0.1, 3.0]);
        assert_coeffs("translate ( 1 2 )", [1.0, 0.0, 0.0, 1.0, 1.0, 2.0]);
    }

    #[test]
    fn an_unparsable_transform_is_none() {
        for t in [
            "translate(1,2,3)",
            "rotate(1 2)",
            "matrix(1 2 3 4 5)",
            "translate()",
            "translate(,1)",
            "translate(1,)",
            "translate(1,,2)",
            "translate(1",
            "translate 1",
            "Translate(1)",
            "skewx(1)",
            "translate(1) bogus(2)",
            "translate(1),",
            "translate(1px)",
            "translate(e1)",
            "translate(.)",
            "translate(1 2 3 4 5 6 7)",
        ] {
            assert!(parse_svg_transform(t).is_none(), "{t:?} should not parse");
        }
    }

    /// The pixel fixture cannot tell `inf` from `f64`'s 1.6e16 (both shear the
    /// shape out of sight), so the pole is pinned here.
    #[test]
    fn a_skew_of_ninety_degrees_is_not_finite() {
        for t in ["skewX(90)", "skewY(-90)", "skewX(270)"] {
            assert!(!coeffs(t).iter().all(|c| c.is_finite()), "{t}");
        }
        assert!(coeffs("skewX(89.9)").iter().all(|c| c.is_finite()));
    }

    #[test]
    fn radii_follow_svg2s_auto_rule() {
        assert_eq!(resolve_radii(None, None), (0.0, 0.0));
        assert_eq!(resolve_radii(Some(3.0), None), (3.0, 3.0));
        assert_eq!(resolve_radii(None, Some(4.0)), (4.0, 4.0));
        assert_eq!(resolve_radii(Some(3.0), Some(4.0)), (3.0, 4.0));
    }

    #[test]
    fn default_aspect_uniform_scales_and_centers() {
        // Square viewBox in a wide container — content should fit by height
        // and be centered horizontally.
        let (sx, sy, tx, ty) =
            viewbox_to_box_transform((0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 400.0, 200.0), None);
        approx(sx, 2.0); // min(400/100, 200/100) = 2.0
        approx(sy, 2.0);
        approx(tx, 100.0); // (400 - 200) / 2 = 100
        approx(ty, 0.0);
    }

    #[test]
    fn aspect_none_stretches_independently() {
        // The use case in rawdaw's arrangement: a 24:100 viewBox painting a
        // long horizontal gridline strip. Without "none" support, the lines
        // bunch into the center; with it, they fill the full width.
        let (sx, sy, tx, ty) = viewbox_to_box_transform(
            (0.0, 0.0, 24.0, 100.0),
            (0.0, 0.0, 1600.0, 150.0),
            Some("none"),
        );
        approx(sx, 1600.0 / 24.0);
        approx(sy, 1.5);
        approx(tx, 0.0);
        approx(ty, 0.0);
    }

    #[test]
    fn aspect_none_honors_layout_origin_and_vb_offset() {
        // Layout starts at (10, 20); viewBox starts at (5, 5). Both contribute
        // to the translate.
        let (sx, sy, tx, ty) = viewbox_to_box_transform(
            (5.0, 5.0, 100.0, 100.0),
            (10.0, 20.0, 400.0, 200.0),
            Some("none"),
        );
        approx(sx, 4.0);
        approx(sy, 2.0);
        approx(tx, 10.0 - 5.0 * 4.0); // -10
        approx(ty, 20.0 - 5.0 * 2.0); // 10
    }

    #[test]
    fn unknown_aspect_string_falls_back_to_default() {
        // Anything other than literal "none" — including the SVG default
        // `xMidYMid meet` — uses uniform scale. The current implementation
        // doesn't distinguish among the nine alignment modes; this test
        // pins that behavior so future spec-completion changes have a
        // baseline to update.
        let (sx_a, sy_a, tx_a, ty_a) =
            viewbox_to_box_transform((0.0, 0.0, 100.0, 100.0), (0.0, 0.0, 400.0, 200.0), None);
        let (sx_b, sy_b, tx_b, ty_b) = viewbox_to_box_transform(
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 400.0, 200.0),
            Some("xMidYMid meet"),
        );
        approx(sx_a, sx_b);
        approx(sy_a, sy_b);
        approx(tx_a, tx_b);
        approx(ty_a, ty_b);
    }
}
