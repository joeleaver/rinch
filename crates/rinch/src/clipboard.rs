//! Cross-platform clipboard support for text and images.
//!
//! This module re-exports the [`rinch_clipboard`] crate which provides
//! clipboard operations on both native platforms and WASM.
//!
//! # Example
//!
//! ```ignore
//! use rinch::clipboard::{copy_text, paste_text, has_text};
//!
//! // Copy text to clipboard
//! copy_text("Hello, clipboard!").unwrap();
//!
//! // Check if clipboard has text
//! if has_text() {
//!     // Paste text from clipboard
//!     if let Ok(text) = paste_text() {
//!         println!("Clipboard: {}", text);
//!     }
//! }
//! ```

pub use rinch_clipboard::*;
