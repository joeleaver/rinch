//! The classes and attributes the derivation reads and writes on the tree,
//! and the walks that find a stepper's steps.

use super::*;

/// The class a clickable step carries.
pub(super) const CLICKABLE_CLASS: &str = "rinch-stepper__step--clickable";

/// The class a loading step carries.
pub(super) const LOADING_CLASS: &str = "rinch-stepper__step--loading";

/// The class a [`StepperStep::disabled`] step carries.
pub(super) const DISABLED_CLASS: &str = "rinch-stepper__step--disabled";

/// A step's derivation position, 0-based, set by [`settle_steps`] on every
/// pass regardless of clickability (issue #737).
///
/// Distinct from [`STEP_ATTR`], which a step's own `step` prop can override
/// for the *label* only — `allow_next_steps_select` and
/// [`Stepper::on_step_click`] both count positions, and a second notion of
/// index would disagree with them the same way the state derivation would.
/// A click handler reads this back at **click** time rather than closing
/// over the position it saw when it registered (the #714 pattern), because a
/// later insertion or removal can renumber the step after the handler is
/// wired (issues #716, #745).
pub(super) const POSITION_ATTR: &str = "data-step-position";

/// Set by [`settle_steps`] on a step once it is fully derived: the
/// [`Derivation::settler`] of the stepper render that derived it (issue #748).
///
/// With [`POSITION_ATTR`] it is what lets a later pass tell a step it has
/// nothing to do for. The position alone cannot: a step moved in from another
/// stepper can land at the position it left, under different props.
pub(super) const SETTLED_BY_ATTR: &str = "data-stepper-settled";

/// The plain HTML `disabled` boolean attribute, read tag-agnostically
/// (`node_is_disabled` honours it on any tag, not only the ones a browser
/// does) to mean exactly what it means on a `<button>`: never clickable,
/// never focusable, whatever else granted it the class.
pub(super) const DISABLED_ATTR: &str = "disabled";

/// Set by [`StepperStep::render`] when the step asked for its own
/// clickability (`allow_step_click` or `allow_step_select`), so
/// [`settle_step_clickability`] can tell that ask apart from a class an
/// earlier pass of its own granted — a class already on the node cannot say
/// who put it there.
pub(super) const OWN_CLICKABLE_ATTR: &str = "data-step-own-clickable";

/// Set by [`settle_step_clickability`] once a step's click handler is
/// registered, and never removed while the step lives: the registration
/// itself is paid once, and `data-rid` is set from this record — not
/// re-derived — whenever the step becomes reachable again after a position
/// shift made it briefly unreachable.
///
/// The value is a space-separated list of `<settler>:<handler id>` entries,
/// one per stepper render that has wired the step (issue #1428). A handler
/// closes over the `on_step_click` of the render that registered it, so a
/// render restores `data-rid` only from its own entry
/// ([`recorded_handler`]) and registers — and adds an entry — when it has
/// none: a step moved from one stepper to another answers to the one it is
/// in, and a step moved back finds the handler its first stepper registered.
/// It was a bare id, which the second stepper took for its own.
///
/// **An entry whose handler is no longer registered is dropped the next time
/// an entry is added**, so the list holds the live renders that have wired the
/// step, plus whatever died since the last registration. A handler is owned by
/// the render that registered it — at render time and in a late pass alike
/// (`as_owned_by`) — and freed with that render's scope, and a settler is never
/// reused, so a dead entry is only weight: unpruned, a stepper re-rendered or
/// remounted over step nodes the caller kept grew the record by one entry per
/// render. A stepper rendered with no owner keeps its handlers, and so its
/// entries, for the app's life.
pub(super) const CLICK_HANDLER_ID_ATTR: &str = "data-stepper-click-handler";

/// The handler id `settler` recorded in a [`CLICK_HANDLER_ID_ATTR`] value.
pub(super) fn recorded_handler(record: Option<&str>, settler: &str) -> Option<usize> {
    record?
        .split_whitespace()
        .filter_map(handler_of)
        .find_map(|(by, id)| (by == settler).then_some(id))
}

/// One `<settler>:<handler id>` entry of a [`CLICK_HANDLER_ID_ATTR`] value.
pub(super) fn handler_of(entry: &str) -> Option<(&str, usize)> {
    let (by, id) = entry.split_once(':')?;
    Some((by, id.parse().ok()?))
}

pub(super) fn is_registered(handler: usize) -> bool {
    rinch_core::events::has_click_handler(rinch_core::events::EventHandlerId(handler))
}

/// The three **content keys** a step's icon box deals in.
///
/// A key is not a state: `completed` and `progress` name the two glyphs a step
/// (or its [`Stepper`]) can supply for a particular state, and `base` names the
/// one glyph that serves every state neither of those covers — the step's own
/// `icon`, or failing that its number. Two states therefore share a key
/// whenever the step has no progress icon, which is the common case, and moving
/// such a step between them costs nothing but a renumber.
pub(super) const KEY_COMPLETED: &str = "completed";
/// See [`KEY_COMPLETED`].
pub(super) const KEY_PROGRESS: &str = "progress";
/// See [`KEY_COMPLETED`].
pub(super) const KEY_BASE: &str = "base";

/// Set on a step's icon box: the content keys the box holds a **real** glyph
/// for, space separated.
///
/// Written by [`StepperStep`] from its own icon props. A key that is absent is
/// one where what is showing (or parked) is a built-in placeholder — the tick, or
/// the number — which a [`Stepper`]'s own default for that key may replace;
/// [`Stepper`] then adds the key, so the substitution happens once however many
/// times the pass re-runs (issue #716).
pub(super) const ICON_HAS_ATTR: &str = "data-icon-has";

/// Set on a step's icon box: which content key the live glyph stands for.
///
/// The box's other children are hidden alternates, each labelled by
/// [`ICON_ALT_ATTR`]. A box with no `data-icon-live` was not built by
/// [`StepperStep`] and [`Stepper`] leaves its contents alone.
pub(super) const ICON_LIVE_ATTR: &str = "data-icon-live";

/// Set by [`StepperStep`] on itself when the caller named a `state`.
///
/// Presence is the whole value, and it is the only thing that can carry it:
/// `state: String` renders an explicit `"inactive"` and an unset field into the
/// same class, so [`Stepper`] cannot tell a step that chose its state from one
/// waiting to be told. `Radio` records its `size` the same way, for the same
/// reason (issue #707).
pub(super) const STATE_ATTR: &str = "data-state";

/// The step's index, set by [`StepperStep`] when the caller named one and by
/// [`Stepper`] from the step's position otherwise.
pub(super) const STEP_ATTR: &str = "data-step";

/// Set by [`Stepper`] beside [`STEP_ATTR`] on a step it numbered itself.
///
/// Presence is the whole value, and the derivation cannot re-run without it:
/// once the first pass has numbered a step, `data-step` is present whoever wrote
/// it, so a later pass would read a caller's ask where there was none and leave
/// the step at a position it no longer occupies (issue #716).
pub(super) const STEP_DERIVED_ATTR: &str = "data-step-derived";

/// The wrapper class of an alternate icon parked in a step's icon box.
pub(super) const ICON_ALT_CLASS: &str = "rinch-stepper__step-icon-alt";

/// Which [content key](KEY_COMPLETED) an alternate icon is for.
pub(super) const ICON_ALT_ATTR: &str = "data-icon-for";

/// One step of a stepper.
pub(super) const STEP_CLASS: &str = "rinch-stepper__step";

/// On the content region once `active` is past the last step: shows the
/// completed block (issue #2195).
pub(super) const CONTENT_COMPLETED_CLASS: &str = "rinch-stepper__content--completed";

/// On the stepper when [`Stepper::content_for_active_only`] is off: every step
/// shows its own content inside the step (issue #2195).
pub(super) const CONTENT_IN_STEPS_CLASS: &str = "rinch-stepper--content-in-steps";

/// A step's body: its label, its description and, last, its content wrapper.
pub(super) const STEP_BODY_CLASS: &str = "rinch-stepper__step-body";

/// A step's own content wrapper, which holds its children (issue #2195). It is
/// the last child of the step's body.
pub(super) const STEP_CONTENT_CLASS: &str = "rinch-stepper__step-content";

/// A [`StepperCompleted`] block, which is **not a position** — see
/// [`collect_steps`] for why it is skipped rather than stopped at.
pub(super) const COMPLETED_CLASS: &str = "rinch-stepper__completed";

/// Does `node` carry `class` as a whole class token?
pub(super) fn has_class(node: &NodeHandle, class: &str) -> bool {
    node.get_attribute("class")
        .unwrap_or_default()
        .split_whitespace()
        .any(|c| c == class)
}

/// The classes [`collect_steps`] stops descending at, read back upwards by the
/// late-child observers so the two halves name one boundary.
///
/// [`COMPLETED_CLASS`] is here as a **cost** rule, not a correctness one, and a
/// mutant narrowing this back to `[STEP_CLASS]` therefore survives the suite: a
/// change inside a completed block cannot alter any step's position, because
/// `collect_steps` does not look in there, so the pass it declines would have
/// re-derived identical answers over an identical list. It is here so that
/// editing content inside a completed block does not re-walk the whole stepper.
pub(super) const STEP_BOUNDARY: &[&str] = &[STEP_CLASS, COMPLETED_CLASS];

/// Every step in `node`'s subtree, in document order, and the
/// [`StepperCompleted`] blocks the walk skips: the ones that belong to this
/// stepper, which the content region takes in (issue #2195). One walk for both,
/// so the region adds no class read per step to a pass (the #748 cost pins).
///
/// Steps are found by class rather than taken as `children` directly: a `for`
/// loop or a conditional puts wrapper nodes between the stepper and its steps.
/// The walk stops at each step, so a nested stepper inside a step's content is
/// not this one's to renumber.
///
/// It stops at a [`StepperCompleted`] too, and for the same reason (issue #741):
/// that block is what a stepper shows *instead of* its steps once they are all
/// done, so it is not a position and neither is anything inside it. The walk
/// used to descend into it, which re-derived a nested stepper's steps at the
/// outer stepper's indices.
///
/// **It is a skip, not a stop.** Mantine's `Stepper.Completed` is conventionally
/// written last but is not required to be, and Mantine never stops counting
/// steps at it. Reading #741 as "stop the walk" — which this did before review —
/// makes a stepper whose completed block is written *first* derive nothing at
/// all: no step numbered, every step inactive, and two steps both drawing the
/// number `1`. So the block is skipped and the walk goes on, and a step after it
/// is a position like any other.
pub(super) fn collect_steps(node: &NodeHandle) -> (Vec<NodeHandle>, Vec<NodeHandle>) {
    fn walk(node: &NodeHandle, steps: &mut Vec<NodeHandle>, completed: &mut Vec<NodeHandle>) {
        // One read for both questions: this walk runs over every step on every
        // change to the list (issue #748).
        let class = node.get_attribute("class").unwrap_or_default();
        let carries = |wanted: &str| class.split_whitespace().any(|c| c == wanted);
        // Asked first, so a node somehow carrying both classes is not a
        // position — the narrower answer.
        if carries(COMPLETED_CLASS) {
            completed.push(node.clone());
            return;
        }
        if carries(STEP_CLASS) {
            steps.push(node.clone());
            return;
        }
        for child in node.children() {
            walk(&child, steps, completed);
        }
    }
    let (mut steps, mut completed) = (Vec::new(), Vec::new());
    walk(node, &mut steps, &mut completed);
    (steps, completed)
}

/// A step's icon box, which is its first child.
pub(super) fn step_icon_box(step: &NodeHandle) -> Option<NodeHandle> {
    step.children()
        .into_iter()
        .find(|child| has_class(child, "rinch-stepper__step-icon"))
}
