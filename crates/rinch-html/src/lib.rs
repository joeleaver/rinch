//! Showing HTML you did not write: mail, feed items, help pages from a
//! server, rich chat.
//!
//! - [`clean`] reduces a document to what is safe to put into the app's DOM:
//!   an allow-list (no scripts, forms, frames or event attributes), its
//!   `<style>` sheets applied inline and dropped, inline styles filtered so
//!   nothing escapes its box, every picture decided by the app's
//!   [`Policy`], links limited to the schemes the app allows.
//! - [`HtmlView`] shows a cleaned document and tells the app which link was
//!   clicked (`onlink`), through `rinch_core::click_link`.
//! - [`text`] gives a cleaned document's text: what a reader sees, and apart
//!   from it the text the document hides (for a program reading the text,
//!   such as an AI agent, to be told about).
//!
//! Enabled in the `rinch` facade by its `html` feature, as `rinch::html`.

mod bound;
mod clean;
mod depth;
mod links;
mod media;
mod style;
mod text;
mod view;

pub use clean::{Cleaned, PictureRule, Policy, clean, clean_at};
pub use links::{LinkCheck, TooltipFn, Tooltips, Warning, check as check_link, default_tooltip};
pub use media::{Queries, queries, signature};
pub use text::{Text, text, text_on};
pub use view::HtmlView;
