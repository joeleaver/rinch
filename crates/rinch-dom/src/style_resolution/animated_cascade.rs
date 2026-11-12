//! A running `color` / `font-size` transition or animation goes through the
//! cascade (#777, #787).
//!
//! The tick writes a sample into the node's own `ComputedStyle`, and for most
//! properties that is the whole of its reach. Two are different. `color` and
//! `font-size` **inherit**, and CSS inherits the *animated* value — so a
//! button's label span fades with the button, and a block's text grows with
//! it — and `font-size` is the basis of every `em` length on the node itself.
//! Stylo had already resolved all of those against the unanimated value, so
//! the label snapped to the end colour and a `width: 6em` stayed put.
//!
//! So such a node is cascaded with its samples **in** Stylo's cascade, at the
//! animation and transition levels where a browser puts them
//! ([`RinchDocument::cascade_samples`]), and that style is its primary — the
//! one its descendants inherit, and the one its `ComputedStyle` is built
//! from. The rules alone are cascaded too, into `Node::base_style`: that is
//! the cascaded value a transition diffs against and an animation's implicit
//! stop takes, so a sample is never read back as a change.
//!
//! Each tick that moves such a sample restyles the node
//! (`RinchDocument::restyle_animated_cascade`), and Stylo's own child-cascade
//! rule carries the change to whatever inherits it. A `color` sample is put
//! in the cascade only for a node with an element child (a generated box
//! included): a text child takes its colour from its parent's
//! `ComputedStyle` at paint, which the tick already wrote, so a button whose
//! text is its own pays no per-frame restyle for its colour transition.

use servo_arc::Arc as ServoArc;
use style::properties::{Importance, PropertyDeclaration, PropertyDeclarationBlock};
use style::values::specified;

use crate::RinchDocument;
use crate::animation::AnimationResult;
use crate::node::RawNodeId;
use crate::transition::{AnimatableValue, TransitionProperty};

/// One node's samples of the properties that go through the cascade: those
/// of its animations, and those of its transitions, which Stylo cascades
/// above animations (and above `!important`).
#[derive(Default)]
pub(crate) struct CascadeSamples {
    pub(crate) animations: Vec<(TransitionProperty, AnimatableValue)>,
    pub(crate) transitions: Vec<(TransitionProperty, AnimatableValue)>,
}

impl CascadeSamples {
    pub(crate) fn is_empty(&self) -> bool {
        self.animations.is_empty() && self.transitions.is_empty()
    }
}

fn set(
    list: &mut Vec<(TransitionProperty, AnimatableValue)>,
    p: TransitionProperty,
    v: AnimatableValue,
) {
    match list.iter_mut().find(|(q, _)| *q == p) {
        Some(slot) => slot.1 = v,
        None => list.push((p, v)),
    }
}

fn declaration(prop: TransitionProperty, value: &AnimatableValue) -> Option<PropertyDeclaration> {
    match (prop, value) {
        (TransitionProperty::FontSize, AnimatableValue::Float(px)) => {
            Some(PropertyDeclaration::FontSize(specified::FontSize::Length(
                specified::LengthPercentage::Length(specified::NoCalcLength::from_px(px.max(0.0))),
            )))
        }
        (TransitionProperty::Color, AnimatableValue::Color(c)) => Some(PropertyDeclaration::Color(
            specified::ColorPropertyValue(specified::Color::from_absolute_color(
                crate::computed_style::absolute_from_peniko(*c),
            )),
        )),
        _ => None,
    }
}

impl RinchDocument {
    /// Whether a `color` sample on `node_id` has to reach the cascade: it has
    /// an element child to inherit it (see the module doc).
    fn colour_reaches_children(&self, node_id: RawNodeId) -> bool {
        self.tree.nodes[node_id]
            .children
            .iter()
            .any(|&c| self.tree.nodes.get(c).is_some_and(|n| n.is_element()))
    }

    /// The samples of `node_id`'s running `color` / `font-size` transitions
    /// and animations at `now`, the ones the cascade has to see (module doc).
    /// Empty — and cheap — for a node with none.
    pub(crate) fn cascade_samples(&self, node_id: RawNodeId, now: f64) -> CascadeSamples {
        let mut out = CascadeSamples::default();
        let anims = self.tree.active_animations.get(&node_id);
        let trans = self.tree.active_transitions.get(&node_id);
        if anims.is_none() && trans.is_none() {
            return out;
        }
        let mut colour = None;
        let mut wants_colour =
            || *colour.get_or_insert_with(|| self.colour_reaches_children(node_id));
        if let Some(anims) = anims {
            // Later animations in the list win, as in the composite order.
            for anim in anims {
                if !(anim.animates(TransitionProperty::FontSize)
                    || anim.animates(TransitionProperty::Color))
                {
                    continue;
                }
                if let AnimationResult::Values(values) = anim.values_at(now) {
                    for (p, v) in values {
                        match p {
                            TransitionProperty::FontSize => set(&mut out.animations, p, v),
                            TransitionProperty::Color if wants_colour() => {
                                set(&mut out.animations, p, v)
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if let Some(trans) = trans {
            for p in [TransitionProperty::FontSize, TransitionProperty::Color] {
                if p == TransitionProperty::Color && !trans.contains_key(&p) {
                    continue;
                }
                if p == TransitionProperty::Color && !wants_colour() {
                    continue;
                }
                if let Some(v) = trans.get(&p).and_then(|t| t.value_at(now)) {
                    set(&mut out.transitions, p, v);
                }
            }
        }
        out
    }

    /// `samples` as the declaration blocks Stylo's `AnimationDeclarations`
    /// carries: the animations' and the transitions'.
    pub(crate) fn sample_blocks(
        &self,
        samples: &CascadeSamples,
    ) -> style::properties::AnimationDeclarations {
        let block = |list: &[(TransitionProperty, AnimatableValue)]| {
            let mut pdb = PropertyDeclarationBlock::new();
            for (p, v) in list {
                if let Some(d) = declaration(*p, v) {
                    pdb.push(d, Importance::Normal);
                }
            }
            (!pdb.is_empty()).then(|| ServoArc::new(self.tree.guard.wrap(pdb)))
        };
        style::properties::AnimationDeclarations {
            animations: block(&samples.animations),
            transitions: block(&samples.transitions),
        }
    }

    /// Whether `node_id`'s primary style is out of step with what
    /// [`Self::cascade_samples`] asks of it: it has samples the cascade did
    /// not see, or it was cascaded with samples that have since gone.
    pub(crate) fn animated_cascade_stale(&self, node_id: RawNodeId, now: f64) -> bool {
        let has_samples = !self.cascade_samples(node_id, now).is_empty();
        has_samples != self.tree.nodes[node_id].base_style.is_some()
    }
}

impl RinchDocument {
    /// The nodes whose running transitions a tick moves through the cascade:
    /// a `font-size` one, or a `color` one with an element child. Read before
    /// the tick, so the frame that finishes one is among them — its restyle
    /// drops the samples from the cascade.
    pub(crate) fn transition_cascade_nodes(&self) -> Vec<RawNodeId> {
        self.tree
            .active_transitions
            .iter()
            .filter(|(id, props)| {
                self.tree.nodes.contains(**id)
                    && (props.contains_key(&TransitionProperty::FontSize)
                        || (props.contains_key(&TransitionProperty::Color)
                            && self.colour_reaches_children(**id)))
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// The animation twin of [`Self::transition_cascade_nodes`], for the
    /// animations a tick can move (not paused, not settled: their sample is
    /// constant and the cascade that wrote it already put it in the cascade).
    pub(crate) fn animation_cascade_nodes(&self) -> Vec<RawNodeId> {
        self.tree
            .active_animations
            .iter()
            .filter(|(id, anims)| {
                self.tree.nodes.contains(**id)
                    && anims.iter().any(|a| {
                        !a.is_paused()
                            && !a.settled
                            && (a.animates(TransitionProperty::FontSize)
                                || (a.animates(TransitionProperty::Color)
                                    && self.colour_reaches_children(**id)))
                    })
            })
            .map(|(id, _)| *id)
            .collect()
    }

    /// Restyle each of `nodes`, so the next cascade puts its new samples —
    /// or, once they have finished, none — into its style and hands them to
    /// whatever inherits them.
    pub(crate) fn restyle_animated_cascade(&mut self, nodes: Vec<RawNodeId>) {
        for id in nodes {
            self.mark_restyle(id, false);
        }
    }
}

impl RinchDocument {
    /// Whether `node_id` or an element above it was cascaded with samples
    /// (`Node::base_style`), so a colour change the cascade finds on it can be
    /// a frame of a running `color` transition or animation.
    pub(crate) fn colour_is_moving(&self, node_id: RawNodeId) -> bool {
        let mut cur = Some(node_id);
        while let Some(id) = cur {
            let Some(node) = self.tree.nodes.get(id) else {
                return false;
            };
            if !node.is_element() {
                return false;
            }
            if node.base_style.is_some() {
                return true;
            }
            cur = node.parent;
        }
        false
    }
}
