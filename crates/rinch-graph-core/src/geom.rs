//! Graph-space geometry. `f32`, the unit rinch's pointer events use.

/// A point in graph space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Point { x, y }
    }

    /// This point moved by `(dx, dy)`.
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Point::new(self.x + dx, self.y + dy)
    }

    /// This point snapped to a grid of `step` (unchanged when `step <= 0`).
    pub fn snapped(self, step: f32) -> Self {
        if step <= 0.0 {
            return self;
        }
        Point::new(
            (self.x / step).round() * step,
            (self.y / step).round() * step,
        )
    }
}

/// A size in graph space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Size { width, height }
    }
}

/// An axis-aligned rectangle in graph space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle with corners `a` and `b`, in either order.
    pub fn from_corners(a: Point, b: Point) -> Self {
        let x = a.x.min(b.x);
        let y = a.y.min(b.y);
        Rect::new(x, y, (a.x - b.x).abs(), (a.y - b.y).abs())
    }

    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Rect::new(origin.x, origin.y, size.width, size.height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    /// Whether the two rectangles overlap (touching edges count).
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x <= other.right()
            && other.x <= self.right()
            && self.y <= other.bottom()
            && other.y <= self.bottom()
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_in_any_order() {
        let r = Rect::from_corners(Point::new(10.0, 20.0), Point::new(-5.0, 4.0));
        assert_eq!(r, Rect::new(-5.0, 4.0, 15.0, 16.0));
    }

    #[test]
    fn intersection_and_union() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let c = Rect::new(30.0, 0.0, 1.0, 1.0);
        assert!(a.intersects(&b));
        assert!(!a.intersects(&c));
        assert_eq!(a.union(&c), Rect::new(0.0, 0.0, 31.0, 10.0));
    }

    #[test]
    fn snapping() {
        assert_eq!(Point::new(14.0, 16.0).snapped(10.0), Point::new(10.0, 20.0));
        assert_eq!(Point::new(14.0, 16.0).snapped(0.0), Point::new(14.0, 16.0));
    }
}
