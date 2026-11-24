//! The content region below the steps (issue #2195).

use super::*;

/// The `.rinch-stepper__content` region of one stepper render, and what it has
/// taken in (issue #2195).
///
/// Content is **moved** into the region, never copied: a step's children are
/// live nodes with effects and handlers of their own. So each node taken in
/// remembers where it came from, and goes back there — into its step, before
/// the separator — as soon as it is no longer the one to show. That is also
/// what makes a step that leaves the stepper (a `for` reconcile, a move to
/// another stepper) take its content with it.
#[derive(Debug, Clone)]
pub(super) struct ContentRegion {
    pub(super) region: NodeHandle,
    pub(super) shown: Rc<RefCell<Vec<Shown>>>,
}

/// A step's content the [`ContentRegion`] has taken in.
#[derive(Debug)]
pub(super) struct Shown {
    pub(super) node: NodeHandle,
    /// The step it was taken from.
    pub(super) home: NodeHandle,
    /// Its next sibling there (the separator), to put it back in front of.
    pub(super) before: Option<NodeHandle>,
}

impl Shown {
    /// Put the content back into its step.
    ///
    /// Not when the step now sits **inside** the content (an app moved it
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

/// The `.rinch-stepper__step-content` child of a step, if it is still there.
pub(super) fn step_content(step: &NodeHandle) -> Option<NodeHandle> {
    step.children()
        .into_iter()
        .find(|child| has_class(child, STEP_CONTENT_CLASS))
}

/// Every [`StepperCompleted`] block under `node` that belongs to this stepper:
/// the walk [`collect_steps`] makes, collecting the blocks it skips.
pub(super) fn collect_completed(node: &NodeHandle) -> Vec<NodeHandle> {
    fn walk(node: &NodeHandle, out: &mut Vec<NodeHandle>) {
        let class = node.get_attribute("class").unwrap_or_default();
        let carries = |wanted: &str| class.split_whitespace().any(|c| c == wanted);
        if carries(COMPLETED_CLASS) {
            out.push(node.clone());
            return;
        }
        if carries(STEP_CLASS) {
            return;
        }
        for child in node.children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    for child in node.children() {
        walk(&child, &mut out);
    }
    out
}

/// Bring the content region into line with `active` (issue #2195).
///
/// Mantine's rule: the content of the step at `active`; once `active` is past
/// the last step, the completed block.
///
/// **A step's content** is moved into the region while its step is the one at
/// `active`, and back into its step as soon as it is not — another step became
/// the active one, or the step left the stepper, which then takes its content
/// with it. Content something else took out of the region is forgotten, not
/// put back.
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
pub(super) fn settle_content(steps_container: &NodeHandle, steps: &[NodeHandle], d: &Derivation) {
    let region = &d.content.region;
    let region_id = region.node_id();
    let mut shown = d.content.shown.borrow_mut();

    let target = steps.get(d.active as usize);
    shown.retain(|s| {
        if s.node.parent_node().map(|p| p.node_id()) != Some(region_id) {
            return false;
        }
        let wanted = target.is_some_and(|t| t.node_id() == s.home.node_id());
        if !wanted {
            s.return_home();
        }
        wanted
    });

    if let Some(step) = target
        && shown.is_empty()
        && let Some(content) = step_content(step)
    {
        let before = content.next_sibling();
        region.append_child(&content);
        shown.push(Shown {
            node: content,
            home: step.clone(),
            before,
        });
    }

    for block in collect_completed(steps_container) {
        region.append_child(&block);
    }

    let completed = target.is_none();
    if has_class(region, CONTENT_COMPLETED_CLASS) != completed {
        region.toggle_class(CONTENT_COMPLETED_CLASS);
    }
}
