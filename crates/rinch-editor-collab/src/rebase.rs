//! Rebasing local steps over a remote mapping — the ProseMirror collab primitive.
//!
//! When remote changes shift positions, a peer's not-yet-confirmed local steps must be
//! **rebased** over that shift before they can be re-applied (design §8: "local
//! unconfirmed steps are rebased over the remote mapping via `StepMap` — this is why
//! steps must be `map`-able"). This is exactly [`Step::map`]: each step is moved through
//! the [`Mapping`]; a step whose range was entirely deleted drops out.
//!
//! The session's default path projects every local edit immediately, so it carries no
//! unconfirmed backlog — but this primitive is the building block a throttled or
//! authority-style driver needs, and it is exercised by the milestone's rebase test.

use rinch_editor_core::{Mapping, Step};

/// Rebase a batch of local steps over `over`, dropping any step that no longer applies
/// (its content was entirely removed by the remote change).
pub fn rebase_steps(steps: &[Box<dyn Step>], over: &Mapping) -> Vec<Box<dyn Step>> {
    steps.iter().filter_map(|s| s.map(over)).collect()
}
