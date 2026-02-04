//! Web backend for rinch using web-sys and wasm-bindgen.
//!
//! This crate provides WASM implementations of the rinch-platform traits,
//! enabling rinch applications to run in web browsers.

pub mod window;
pub mod renderer;
pub mod event_loop;

pub use window::WebWindow;
pub use renderer::WebRenderer;
pub use event_loop::{WebEventLoop, WebWaker};
