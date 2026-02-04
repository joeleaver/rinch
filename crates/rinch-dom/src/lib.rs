//! rinch-dom: Custom layout engine for Rinch.
//!
//! Replaces blitz with a direct Taffy + Parley + Vello pipeline.
//! Implements the [`DomDocument`] trait from rinch-core.

pub mod node;
pub mod layout;
pub mod paint;
pub mod stylesheet;
pub mod testing;
pub mod computed_style;
pub mod stylo_impl;
pub mod text_query;
mod dom_impl;

pub use node::{Node, NodeKind, NodeTree, ElementData, TextData, DirtyFlags, LayoutResult, NodeContext, TextMeasure, DisplayMode, InlineLayout};
pub use dom_impl::RinchDocument;
pub use computed_style::ComputedStyle;
