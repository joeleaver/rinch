// ── Free functions (platform-agnostic hit testing) ───────────────────────────

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
pub(crate) use resize::{
    detect_resize_edge, is_corner_resize, pointer_on_scrollbar_thumb, resize_direction_to_cursor,
};
#[cfg(feature = "windowing")]
pub(crate) use scroll::page_scroller;
#[cfg(test)]
pub(crate) use scroll::{
    SCROLLBAR_HIT_THICKNESS, find_horizontal_scroll_container,
    find_horizontal_scroll_container_at_point, find_scroll_container,
    find_scroll_container_at_point,
};
pub(crate) use scroll::{ScrollbarHit, find_scrollbar_hit, wheel_scroll_target};
pub(crate) use targets::{
    element_of_target, event_target_at, painted_element_box, pointer_in_node, text_node_at,
};
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
#[path = "../hit_test_prune_tests.rs"]
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
