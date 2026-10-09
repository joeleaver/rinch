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

/// Whether this browser has `Node.moveBefore`, the move that keeps a node's
/// state (issue #1483). rinch-web moves a connected node with it where it
/// exists, and such a move keeps the focus and fires no focus event
/// (measured in Chrome 153); elsewhere a moved node loses the focus.
#[allow(dead_code)]
pub fn has_move_before() -> bool {
    js_sys::eval("typeof Element.prototype.moveBefore === 'function'")
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
