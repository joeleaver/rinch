//! The content region below the steps (issue #2195).

use super::*;

/// The `.rinch-stepper__content` region of one stepper render, and what it has
/// taken in (issue #2195).
///
/// Content is **moved** into the region, never copied: a step's children are
/// live nodes with effects and handlers of their own. So each node taken in
/// remembers where it came from, and goes back there — into its step's body,
/// under the label — as soon as it is no longer the one to show. That is also
/// what makes a step that leaves the stepper (a `for` reconcile, a move to
/// another stepper) take its content with it, and a step that is
/// **discarded** retire it: the content is the step's, so it does not outlive
/// it.
#[derive(Debug, Clone)]
pub(super) struct ContentRegion {
    pub(super) region: NodeHandle,
    /// [`Stepper::content_for_active_only`]: whether the active step's content
    /// is moved into the region at all. When it is not, every step keeps its
    /// content and only the completed block comes here.
    pub(super) for_active_only: bool,
    pub(super) shown: Rc<RefCell<Vec<Shown>>>,
    /// Whether the region carries `--completed`. Kept here rather than read
    /// back from the region's `class`: the region is this render's own node,
    /// and a read per pass would cost every change to the list one more
    /// attribute read (the #748 cost pins).
    pub(super) completed: Rc<Cell<bool>>,
}

/// A step's content the [`ContentRegion`] has taken in.
#[derive(Debug)]
pub(super) struct Shown {
    pub(super) node: NodeHandle,
    /// The step it was taken from.
    pub(super) step: NodeHandle,
    /// Its parent there, the step's `.rinch-stepper__step-body`, to put it
    /// back into.
    pub(super) home: NodeHandle,
    /// Its next sibling there, to put it back in front of (`None` for the
    /// body's last child, where a step renders it).
    pub(super) before: Option<NodeHandle>,
}

impl Shown {
    /// Put the content back into its step's body.
    ///
    /// Not when the body now sits **inside** the content (an app moved it
    /// there while the content was shown): putting the content into it would
    /// make the content its own ancestor, a cycle every tree walk then spins
    /// in for ever. The content stays where it is and is forgotten, as content
    /// something else took out of the region is.
    fn return_home(&self) {
        if is_inclusive_ancestor(&self.node, &self.home) {
            return;
        }
        match &self.before {
            Some(before)
                if before.parent_node().map(|p| p.node_id()) == Some(self.home.node_id()) =>
            {
                self.home.insert_before(&self.node, before)
            }
            _ => self.home.append_child(&self.node),
        }
    }
}

/// Whether `node` is `ancestor` or lies under it.
fn is_inclusive_ancestor(ancestor: &NodeHandle, node: &NodeHandle) -> bool {
    let target = ancestor.node_id();
    let mut current = Some(node.clone());
    while let Some(n) = current {
        if n.node_id() == target {
            return true;
        }
        current = n.parent_node();
    }
    false
}

/// A step's `.rinch-stepper__step-content`, under its
/// `.rinch-stepper__step-body`, with that body: `(body, content)`, if both are
/// still there.
pub(super) fn step_content(step: &NodeHandle) -> Option<(NodeHandle, NodeHandle)> {
    let body = step
        .children()
        .into_iter()
        .find(|child| has_class(child, STEP_BODY_CLASS))?;
    let content = body
        .children()
        .into_iter()
        .find(|child| has_class(child, STEP_CONTENT_CLASS))?;
    Some((body, content))
}

/// Bring the content region into line with `active` (issue #2195).
///
/// Mantine's rule: the content of the step at `active`; once `active` is past
/// the last step, the completed block.
///
/// **A step's content** is moved into the region while its step is the one at
/// `active`, and back into its step as soon as it is not — another step became
/// the active one, or the step left the stepper, which then takes its content
/// with it. A step that was **discarded** (a `for` row discards the step it
/// built) takes its content with it the same way: the content is discarded,
/// since the recursive discard of the step could not reach it in the region.
/// Content something else took out of the region is forgotten, not put back
/// and not discarded: it is the app's now. With
/// [`Stepper::content_for_active_only`] off, no step's content is taken in.
///
/// **A completed block** is moved into the region whatever `active` is, and
/// shown there only by the `--completed` modifier the region carries once
/// `active` is past the last step. Left among the steps it would be a sibling
/// of the last one, which then stops being `:last-child` and draws a separator
/// to nothing. A block an `if` or a `for` puts back among the steps is an
/// arrival, so the next pass takes it too; one they remove simply goes.
///
/// Idempotent, as [`settle_steps`] is, and it runs at the end of every pass: a
/// pass with nothing to change moves nothing and writes nothing.
pub(super) fn settle_content(steps: &[NodeHandle], completed: &[NodeHandle], d: &Derivation) {
    let region = &d.content.region;
    let region_id = region.node_id();
    let mut shown = d.content.shown.borrow_mut();

    let target = steps.get(d.active as usize);
    shown.retain(|s| {
        if s.node.parent_node().map(|p| p.node_id()) != Some(region_id) {
            return false;
        }
        if s.step.is_retired() {
            s.node.discard();
            return false;
        }
        let wanted = target.is_some_and(|t| t.node_id() == s.step.node_id());
        if !wanted {
            s.return_home();
        }
        wanted
    });

    if d.content.for_active_only
        && let Some(step) = target
        && shown.is_empty()
        && let Some((body, content)) = step_content(step)
    {
        let before = content.next_sibling();
        region.append_child(&content);
        shown.push(Shown {
            node: content,
            step: step.clone(),
            home: body,
            before,
        });
    }

    for block in completed {
        region.append_child(block);
    }

    let past_the_end = target.is_none();
    if d.content.completed.replace(past_the_end) != past_the_end {
        region.toggle_class(CONTENT_COMPLETED_CLASS);
    }
}
