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
//!
//! Enabled in the `rinch` facade by its `html` feature, as `rinch::html`.

mod clean;
mod media;
mod view;

pub use clean::{Cleaned, PictureRule, Policy, clean, clean_at};
pub use media::signature;
pub use view::HtmlView;
