#![allow(clippy::collapsible_if)]
// See the identical attribute (and its rationale) in `rinch-core/src/lib.rs`
// (#598, #372): Android's target spec has no native ELF thread-local support,
// so `std::sys::thread_local` erases the const/non-const distinction
// `missing_const_for_thread_local` checks for before the lint ever runs, and it
// fires on this target on statics that are already `const { .. }`.
#![cfg_attr(target_os = "android", allow(clippy::missing_const_for_thread_local))]
//! rinch-dom: Custom layout engine for Rinch.
//!
//! Uses a direct Taffy + Parley + Vello pipeline.
//! Implements the [`DomDocument`] trait from rinch-core.

pub mod animation;
pub mod attr_name;
mod calc_layout;
pub mod computed_style;
mod dom_impl;
pub mod fonts;
pub mod hit_cache;
pub mod html_parser;
pub mod html_serializer;
mod ifc;
pub mod ifc_scope;
pub mod image_cache;
pub mod layout;
mod layout_engine;
pub mod node;
mod out_of_flow;
pub mod paint;
pub mod perf;
pub mod select;
pub mod stacking;
mod style_resolution;
pub mod stylesheet;
pub mod stylo_impl;
pub mod testing;
pub mod text_query;
pub mod transition;

pub use computed_style::ComputedStyle;
pub use dom_impl::RinchDocument;
pub use ifc::TreeCheckVerdict;
pub use node::{
    DirtyFlags, DisplayMode, ElementData, IfcTextRange, InlineFlowRole, InlineLayout, LayoutResult,
    Node, NodeContext, NodeKind, NodeTree, TextData, TextMeasure, first_legend_child,
    node_is_disabled, node_is_disabled_in_tree, tag_is_disableable,
};

/// TEMPORARY (#1036 workflow self-test, reverted before merge): an API that
/// exists only on this branch, so a benchmark calling it cannot build on `main`.
#[doc(hidden)]
pub fn perf_self_test_1036(n: u64) -> u64 {
    (0..n).fold(0, |a, i| a.wrapping_mul(31).wrapping_add(i))
}
