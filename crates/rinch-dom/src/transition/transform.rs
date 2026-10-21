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
//!    pair. When either matrix is not a 2D one (a 3D function in a
//!    remainder, a `matrix3d()` that leaves the plane), the 4×4s are
//!    decomposed in 3D before anything is flattened — css-transforms-2's
//!    translation, scale, three shears, perspective and a quaternion slerped
//!    the shorter way, a 2D end still decomposed in 2D — and the result is
//!    kept as a `matrix3d()`, z and all (#989, see `interpolate_mat4`).
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
    /// `matrix3d()`, and what a 3D decomposed interpolation leaves behind
    /// (#989), as a row-major 4×4 (`m[row][col]`, column vectors — so CSS's
    /// `m41, m42` are `m[0][3], m[1][3]`).
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
                let (s, c) = sin_cos_snapped(*angle);
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
            // Decomposed as Chrome does: in 2D when both are 2D matrices, in
            // 3D otherwise (#989). A `matrix3d()` has no box-relative part.
            (TransformOp::Matrix3D(a), TransformOp::Matrix3D(b)) => {
                let (ma, mb) = (Mat4::from_m(a), Mat4::from_m(b));
                match interpolate_mat4(&ma, &mb, t) {
                    Some(r) => TransformOp::Matrix3D(r.whole().m),
                    None => {
                        if t < 0.5 {
                            self.clone()
                        } else {
                            to.clone()
                        }
                    }
                }
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
        compose_mat4(ops).flatten()
    }
}

/// A function list composed as a 4×4, in list order, not flattened.
fn compose_mat4(ops: &[TransformOp]) -> Mat4 {
    ops.iter()
        .fold(Mat4::IDENTITY, |acc, op| acc.then(&op.to_mat4()))
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
        let (ra, rb) = (&a[prefix.min(a.len())..], &b[prefix.min(b.len())..]);
        if ra.iter().chain(rb).all(TransformOp::is_planar) {
            out.push(TransformOp::Matrix(interpolate_affine(
                &compose(ra),
                &compose(rb),
                t,
            )));
        } else {
            let (ma, mb) = (compose_mat4(ra), compose_mat4(rb));
            match interpolate_mat4(&ma, &mb, t) {
                Some(r) => r.push_ops(&mut out),
                // No decomposition: the pair flips half way, as in 2D.
                None => out.extend(if t < 0.5 { ra } else { rb }.iter().cloned()),
            }
        }
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
/// transition from `none` to `rotate(0deg)`. It moves nothing, so rinch runs
/// no frames for it. Since #415 `rotate(0deg)` at rest is a stacking context
/// just as a running transition is, so `none` → `rotate(0deg)` ends where
/// Chrome's does at once; the reverse, `rotate(0deg)` → `none`, stops being a
/// stacking context at once where Chrome's stays one until its run ends.
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
    } else if a.iter().chain(b).all(TransformOp::is_planar) {
        let (ca, cb) = (compose(a), compose(b));
        ca.matrix.iter().zip(&cb.matrix).all(|(p, q)| close(p, q))
            && ca.pct_w.iter().zip(&cb.pct_w).all(|(p, q)| close(p, q))
            && ca.pct_h.iter().zip(&cb.pct_h).all(|(p, q)| close(p, q))
    } else {
        // The 4×4s, not their flattenings: `scaleY(0.5)` and `rotateX(60deg)`
        // flatten alike, and their interpolation leaves the plane (#989).
        let (ca, cb) = (compose_mat4(a), compose_mat4(b));
        let rows = |m: &[[f64; 4]], n: &[[f64; 4]]| {
            m.iter()
                .flatten()
                .zip(n.iter().flatten())
                .all(|(p, q)| close(p, q))
        };
        rows(&ca.m, &cb.m) && rows(&ca.pct_w, &cb.pct_w) && rows(&ca.pct_h, &cb.pct_h)
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

/// A 4×4 decomposed as css-transforms-2 §"Decomposing a 3D matrix" does it,
/// in Chromium's form (`gfx::DecomposedTransform`): the matrix is
/// `P · T · R · K · S` — perspective, translation, the rotation `quaternion`,
/// the shears `K = [[1, xy, xz], [0, 1, yz], [0, 0, 1]]` and the scale.
#[derive(Debug, Clone, Copy)]
struct Decomposed3 {
    translate: [f64; 3],
    scale: [f64; 3],
    /// `[xy, xz, yz]`.
    skew: [f64; 3],
    /// The `w` row.
    perspective: [f64; 4],
    /// `[x, y, z, w]`.
    quaternion: [f64; 4],
}

impl Mat4 {
    /// Chromium's `Is2dTransform`: the matrix maps the plane onto itself with
    /// no perspective, so it is a 2D affine lifted ([`Mat4::lift`]).
    fn is_2d(&self) -> bool {
        let m = &self.m;
        m[2][0] == 0.0
            && m[2][1] == 0.0
            && m[0][2] == 0.0
            && m[1][2] == 0.0
            && m[2][2] == 1.0
            && m[2][3] == 0.0
            && m[3] == [0.0, 0.0, 0.0, 1.0]
    }

    /// Decompose as Chrome does: a [2D](Self::is_2d) matrix by the 2D
    /// decomposition ([`decompose`], so a reflection is one negative scale),
    /// anything else by the 3D one. `None` where there is none: a singular
    /// linear part, or an `m44` of 0.
    fn decompose(&self) -> Option<Decomposed3> {
        if self.is_2d() {
            let a = self.flatten().matrix;
            let d = decompose(&a)?;
            let half = d.angle / 2.0;
            return Some(Decomposed3 {
                translate: [a[4], a[5], 0.0],
                scale: [d.scale[0], d.scale[1], 1.0],
                skew: [d.skew, 0.0, 0.0],
                perspective: [0.0, 0.0, 0.0, 1.0],
                quaternion: [0.0, 0.0, half.sin(), half.cos()],
            });
        }
        let w = self.m[3][3];
        if w == 0.0 || !w.is_finite() {
            return None;
        }
        let m = self.m.map(|row| row.map(|x| x / w));
        // The linear part, whose inverse also solves for the perspective.
        let lin = [
            [m[0][0], m[0][1], m[0][2]],
            [m[1][0], m[1][1], m[1][2]],
            [m[2][0], m[2][1], m[2][2]],
        ];
        let inv = invert3(&lin)?;
        // `m = P · N`, where `N` is `m` with its `w` row `(0, 0, 0, 1)` and
        // `P` the identity with `m`'s `w` row: so `p = r · N⁻¹`.
        let r = m[3];
        let perspective = if r[0] != 0.0 || r[1] != 0.0 || r[2] != 0.0 {
            let t = [m[0][3], m[1][3], m[2][3]];
            let mut p = [0.0; 4];
            for (j, pj) in p.iter_mut().take(3).enumerate() {
                *pj = (0..3).map(|i| r[i] * inv[i][j]).sum();
            }
            // Column 3 of `N⁻¹` is `(-A⁻¹ t, 1)`.
            p[3] = r[3]
                - (0..3)
                    .map(|i| r[i] * (0..3).map(|k| inv[i][k] * t[k]).sum::<f64>())
                    .sum::<f64>();
            p
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let translate = [m[0][3], m[1][3], m[2][3]];
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let len = |a: [f64; 3]| dot(a, a).sqrt();
        let comb =
            |a: [f64; 3], b: [f64; 3], k: f64| [a[0] - k * b[0], a[1] - k * b[1], a[2] - k * b[2]];
        let div = |a: [f64; 3], k: f64| if k != 0.0 { a.map(|x| x / k) } else { a };
        let mut col = [0, 1, 2].map(|c| [m[0][c], m[1][c], m[2][c]]);
        let mut scale = [0.0; 3];
        let mut skew = [0.0; 3];
        scale[0] = len(col[0]);
        col[0] = div(col[0], scale[0]);
        skew[0] = dot(col[0], col[1]);
        col[1] = comb(col[1], col[0], skew[0]);
        scale[1] = len(col[1]);
        col[1] = div(col[1], scale[1]);
        skew[0] = div([skew[0], 0.0, 0.0], scale[1])[0];
        skew[1] = dot(col[0], col[2]);
        col[2] = comb(col[2], col[0], skew[1]);
        skew[2] = dot(col[1], col[2]);
        col[2] = comb(col[2], col[1], skew[2]);
        scale[2] = len(col[2]);
        col[2] = div(col[2], scale[2]);
        if scale[2] != 0.0 {
            skew[1] /= scale[2];
            skew[2] /= scale[2];
        }
        // A reflection: the 3D decomposition negates every scale and column.
        let c12 = [
            col[1][1] * col[2][2] - col[1][2] * col[2][1],
            col[1][2] * col[2][0] - col[1][0] * col[2][2],
            col[1][0] * col[2][1] - col[1][1] * col[2][0],
        ];
        if dot(col[0], c12) < 0.0 {
            for i in 0..3 {
                scale[i] = -scale[i];
                col[i] = col[i].map(|x| -x);
            }
        }
        // The rotation matrix's entries, `q[row][col]`, to a quaternion by
        // the branch Chromium takes (it avoids the spec's degenerate case).
        let q = |r: usize, c: usize| col[c][r];
        let trace = q(0, 0) + q(1, 1) + q(2, 2);
        let quaternion = if trace > 0.0 {
            let r = (1.0 + trace).sqrt();
            let h = 0.5 / r;
            [
                (q(2, 1) - q(1, 2)) * h,
                (q(0, 2) - q(2, 0)) * h,
                (q(1, 0) - q(0, 1)) * h,
                0.5 * r,
            ]
        } else if q(0, 0) > q(1, 1) && q(0, 0) > q(2, 2) {
            let r = (1.0 + q(0, 0) - q(1, 1) - q(2, 2)).sqrt();
            let h = 0.5 / r;
            [
                0.5 * r,
                (q(0, 1) + q(1, 0)) * h,
                (q(0, 2) + q(2, 0)) * h,
                (q(2, 1) - q(1, 2)) * h,
            ]
        } else if q(1, 1) > q(2, 2) {
            let r = (1.0 - q(0, 0) + q(1, 1) - q(2, 2)).sqrt();
            let h = 0.5 / r;
            [
                (q(0, 1) + q(1, 0)) * h,
                0.5 * r,
                (q(1, 2) + q(2, 1)) * h,
                (q(0, 2) - q(2, 0)) * h,
            ]
        } else {
            let r = (1.0 - q(0, 0) - q(1, 1) + q(2, 2)).sqrt();
            let h = 0.5 / r;
            [
                (q(0, 2) + q(2, 0)) * h,
                (q(1, 2) + q(2, 1)) * h,
                0.5 * r,
                (q(1, 0) - q(0, 1)) * h,
            ]
        };
        Some(Decomposed3 {
            translate,
            scale,
            skew,
            perspective,
            quaternion,
        })
    }

    /// The matrix [`Mat4::decompose`] took apart, put back together: the
    /// perspective, then the rest.
    fn recompose(d: &Decomposed3) -> (Mat4, Mat4) {
        let mut p = Mat4::IDENTITY;
        p.m[3] = d.perspective;
        let [x, y, z, w] = d.quaternion;
        let mut rest = Mat4::IDENTITY;
        rest.m[0][3] = d.translate[0];
        rest.m[1][3] = d.translate[1];
        rest.m[2][3] = d.translate[2];
        let mut r = Mat4::IDENTITY;
        r.m[0][0] = 1.0 - 2.0 * (y * y + z * z);
        r.m[1][0] = 2.0 * (x * y + z * w);
        r.m[2][0] = 2.0 * (x * z - y * w);
        r.m[0][1] = 2.0 * (x * y - z * w);
        r.m[1][1] = 1.0 - 2.0 * (x * x + z * z);
        r.m[2][1] = 2.0 * (y * z + x * w);
        r.m[0][2] = 2.0 * (x * z + y * w);
        r.m[1][2] = 2.0 * (y * z - x * w);
        r.m[2][2] = 1.0 - 2.0 * (x * x + y * y);
        let mut k = Mat4::IDENTITY;
        k.m[0][1] = d.skew[0];
        k.m[0][2] = d.skew[1];
        k.m[1][2] = d.skew[2];
        let mut s = Mat4::IDENTITY;
        s.m[0][0] = d.scale[0];
        s.m[1][1] = d.scale[1];
        s.m[2][2] = d.scale[2];
        (p, rest.then(&r).then(&k).then(&s))
    }
}

/// `sin_cos`, exact at a multiple of a quarter turn, as Chromium's
/// `SinCosDegrees` is: `rotateX(360deg)` is then exactly the identity, so a
/// list holding it can still be a 2D matrix ([`Mat4::is_2d`]).
fn sin_cos_snapped(rad: f64) -> (f64, f64) {
    let q = rad / std::f64::consts::FRAC_PI_2;
    let n = q.round();
    if (q - n).abs() > 1e-6 * n.abs().max(1.0) {
        return rad.sin_cos();
    }
    match (n as i64).rem_euclid(4) {
        0 => (0.0, 1.0),
        1 => (1.0, 0.0),
        2 => (0.0, -1.0),
        _ => (-1.0, 0.0),
    }
}

/// The inverse of a 3×3, or `None` for a singular one.
fn invert3(a: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let cof =
        |r0: usize, r1: usize, c0: usize, c1: usize| a[r0][c0] * a[r1][c1] - a[r0][c1] * a[r1][c0];
    let det = a[0][0] * cof(1, 2, 1, 2) - a[0][1] * cof(1, 2, 0, 2) + a[0][2] * cof(1, 2, 0, 1);
    if det == 0.0 || !det.is_finite() {
        return None;
    }
    let adj = [
        [cof(1, 2, 1, 2), -cof(0, 2, 1, 2), cof(0, 1, 1, 2)],
        [-cof(1, 2, 0, 2), cof(0, 2, 0, 2), -cof(0, 1, 0, 2)],
        [cof(1, 2, 0, 1), -cof(0, 2, 0, 1), cof(0, 1, 0, 1)],
    ];
    Some(adj.map(|row| row.map(|x| x / det)))
}

/// Chromium's `Quaternion::Slerp`, the shorter arc.
fn slerp(from: [f64; 4], to: [f64; 4], t: f64) -> [f64; 4] {
    let mut from = from;
    let mut cos = from.iter().zip(&to).map(|(a, b)| a * b).sum::<f64>();
    if cos < 0.0 {
        from = from.map(|x| -x);
        cos = -cos;
    }
    let cos = cos.min(1.0);
    let sin = (1.0 - cos * cos).sqrt();
    if sin < 1e-5 {
        return from;
    }
    let half = cos.acos();
    let (ka, kb) = (((1.0 - t) * half).sin() / sin, (t * half).sin() / sin);
    [0, 1, 2, 3].map(|i| ka * from[i] + kb * to[i])
}

/// An interpolated 4×4: the perspective, the box-relative translation
/// (`[x, y]` per unit of the box's width and height) and the rest, in that
/// order, as [`Decomposed3`] composes them.
struct Interpolated4 {
    perspective: Mat4,
    pct_w: [f64; 2],
    pct_h: [f64; 2],
    rest: Mat4,
}

impl Interpolated4 {
    fn has_pct(&self) -> bool {
        self.pct_w.iter().chain(&self.pct_h).any(|x| *x != 0.0)
    }

    /// The whole matrix, the box-relative part left out.
    fn whole(&self) -> Mat4 {
        self.perspective.then(&self.rest)
    }

    /// The fewest functions that compose to this: a `matrix3d()`, with the
    /// box-relative translation as a `matrix()` in front of the rest (and the
    /// perspective, when there is one, in front of that).
    fn push_ops(&self, out: &mut Vec<TransformOp>) {
        // Two 2D ends interpolate to a 2D matrix: keep the list planar.
        if self.perspective.m == Mat4::IDENTITY.m && self.rest.is_2d() {
            let f = self.rest.flatten();
            out.push(TransformOp::Matrix(Affine {
                matrix: f.matrix,
                pct_w: self.pct_w,
                pct_h: self.pct_h,
            }));
            return;
        }
        if !self.has_pct() {
            out.push(TransformOp::Matrix3D(self.whole().m));
            return;
        }
        if self.perspective.m != Mat4::IDENTITY.m {
            out.push(TransformOp::Matrix3D(self.perspective.m));
        }
        out.push(TransformOp::Matrix(Affine {
            matrix: Affine::IDENTITY.matrix,
            pct_w: self.pct_w,
            pct_h: self.pct_h,
        }));
        out.push(TransformOp::Matrix3D(self.rest.m));
    }
}

/// Interpolate two 4×4s by decomposition, as Chrome's `gfx::Transform::Blend`
/// does (#989): each end is decomposed on its own ([`Mat4::decompose`] — in
/// 2D for a 2D matrix), translation, scale, shears and perspective are lerped
/// and the quaternion slerped the shorter way. Two 2D ends are left to
/// [`interpolate_affine`], whose result is the same. The box-relative
/// translation is lerped beside the translation; what of it is not a
/// translation (a perspective's `w` row met it) is dropped, as
/// [`Mat4::flatten`] drops it. `None` when either end has no decomposition.
fn interpolate_mat4(a: &Mat4, b: &Mat4, t: f64) -> Option<Interpolated4> {
    let mix = |x: f64, y: f64| x + (y - x) * t;
    if a.is_2d() && b.is_2d() {
        let (fa, fb) = (a.flatten(), b.flatten());
        if decompose(&fa.matrix).is_none() || decompose(&fb.matrix).is_none() {
            return None;
        }
        let r = interpolate_affine(&fa, &fb, t);
        return Some(Interpolated4 {
            perspective: Mat4::IDENTITY,
            pct_w: r.pct_w,
            pct_h: r.pct_h,
            rest: Mat4::lift(&Affine::from_matrix(r.matrix)),
        });
    }
    let (da, db) = (a.decompose()?, b.decompose()?);
    let lerp = |x: &[f64], y: &[f64], out: &mut [f64]| {
        for i in 0..out.len() {
            out[i] = mix(x[i], y[i]);
        }
    };
    let mut d = da;
    lerp(&da.translate, &db.translate, &mut d.translate);
    lerp(&da.scale, &db.scale, &mut d.scale);
    lerp(&da.skew, &db.skew, &mut d.skew);
    lerp(&da.perspective, &db.perspective, &mut d.perspective);
    d.quaternion = slerp(da.quaternion, db.quaternion, t);
    let (perspective, rest) = Mat4::recompose(&d);
    let pct = |m: &Mat4, rows: &BoxRows| [rows[0][3] / m.m[3][3], rows[1][3] / m.m[3][3]];
    let (aw, ah, bw, bh) = (
        pct(a, &a.pct_w),
        pct(a, &a.pct_h),
        pct(b, &b.pct_w),
        pct(b, &b.pct_h),
    );
    Some(Interpolated4 {
        perspective,
        pct_w: [mix(aw[0], bw[0]), mix(aw[1], bw[1])],
        pct_h: [mix(ah[0], bh[0]), mix(ah[1], bh[1])],
        rest,
    })
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
