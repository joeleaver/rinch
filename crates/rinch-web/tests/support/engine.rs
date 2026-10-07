//! Which browser engine a test is running in
//! (`#[path = "support/engine.rs"] mod engine;`).
//!
//! For positive controls that pin what a browser itself answers. Those answers
//! differ between engines — that difference is usually why the product code
//! under test exists — so a control states each engine's measured answer
//! rather than asserting one engine's in all of them.

/// Firefox (Gecko). Its user agent carries a `Gecko/<build date>` token;
/// Chromium's and WebKit's say `like Gecko`, with no slash.
#[allow(dead_code)]
pub fn is_gecko() -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Gecko/"))
}
