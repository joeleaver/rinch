//! DOM traits: [`IntoNode`], [`DomDocument`] and its supertraits, and the
//! types their methods take.
//!
//! - `into_node.rs`: [`IntoNode`].
//! - `types.rs`: the value types and constants, and [`next_doc_key`].
//! - `document.rs`: the [`DomDocument`] trait, which is its five supertraits:
//!   - `tree.rs`: [`DomTree`], the node tree.
//!   - `content.rs`: [`DomContent`], a node's text, attributes and styles.
//!   - `focus.rs`: [`DomFocus`], focus and text-control selection.
//!   - `layout.rs`: [`DomLayout`], layout and text geometry.
//!   - `scroll.rs`: [`DomScroll`], scrolling and the page scroll lock.

mod content;
mod document;
mod focus;
mod into_node;
mod layout;
mod scroll;
mod tree;
mod types;

pub use content::DomContent;
pub use document::DomDocument;
pub use focus::DomFocus;
pub use into_node::IntoNode;
pub use layout::DomLayout;
pub use scroll::DomScroll;
pub use tree::DomTree;
pub use types::{
    CARET_WIDTH, CaretAffinity, FocusIntoPolicy, GeneratedFlatRange, GlyphBounds, HighlightText,
    NodeFont, SelectionDirection, next_doc_key,
};
