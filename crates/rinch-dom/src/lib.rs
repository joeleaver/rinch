#![allow(clippy::collapsible_if)]
//! rinch-dom: Custom layout engine for Rinch.
//!
//! Uses a direct Taffy + Parley + Vello pipeline.
//! Implements the [`DomDocument`] trait from rinch-core.

pub mod animation;
pub mod computed_style;
mod dom_impl;
pub mod fonts;
pub mod html_parser;
pub mod html_serializer;
mod ifc;
pub mod image_cache;
pub mod layout;
mod layout_engine;
pub mod node;
mod out_of_flow;
pub mod paint;
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
pub use node::{
    DirtyFlags, DisplayMode, ElementData, IfcTextRange, InlineLayout, LayoutResult, Node,
    NodeContext, NodeKind, NodeTree, TextData, TextMeasure,
};
