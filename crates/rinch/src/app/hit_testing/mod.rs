//! Hit testing: which node a point lands on, and the pointer questions built
//! on that answer (scroll containers, scrollbars, window resize edges, event
//! targets). Platform-agnostic free functions over a laid-out
//! `rinch_dom::NodeTree`; the event loop and the tests call them.
//!
//! - `walk` — [`hit_test`] and the recursive `hit_test_node` walk, with
//!   `descend` (the per-node offset, `position: fixed` hoist and transform
//!   inverse every probe shares).
//! - `paint_order` — stacking-context sequences in paint order (cached in
//!   `rinch_dom::hit_cache`), and the phased probe of a run whose children
//!   paint in CSS 2.1 Appendix E passes (#2276).
//! - `extent` — the flow extent of a subtree, which lets the walk prune
//!   subtrees the point cannot reach.
//! - `probe` — the `Probe` trait: what a walk is looking for (the plain hit,
//!   a scrollbar under the point, a scroll container).
//! - `targets` — what a hit means for events: text under the point, inline
//!   boxes outside their parent, the event target, [`painted_element_box`].
//! - `scroll` — scroll containers, the wheel target and scrollbar hits.
//! - `resize` — window resize edges, the cursor for them, and the scrollbar
//!   thumb check that keeps a resize from starting on a thumb.
//!
//! Tests: `tests/` (by topic), the pruned-vs-unpruned oracle
//! (`../hit_test_prune_tests/`), and the per-issue files below.

mod extent;
mod paint_order;
mod probe;
mod resize;
mod scroll;
mod targets;
mod walk;

#[cfg(feature = "windowing")]
pub(super) use paint_order::plain_paint_order;
pub(super) use resize::cursor_value_to_style;
pub(super) use resize::{
    detect_resize_edge, is_corner_resize, pointer_on_scrollbar_thumb, resize_direction_to_cursor,
};
#[cfg(feature = "windowing")]
pub(super) use scroll::page_scroller;
#[cfg(test)]
pub(super) use scroll::{
    SCROLLBAR_HIT_THICKNESS, find_horizontal_scroll_container,
    find_horizontal_scroll_container_at_point, find_scroll_container,
    find_scroll_container_at_point,
};
pub(super) use scroll::{ScrollbarHit, find_scrollbar_hit, wheel_scroll_target};
pub(super) use targets::{element_of_target, event_target_at, pointer_in_node, text_node_at};
// The two used outside `app/` (the shell, the embed API, the menu bar,
// `testing`).
pub(crate) use targets::painted_element_box;
pub(crate) use walk::hit_test;

#[cfg(test)]
use super::ScrollAxis;
#[cfg(test)]
use extent::flow_extent;
#[cfg(test)]
use paint_order::phased_run_hit_with;
#[cfg(test)]
use targets::{inline_box_outside, visible_text_at};
#[cfg(test)]
use walk::{Frame, descend};

#[cfg(test)]
#[path = "../hit_test_prune_tests/mod.rs"]
mod prune_tests;

#[cfg(test)]
#[path = "../scrollbar_paint_order_1727_tests.rs"]
mod scrollbar_paint_order_1727_tests;

#[cfg(test)]
#[path = "../hidden_ancestor_hit_843_tests.rs"]
mod hidden_ancestor_hit_843_tests;

#[cfg(test)]
#[path = "../paint_phases_hit_2276_tests.rs"]
mod paint_phases_hit_2276_tests;

#[cfg(test)]
#[path = "../overflow_line_hit_2956_tests.rs"]
mod overflow_line_hit_2956_tests;

#[cfg(test)]
#[path = "../relative_inline_hit_2346_tests.rs"]
mod relative_inline_hit_2346_tests;

#[cfg(test)]
#[path = "../inline_box_hit_2618_tests.rs"]
mod inline_box_hit_2618_tests;

#[cfg(test)]
#[path = "../inline_box_hit_2647_tests.rs"]
mod inline_box_hit_2647_tests;

#[cfg(test)]
#[path = "../fixed_escape_hit_549_tests.rs"]
mod fixed_escape_hit_549_tests;

#[cfg(test)]
#[path = "../absolute_escape_hit_2815_tests.rs"]
mod absolute_escape_hit_2815_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "../hidden_ancestor_hit_review_2008_tests.rs"]
mod review_2008_probe_tests;

// Paints with the software painter, which `embed` alone does not build.
#[cfg(all(test, software_shell))]
#[path = "../fixed_over_scroller_2038_tests.rs"]
mod fixed_over_scroller_2038_tests;

// Paints with the software painter, which `embed` alone does not build.
#[cfg(all(test, software_shell))]
#[path = "../fixed_in_transform_1372_tests.rs"]
mod fixed_in_transform_1372_tests;

#[cfg(test)]
#[path = "../filter_hit_2719_tests.rs"]
mod filter_hit_2719_tests;
