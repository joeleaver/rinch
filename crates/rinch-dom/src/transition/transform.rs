//! Transform function lists, and how they interpolate (#414).
//!
//! A computed `transform` keeps the function list it was composed from
//! ([`TransformValue::functions`](crate::computed_style::TransformValue)),
//! because CSS never interpolates the composed matrix entry by entry — doing so
//! gives the right rotation angle with a uniform scale of `cos(Δθ/2)`, so a
//! 90° turn shrank to 70.7% at its midpoint and a 180° one to a point.
//!
//! [`interpolate_lists`] is css-transforms-2 §"Interpolation of Transforms" as
//! Chrome implements it, measured in Chrome 153 (the fixtures in
//! `tests/transform_interpolation_tests.rs`):
//!
//! 1. Pair the lists function by function. Two functions pair when they share a
//!    primitive: every `translate*` is a translate, every `scale*` a scale.
//!    `skewX`, `skewY` and `skew` do **not** pair with one another in Chrome,
//!    and `matrix()` pairs with `matrix()`.
//! 2. A paired function interpolates its arguments linearly — so `rotate` goes
//!    170° → −170° through 0°, not the short way.
//! 3. Where one list runs out while every pair so far matched, it is padded
//!    with the identity function of the other list's type. `none` is the empty
//!    list, so it pads completely.
//! 4. From the first pair that does not match, the rest of each list is
//!    composed to one matrix and the two are interpolated by **decomposition**
//!    (translation, scale, shear and rotation angle, each interpolated, then
//!    recomposed — Chromium's decomposition, see [`interpolate_affine`]). A
//!    `matrix()` pair is interpolated the same way.
//!
//! A percentage `translate` stays exact throughout. It lives in the
//! `Translate` function's `pct` until the list is composed, and in an
//! [`Affine`]'s `pct_w`/`pct_h` after that; a decomposition interpolates the
//! translation linearly and never mixes it into the rotation or scale, so the
//! percentage coefficients interpolate linearly beside the pixel part.

use serde::Serialize;

/// A 2D affine transform whose translation may depend on the element's border
/// box: the final `(e, f)` is `matrix[4..6] + pct_w·W + pct_h·H`.
///
/// This is what [`TransformValue`](crate::computed_style::TransformValue)
/// stores composed; see its type doc for why four coefficients suffice.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Affine {
    /// `[a, b, c, d, e, f]`, column-vector convention.
    pub matrix: [f64; 6],
    /// The `(e, f)` contribution per unit of the element's width.
    pub pct_w: [f64; 2],
    /// The `(e, f)` contribution per unit of the element's height.
    pub pct_h: [f64; 2],
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        pct_w: [0.0, 0.0],
        pct_h: [0.0, 0.0],
    };

    /// A plain matrix, with no box-relative part.
    pub fn from_matrix(matrix: [f64; 6]) -> Self {
        Affine {
            matrix,
            pct_w: [0.0, 0.0],
            pct_h: [0.0, 0.0],
        }
    }

    /// `self` followed by `next` in list order — `self · next`, so `next`
    /// takes effect in the frame `self` establishes. The box-relative part of
    /// `next` is carried through `self`'s linear part, exactly as a percentage
    /// translate later in a list is (#212).
    pub fn then(&self, next: &Affine) -> Affine {
        let a = &self.matrix;
        let b = &next.matrix;
        let lin = |v: [f64; 2]| [a[0] * v[0] + a[2] * v[1], a[1] * v[0] + a[3] * v[1]];
        let (nw, nh) = (lin(next.pct_w), lin(next.pct_h));
        Affine {
            matrix: [
                a[0] * b[0] + a[2] * b[1],
                a[1] * b[0] + a[3] * b[1],
                a[0] * b[2] + a[2] * b[3],
                a[1] * b[2] + a[3] * b[3],
                a[0] * b[4] + a[2] * b[5] + a[4],
                a[1] * b[4] + a[3] * b[5] + a[5],
            ],
            pct_w: [self.pct_w[0] + nw[0], self.pct_w[1] + nw[1]],
            pct_h: [self.pct_h[0] + nh[0], self.pct_h[1] + nh[1]],
        }
    }
}

/// One transform function of a computed `transform` list, in the form it is
/// interpolated in. Angles are radians; a `pct` is a fraction (`0.5` = 50%).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TransformOp {
    /// Every `translate*`: a pixel part and a fraction of the border box's
    /// width (x) and height (y). A `calc()` carries both.
    Translate {
        px: [f64; 2],
        pct: [f64; 2],
    },
    /// Every `scale*`.
    Scale(f64, f64),
    Rotate(f64),
    SkewX(f64),
    SkewY(f64),
    Skew(f64, f64),
    /// `matrix()`, and what a decomposed interpolation leaves behind — which
    /// may carry a box-relative translation, hence an [`Affine`].
    Matrix(Affine),
}

impl TransformOp {
    /// This function as an affine transform.
    pub fn to_affine(&self) -> Affine {
        match self {
            TransformOp::Translate { px, pct } => Affine {
                matrix: [1.0, 0.0, 0.0, 1.0, px[0], px[1]],
                pct_w: [pct[0], 0.0],
                pct_h: [0.0, pct[1]],
            },
            TransformOp::Scale(sx, sy) => Affine::from_matrix([*sx, 0.0, 0.0, *sy, 0.0, 0.0]),
            TransformOp::Rotate(rad) => {
                let (sin, cos) = rad.sin_cos();
                Affine::from_matrix([cos, sin, -sin, cos, 0.0, 0.0])
            }
            TransformOp::SkewX(rad) => Affine::from_matrix([1.0, 0.0, rad.tan(), 1.0, 0.0, 0.0]),
            TransformOp::SkewY(rad) => Affine::from_matrix([1.0, rad.tan(), 0.0, 1.0, 0.0, 0.0]),
            TransformOp::Skew(ax, ay) => {
                Affine::from_matrix([1.0, ay.tan(), ax.tan(), 1.0, 0.0, 0.0])
            }
            TransformOp::Matrix(m) => *m,
        }
    }

    /// Whether the two functions interpolate as a pair (step 1 of the module
    /// doc).
    fn pairs_with(&self, other: &TransformOp) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }

    /// The identity function of this function's type — what `none`, or the
    /// shorter of two lists, is padded with.
    fn identity_like(&self) -> TransformOp {
        match self {
            TransformOp::Translate { .. } => TransformOp::Translate {
                px: [0.0, 0.0],
                pct: [0.0, 0.0],
            },
            TransformOp::Scale(..) => TransformOp::Scale(1.0, 1.0),
            TransformOp::Rotate(_) => TransformOp::Rotate(0.0),
            TransformOp::SkewX(_) => TransformOp::SkewX(0.0),
            TransformOp::SkewY(_) => TransformOp::SkewY(0.0),
            TransformOp::Skew(..) => TransformOp::Skew(0.0, 0.0),
            TransformOp::Matrix(_) => TransformOp::Matrix(Affine::IDENTITY),
        }
    }

    /// The arguments, flattened, for interpolation and comparison of a pair.
    fn args(&self) -> ArgVec {
        match self {
            TransformOp::Translate { px, pct } => ArgVec::of(&[px[0], px[1], pct[0], pct[1]]),
            TransformOp::Scale(x, y) | TransformOp::Skew(x, y) => ArgVec::of(&[*x, *y]),
            TransformOp::Rotate(a) | TransformOp::SkewX(a) | TransformOp::SkewY(a) => {
                ArgVec::of(&[*a])
            }
            TransformOp::Matrix(m) => ArgVec::of(&[
                m.matrix[0],
                m.matrix[1],
                m.matrix[2],
                m.matrix[3],
                m.matrix[4],
                m.matrix[5],
                m.pct_w[0],
                m.pct_w[1],
                m.pct_h[0],
                m.pct_h[1],
            ]),
        }
    }

    /// Interpolate a pair that [`pairs_with`](Self::pairs_with) holds for.
    fn interpolate_pair(&self, to: &TransformOp, t: f64) -> TransformOp {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        match (self, to) {
            (
                TransformOp::Translate { px: ap, pct: ac },
                TransformOp::Translate { px: bp, pct: bc },
            ) => TransformOp::Translate {
                px: [mix(ap[0], bp[0]), mix(ap[1], bp[1])],
                pct: [mix(ac[0], bc[0]), mix(ac[1], bc[1])],
            },
            (TransformOp::Scale(ax, ay), TransformOp::Scale(bx, by)) => {
                TransformOp::Scale(mix(*ax, *bx), mix(*ay, *by))
            }
            (TransformOp::Rotate(a), TransformOp::Rotate(b)) => TransformOp::Rotate(mix(*a, *b)),
            (TransformOp::SkewX(a), TransformOp::SkewX(b)) => TransformOp::SkewX(mix(*a, *b)),
            (TransformOp::SkewY(a), TransformOp::SkewY(b)) => TransformOp::SkewY(mix(*a, *b)),
            (TransformOp::Skew(ax, ay), TransformOp::Skew(bx, by)) => {
                TransformOp::Skew(mix(*ax, *bx), mix(*ay, *by))
            }
            (TransformOp::Matrix(a), TransformOp::Matrix(b)) => {
                TransformOp::Matrix(interpolate_affine(a, b, t))
            }
            _ => unreachable!("interpolate_pair is only called on a pair"),
        }
    }
}

/// A small fixed-capacity argument list, so comparing two function lists
/// allocates nothing.
struct ArgVec {
    len: usize,
    v: [f64; 10],
}

impl ArgVec {
    fn of(args: &[f64]) -> Self {
        let mut v = [0.0; 10];
        v[..args.len()].copy_from_slice(args);
        ArgVec { len: args.len(), v }
    }
    fn as_slice(&self) -> &[f64] {
        &self.v[..self.len]
    }
}

/// Compose a function list, in list order.
pub fn compose(ops: &[TransformOp]) -> Affine {
    ops.iter()
        .fold(Affine::IDENTITY, |acc, op| acc.then(&op.to_affine()))
}

/// The composed matrix of `ops`, without its box-relative part.
pub fn compose_matrices(ops: &[TransformOp]) -> [f64; 6] {
    compose(ops).matrix
}

/// How the two lists line up, per steps 1 and 3 of the module doc: the length
/// of the prefix that interpolates function by function (padding included),
/// and whether anything after it has to be decomposed.
fn matching_prefix(a: &[TransformOp], b: &[TransformOp]) -> usize {
    let shorter = a.len().min(b.len());
    for i in 0..shorter {
        if !a[i].pairs_with(&b[i]) {
            return i;
        }
    }
    a.len().max(b.len())
}

/// The `i`th function of each list, the missing one padded with the identity
/// of the other's type. Only called below [`matching_prefix`].
fn pair_at(a: &[TransformOp], b: &[TransformOp], i: usize) -> (TransformOp, TransformOp) {
    match (a.get(i), b.get(i)) {
        (Some(x), Some(y)) => (x.clone(), y.clone()),
        (Some(x), None) => (x.clone(), x.identity_like()),
        (None, Some(y)) => (y.identity_like(), y.clone()),
        (None, None) => unreachable!("below the longer list's length"),
    }
}

/// Interpolate two computed transform function lists at progress `t`.
///
/// The result is a function list too — a transition interrupted part way
/// starts its successor *from* this value, and has to be able to go on pairing
/// it function by function.
pub fn interpolate_lists(a: &[TransformOp], b: &[TransformOp], t: f64) -> Vec<TransformOp> {
    let prefix = matching_prefix(a, b);
    let mut out: Vec<TransformOp> = (0..prefix)
        .map(|i| {
            let (x, y) = pair_at(a, b, i);
            x.interpolate_pair(&y, t)
        })
        .collect();
    if prefix < a.len().max(b.len()) {
        let ra = compose(&a[prefix.min(a.len())..]);
        let rb = compose(&b[prefix.min(b.len())..]);
        out.push(TransformOp::Matrix(interpolate_affine(&ra, &rb, t)));
    }
    out
}

/// Whether interpolating between the two lists would hold still — the
/// transition machinery's "the value did not change".
///
/// Lists that pair function by function are equal when every pair's arguments
/// are, so `rotate(0deg)` and `rotate(360deg)` differ (a full turn is a
/// change) while `none` and `rotate(0deg)` do not (the padding makes them the
/// same list). Lists that do not pair are equal when they compose to the same
/// transform, since their interpolation is a decomposition of exactly that.
///
/// The one place this departs from Chrome is that last pair: Chrome starts a
/// transition from `none` to `rotate(0deg)`. It moves nothing, and in rinch a
/// transitioning transform is a stacking context for the length of the run
/// while `rotate(0deg)` at rest is not (#415), so starting one would make the
/// element's stacking flicker for no visible change.
pub fn lists_equivalent(a: &[TransformOp], b: &[TransformOp]) -> bool {
    let close = |x: &f64, y: &f64| (x - y).abs() <= 0.001;
    let len = a.len().max(b.len());
    if matching_prefix(a, b) == len {
        (0..len).all(|i| {
            let (x, y) = pair_at(a, b, i);
            let (xa, ya) = (x.args(), y.args());
            xa.as_slice()
                .iter()
                .zip(ya.as_slice())
                .all(|(p, q)| close(p, q))
        })
    } else {
        let (ca, cb) = (compose(a), compose(b));
        ca.matrix.iter().zip(&cb.matrix).all(|(p, q)| close(p, q))
            && ca.pct_w.iter().zip(&cb.pct_w).all(|(p, q)| close(p, q))
            && ca.pct_h.iter().zip(&cb.pct_h).all(|(p, q)| close(p, q))
    }
}

/// A 2D matrix's linear part decomposed as `L = R(angle) · K · S`, where `K`
/// is the shear `[[1, skew], [0, 1]]` and `S` the scale. The translation is
/// not decomposed at all: it interpolates linearly.
///
/// This is Chromium's `Matrix44::Decompose2d` (Gram–Schmidt: the x scale is
/// the first column's length, the shear its projection onto the second), which
/// is what Chrome 153 interpolates with — measured, not assumed. The
/// css-transforms-1 pseudo-code ("Decomposing a 2D matrix") keeps a residual
/// 2×2 matrix instead of one shear, and gives visibly different values for a
/// sheared pair (`skewX(10deg)` → `skewY(20deg)` differs in the third entry at
/// 0.2 by 0.0014, `skewX(30deg) rotate(20deg)` → `scale(2, 0.5)
/// rotate(200deg)` by 0.04). `tests/transform_interpolation_tests.rs` pins
/// the Chrome numbers.
#[derive(Debug, Clone, Copy)]
struct Decomposed {
    scale: [f64; 2],
    skew: f64,
    angle: f64,
}

/// `None` for a singular matrix, which has no decomposition.
fn decompose(m: &[f64; 6]) -> Option<Decomposed> {
    let [a, b, c, d, ..] = *m;
    let det = a * d - b * c;
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    // A negative determinant is a reflection; one axis's scale carries it.
    // Flipping both would be a half turn, which the rotation already covers.
    let (mut sx, mut sy) = (1.0, 1.0);
    if det < 0.0 {
        if a < d {
            sx = -1.0;
        } else {
            sy = -1.0;
        }
    }
    sx *= a.hypot(b);
    let (u0x, u0y) = (a / sx, b / sx);
    // The second column's component along the first is the (scaled) shear.
    let shear = u0x * c + u0y * d;
    let (rx, ry) = (c - u0x * shear, d - u0y * shear);
    sy *= rx.hypot(ry);
    Some(Decomposed {
        scale: [sx, sy],
        skew: shear / sy,
        angle: u0y.atan2(u0x),
    })
}

fn recompose(d: &Decomposed, translate: [f64; 2]) -> [f64; 6] {
    let (sin, cos) = d.angle.sin_cos();
    // R · K: the first column is R's, the second is R·(skew, 1).
    let rk = [cos, sin, cos * d.skew - sin, sin * d.skew + cos];
    // · S
    [
        rk[0] * d.scale[0],
        rk[1] * d.scale[0],
        rk[2] * d.scale[1],
        rk[3] * d.scale[1],
        translate[0],
        translate[1],
    ]
}

/// Interpolate two affine transforms by decomposition. The translation —
/// pixel and box-relative parts alike — is linear; scale and shear are linear;
/// the rotation takes the shorter way round (Chrome slerps a quaternion, which
/// for two rotations about one axis is exactly that, a half turn going the way
/// the angles are written). A singular endpoint has no decomposition, and the
/// pair then flips from one to the other half way, as in Chrome.
pub fn interpolate_affine(a: &Affine, b: &Affine, t: f64) -> Affine {
    use std::f64::consts::{PI, TAU};
    let mix = |x: f64, y: f64| x + (y - x) * t;

    let (Some(da), Some(db)) = (decompose(&a.matrix), decompose(&b.matrix)) else {
        return if t < 0.5 { *a } else { *b };
    };

    let mut turn = db.angle - da.angle;
    if turn > PI {
        turn -= TAU;
    } else if turn < -PI {
        turn += TAU;
    }

    let d = Decomposed {
        scale: [mix(da.scale[0], db.scale[0]), mix(da.scale[1], db.scale[1])],
        skew: mix(da.skew, db.skew),
        angle: da.angle + turn * t,
    };
    let translate = [mix(a.matrix[4], b.matrix[4]), mix(a.matrix[5], b.matrix[5])];
    Affine {
        matrix: recompose(&d, translate),
        pct_w: [mix(a.pct_w[0], b.pct_w[0]), mix(a.pct_w[1], b.pct_w[1])],
        pct_h: [mix(a.pct_h[0], b.pct_h[0]), mix(a.pct_h[1], b.pct_h[1])],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f64; 6], b: &[f64; 6]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    /// Decomposition is exact: recomposing gives the matrix back, for shears,
    /// reflections and rotations past 90° alike. Interpolation at `t = 0` and
    /// `t = 1` depends on it.
    #[test]
    fn decompose_then_recompose_round_trips() {
        let cases = [
            [1.0, 0.0, 0.0, 1.0, 3.0, 4.0],
            [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
            [2.0, 0.5, 0.3, -1.5, 0.0, 0.0],
            [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [1.2, -0.7, 2.1, 0.4, 5.0, -6.0],
            [-0.5, -0.8, 0.9, -0.1, 0.0, 0.0],
        ];
        for m in cases {
            let d = decompose(&m).expect("invertible");
            let r = recompose(&d, [m[4], m[5]]);
            assert!(close(&m, &r), "{m:?} came back as {r:?} via {d:?}");
        }
    }

    #[test]
    fn interpolation_endpoints_are_the_endpoints() {
        let a = Affine::from_matrix([2.0, 0.5, 0.3, -1.5, 1.0, 2.0]);
        let b = Affine::from_matrix([-0.5, -0.8, 0.9, -0.1, 7.0, -3.0]);
        assert!(close(&interpolate_affine(&a, &b, 0.0).matrix, &a.matrix));
        assert!(close(&interpolate_affine(&a, &b, 1.0).matrix, &b.matrix));
    }
}
