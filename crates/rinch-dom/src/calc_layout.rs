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
//!    would hand a plain percentage, and writes the resolved *length* into the
//!    Taffy style.
//! 3. `resolve_layout` re-runs the compute until nothing changes (a calc that
//!    feeds another calc's basis converges one level per pass), so on the
//!    converged path no layout result is ever read from a seed value. A run
//!    that hits the iteration cap (a residual content-feedback corner, e.g.
//!    `min-size: auto` growth against a nominally definite axis) is laid out
//!    from the last iterate — wrong but bounded — and reported once per
//!    process on stderr.
//!
//! ## The basis, definite or not
//!
//! The percentage part resolves only against a **definite** basis — an axis
//! whose size is derivable without measuring content. Resolving against an
//! auto-sized axis would feed the fixpoint its own output: `row-gap:
//! calc(10% + 5px)` in an auto-height column inflates the very height it
//! resolves against and diverges (13× tall before this gate existed). CSS and
//! Taffy resolve a percentage against an indefinite basis as zero/auto
//! (`maybe_resolve(None)`), so a `Calc` there does the same: padding, margin,
//! inset and gap take their length part only; a size behaves as `auto`.
//! [`RinchDocument::calc_axis_definite`] is the (conservative) judge — an
//! axis it cannot prove definite is treated as indefinite, which degrades to
//! the px part rather than diverging.
//!
//! Which box: for an in-flow node, the Taffy parent's **content box** (and a
//! node's own content box for its gaps); for an absolutely positioned node,
//! the parent's **padding box** — Taffy subtracts only the border for an
//! absolute child (`flexbox.rs:2166`, `block.rs:580`), as CSS says.
//!
//! Deliberate parity with the plain-percentage rules elsewhere:
//! - An ICB-absolute box's own `width`/`height` `Calc` is baked from the
//!   viewport by `out_of_flow::apply_out_of_flow_size_overrides` exactly like
//!   a plain `Percent` there (#204), so this pass leaves those two fields
//!   alone for such nodes.
//! - Everything `out_of_flow` leaves parent-resolved for plain percentages
//!   (#386 — min/max sizes, padding/margin on the box) stays parent-resolved
//!   for `Calc` too.
//! - The out-of-flow *position* patch goes through
//!   `LengthPercentageAutoValue::resolve`, which resolves a `Calc` against
//!   the real containing block exactly.

use crate::RinchDocument;
use crate::computed_style::{
    ComputedStyle, DimensionValue, LengthPercentageAutoValue, LengthPercentageValue, PositionValue,
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

/// One axis of a box, for the definiteness walk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// Overwrite `target` with the resolved length when `v` is a `Calc`.
/// `basis` is `None` for an indefinite basis: the percentage part then
/// resolves to zero — the length part alone, floored at zero the way stylo's
/// own clamped `resolve()` floors a non-negative property (padding, gap).
fn patch_lp(target: &mut taffy::LengthPercentage, v: LengthPercentageValue, basis: Option<f32>) {
    if let LengthPercentageValue::Calc { px, pct } = v {
        let resolved = match basis {
            Some(b) => px + pct * b,
            None => px,
        };
        *target = taffy::LengthPercentage::length(resolved.max(0.0));
    }
}

/// Same for margin/inset values — no floor, negatives are legal.
fn patch_lpa(
    target: &mut taffy::LengthPercentageAuto,
    v: LengthPercentageAutoValue,
    basis: Option<f32>,
) {
    if let LengthPercentageAutoValue::Calc { px, pct } = v {
        let resolved = match basis {
            Some(b) => px + pct * b,
            None => px,
        };
        *target = taffy::LengthPercentageAuto::length(resolved);
    }
}

/// Same for sizes — floored at zero, a box cannot be negative-sized. Against
/// an indefinite basis a percentage size behaves as `auto` (CSS 10.5), so the
/// whole `Calc` does too, replacing the seed rather than keeping it.
fn patch_dim(target: &mut taffy::Dimension, v: DimensionValue, basis: Option<f32>) {
    if matches!(v, DimensionValue::Calc { .. }) {
        *target = match basis {
            Some(b) => taffy::Dimension::length(v.resolve_calc(b).unwrap_or(0.0)),
            None => taffy::Dimension::auto(),
        };
    }
}

impl RinchDocument {
    /// The rinch node whose Taffy node is `node_id`'s Taffy parent: the
    /// nearest ancestor that owns a Taffy node (`display: contents` ancestors
    /// generate none and their children are reparented in Taffy).
    fn taffy_parent_rinch(&self, node_id: usize) -> Option<usize> {
        let mut current = self.tree.get(node_id)?.parent?;
        loop {
            let node = self.tree.get(current)?;
            if node.taffy_id.is_some() {
                return Some(current);
            }
            current = node.parent?;
        }
    }

    /// Whether `node_id`'s size on `axis` is *definite* — derivable without
    /// measuring content — for percentage-resolution purposes. Conservative:
    /// an axis this cannot prove definite is treated as indefinite, which
    /// resolves the percentage part as zero/auto instead of risking the
    /// self-feeding divergence the module doc describes. Knowingly simpler
    /// than the flexbox spec's full definiteness rules (a definite-flex-basis
    /// main axis, an inset-derived absolute size and grid axes all answer
    /// "indefinite" here).
    fn calc_axis_definite(&self, node_id: usize, axis: Axis, depth: u32) -> bool {
        if depth > 64 {
            return false;
        }
        let Some(node) = self.tree.get(node_id) else {
            return false;
        };
        let size = match axis {
            Axis::X => node.computed_style.width,
            Axis::Y => node.computed_style.height,
        };
        match size {
            DimensionValue::Length(_) => true,
            // A percentage (or calc) is definite iff what it resolves
            // against is.
            DimensionValue::Percent(_) | DimensionValue::Calc { .. } => {
                match self.taffy_parent_rinch(node_id) {
                    Some(p) => self.calc_axis_definite(p, axis, depth + 1),
                    None => true, // the root resolves against the viewport
                }
            }
            DimensionValue::Auto => {
                // An auto-sized absolute shrinks to fit its content.
                if matches!(
                    node.computed_style.position,
                    PositionValue::Absolute | PositionValue::Fixed
                ) {
                    return false;
                }
                let Some(p) = self.taffy_parent_rinch(node_id) else {
                    return true; // the root is viewport-sized
                };
                // What Taffy does with the auto depends on the parent's
                // layout mode — read the parent's *Taffy* style, which
                // carries rinch's element-default display/direction.
                let Some(p_taffy) = self.tree.get(p).and_then(|n| n.taffy_id) else {
                    return false;
                };
                let Ok(p_style) = self.tree.taffy.style(p_taffy) else {
                    return false;
                };
                let (p_display, p_direction, p_align_items) =
                    (p_style.display, p_style.flex_direction, p_style.align_items);
                match p_display {
                    taffy::Display::Flex => {
                        let main_is_y = matches!(
                            p_direction,
                            taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse
                        );
                        if (axis == Axis::Y) == main_is_y {
                            // Auto main size: content- and flex-sized.
                            false
                        } else {
                            // Auto cross size: definite when stretched to a
                            // definite container axis.
                            let own_align = node
                                .taffy_id
                                .and_then(|t| self.tree.taffy.style(t).ok())
                                .and_then(|s| s.align_self);
                            let stretched = match own_align.or(p_align_items) {
                                None => true, // unset align-items = stretch
                                Some(a) => a.keyword == taffy::AlignItemsKeyword::Stretch,
                            };
                            stretched && self.calc_axis_definite(p, axis, depth + 1)
                        }
                    }
                    taffy::Display::Block => {
                        // Block-level auto width fills the containing block;
                        // auto height is content-sized.
                        axis == Axis::X && self.calc_axis_definite(p, axis, depth + 1)
                    }
                    _ => false,
                }
            }
        }
    }

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
        // Basis box of a laid-out Taffy node, or the viewport for a node with
        // no Taffy parent — the same fallback the rest of the engine uses for
        // the root's containing block. `content_box: false` keeps the padding
        // (the padding-box basis an absolute child resolves against).
        let basis_box = |taffy: &taffy::TaffyTree<crate::node::NodeContext>,
                         id: Option<taffy::NodeId>,
                         content_box: bool|
         -> (f32, f32) {
            let Some(id) = id else {
                return (viewport.width, viewport.height);
            };
            match taffy.layout(id) {
                Ok(l) => {
                    let (mut w, mut h) = (
                        l.size.width - l.border.left - l.border.right,
                        l.size.height - l.border.top - l.border.bottom,
                    );
                    if content_box {
                        w -= l.padding.left + l.padding.right;
                        h -= l.padding.top + l.padding.bottom;
                    }
                    (w, h)
                }
                Err(_) => (viewport.width, viewport.height),
            }
        };

        let mut changed = false;
        for (id, taffy_id) in targets {
            let parent = self.tree.taffy.parent(taffy_id);
            let is_abs = matches!(
                self.tree.nodes[id].computed_style.position,
                PositionValue::Absolute | PositionValue::Fixed
            );
            // An absolute child resolves against the parent's padding box;
            // an in-flow one against its content box.
            let (pw, ph) = basis_box(&self.tree.taffy, parent, !is_abs);
            let (ow, oh) = basis_box(&self.tree.taffy, Some(taffy_id), true);

            // Definiteness per basis axis: an axis that is content-sized
            // resolves the percentage part as zero/auto instead (module doc).
            let parent_rinch = self.taffy_parent_rinch(id);
            let (pw_def, ph_def) = match parent_rinch {
                Some(p) => (
                    self.calc_axis_definite(p, Axis::X, 0),
                    self.calc_axis_definite(p, Axis::Y, 0),
                ),
                None => (true, true), // viewport
            };
            let (ow_def, oh_def) = (
                self.calc_axis_definite(id, Axis::X, 0),
                self.calc_axis_definite(id, Axis::Y, 0),
            );
            let basis = |b: f32, definite: bool| (definite && b.is_finite()).then_some(b);
            let (pw, ph) = (basis(pw, pw_def), basis(ph, ph_def));
            let (ow, oh) = (basis(ow, ow_def), basis(oh, oh_def));

            // flex-basis resolves against the flex container's inner main size.
            let parent_main = match parent
                .and_then(|p| self.tree.taffy.style(p).ok())
                .map(|s| s.flex_direction)
            {
                Some(taffy::FlexDirection::Column | taffy::FlexDirection::ColumnReverse) => ph,
                _ => pw,
            };

            // An ICB-absolute's own width/height Calc was baked from the
            // viewport at style-apply time (out_of_flow, #204) — leave them.
            let skip_own_size = crate::out_of_flow::out_of_flow_kind(&self.tree, id)
                == Some(crate::out_of_flow::OutOfFlowKind::IcbAbsolute);

            let before = match self.tree.taffy.style(taffy_id) {
                Ok(s) => s.clone(),
                Err(_) => continue,
            };
            let mut ts = before.clone();
            let cs = &self.tree.nodes[id].computed_style;

            if !skip_own_size {
                patch_dim(&mut ts.size.width, cs.width, pw);
                patch_dim(&mut ts.size.height, cs.height, ph);
            }
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
