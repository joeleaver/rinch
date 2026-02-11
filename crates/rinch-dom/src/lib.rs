//! rinch-dom: Custom layout engine for Rinch.
//!
//! Replaces blitz with a direct Taffy + Parley + Vello pipeline.
//! Implements the [`DomDocument`] trait from rinch-core.

pub mod computed_style;
mod dom_impl;
pub mod html_parser;
mod ifc;
pub mod layout;
mod layout_engine;
mod style_resolution;
pub mod node;
pub mod paint;
pub mod stylesheet;
pub mod stylo_impl;
pub mod testing;
pub mod text_query;

pub use computed_style::ComputedStyle;
pub use dom_impl::RinchDocument;
pub use node::{
    DirtyFlags, DisplayMode, ElementData, IfcTextRange, InlineLayout, LayoutResult, Node,
    NodeContext, NodeKind, NodeTree, TextData, TextMeasure,
};
