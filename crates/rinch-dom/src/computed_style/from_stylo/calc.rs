//! Decomposing a stylo `LengthPercentage` into a `(px, pct)` pair (#278/#404).
//!
//! stylo's `to_length()` and `to_percentage()` both answer `None` for
//! `Unpacked::Calc`, so a mixed `calc(50% - 10px)` used to fall into every
//! converter's else-arm and silently degrade — to `0` (insets, margins), to
//! `Zero` (padding, gap, radii), to `50%` (transform-origin), or to no
//! translation at all (#404). This module recovers the affine components
//! exactly instead.
//!
//! ## How
//!
//! `CalcLengthPercentage`'s node tree is private, but its `resolve(basis)` is
//! public, and for a plain (sum-of-terms) calc it is **affine** in the basis:
//! `f(basis) = px + pct·basis`. Two evaluations recover the pair exactly.
//!
//! One trap: `resolve()` also applies the property's `AllowedNumericType`
//! clamp (stylo `length_percentage.rs`, `CalcLengthPercentage::resolve`), so
//! for a non-negative property like padding, `calc(50% - 10px)` answers `0`
//! at basis 0 *and* 1 — probing only there reproduces the very bug this
//! fixes. So the probe is staged:
//!
//! 1. Fit the affine through `f(0)`, `f(1)`; accept it if it predicts
//!    `f(2^20)` and `f(2^21)` — exact for any unclamped affine calc.
//! 2. Accept the same fit if it predicts the large probes *after* flooring at
//!    zero — exact for a clamped affine whose clamp is inactive near 0
//!    (`padding: calc(20px - 10%)`).
//! 3. Otherwise take the tangent through the two large probes — exact for a
//!    clamped affine whose clamp is active near 0 (`padding: calc(50% -
//!    10px)`: invisible at small bases, plain affine at large ones). The
//!    large bases are powers of two so `pct·basis` is exact in f32.
//!
//! ## Named limitation: non-affine calc
//!
//! A calc that is genuinely not affine — `min()` / `max()` / `clamp()` /
//! `round()` with a percentage inside — has no `(px, pct)` representation at
//! all. Step 3 stores its **large-basis linearization**: `min(50%, 100px)`
//! becomes `100px`, i.e. the arm that wins in a large container. That is
//! wrong in containers on the other side of the breakpoint (a 150px-wide box
//! would get 100 instead of 75) and is accepted deliberately: the exact value
//! is not representable without carrying the calc tree, and the previous
//! answer for these was a silent `(0, 0)`. Exact non-affine support would
//! need either a stylo calc tree carried to resolution time or Taffy's calc
//! pointer, which `TaffyTree` resolves to `0.0` unconditionally (taffy-0.12.2,
//! `src/tree/taffy_tree.rs:391`).

use style::values::computed::{Length, LengthPercentage};

/// 2^20 — large enough that a non-negative clamp on a rising affine is
/// inactive, a power of two so `pct * K` is exact in f32.
const K: f32 = 1_048_576.0;

/// Absolute-plus-relative tolerance for "this probe matches the model".
/// resolve() sums f32 leaf terms, so a few ulps of drift at 2^21 (~0.25 each)
/// are expected; a genuine min/max arm switch diverges by whole pixels.
fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= 0.5 + 1e-5 * a.abs().max(b.abs())
}

/// Split a computed `LengthPercentage` into `(px, pct)` with
/// `value = px + pct * basis` (`pct` as a fraction: 0.5 = 50%).
///
/// Exact for plain lengths, plain percentages, and any affine `calc()` —
/// clamped by its property's non-negative range or not. A non-affine calc
/// gets its large-basis linearization (see the module doc). The pair is the
/// *unclamped* affine: a non-negative consumer floors its resolution at zero
/// itself.
pub(crate) fn split_length_percentage(lp: &LengthPercentage) -> (f32, f32) {
    if let Some(len) = lp.to_length() {
        return (len.px(), 0.0);
    }
    if let Some(pct) = lp.to_percentage() {
        return (0.0, pct.0);
    }

    // Unpacked::Calc — probe resolve().
    let f0 = lp.resolve(Length::new(0.0)).px();
    let f1 = lp.resolve(Length::new(1.0)).px();
    let fk = lp.resolve(Length::new(K)).px();
    let f2k = lp.resolve(Length::new(2.0 * K)).px();
    if !(f0.is_finite() && f1.is_finite() && fk.is_finite() && f2k.is_finite()) {
        return (if f0.is_finite() { f0 } else { 0.0 }, 0.0);
    }

    // Step 1: the affine through the two small probes, exact when no clamp
    // interferes anywhere.
    let s_px = f0;
    let s_pct = f1 - f0;
    if approx(s_px + s_pct * K, fk) && approx(s_px + s_pct * 2.0 * K, f2k) {
        return (s_px, s_pct);
    }

    // Step 2: same fit, clamped-at-zero model — a falling affine under a
    // non-negative clamp is unclamped near 0 and floored at the large probes.
    if approx((s_px + s_pct * K).max(0.0), fk) && approx((s_px + s_pct * 2.0 * K).max(0.0), f2k) {
        return (s_px, s_pct);
    }

    // Step 3: the large-basis tangent — exact for a rising affine whose
    // non-negative clamp swallowed the small probes; the documented
    // linearization for a genuinely non-affine calc.
    let pct = (f2k - fk) / K;
    let px = fk - pct * K;
    if px.is_finite() && pct.is_finite() {
        (px, pct)
    } else {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use style::OwnedSlice;
    use style::values::computed::Percentage;
    use style::values::computed::length_percentage::{CalcLengthPercentageLeaf, CalcNode};
    use style_traits::values::specified::AllowedNumericType;

    /// A computed `calc(<pct> + <px>)` with the given clamping mode, built the
    /// way stylo builds one — so these tests pin the pinned stylo's actual
    /// `resolve()` behavior, clamping included, not a model of it.
    fn mixed_calc(px: f32, pct: f32, clamp: AllowedNumericType) -> LengthPercentage {
        let node = CalcNode::Sum(OwnedSlice::from(vec![
            CalcNode::Leaf(CalcLengthPercentageLeaf::Percentage(Percentage(pct))),
            CalcNode::Leaf(CalcLengthPercentageLeaf::Length(Length::new(px))),
        ]));
        LengthPercentage::new_calc(node, clamp)
    }

    fn assert_split(lp: &LengthPercentage, px: f32, pct: f32) {
        let (got_px, got_pct) = split_length_percentage(lp);
        assert!(
            (got_px - px).abs() < 1e-3,
            "px: expected {px}, got {got_px}"
        );
        assert!(
            (got_pct - pct).abs() < 1e-5,
            "pct: expected {pct}, got {got_pct}"
        );
    }

    #[test]
    fn plain_length_passes_through() {
        assert_split(&LengthPercentage::new_length(Length::new(12.5)), 12.5, 0.0);
    }

    #[test]
    fn plain_percentage_passes_through() {
        assert_split(&LengthPercentage::new_percent(Percentage(0.25)), 0.0, 0.25);
    }

    /// `left: calc(50% - 10px)` — unclamped, both small probes visible.
    #[test]
    fn affine_unclamped_is_exact() {
        assert_split(&mixed_calc(-10.0, 0.5, AllowedNumericType::All), -10.0, 0.5);
    }

    /// `padding: calc(50% - 10px)` — the #278-family trap: NonNegative
    /// clamping makes resolve(0) and resolve(1) both answer 0, so the naive
    /// two-point trick returns (0, 0), the exact bug being fixed. The staged
    /// probe recovers the affine from the large bases.
    #[test]
    fn affine_nonnegative_rising_recovers_from_clamp() {
        assert_split(
            &mixed_calc(-10.0, 0.5, AllowedNumericType::NonNegative),
            -10.0,
            0.5,
        );
    }

    /// `padding: calc(20px - 10%)` — the opposite clamp shape: unclamped near
    /// 0, floored at the large probes. The small-basis fit must win here; the
    /// large-basis tangent alone would answer (0, 0).
    #[test]
    fn affine_nonnegative_falling_keeps_small_fit() {
        assert_split(
            &mixed_calc(20.0, -0.1, AllowedNumericType::NonNegative),
            20.0,
            -0.1,
        );
    }

    /// A NonNegative calc negative everywhere resolves to 0 at every basis,
    /// and (0, 0) is its correct representation.
    #[test]
    fn affine_nonnegative_always_negative_is_zero() {
        assert_split(
            &mixed_calc(-10.0, -0.1, AllowedNumericType::NonNegative),
            0.0,
            0.0,
        );
    }

    /// Positive px and pct under NonNegative: never clamped, exact via the
    /// small probes.
    #[test]
    fn affine_nonnegative_inactive_clamp_is_exact() {
        assert_split(
            &mixed_calc(10.0, 0.5, AllowedNumericType::NonNegative),
            10.0,
            0.5,
        );
    }

    /// `min(50%, 100px)`: genuinely non-affine — the documented behavior is
    /// the large-basis linearization, i.e. the 100px arm.
    #[test]
    fn non_affine_min_takes_large_basis_linearization() {
        let node = CalcNode::MinMax(
            OwnedSlice::from(vec![
                CalcNode::Leaf(CalcLengthPercentageLeaf::Percentage(Percentage(0.5))),
                CalcNode::Leaf(CalcLengthPercentageLeaf::Length(Length::new(100.0))),
            ]),
            style::values::generics::calc::MinMaxOp::Min,
        );
        let lp = LengthPercentage::new_calc(node, AllowedNumericType::All);
        assert_split(&lp, 100.0, 0.0);
    }

    /// `max(50%, 100px)`: the large-basis arm is the percentage one.
    #[test]
    fn non_affine_max_takes_large_basis_linearization() {
        let node = CalcNode::MinMax(
            OwnedSlice::from(vec![
                CalcNode::Leaf(CalcLengthPercentageLeaf::Percentage(Percentage(0.5))),
                CalcNode::Leaf(CalcLengthPercentageLeaf::Length(Length::new(100.0))),
            ]),
            style::values::generics::calc::MinMaxOp::Max,
        );
        let lp = LengthPercentage::new_calc(node, AllowedNumericType::All);
        assert_split(&lp, 0.0, 0.5);
    }

    /// The split round-trips through resolve at an ordinary container size —
    /// the property the layout fixpoint depends on.
    #[test]
    fn split_agrees_with_resolve_at_ordinary_bases() {
        for &(px, pct) in &[(-10.0f32, 0.5f32), (5.0, 0.25), (30.0, -0.2), (0.0, 1.0)] {
            let lp = mixed_calc(px, pct, AllowedNumericType::All);
            let (got_px, got_pct) = split_length_percentage(&lp);
            for &basis in &[0.0f32, 137.0, 300.0, 1920.0] {
                let direct = lp.resolve(Length::new(basis)).px();
                let via_split = got_px + got_pct * basis;
                assert!(
                    (direct - via_split).abs() < 1e-2,
                    "px={px} pct={pct} basis={basis}: resolve()={direct}, split={via_split}"
                );
            }
        }
    }
}
