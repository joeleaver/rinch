//! Rinch Test Harness — "Playwright for Rinch"
//!
//! An MCP server that enables automated testing of rinch applications.
//! When included in a rinch app, it exposes tools for screenshots, DOM
//! inspection, and input injection that Claude can use directly.
//!
//! # Architecture
//!
//! The harness uses a channel-based design:
//! - MCP tools send [`TestCommand`]s through a channel
//! - The rinch event loop receives commands and executes them
//! - Results are sent back through a oneshot channel per command
//!
//! # Usage
//!
//! The harness is integrated into the rinch runtime automatically when
//! the `test-harness` feature is enabled and `RINCH_TEST=1` is set.
//!
//! To connect Claude to the running app, configure `.claude/settings.json`:
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "rinch-test": {
//!       "command": "cargo",
//!       "args": ["run", "-p", "hello-rinch-dom", "--features", "test-harness"],
//!       "env": { "RINCH_TEST": "1" }
//!     }
//!   }
//! }
//! ```

pub mod commands;
pub mod mcp_server;

pub use commands::{
    CommandReceiver, CommandSender, TestCommand, TestCommandKind, TestCommandResult,
};
pub use mcp_server::start_mcp_server;

use std::sync::{Arc, mpsc};

/// Create a test harness channel pair.
///
/// The `notify` callback is called after each command is sent, to wake
/// the event loop. Typically this sends a `RinchNativeEvent::TestCommand`
/// via the `EventLoopProxy`.
pub fn create_harness(
    notify: impl Fn() + Send + Sync + 'static,
) -> (CommandSender, CommandReceiver) {
    let (tx, rx) = mpsc::channel();
    let sender = CommandSender::new(tx, Some(Arc::new(notify)));
    (sender, CommandReceiver(rx))
}
