//! Writing a node's inline `style`: merging declarations into the attribute,
//! invalidating the cascade, and the inset fast path that skips it.

use rinch_core::dom::{serialize_declarations, split_declarations_keeping_duplicates};

use style::properties::{
    LonghandId, PropertyDeclaration, PropertyDeclarationBlock, PropertyDeclarationId, PropertyId,
};
use style::values::generics::position::GenericInset;
use style::values::specified::LengthPercentage;

use crate::computed_style::{LengthPercentageAutoValue, PositionValue};
use crate::node::DirtyFlags;
use crate::node::RawNodeId;

use super::{RinchDocument, parse_inline_style};

/// The four inset longhands, in the order an [`InsetBatch`] stores them.
const INSET_SIDES: [(&str, LonghandId); 4] = [
    ("left", LonghandId::Left),
    ("top", LonghandId::Top),
    ("right", LonghandId::Right),
    ("bottom", LonghandId::Bottom),
];

/// The insets one `set_styles` batch asks the fast path to write, indexed like
/// [`INSET_SIDES`]; `None` is a side the batch does not touch.
type InsetBatch = [Option<LengthPercentageAutoValue>; 4];

/// The value of inset longhand `id` in `pdb`, if the fast path can hand it to
/// Taffy without a cascade: `auto`, an absolute length (`px`, `pt`, `in`, …)
/// or a percentage — what `inset_from_stylo_generic` would make of the
/// computed value. `None` for anything that needs the cascade — `em`/`rem`
/// (font size), `vh`/`vw` (Stylo's device), `calc()`, `var()`, a CSS-wide
/// keyword, an anchor function — or a declaration Stylo rejected (a unitless
/// `10`, `10 px`, `NaNpx`), so the block holds no value for it at all.
fn plain_inset(
    pdb: &PropertyDeclarationBlock,
    id: LonghandId,
) -> Option<LengthPercentageAutoValue> {
    let (declaration, _importance) = pdb.get(PropertyDeclarationId::Longhand(id))?;
    let inset = match declaration {
        PropertyDeclaration::Left(v)
        | PropertyDeclaration::Top(v)
        | PropertyDeclaration::Right(v)
        | PropertyDeclaration::Bottom(v) => v,
        _ => return None,
    };
    match inset {
        GenericInset::Auto => Some(LengthPercentageAutoValue::Auto),
        GenericInset::LengthPercentage(LengthPercentage::Length(len)) => {
            let px = len.to_px_if_absolute()?;
            px.is_finite()
                .then_some(LengthPercentageAutoValue::Length(px))
        }
        GenericInset::LengthPercentage(LengthPercentage::Percentage(pct)) => {
            Some(LengthPercentageAutoValue::Percent(pct.get()))
        }
        _ => None,
    }
}

impl RinchDocument {
    /// The node's inline `style` attribute with `properties` merged in — a
    /// later declaration of a property replaces the earlier one **in place**,
    /// keeping every other declaration where the author wrote it.
    ///
    /// Property names are matched ASCII case-insensitively, and `properties`
    /// names go through [`rinch_core::dom::normalize_property_name`] on the way
    /// in so the caller's spelling meets the attribute's on equal terms (#711).
    ///
    /// The parse and the join are
    /// [`rinch_core::dom::split_declarations`]/[`serialize_declarations`], the
    /// same pair `rsx!`'s `style:` prop composes with — one parser and one
    /// rule for every author of an inline style (#670). `rinch-dom` had its own
    /// `parse_style_string` until then, which split on a bare `;`/`:` and so
    /// destroyed any value carrying either: a
    /// `background-image: url(data:image/png;base64,…)` already in the
    /// attribute came back out of an unrelated `set_style` as
    /// `background-image: url(data:image/png`, i.e. gone. It also collapsed a
    /// property declared twice at the **first** declaration's position where
    /// CSSOM collapses at the last, which is behaviour rather than spelling as
    /// soon as a shorthand is involved — Chrome 150 computes `left: 4px` for
    /// `inset: 0px; left: 25px; inset: 4px`, and the first position gives
    /// `25px`. (The *position* is the last declaration's; the *value* that
    /// survives is the `!important` one where the two disagree — #711.)
    ///
    /// Order is load-bearing, not cosmetic (#265). `set_styles` parses this
    /// string into the declaration block Stylo cascades, so whatever order
    /// comes out here *is* the order two declarations of the same longhand —
    /// or a shorthand and one of its longhands — resolve in. This used to
    /// round-trip through a `HashMap`, which reshuffled the whole attribute on
    /// every write with a per-process random order: `set_style("left", …)` on
    /// a node whose attribute already said `inset: 0` produced `left` first
    /// (loses) or `inset` first (wins) depending on the run, so a single
    /// `assert` could pass all day and fail in CI. A `Vec` makes it a fact
    /// about the input instead of a fact about the process.
    ///
    /// **A written property moves past a later declaration that covers it**
    /// (#470). CSSOM keeps a list of longhands — a shorthand is expanded when
    /// it is parsed — so `style.left = "10px"` on `"left: 5px; inset: 0"`
    /// replaces the one `left` longhand and `left` computes to `10px`.
    /// Rewritten in place in this string, `left: 10px` would still sit before
    /// the `inset: 0` that covers it, and lose the cascade to it. So a property
    /// that a declaration *after* its slot overlaps ([`declarations_overlap`])
    /// is taken out of its slot and appended, where it wins as it does in a
    /// browser; every other rewrite stays where it stands. The string stays the
    /// source of truth and keeps the author's spelling — what moves is one
    /// declaration, and only when its position decides the cascade.
    ///
    /// **A normal write replaces an `!important` longhand it shares** (#1298).
    /// CSSOM's `setProperty` replaces the longhand outright, importance and
    /// all, so `left: 25px` on `inset: 0 !important` computes `25px` in
    /// Chrome. A string can only say that by expanding the important
    /// declaration into its longhands (Stylo's serialisation of each, still
    /// `!important`) and dropping the ones written; a written longhand takes
    /// the slot it held in the expansion, which is where Chrome's block keeps
    /// it. Only a *shared longhand* counts here, not the logical-group overlap
    /// the move above uses: Chrome's `setProperty("left", …)` leaves an
    /// important `inset-inline` winning (measured, Chrome 157).
    ///
    /// **A duplicated property is collapsed as CSSOM's parse collapses it**
    /// (#1353), whether or not this call writes it: Stylo judges each of its
    /// declarations, an invalid one drops out, and the winner (the last
    /// important valid one, else the last valid one) keeps its own slot.
    /// Chrome's block never held the losers, so its `getAttribute` after any
    /// `setProperty` shows the property once. A property Stylo does not know
    /// (rinch's raw-read SVG properties) collapses syntactically, as
    /// [`rinch_core::dom::split_declarations`] does.
    ///
    /// The existing attribute is read with
    /// [`split_declarations_keeping_duplicates`], not the syntactically
    /// collapsing `split_declarations` (#722): `color: notacolor !important;
    /// color: blue` collapsed to the invalid important declaration before
    /// Stylo saw it (black, where Chrome computes blue). The collapse above
    /// asks Stylo instead. A property this call *is* writing has no such
    /// ambiguity — the caller supplies its one new value — so every existing
    /// occurrence of that name collapses to a single slot (the last one,
    /// matching the position rule above).
    pub(super) fn merged_inline_style(
        &self,
        node_id: RawNodeId,
        properties: &[(&str, &str)],
    ) -> String {
        let mut decls: Vec<(String, String)> = self.tree.nodes[node_id]
            .attributes
            .get("style")
            .map(|s| split_declarations_keeping_duplicates(s))
            .unwrap_or_default();
        collapse_duplicates_as_cssom(&mut decls);
        for &(property, value) in properties {
            // The caller's name goes through the same rule the parsed ones did
            // (#711): CSSOM lowercases a non-custom name at both ends, so
            // `set_style("COLOR", …)` overwrites an existing `color` rather
            // than declaring the property a second time.
            let property = rinch_core::dom::normalize_property_name(property);
            if replace_important_longhands(&mut decls, &property, value) {
                continue;
            }
            // Every existing occurrence of this property, in order. With the
            // duplicate-preserving parse above there can be more than one —
            // an author's own literal duplicate the attribute never had a
            // reason to collapse until a write actually touches that name.
            let mut positions: Vec<usize> = decls
                .iter()
                .enumerate()
                .filter(|(_, (k, _))| k.as_str() == &*property)
                .map(|(i, _)| i)
                .collect();
            match positions.pop() {
                Some(slot) => {
                    // `slot` is the LAST occurrence (positions is ascending),
                    // so every remaining entry in `positions` sits before it.
                    // Drop them — this write is an unambiguous new value, so
                    // nothing is lost by collapsing the property's other
                    // occurrences to it — highest index first to keep the
                    // rest of `positions` valid as we go.
                    for &at in positions.iter().rev() {
                        decls.remove(at);
                    }
                    let slot = slot - positions.len();

                    let covered_later = slot + 1 < decls.len() && {
                        let written = declared_longhands(&property);
                        decls[slot + 1..]
                            .iter()
                            .any(|(k, _)| declarations_overlap(&written, &declared_longhands(k)))
                    };
                    if covered_later {
                        let (name, _) = decls.remove(slot);
                        decls.push((name, value.to_string()));
                    } else {
                        decls[slot].1 = value.to_string();
                    }
                }
                None => decls.push((property.into_owned(), value.to_string())),
            }
        }
        serialize_declarations(&decls)
    }

    /// The normal path after an inline style change: drop the cached Stylo
    /// data so the next `resolve_layout` re-cascades this node from its
    /// declaration block.
    ///
    /// The inline formatting context it takes part in is **not** invalidated
    /// here. The re-cascade compares the node's old and new text inputs
    /// (`ComputedStyle::same_text_layout_inputs`) and drops the IFC only when
    /// they differ, so a `left`/`width`/`background` write — the per-frame
    /// shape of a drag or an animation driven through `set_style` — no longer
    /// re-shapes the text inside the node it moves.
    pub(super) fn invalidate_inline_style(&mut self, node_id: RawNodeId) {
        // Restyle this element (the snapshot above told the invalidator about
        // the attribute), keeping its old style for the cascade to compare
        // against: its children follow only if something they inherit moved.
        self.mark_restyle(node_id, false);
        self.push_dirty_flags(
            node_id,
            DirtyFlags::STYLE | DirtyFlags::LAYOUT | DirtyFlags::PAINT,
        );
    }

    /// What the inset fast path may write for this `set_styles` batch, or
    /// `None` when the batch must take the normal path.
    ///
    /// Changing only `left`/`top`/`right`/`bottom` on a `position: absolute`
    /// element moves it within its containing block — siblings and children
    /// are unaffected — so the cascade and IFC invalidation the normal path
    /// pays are skipped and `ComputedStyle` and the Taffy inset are written
    /// directly. The values come out of `pdb`, the declaration block Stylo
    /// just parsed from the merged style attribute, never from a parser of our
    /// own: what the fast path applies is by construction what the next full
    /// cascade of that same block applies, so the two cannot disagree (#236)
    /// — not on a unitless number Stylo drops, not on `inset:` versus `left:`
    /// precedence, not on rounding.
    ///
    /// Declined — `None`, nothing touched — for:
    /// - a batch that is empty or not inset-only;
    /// - a node that is not `position: absolute`. `fixed` is excluded on
    ///   purpose: `apply_stylo_styles_to_taffy` bakes its Taffy *size* from
    ///   its insets, so an inset change is not inset-only for it — children
    ///   would be laid out against the stale size until the next restyle. An
    ///   absolute with no positioned ancestor is excluded for the same reason
    ///   — its size is baked from the initial containing block (#204);
    /// - a requested inset that is not a plain value in the block (see
    ///   [`plain_inset`]);
    /// - a requested inset some matched rule declares `!important` (#277; see
    ///   [`Self::important_rule_declares_inset`]). The cascade puts that
    ///   declaration above a normal inline one, so writing the inline value
    ///   would move the box until the next restyle snapped it back;
    /// - a node whose `transition` covers an inset
    ///   ([`crate::transition::TransitionProperty::covers_inset`], #280). No
    ///   inset is animatable today, so this declines nothing yet; it is here
    ///   so the day one is, the move reaches the cascade's transition hooks
    ///   instead of jumping.
    pub(super) fn inset_fast_path_values(
        &self,
        node_id: RawNodeId,
        properties: &[(&str, &str)],
        pdb: &PropertyDeclarationBlock,
    ) -> Option<InsetBatch> {
        if properties.is_empty()
            || self.tree.nodes[node_id].computed_style.position != PositionValue::Absolute
            // An absolute with no positioned ancestor is excluded for exactly
            // the reason `fixed` is: its Taffy *size* is baked from its insets
            // against the initial containing block (#204), so an inset change
            // is not inset-only for it either. One resolved against a
            // non-parent ancestor (#386) is **not** excluded: the fast path
            // re-derives its bake from the new insets
            // (`rebake_after_inset_write`), and its position comes from them
            // as it is read back.
            || crate::out_of_flow::out_of_flow_kind(&self.tree, node_id)
                == Some(crate::out_of_flow::OutOfFlowKind::IcbAbsolute)
        {
            return None;
        }
        let mut batch: InsetBatch = [None; 4];
        for &(property, _) in properties {
            let slot = INSET_SIDES.iter().position(|(name, _)| *name == property)?;
            batch[slot] = Some(plain_inset(pdb, INSET_SIDES[slot].1)?);
        }
        let node = &self.tree.nodes[node_id];
        if node
            .transition_specs
            .iter()
            .any(|spec| spec.property.covers_inset())
        {
            return None;
        }
        let sides = INSET_SIDES
            .iter()
            .zip(batch.iter())
            .filter(|(_, v)| v.is_some())
            .map(|((_, id), _)| *id);
        for id in sides {
            if self.important_rule_declares_inset(node_id, id) {
                return None;
            }
        }
        Some(batch)
    }

    /// Whether any rule the node matched on its last cascade declares inset
    /// longhand `id` at an `!important` level (#277).
    ///
    /// Walks the node's Stylo rule node chain — one entry per matched rule,
    /// already sorted by the cascade — and reads only the important levels'
    /// declaration blocks, so a node with no important rule pays the walk and
    /// nothing else. An `inset` shorthand is stored as its four longhands, so
    /// `inset: 0 !important` answers for every side. A *logical* inset
    /// (`inset-inline-start`, `inset-block-end`, and the `inset-inline` /
    /// `inset-block` shorthands that expand to them) is its own longhand that
    /// the cascade maps to a physical side only later, by writing mode and
    /// direction; any important one answers `true` for every side rather than
    /// repeating that mapping here.
    ///
    /// Conservative on purpose: it does not ask *which* important declaration
    /// wins, so a node whose own style attribute already carried an
    /// `!important` inset also declines, and the write takes the cascade —
    /// correct, merely not fast. The chain is the last cascade's; a selector
    /// change since then is already queued for a restyle that recomputes the
    /// inset from the full block, so a stale answer either way costs at most
    /// the fast path, never the result.
    fn important_rule_declares_inset(&self, node_id: RawNodeId, id: LonghandId) -> bool {
        let data = self.tree.nodes[node_id].stylo_element_data.borrow();
        let Some(rules) = data
            .as_ref()
            .and_then(|d| d.styles.get_primary())
            .and_then(|cv| cv.rules.as_ref())
        else {
            return false;
        };
        let guard = self.tree.guard.read();
        rules.self_and_ancestors().any(|rule| {
            rule.cascade_level().is_important()
                && rule.style_source().is_some_and(|source| {
                    source
                        .read(&guard)
                        .declaration_importance_iter()
                        .any(|(decl, importance)| {
                            importance.important()
                                && matches!(
                                    decl.id(),
                                    PropertyDeclarationId::Longhand(l)
                                        if l == id
                                            || matches!(
                                                l,
                                                LonghandId::InsetInlineStart
                                                    | LonghandId::InsetInlineEnd
                                                    | LonghandId::InsetBlockStart
                                                    | LonghandId::InsetBlockEnd
                                            )
                                )
                        })
                })
        })
    }

    /// Write `insets` to `ComputedStyle` and the Taffy inset, and let Taffy
    /// place the node on the next `resolve_layout`.
    ///
    /// The node's *position* is never computed here. `LayoutResult` is
    /// parent-border-box-relative, an inset is padding-box-relative with the
    /// margin on top, and only Taffy knows the containing block, the border
    /// and the rounding. Taffy's `set_style` clears this node's cache but not
    /// its children's, so the recompute repositions the node and reuses
    /// everything below it. Writing the position by hand is how #236 happened.
    pub(super) fn apply_inset_fast_path(&mut self, node_id: RawNodeId, insets: &InsetBatch) {
        let computed = &mut self.tree.nodes[node_id].computed_style;
        let [left, top, right, bottom] = *insets;
        if let Some(v) = left {
            computed.left = v;
        }
        if let Some(v) = top {
            computed.top = v;
        }
        if let Some(v) = right {
            computed.right = v;
        }
        if let Some(v) = bottom {
            computed.bottom = v;
        }
        // The Taffy inset is `to_taffy_style`'s image of these four fields;
        // re-derive all of it rather than patching one side.
        let inset = taffy::Rect {
            left: computed.left.to_taffy(),
            top: computed.top.to_taffy(),
            right: computed.right.to_taffy(),
            bottom: computed.bottom.to_taffy(),
        };
        if let Some(taffy_id) = self.tree.nodes[node_id].taffy_id
            && let Ok(mut ts) = self.tree.taffy.style(taffy_id).cloned()
        {
            ts.inset = inset;
            // A box sized by its insets against a non-parent containing
            // block (#386) carries that size as a bake: write it for the new
            // insets, or the pass after the compute has to, at the price of
            // a second compute.
            crate::out_of_flow::rebake_after_inset_write(&self.tree, node_id, &mut ts);
            let _ = self.tree.taffy.set_style(taffy_id, ts);
        }

        // Let Taffy place the node on the next resolve_layout. Unconditional:
        // a node without a Taffy id must still be re-laid out rather than
        // silently kept where it was.
        self.tree.layout_dirty = true;
        self.push_dirty_flags(node_id, DirtyFlags::LAYOUT | DirtyFlags::PAINT);
        // No full repaint (#280). The move reaches the dirty region the way
        // any other layout change does: `read_layout_results` pushes the node
        // when its box changes, and `prev_layout` still names the box it was
        // last painted in however many moves and resolves land before the
        // next paint (`NodeTree::consume_paint_dirty`), so the region covers
        // the old rect as well as the new one.
    }
}

/// The longhands a declaration of `name` sets: itself for a longhand, its
/// sub-properties for a shorthand, none for a custom or unknown property (a
/// custom property is one name, and two declarations of it never coexist in a
/// split attribute).
fn declared_longhands(name: &str) -> Vec<LonghandId> {
    match PropertyId::parse_enabled_for_all_content(name) {
        Ok(id) => match id.as_shorthand() {
            Ok(shorthand) => shorthand.longhands().collect(),
            Err(PropertyDeclarationId::Longhand(longhand)) => vec![longhand],
            Err(PropertyDeclarationId::Custom(_)) => Vec::new(),
        },
        Err(()) => Vec::new(),
    }
}

/// Collapse every property declared more than once to the one declaration a
/// CSSOM parse keeps (#1353): Stylo judges each, an invalid one drops out, and
/// the winner — the last important valid declaration, else the last valid one
/// — stays in its own slot. Chrome measured (157): `color: red; gap: 2px;
/// color: blue` keeps the last; `color: blue; gap: 2px; color: notacolor`
/// keeps the first; `color: notacolor !important; color: blue` keeps blue.
///
/// A property Stylo does not know (rinch reads a few SVG ones raw, #705)
/// counts every declaration as valid, which is
/// [`rinch_core::dom::split_declarations`]'s syntactic rule. When no
/// declaration of a property is valid, the last stays: none of them applies,
/// and keeping one keeps the string's spelling for raw readers.
///
/// Free when no property repeats, which is every attribute rinch writes.
fn collapse_duplicates_as_cssom(decls: &mut Vec<(String, String)>) {
    let repeats =
        |decls: &[(String, String)], i: usize| decls[i + 1..].iter().any(|(k, _)| *k == decls[i].0);
    if !(0..decls.len()).any(|i| repeats(decls, i)) {
        return;
    }
    let mut keep = vec![true; decls.len()];
    for i in 0..decls.len() {
        let name = &decls[i].0;
        // Each repeated name once, at its first occurrence.
        if decls[..i].iter().any(|(k, _)| k == name) || !repeats(decls, i) {
            continue;
        }
        let known = PropertyId::parse_enabled_for_all_content(name).is_ok();
        let occurrences: Vec<usize> = (i..decls.len()).filter(|&j| decls[j].0 == *name).collect();
        // (index, important) of each declaration a CSSOM parse would keep.
        let valid: Vec<(usize, bool)> = occurrences
            .iter()
            .filter_map(|&j| {
                let value = &decls[j].1;
                if !known {
                    return Some((j, rinch_core::dom::is_important(value)));
                }
                let pdb = parse_inline_style(&format!("{name}: {value}"));
                let important = pdb
                    .declaration_importance_iter()
                    .any(|(_, imp)| imp.important());
                (!pdb.is_empty()).then_some((j, important))
            })
            .collect();
        let winner = valid
            .iter()
            .rev()
            .find(|(_, important)| *important)
            .or(valid.last())
            .map(|&(j, _)| j)
            .unwrap_or(*occurrences.last().expect("a repeated name occurs"));
        for j in occurrences {
            keep[j] = j == winner;
        }
    }
    let mut keep = keep.into_iter();
    decls.retain(|_| keep.next().unwrap_or(true));
}

/// A normal write of `property` against an `!important` declaration of
/// another property sharing a longhand with it (#1298): CSSOM's `setProperty`
/// replaces that longhand outright, so the important declaration is expanded
/// into its longhands (Stylo's serialisation of each, still important) minus
/// the written ones. A written **longhand** takes its slot in the last such
/// expansion — Chrome keeps it there — and its other declarations go; that
/// case returns `true`, the write done. Otherwise (a shorthand written, or no
/// important overlap) the caller's ordinary placement runs on the result.
///
/// An important declaration whose longhands do not serialise one by one (a
/// `var()` in a shorthand leaves each longhand pending) is left as it is.
fn replace_important_longhands(
    decls: &mut Vec<(String, String)>,
    property: &str,
    value: &str,
) -> bool {
    if rinch_core::dom::is_important(value) || !decls.iter().any(|(_, v)| v.contains('!')) {
        return false;
    }
    let written = declared_longhands(property);
    if written.is_empty() {
        return false;
    }
    let mut placed: Option<usize> = None;
    let mut i = decls.len();
    while i > 0 {
        i -= 1;
        let (name, decl_value) = &decls[i];
        if name == property || !rinch_core::dom::is_important(decl_value) {
            continue;
        }
        if !declared_longhands(name).iter().any(|l| written.contains(l)) {
            continue;
        }
        let Some(longhands) = important_longhands(name, decl_value) else {
            continue;
        };
        let place_here = placed.is_none()
            && written.len() == 1
            && longhands.iter().any(|(id, _, _)| *id == written[0]);
        let mut replacement = Vec::with_capacity(longhands.len());
        for (id, longhand, longhand_value) in longhands {
            if !written.contains(&id) {
                replacement.push((longhand, format!("{longhand_value} !important")));
            } else if place_here {
                placed = Some(i + replacement.len());
                replacement.push((property.to_string(), value.to_string()));
            }
        }
        let grown = replacement.len() as isize - 1;
        decls.splice(i..=i, replacement);
        // An expansion before the placed write shifts it.
        if let Some(at) = placed.as_mut()
            && !place_here
        {
            *at = (*at as isize + grown) as usize;
        }
    }
    let Some(at) = placed else {
        return false;
    };
    let mut index = 0;
    decls.retain(|(k, _)| {
        let keep = index == at || k.as_str() != property;
        index += 1;
        keep
    });
    true
}

/// The longhands an `!important` declaration sets, each with its name and
/// Stylo's serialisation of its value; `None` when Stylo rejects the
/// declaration, it is not important, or a longhand has no value of its own
/// (pending `var()` substitution).
fn important_longhands(name: &str, value: &str) -> Option<Vec<(LonghandId, String, String)>> {
    let pdb = parse_inline_style(&format!("{name}: {value}"));
    let mut out = Vec::new();
    for (declaration, importance) in pdb.declaration_importance_iter() {
        let PropertyDeclarationId::Longhand(id) = declaration.id() else {
            return None;
        };
        let mut serialized = String::new();
        declaration.to_css(&mut serialized).ok()?;
        if !importance.important() || serialized.is_empty() {
            return None;
        }
        out.push((id, id.name().to_string(), serialized));
    }
    (!out.is_empty()).then_some(out)
}

/// Whether a declaration setting `later` decides any longhand of one setting
/// `earlier` when it comes after it (#470): they share a longhand, or — CSSOM's
/// logical-property-group step, which Stylo's `prepare_for_update` implements —
/// one is the logical twin of the other (`left` and `inset-inline-start`), a
/// different longhand that maps onto the same physical side.
fn declarations_overlap(earlier: &[LonghandId], later: &[LonghandId]) -> bool {
    earlier.iter().any(|&a| {
        later.iter().any(|&b| {
            a == b
                || (a.is_logical() != b.is_logical()
                    && a.logical_group().is_some()
                    && a.logical_group() == b.logical_group())
        })
    })
}
