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
//! `tests/transform_interpolation_tests.rs`, and for the 3D functions
//! `tests/transform_3d_tests.rs`):
//!
//! 1. Pair the lists function by function. Two functions pair when they share a
//!    primitive: every `translate*` is a translate (`translateZ` included),
//!    every `scale*` a scale, every `rotate*` a rotation (`rotate`, `rotateX`
//!    and `rotate3d` pair with one another). `skewX`, `skewY` and `skew` do
//!    **not** pair with one another in Chrome, `matrix()` pairs with
//!    `matrix()`, `matrix3d()` with `matrix3d()` and not with `matrix()`, and
//!    `perspective()` with `perspective()`.
//! 2. A paired function interpolates its arguments linearly — so `rotate` goes
//!    170° → −170° through 0°, not the short way. Two rotations about
//!    different axes are the exception: they interpolate by quaternion slerp,
//!    which does take the shorter arc. `perspective(d)` interpolates `1/d`.
//! 3. Where one list runs out while every pair so far matched, it is padded
//!    with the identity function of the other list's type. `none` is the empty
//!    list, so it pads completely.
//! 4. From the first pair that does not match, the rest of each list is
//!    composed to one matrix and the two are interpolated by **decomposition**
//!    (translation, scale, shear and rotation angle, each interpolated, then
//!    recomposed — Chromium's decomposition, see [`interpolate_affine`]). A
//!    `matrix()` pair is interpolated the same way, and so is a `matrix3d()`
//!    pair once flattened. With a 3D function in a remainder this is **not**
//!    Chrome's: Chrome decomposes the 4×4 before flattening it, rinch
//!    decomposes the flattened 2D matrix, which has lost any rotation out of
//!    the page (#989).
//!
//! Not modelled at all: the `perspective` and `transform-style` properties.
//! `transform-origin`'s z component and `backface-visibility` are applied
//! where a list is composed for the page, by [`compose_about_origin_z`]
//! (#997) — not here, since an interpolated list is composed about the same
//! origin as any other.
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
///
/// The 3D functions are kept as 3D functions (#405): a list is composed as
/// 4×4 matrices and only the result is flattened to the [`Affine`] rinch
/// paints with (see [`compose`]), because flattening function by function is
/// wrong — `rotateX(90deg) scaleY(2) rotateX(-90deg)` is the identity, while
/// each function's own flattening is singular.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum TransformOp {
    /// Every `translate*`: a pixel part and a fraction of the border box's
    /// width (x) and height (y), plus the pixel `z` of `translateZ` /
    /// `translate3d` (`<length>` only — a `z` has no percentage). A `calc()`
    /// carries both parts.
    Translate {
        px: [f64; 2],
        pct: [f64; 2],
        z: f64,
    },
    /// Every `scale*`: x, y and z.
    Scale(f64, f64, f64),
    /// `rotate()`, `rotateZ()`, and `rotate3d()` about the z axis: a rotation
    /// in the page.
    Rotate(f64),
    /// `rotateX()`, `rotateY()`, and `rotate3d()` about any other axis. The
    /// axis is a unit vector and never the z axis — [`TransformOp::rotate3d`]
    /// turns that one into [`TransformOp::Rotate`].
    Rotate3D {
        axis: [f64; 3],
        angle: f64,
    },
    SkewX(f64),
    SkewY(f64),
    Skew(f64, f64),
    /// `perspective()`, stored as `1 / d` — the quantity Chrome interpolates
    /// (the matrix entry is `-1/d`). `perspective(none)` is `0`.
    Perspective(f64),
    /// `matrix()`, and what a decomposed interpolation leaves behind — which
    /// may carry a box-relative translation, hence an [`Affine`].
    Matrix(Affine),
    /// `matrix3d()`, as a row-major 4×4 (`m[row][col]`, column vectors — so
    /// CSS's `m41, m42` are `m[0][3], m[1][3]`).
    Matrix3D([[f64; 4]; 4]),
}

impl TransformOp {
    /// `rotate3d(x, y, z, angle)`. The axis need not be normalised; a zero
    /// axis rotates nothing (Chrome draws `rotate3d(0, 0, 0, 30deg)`
    /// unrotated), and the z axis gives a plain [`TransformOp::Rotate`], so
    /// `rotate3d(0, 0, -2, 30deg)` is `rotate(-30deg)`.
    pub fn rotate3d(x: f64, y: f64, z: f64, angle: f64) -> TransformOp {
        let len = (x * x + y * y + z * z).sqrt();
        if len < 1e-12 || !len.is_finite() {
            return TransformOp::Rotate(0.0);
        }
        let axis = [x / len, y / len, z / len];
        if axis[0].abs() < 1e-12 && axis[1].abs() < 1e-12 {
            return TransformOp::Rotate(angle * axis[2].signum());
        }
        TransformOp::Rotate3D { axis, angle }
    }

    /// `perspective(d)`; `f64::INFINITY` is `perspective(none)`. A depth below
    /// 1px is clamped to 1px, as Chrome does (`perspective(0px)` computes to
    /// a `m34` of `-1`).
    pub fn perspective(d: f64) -> TransformOp {
        if d.is_finite() {
            TransformOp::Perspective(1.0 / d.max(1.0))
        } else {
            TransformOp::Perspective(0.0)
        }
    }

    /// `matrix3d()` from its sixteen arguments in CSS order (column-major).
    pub fn matrix3d(v: [f64; 16]) -> TransformOp {
        let mut m = [[0.0; 4]; 4];
        for (col, chunk) in v.chunks(4).enumerate() {
            for (row, x) in chunk.iter().enumerate() {
                m[row][col] = *x;
            }
        }
        TransformOp::Matrix3D(m)
    }

    /// Whether this function maps the page's plane onto itself, so the list
    /// it is in can be composed as 2D affines. A function that is not is
    /// composed as a 4×4 ([`Mat4`]) and the list flattened afterwards.
    fn is_planar(&self) -> bool {
        match self {
            TransformOp::Translate { z, .. } => *z == 0.0,
            TransformOp::Scale(_, _, z) => *z == 1.0,
            TransformOp::Rotate3D { .. } => false,
            TransformOp::Perspective(inv) => *inv == 0.0,
            TransformOp::Matrix3D(_) => false,
            TransformOp::Rotate(_)
            | TransformOp::SkewX(_)
            | TransformOp::SkewY(_)
            | TransformOp::Skew(..)
            | TransformOp::Matrix(_) => true,
        }
    }

    /// This function as an affine transform. Only meaningful for a
    /// [planar](Self::is_planar) function; a 3D one is its 4×4 flattened on
    /// its own, which is what a list of one function draws.
    pub fn to_affine(&self) -> Affine {
        match self {
            TransformOp::Translate { px, pct, .. } => Affine {
                matrix: [1.0, 0.0, 0.0, 1.0, px[0], px[1]],
                pct_w: [pct[0], 0.0],
                pct_h: [0.0, pct[1]],
            },
            TransformOp::Scale(sx, sy, _) => Affine::from_matrix([*sx, 0.0, 0.0, *sy, 0.0, 0.0]),
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
            TransformOp::Rotate3D { .. }
            | TransformOp::Perspective(_)
            | TransformOp::Matrix3D(_) => self.to_mat4().flatten(),
        }
    }

    /// This function as a 4×4.
    fn to_mat4(&self) -> Mat4 {
        match self {
            TransformOp::Translate { px, pct, z } => {
                let mut m = Mat4::IDENTITY;
                m.m[0][3] = px[0];
                m.m[1][3] = px[1];
                m.m[2][3] = *z;
                m.pct_w = Mat4::box_translation(pct[0], 0.0);
                m.pct_h = Mat4::box_translation(0.0, pct[1]);
                m
            }
            TransformOp::Scale(x, y, z) => {
                let mut m = Mat4::IDENTITY;
                m.m[0][0] = *x;
                m.m[1][1] = *y;
                m.m[2][2] = *z;
                m
            }
            TransformOp::Rotate3D { axis, angle } => {
                // css-transforms-2's `rotate3d()` matrix.
                let [x, y, z] = *axis;
                let (s, c) = angle.sin_cos();
                let k = 1.0 - c;
                Mat4 {
                    m: [
                        [
                            1.0 + k * (x * x - 1.0),
                            -z * s + k * x * y,
                            y * s + k * x * z,
                            0.0,
                        ],
                        [
                            z * s + k * x * y,
                            1.0 + k * (y * y - 1.0),
                            -x * s + k * y * z,
                            0.0,
                        ],
                        [
                            -y * s + k * x * z,
                            x * s + k * y * z,
                            1.0 + k * (z * z - 1.0),
                            0.0,
                        ],
                        [0.0, 0.0, 0.0, 1.0],
                    ],
                    pct_w: NO_BOX,
                    pct_h: NO_BOX,
                }
            }
            TransformOp::Perspective(inv) => {
                let mut m = Mat4::IDENTITY;
                m.m[3][2] = -inv;
                m
            }
            TransformOp::Matrix3D(m) => Mat4::from_m(m),
            _ => Mat4::lift(&self.to_affine()),
        }
    }

    /// The primitive a function interpolates as (step 1 of the module doc):
    /// every `translate*` is one, every `scale*` one, and every `rotate*` —
    /// `rotateX` and `rotate3d` included — one. `matrix()` and `matrix3d()`
    /// do not pair with each other in Chrome 153 (measured).
    fn primitive(&self) -> u8 {
        match self {
            TransformOp::Translate { .. } => 0,
            TransformOp::Scale(..) => 1,
            TransformOp::Rotate(_) | TransformOp::Rotate3D { .. } => 2,
            TransformOp::SkewX(_) => 3,
            TransformOp::SkewY(_) => 4,
            TransformOp::Skew(..) => 5,
            TransformOp::Perspective(_) => 6,
            TransformOp::Matrix(_) => 7,
            TransformOp::Matrix3D(_) => 8,
        }
    }

    /// Whether the two functions interpolate as a pair (step 1 of the module
    /// doc).
    fn pairs_with(&self, other: &TransformOp) -> bool {
        self.primitive() == other.primitive()
    }

    /// The identity function of this function's type — what `none`, or the
    /// shorter of two lists, is padded with.
    fn identity_like(&self) -> TransformOp {
        match self {
            TransformOp::Translate { .. } => TransformOp::Translate {
                px: [0.0, 0.0],
                pct: [0.0, 0.0],
                z: 0.0,
            },
            TransformOp::Scale(..) => TransformOp::Scale(1.0, 1.0, 1.0),
            TransformOp::Rotate(_) => TransformOp::Rotate(0.0),
            TransformOp::Rotate3D { axis, .. } => TransformOp::Rotate3D {
                axis: *axis,
                angle: 0.0,
            },
            TransformOp::SkewX(_) => TransformOp::SkewX(0.0),
            TransformOp::SkewY(_) => TransformOp::SkewY(0.0),
            TransformOp::Skew(..) => TransformOp::Skew(0.0, 0.0),
            TransformOp::Perspective(_) => TransformOp::Perspective(0.0),
            TransformOp::Matrix(_) => TransformOp::Matrix(Affine::IDENTITY),
            TransformOp::Matrix3D(_) => TransformOp::Matrix3D(Mat4::IDENTITY.m),
        }
    }

    /// A rotation as `(unit axis, angle)`.
    fn axis_angle(&self) -> ([f64; 3], f64) {
        match self {
            TransformOp::Rotate(a) => ([0.0, 0.0, 1.0], *a),
            TransformOp::Rotate3D { axis, angle } => (*axis, *angle),
            _ => unreachable!("axis_angle is only asked of a rotation"),
        }
    }

    /// The arguments, flattened, for interpolation and comparison of a pair.
    fn args(&self) -> ArgVec {
        match self {
            TransformOp::Translate { px, pct, z } => {
                ArgVec::of(&[px[0], px[1], pct[0], pct[1], *z])
            }
            TransformOp::Scale(x, y, z) => ArgVec::of(&[*x, *y, *z]),
            TransformOp::Skew(x, y) => ArgVec::of(&[*x, *y]),
            TransformOp::Rotate(_) | TransformOp::Rotate3D { .. } => {
                let (axis, angle) = self.axis_angle();
                ArgVec::of(&[axis[0], axis[1], axis[2], angle])
            }
            TransformOp::SkewX(a) | TransformOp::SkewY(a) | TransformOp::Perspective(a) => {
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
            TransformOp::Matrix3D(m) => {
                let mut v = [0.0; 16];
                for (i, x) in m.iter().flatten().enumerate() {
                    v[i] = *x;
                }
                ArgVec::of(&v)
            }
        }
    }

    /// Interpolate a pair that [`pairs_with`](Self::pairs_with) holds for.
    fn interpolate_pair(&self, to: &TransformOp, t: f64) -> TransformOp {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        match (self, to) {
            (
                TransformOp::Translate {
                    px: ap,
                    pct: ac,
                    z: az,
                },
                TransformOp::Translate {
                    px: bp,
                    pct: bc,
                    z: bz,
                },
            ) => TransformOp::Translate {
                px: [mix(ap[0], bp[0]), mix(ap[1], bp[1])],
                pct: [mix(ac[0], bc[0]), mix(ac[1], bc[1])],
                z: mix(*az, *bz),
            },
            (TransformOp::Scale(ax, ay, az), TransformOp::Scale(bx, by, bz)) => {
                TransformOp::Scale(mix(*ax, *bx), mix(*ay, *by), mix(*az, *bz))
            }
            (TransformOp::Rotate(a), TransformOp::Rotate(b)) => TransformOp::Rotate(mix(*a, *b)),
            (TransformOp::Rotate(_) | TransformOp::Rotate3D { .. }, _) => {
                interpolate_rotations(self.axis_angle(), to.axis_angle(), t)
            }
            (TransformOp::SkewX(a), TransformOp::SkewX(b)) => TransformOp::SkewX(mix(*a, *b)),
            (TransformOp::SkewY(a), TransformOp::SkewY(b)) => TransformOp::SkewY(mix(*a, *b)),
            (TransformOp::Skew(ax, ay), TransformOp::Skew(bx, by)) => {
                TransformOp::Skew(mix(*ax, *bx), mix(*ay, *by))
            }
            (TransformOp::Perspective(a), TransformOp::Perspective(b)) => {
                TransformOp::Perspective(mix(*a, *b))
            }
            (TransformOp::Matrix(a), TransformOp::Matrix(b)) => {
                TransformOp::Matrix(interpolate_affine(a, b, t))
            }
            // Flattened and decomposed in 2D, which is exact for a
            // `matrix3d()` that draws a plane figure — the usual reason to
            // write one — and not Chrome's for one that does not: Chrome
            // decomposes in 3D (#989).
            (TransformOp::Matrix3D(a), TransformOp::Matrix3D(b)) => {
                let (fa, fb) = (Mat4::from_m(a).flatten(), Mat4::from_m(b).flatten());
                TransformOp::Matrix3D(Mat4::lift(&interpolate_affine(&fa, &fb, t)).m)
            }
            _ => unreachable!("interpolate_pair is only called on a pair"),
        }
    }
}

/// Interpolate two rotations as Chrome does: about their common axis when
/// they have one (so the angle is linear, and `170deg` → `-170deg` goes the
/// long way through `0deg` as a plain `rotate()` pair does), and otherwise
/// by spherical interpolation of their quaternions.
fn interpolate_rotations(
    (axis_a, angle_a): ([f64; 3], f64),
    (axis_b, angle_b): ([f64; 3], f64),
    t: f64,
) -> TransformOp {
    let mix = |a: f64, b: f64| a + (b - a) * t;
    let dot3 = axis_a[0] * axis_b[0] + axis_a[1] * axis_b[1] + axis_a[2] * axis_b[2];
    if (dot3 - 1.0).abs() < 1e-9 || angle_a == 0.0 && angle_b == 0.0 {
        return TransformOp::rotate3d(axis_a[0], axis_a[1], axis_a[2], mix(angle_a, angle_b));
    }
    if angle_a == 0.0 {
        return TransformOp::rotate3d(axis_b[0], axis_b[1], axis_b[2], mix(0.0, angle_b));
    }
    if angle_b == 0.0 {
        return TransformOp::rotate3d(axis_a[0], axis_a[1], axis_a[2], mix(angle_a, 0.0));
    }
    let quat = |axis: [f64; 3], angle: f64| {
        let (s, c) = (angle / 2.0).sin_cos();
        [axis[0] * s, axis[1] * s, axis[2] * s, c]
    };
    let (qa, mut qb) = (quat(axis_a, angle_a), quat(axis_b, angle_b));
    // Chromium's slerp, which takes the shorter arc: a negative dot product
    // negates one end (`q` and `-q` are the same rotation). Measured —
    // `rotateX(300deg)` → `rotateY(60deg)` goes the short way in Chrome 153,
    // where css-transforms-2's pseudo-code, which negates nothing, would not.
    let mut dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
    if dot < 0.0 {
        qb = qb.map(|x| -x);
        dot = -dot;
    }
    let dot = dot.min(1.0);
    let q = if (dot - 1.0).abs() < 1e-9 {
        qa
    } else {
        let theta = dot.acos();
        let w = (t * theta).sin() / (1.0 - dot * dot).sqrt();
        let s1 = (t * theta).cos() - dot * w;
        [
            s1 * qa[0] + w * qb[0],
            s1 * qa[1] + w * qb[1],
            s1 * qa[2] + w * qb[2],
            s1 * qa[3] + w * qb[3],
        ]
    };
    let angle = 2.0 * q[3].clamp(-1.0, 1.0).acos();
    let s = (angle / 2.0).sin();
    if s.abs() < 1e-12 {
        return TransformOp::Rotate(0.0);
    }
    TransformOp::rotate3d(q[0] / s, q[1] / s, q[2] / s, angle)
}

/// A small fixed-capacity argument list, so comparing two function lists
/// allocates nothing.
struct ArgVec {
    len: usize,
    v: [f64; 16],
}

impl ArgVec {
    fn of(args: &[f64]) -> Self {
        let mut v = [0.0; 16];
        v[..args.len()].copy_from_slice(args);
        ArgVec { len: args.len(), v }
    }
    fn as_slice(&self) -> &[f64] {
        &self.v[..self.len]
    }
}

/// The box-relative part of a [`Mat4`]: its first three rows, per unit of the
/// border box's width or height.
type BoxRows = [[f64; 4]; 3];

const NO_BOX: BoxRows = [[0.0; 4]; 3];

/// A 4×4 transform whose translation may depend on the element's border box,
/// the 3D counterpart of [`Affine`]: the matrix is `m + pct_w·W + pct_h·H`.
///
/// A function puts its box-relative part only in its translation column, but
/// composition moves it: a percentage translate *before* a `perspective()`
/// meets the perspective's `-1/d` in the `z` column, and a `translateZ` after
/// that carries it back into the translation. So the whole of the first three
/// rows is kept. It stays linear under composition for the reason [`Affine`]
/// gives — a product of two box-relative parts has no second-order term,
/// because each has a zero `w` row. What is dropped is a box-relative part the
/// `w` row picks up (a `perspective()` after a percentage translate that a
/// rotation turned into z), where the translation stops being linear in the
/// box's size at all; it goes with the other projective terms
/// [`Mat4::flatten`] drops.
#[derive(Debug, Clone, Copy)]
struct Mat4 {
    m: [[f64; 4]; 4],
    pct_w: BoxRows,
    pct_h: BoxRows,
}

impl Mat4 {
    const IDENTITY: Mat4 = Mat4 {
        m: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        pct_w: NO_BOX,
        pct_h: NO_BOX,
    };

    fn from_m(m: &[[f64; 4]; 4]) -> Mat4 {
        Mat4 {
            m: *m,
            pct_w: NO_BOX,
            pct_h: NO_BOX,
        }
    }

    /// A translation column's box-relative part as [`BoxRows`].
    fn box_translation(x: f64, y: f64) -> BoxRows {
        let mut p = NO_BOX;
        p[0][3] = x;
        p[1][3] = y;
        p
    }

    /// A 2D affine as the 4×4 that leaves z alone.
    fn lift(a: &Affine) -> Mat4 {
        let [a0, b, c, d, e, f] = a.matrix;
        Mat4 {
            m: [
                [a0, c, 0.0, e],
                [b, d, 0.0, f],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            pct_w: Self::box_translation(a.pct_w[0], a.pct_w[1]),
            pct_h: Self::box_translation(a.pct_h[0], a.pct_h[1]),
        }
    }

    /// `self · next`, as [`Affine::then`].
    fn then(&self, next: &Mat4) -> Mat4 {
        let (a, b) = (&self.m, &next.m);
        let mut m = [[0.0; 4]; 4];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, x) in row.iter_mut().enumerate() {
                *x = (0..4).map(|k| a[r][k] * b[k][c]).sum();
            }
        }
        // (A + W·Pa)(B + W·Pb) = AB + W·(A·Pb + Pa·B), rows 0..3 of it: `A·Pb`
        // is `next`'s box-relative part carried through `self` (`Pb` has no
        // `w` row, so only A's first three columns meet it), `Pa·B` is
        // `self`'s carried on through `next`.
        let carry = |pa: &BoxRows, pb: &BoxRows| {
            let mut out = NO_BOX;
            for (r, row) in out.iter_mut().enumerate() {
                for (c, x) in row.iter_mut().enumerate() {
                    *x = (0..3).map(|k| a[r][k] * pb[k][c]).sum::<f64>()
                        + (0..4).map(|k| pa[r][k] * b[k][c]).sum::<f64>();
                }
            }
            out
        };
        Mat4 {
            m,
            pct_w: carry(&self.pct_w, &next.pct_w),
            pct_h: carry(&self.pct_h, &next.pct_h),
        }
    }

    /// A 3×3 minor: the determinant of rows `r` and columns `c`.
    fn minor(&self, r: [usize; 3], c: [usize; 3]) -> f64 {
        let m = &self.m;
        m[r[0]][c[0]] * (m[r[1]][c[1]] * m[r[2]][c[2]] - m[r[1]][c[2]] * m[r[2]][c[1]])
            - m[r[0]][c[1]] * (m[r[1]][c[0]] * m[r[2]][c[2]] - m[r[1]][c[2]] * m[r[2]][c[0]])
            + m[r[0]][c[2]] * (m[r[1]][c[0]] * m[r[2]][c[1]] - m[r[1]][c[1]] * m[r[2]][c[0]])
    }

    /// The determinant, by Laplace expansion along row 0. The box-relative
    /// part is a translation and changes nothing about it.
    fn determinant(&self) -> f64 {
        (0..4)
            .map(|c| {
                let cols = match c {
                    0 => [1, 2, 3],
                    1 => [0, 2, 3],
                    2 => [0, 1, 3],
                    _ => [0, 1, 2],
                };
                let sign = if c % 2 == 0 { 1.0 } else { -1.0 };
                sign * self.m[0][c] * self.minor([1, 2, 3], cols)
            })
            .sum()
    }

    /// Whether Chrome would refuse to invert this, and so neither draws nor
    /// hits an element transformed by it (#1051): a determinant that is not a
    /// normal `f32`. Measured in Chrome 153: `scaleZ(2.5e-38)` is drawn and
    /// `scaleZ(1e-38)` — or `scaleZ(1e-20) scaleZ(1e-20)` — is not, while a
    /// determinant that is only small (`scaleZ(1e-20)`) is drawn.
    fn singular(&self) -> bool {
        let det = self.determinant();
        !det.is_finite() || det.abs() < f32::MIN_POSITIVE as f64
    }

    /// Whether this turns the plane's back to the viewer: the `(3, 3)` entry
    /// of the inverse is negative, i.e. its cofactor and the determinant have
    /// opposite signs (Chrome's `IsBackFaceVisible`, which also answers "no"
    /// for a singular matrix). The box-relative part is a translation and
    /// cannot turn anything.
    fn back_facing(&self) -> bool {
        let det = self.determinant();
        if det == 0.0 || !det.is_finite() {
            return false;
        }
        let cofactor33 = self.minor([0, 1, 3], [0, 1, 3]);
        // Chrome's `kEpsilon` is float epsilon: `scale(0.015) rotateY(180deg)`
        // (`cofactor33 · det = -s⁴ ≈ -5e-8`) still faces the viewer there.
        cofactor33 * det < -(f32::EPSILON as f64)
    }

    /// Project onto the page as Chrome does for an element no ancestor gives
    /// a `perspective` or `preserve-3d`: apply the matrix to the plane
    /// `z = 0`, divide by `w`, drop the resulting `z`. That is the 2D affine
    /// taken from rows and columns 0, 1 and 3, divided by `m44`.
    ///
    /// Exact when the `w` row does not depend on x or y (`m14 = m24 = 0` in
    /// CSS's naming). When it does — a `perspective()` acting on a rotation
    /// out of the page — the figure is a trapezoid no affine can draw, and
    /// those two terms are dropped (#405). A `w` at or below 0 puts the
    /// plane at or behind the viewer, where Chrome 153 draws nothing and
    /// `elementFromPoint` finds nothing; it flattens to the zero matrix, which
    /// paint and hit testing already treat as nothing (`scale(0)`). Of the
    /// box-relative part only the translation is kept: anything it put in the
    /// linear columns came from a projective `w` row too.
    fn flatten(&self) -> Affine {
        let m = &self.m;
        // At or behind the viewer: Chrome draws nothing and hits nothing.
        // The zero matrix is the path `scale(0)` already takes.
        if m[3][3].is_nan() || m[3][3] <= 1e-9 {
            return Affine::from_matrix([0.0; 6]);
        }
        let w = m[3][3];
        Affine {
            matrix: [
                m[0][0] / w,
                m[1][0] / w,
                m[0][1] / w,
                m[1][1] / w,
                m[0][3] / w,
                m[1][3] / w,
            ],
            pct_w: [self.pct_w[0][3] / w, self.pct_w[1][3] / w],
            pct_h: [self.pct_h[0][3] / w, self.pct_h[1][3] / w],
        }
    }
}

/// Compose a function list, in list order, and flatten it to the page.
///
/// A list of [planar](TransformOp::is_planar) functions composes as 2D
/// affines, exactly as before #405. A list with a 3D function in it composes
/// as 4×4 matrices and is flattened once, at the end ([`Mat4::flatten`]).
pub fn compose(ops: &[TransformOp]) -> Affine {
    if ops.iter().all(TransformOp::is_planar) {
        ops.iter()
            .fold(Affine::IDENTITY, |acc, op| acc.then(&op.to_affine()))
    } else {
        ops.iter()
            .fold(Mat4::IDENTITY, |acc, op| acc.then(&op.to_mat4()))
            .flatten()
    }
}

/// [`compose`], about a `transform-origin` whose z is `origin_z` CSS px, and
/// whether the result turns the element's back to the viewer.
///
/// CSS applies a transform as `T(o) · M · T(-o)`. The x and y of the origin
/// commute with the flattening, so paint applies them to the flattened matrix
/// ([`crate::paint::compose_node_transform`]); the z does not — `T(0, 0, -z)`
/// moves the plane off `z = 0` before `M` turns it — so it is applied here,
/// before [`Mat4::flatten`]. A list of planar functions never moves z, so the
/// origin's z changes nothing about it and it is never back-facing.
///
/// "Back-facing" is Chrome's `gfx::Transform::IsBackFaceVisible`: the z of the
/// plane's normal `(0, 0, 1)` carried through the inverse transpose is
/// negative. Measured in Chrome 153, it reads the element's **own** transform —
/// under a turned parent a child with none of its own still faces the viewer —
/// and a mirror (`scaleX(-1)`) is not a turn.
///
/// A [singular](Mat4::singular) 4×4 composes to the zero matrix, the path
/// `scale(0)` and a plane behind the viewer take: Chrome 153 neither draws nor
/// hits such an element, even where its flattening is invertible — `scaleZ(0)`
/// flattens to the identity (#1051). Only here, not in [`compose`]: that one
/// also feeds interpolation, whose ends must keep their flattened values.
pub fn compose_about_origin_z(ops: &[TransformOp], origin_z: f64) -> (Affine, bool) {
    if ops.iter().all(TransformOp::is_planar) {
        return (compose(ops), false);
    }
    let mut m = ops
        .iter()
        .fold(Mat4::IDENTITY, |acc, op| acc.then(&op.to_mat4()));
    // `T(0, 0, z) · M · T(0, 0, -z)`, less its left factor: that one moves
    // only the z the flattening drops, and adds a multiple of the `w` row to
    // the `z` row, which changes neither determinant `back_facing` reads.
    if origin_z != 0.0 {
        let back = TransformOp::Translate {
            px: [0.0, 0.0],
            pct: [0.0, 0.0],
            z: -origin_z,
        };
        m = m.then(&back.to_mat4());
    }
    if m.singular() {
        return (Affine::from_matrix([0.0; 6]), false);
    }
    (m.flatten(), m.back_facing())
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
