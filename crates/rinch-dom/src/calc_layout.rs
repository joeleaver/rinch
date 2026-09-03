//! Resolving mixed `calc()` values for Taffy-consumed properties (#278).
//!
//! A `Calc { px, pct }` (see `computed_style/from_stylo/calc.rs`) has no Taffy
//! representation: Taffy 0.12's calc pointer (`CompactLength::calc`) is only
//! resolvable by callers implementing the layout-tree traits themselves —
//! `TaffyTree`'s `LayoutPartialTree::resolve_calc_value` is hardcoded to
//! `0.0` (taffy-0.12.2, `src/tree/taffy_tree.rs:391`), with no hook. So rinch
//! resolves these itself with the two-pass shape
//! `ifc::resolve_percentage_inline_blocks` (#120) already uses:
//!
//! 1. `to_taffy()` seeds a `Calc` field with its length part.
//! 2. After each Taffy compute, [`RinchDocument::resolve_layout_calcs`] walks
//!    the nodes carrying `Calc` values, resolves each against the basis Taffy
//!    would hand a plain percentage — the Taffy parent's content box for
//!    sizes/margins/paddings/insets, the node's own content box for gaps, the
//!    parent's main axis for flex-basis — and writes the resolved *length*
//!    into the Taffy style.
//! 3. `resolve_layout` re-runs the compute until nothing changes (a calc that
//!    feeds another calc's basis converges one level per pass), so no layout
//!    result is ever read from a seed value.
//!
//! On a steady frame the pass is one cheap scan: the resolved values match
//! what is already baked, nothing is set, no extra compute runs.
//!
//! Deliberate parity with the plain-percentage limitations (#386): the basis
//! is always the *Taffy* parent, so a `calc()` size on a `position: fixed` /
//! ICB-absolute box resolves against its Taffy parent exactly the way a plain
//! percentage there does. The out-of-flow *position* patch, by contrast, goes
//! through `LengthPercentageAutoValue::resolve`, which resolves a `Calc`
//! against the real containing block exactly.

use crate::RinchDocument;
use crate::computed_style::{
    ComputedStyle, DimensionValue, LengthPercentageAutoValue, LengthPercentageValue,
};

impl ComputedStyle {
    /// Whether any Taffy-consumed field of this style carries a mixed calc
    /// that [`RinchDocument::resolve_layout_calcs`] must resolve.
    /// Paint-resolved fields (border-radius, transform-origin) are absent on
    /// purpose — their consumers resolve `Calc` at paint time.
    pub(crate) fn has_layout_calc(&self) -> bool {
        use DimensionValue as D;
        use LengthPercentageAutoValue as A;
        use LengthPercentageValue as L;
        let d = |v: &D| matches!(v, D::Calc { .. });
        let l = |v: &L| matches!(v, L::Calc { .. });
        let a = |v: &A| matches!(v, A::Calc { .. });
        d(&self.width)
            || d(&self.height)
            || d(&self.min_width)
            || d(&self.min_height)
            || d(&self.max_width)
            || d(&self.max_height)
            || d(&self.flex_basis)
            || l(&self.padding_top)
            || l(&self.padding_right)
            || l(&self.padding_bottom)
            || l(&self.padding_left)
            || l(&self.gap_row)
            || l(&self.gap_column)
            || a(&self.margin_top)
            || a(&self.margin_right)
            || a(&self.margin_bottom)
            || a(&self.margin_left)
            || a(&self.top)
            || a(&self.right)
            || a(&self.bottom)
            || a(&self.left)
    }
}

/// Overwrite `target` with the resolved length when `v` is a `Calc` and the
/// basis is usable. Non-negative: padding and gap cannot be negative, exactly
/// as stylo's own clamped `resolve()` answers.
fn patch_lp(target: &mut taffy::LengthPercentage, v: LengthPercentageValue, basis: f32) {
    if let LengthPercentageValue::Calc { px, pct } = v
        && basis.is_finite()
    {
        *target = taffy::LengthPercentage::length((px + pct * basis).max(0.0));
    }
}

/// Same for margin/inset values — no floor, negatives are legal.
fn patch_lpa(target: &mut taffy::LengthPercentageAuto, v: LengthPercentageAutoValue, basis: f32) {
    if let LengthPercentageAutoValue::Calc { px, pct } = v
        && basis.is_finite()
    {
        *target = taffy::LengthPercentageAuto::length(px + pct * basis);
    }
}

/// Same for sizes — floored at zero, a box cannot be negative-sized.
fn patch_dim(target: &mut taffy::Dimension, v: DimensionValue, basis: f32) {
    if basis.is_finite()
        && let Some(resolved) = v.resolve_calc(basis)
    {
        *target = taffy::Dimension::length(resolved);
    }
}

impl RinchDocument {
    /// Resolve every `Calc` layout value against the sizes the last Taffy
    /// compute produced, writing resolved lengths into the Taffy styles.
    /// Returns whether anything changed — the caller re-runs the compute
    /// until this answers `false` (see `resolve_layout`).
    pub(crate) fn resolve_layout_calcs(&mut self) -> bool {
        let mut targets: Vec<(usize, taffy::NodeId)> = Vec::new();
        for (id, node) in &self.tree.nodes {
            if let Some(taffy_id) = node.taffy_id
                && node.computed_style.has_layout_calc()
            {
                targets.push((id, taffy_id));
            }
        }
        if targets.is_empty() {
            return false;
        }

        let viewport = self.tree.viewport;
        // Content-box size of a laid-out Taffy node, or the viewport for a
        // node with no Taffy parent — the same fallback the rest of the
        // engine uses for the root's containing block.
        let inner_size = |taffy: &taffy::TaffyTree<crate::node::NodeContext>,
                          id: Option<taffy::NodeId>|
         -> (f32, f32) {
            let Some(id) = id else {
                return (viewport.width, viewport.height);
            };
            match taffy.layout(id) {
                Ok(l) => (
                    l.size.width
                        - l.padding.left
                        - l.padding.right
                        - l.border.left
                        - l.border.right,
                    l.size.height
                        - l.padding.top
                        - l.padding.bottom
                        - l.border.top
                        - l.border.bottom,
                ),
                Err(_) => (viewport.width, viewport.height),
            }
        };

        let mut changed = false;
        for (id, taffy_id) in targets {
            let parent = self.tree.taffy.parent(taffy_id);
            let (pw, ph) = inner_size(&self.tree.taffy, parent);
            let (ow, oh) = inner_size(&self.tree.taffy, Some(taffy_id));
            // flex-basis resolves against the flex container's inner main size.
            let parent_main = match parent
                .and_then(|p| self.tree.taffy.style(p).ok())
                .map(|s| s.flex_direction)
            {
                Some(taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse) => ph,
                _ => pw,
            };

            let before = match self.tree.taffy.style(taffy_id) {
                Ok(s) => s.clone(),
                Err(_) => continue,
            };
            let mut ts = before.clone();
            let cs = &self.tree.nodes[id].computed_style;

            patch_dim(&mut ts.size.width, cs.width, pw);
            patch_dim(&mut ts.size.height, cs.height, ph);
            patch_dim(&mut ts.min_size.width, cs.min_width, pw);
            patch_dim(&mut ts.min_size.height, cs.min_height, ph);
            patch_dim(&mut ts.max_size.width, cs.max_width, pw);
            patch_dim(&mut ts.max_size.height, cs.max_height, ph);
            patch_dim(&mut ts.flex_basis, cs.flex_basis, parent_main);
            // Padding and margin percentages resolve against the containing
            // block's *inline* size on every side, insets against their own
            // axis — mirroring how Taffy resolves the plain-percent forms.
            patch_lp(&mut ts.padding.top, cs.padding_top, pw);
            patch_lp(&mut ts.padding.right, cs.padding_right, pw);
            patch_lp(&mut ts.padding.bottom, cs.padding_bottom, pw);
            patch_lp(&mut ts.padding.left, cs.padding_left, pw);
            patch_lpa(&mut ts.margin.top, cs.margin_top, pw);
            patch_lpa(&mut ts.margin.right, cs.margin_right, pw);
            patch_lpa(&mut ts.margin.bottom, cs.margin_bottom, pw);
            patch_lpa(&mut ts.margin.left, cs.margin_left, pw);
            patch_lpa(&mut ts.inset.top, cs.top, ph);
            patch_lpa(&mut ts.inset.bottom, cs.bottom, ph);
            patch_lpa(&mut ts.inset.left, cs.left, pw);
            patch_lpa(&mut ts.inset.right, cs.right, pw);
            // Gap resolves against the container's own content box, per axis.
            patch_lp(&mut ts.gap.width, cs.gap_column, ow);
            patch_lp(&mut ts.gap.height, cs.gap_row, oh);

            if before != ts {
                let _ = self.tree.taffy.set_style(taffy_id, ts);
                changed = true;
            }
        }
        changed
    }
}
