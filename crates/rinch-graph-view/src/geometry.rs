//! The viewport transform and edge curves. Pure math, no DOM.

use rinch_graph_core::{Point, Rect, Size};

/// Pan and zoom: a graph point `p` is drawn at `p * zoom + (x, y)` in the
/// canvas's own pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Self {
        Viewport {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// A canvas-pixel point to graph space.
    pub fn to_graph(&self, canvas: Point) -> Point {
        Point::new(
            (canvas.x - self.x) / self.zoom,
            (canvas.y - self.y) / self.zoom,
        )
    }

    /// A graph point to canvas pixels.
    pub fn to_canvas(&self, graph: Point) -> Point {
        Point::new(graph.x * self.zoom + self.x, graph.y * self.zoom + self.y)
    }

    /// Zoom to `zoom` (clamped to `[min, max]`) keeping the graph point under
    /// `anchor` (canvas pixels) where it is.
    pub fn zoomed_around(&self, zoom: f32, anchor: Point, min: f32, max: f32) -> Viewport {
        let zoom = zoom.clamp(min, max);
        let g = self.to_graph(anchor);
        Viewport {
            x: anchor.x - g.x * zoom,
            y: anchor.y - g.y * zoom,
            zoom,
        }
        .pixel_aligned()
    }

    /// The viewport that shows `bounds` centred in a canvas of `canvas` size
    /// with `padding` pixels to spare, at no more than `max_zoom`.
    pub fn fitting(bounds: Rect, canvas: Size, padding: f32, min: f32, max_zoom: f32) -> Viewport {
        let avail_w = (canvas.width - 2.0 * padding).max(1.0);
        let avail_h = (canvas.height - 2.0 * padding).max(1.0);
        let zoom = if bounds.width <= 0.0 || bounds.height <= 0.0 {
            1.0
        } else {
            (avail_w / bounds.width).min(avail_h / bounds.height)
        }
        .clamp(min, max_zoom);
        let c = bounds.center();
        Viewport {
            x: canvas.width / 2.0 - c.x * zoom,
            y: canvas.height / 2.0 - c.y * zoom,
            zoom,
        }
        .pixel_aligned()
    }

    /// The pan rounded to whole pixels, so text at zoom 1 stays sharp.
    pub fn pixel_aligned(self) -> Viewport {
        Viewport {
            x: self.x.round(),
            y: self.y.round(),
            zoom: self.zoom,
        }
    }
}

/// Which way wires leave outputs and enter inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    /// Outputs on the right, inputs on the left (shader and audio graphs).
    #[default]
    LeftToRight,
    /// Outputs at the bottom, inputs at the top (org charts, trees).
    TopToBottom,
}

/// The shape of a wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeShape {
    /// A smooth curve leaving and entering along the [`Flow`].
    #[default]
    Bezier,
    /// A straight line.
    Straight,
    /// Right angles: out along the flow, across, and in.
    Step,
}

/// The control points of a wire from `a` (an output) to `b` (an input):
/// `(a, c1, c2, b)`. A straight line and a step have their own path; this is
/// the cubic every shape's hit test samples.
pub fn bezier_points(a: Point, b: Point, flow: Flow) -> [Point; 4] {
    match flow {
        Flow::LeftToRight => {
            let d = ((b.x - a.x).abs() / 2.0).max(40.0);
            [a, a.offset(d, 0.0), b.offset(-d, 0.0), b]
        }
        Flow::TopToBottom => {
            let d = ((b.y - a.y).abs() / 2.0).max(40.0);
            [a, a.offset(0.0, d), b.offset(0.0, -d), b]
        }
    }
}

/// The corner points of a step wire, ends included.
fn step_points(a: Point, b: Point, flow: Flow) -> [Point; 4] {
    match flow {
        Flow::LeftToRight => {
            let mx = (a.x + b.x) / 2.0;
            [a, Point::new(mx, a.y), Point::new(mx, b.y), b]
        }
        Flow::TopToBottom => {
            let my = (a.y + b.y) / 2.0;
            [a, Point::new(a.x, my), Point::new(b.x, my), b]
        }
    }
}

/// The SVG path data (`d`) of a wire from `a` to `b`.
pub fn edge_path(a: Point, b: Point, shape: EdgeShape, flow: Flow) -> String {
    match shape {
        EdgeShape::Straight => format!("M{} {} L{} {}", a.x, a.y, b.x, b.y),
        EdgeShape::Bezier => {
            let [a, c1, c2, b] = bezier_points(a, b, flow);
            format!(
                "M{} {} C{} {} {} {} {} {}",
                a.x, a.y, c1.x, c1.y, c2.x, c2.y, b.x, b.y
            )
        }
        EdgeShape::Step => {
            let [a, p, q, b] = step_points(a, b, flow);
            format!(
                "M{} {} L{} {} L{} {} L{} {}",
                a.x, a.y, p.x, p.y, q.x, q.y, b.x, b.y
            )
        }
    }
}

/// The shortest distance from `p` to the wire from `a` to `b`, in graph
/// units. Used to pick an edge under the pointer (SVG paths are not hit-tested
/// by their stroke).
pub fn distance_to_edge(p: Point, a: Point, b: Point, shape: EdgeShape, flow: Flow) -> f32 {
    let polyline: Vec<Point> = match shape {
        EdgeShape::Straight => vec![a, b],
        EdgeShape::Step => step_points(a, b, flow).to_vec(),
        EdgeShape::Bezier => {
            let [p0, p1, p2, p3] = bezier_points(a, b, flow);
            (0..=32)
                .map(|i| {
                    let t = i as f32 / 32.0;
                    let u = 1.0 - t;
                    let w0 = u * u * u;
                    let w1 = 3.0 * u * u * t;
                    let w2 = 3.0 * u * t * t;
                    let w3 = t * t * t;
                    Point::new(
                        w0 * p0.x + w1 * p1.x + w2 * p2.x + w3 * p3.x,
                        w0 * p0.y + w1 * p1.y + w2 * p2.y + w3 * p3.y,
                    )
                })
                .collect()
        }
    };
    polyline
        .windows(2)
        .map(|w| distance_to_segment(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.x + t * dx, a.y + t * dy);
    ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_round_trip_and_zoom_anchor() {
        let v = Viewport {
            x: 30.0,
            y: -10.0,
            zoom: 2.0,
        };
        let g = Point::new(5.0, 7.0);
        assert_eq!(v.to_graph(v.to_canvas(g)), g);
        let anchor = Point::new(100.0, 100.0);
        let before = v.to_graph(anchor);
        let z = v.zoomed_around(0.5, anchor, 0.1, 4.0);
        let after = z.to_graph(anchor);
        // The pan is rounded to whole pixels: the anchor moves less than one.
        assert!(
            ((after.x - before.x) * z.zoom).abs() <= 0.5,
            "the point under the anchor stays"
        );
        assert!(((after.y - before.y) * z.zoom).abs() <= 0.5);
        assert_eq!(v.zoomed_around(50.0, anchor, 0.1, 4.0).zoom, 4.0);
    }

    #[test]
    fn fitting_centres_the_bounds() {
        let v = Viewport::fitting(
            Rect::new(100.0, 100.0, 200.0, 100.0),
            Size::new(440.0, 440.0),
            20.0,
            0.1,
            4.0,
        );
        assert_eq!(v.zoom, 2.0);
        assert_eq!(
            v.to_canvas(Point::new(200.0, 150.0)),
            Point::new(220.0, 220.0)
        );
    }

    #[test]
    fn distance_to_each_shape() {
        let a = Point::new(0.0, 0.0);
        let b = Point::new(100.0, 100.0);
        for shape in [EdgeShape::Bezier, EdgeShape::Straight, EdgeShape::Step] {
            assert!(distance_to_edge(a, a, b, shape, Flow::LeftToRight) < 0.01);
            assert!(distance_to_edge(b, a, b, shape, Flow::LeftToRight) < 0.01);
            assert!(
                distance_to_edge(Point::new(0.0, 300.0), a, b, shape, Flow::LeftToRight) > 50.0
            );
        }
        // The step's vertical leg is at x = 50.
        assert!(
            distance_to_edge(
                Point::new(50.0, 50.0),
                a,
                b,
                EdgeShape::Step,
                Flow::LeftToRight
            ) < 0.01
        );
        assert!(edge_path(a, b, EdgeShape::Bezier, Flow::TopToBottom).starts_with("M0 0 C0 50"));
    }
}
