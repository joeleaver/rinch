//! Total, attr-aware serialization of the document model.
//!
//! Everything here is **schema-derived** and **total**: a node or mark either
//! round-trips faithfully, or the boundary returns an error — content is never
//! silently dropped or rendered to a garbage tag. This structurally closes #59
//! and its whole class (the old serializer dropped marks, had no hr/image, and
//! modelled `hard_break` as a string).
//!
//! - [`doc_json`] (feature `serde`): the durable [`DocNode`] save/load wire shape.
//! - [`html`]: schema-driven HTML serialize (copy) / parse (paste), whitelist-driven.
//! - [`markdown`] (feature `markdown`): markdown I/O via pulldown-cmark.
//! - [`text`]: plain-text clipboard interchange (the lossy `text/plain` fall-back).

#[cfg(feature = "serde")]
pub mod doc_json;
pub mod html;
#[cfg(feature = "markdown")]
pub mod markdown;
pub mod text;

#[cfg(feature = "serde")]
pub use doc_json::{DocMark, DocNode, JsonAttr};
pub use html::{mark_dom_tag, node_dom_tag, node_to_html, slice_from_html, slice_to_html};
#[cfg(feature = "markdown")]
pub use markdown::{doc_from_markdown, doc_to_markdown};
pub use text::{slice_from_text, slice_to_text};
