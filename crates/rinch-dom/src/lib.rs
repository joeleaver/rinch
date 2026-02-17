//! rinch-dom: Custom layout engine for Rinch.
//!
//! Uses a direct Taffy + Parley + Vello pipeline.
//! Implements the [`DomDocument`] trait from rinch-core.

pub mod computed_style;
mod dom_impl;
pub mod html_parser;
pub mod image_cache;
mod ifc;
pub mod layout;
mod layout_engine;
pub mod node;
pub mod paint;
mod style_resolution;
pub mod stylesheet;
pub mod stylo_impl;
pub mod testing;
pub mod text_query;
pub mod transition;

pub use computed_style::ComputedStyle;
pub use dom_impl::RinchDocument;
pub use node::{
    DirtyFlags, DisplayMode, ElementData, IfcTextRange, InlineLayout, LayoutResult, Node,
    NodeContext, NodeKind, NodeTree, TextData, TextMeasure,
};
