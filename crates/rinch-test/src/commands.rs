//! Command types for communication between MCP server and rinch runtime.

use std::sync::{Arc, mpsc};

/// Sender half — cloned into MCP tool handlers.
///
/// Includes an optional notify callback that wakes the event loop
/// after sending a command.
#[derive(Clone)]
pub struct CommandSender {
    tx: mpsc::Sender<TestCommand>,
    notify: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Receiver half — consumed by the rinch event loop.
pub struct CommandReceiver(pub mpsc::Receiver<TestCommand>);

/// A command sent from an MCP tool to the rinch event loop.
pub struct TestCommand {
    /// What to do.
    pub kind: TestCommandKind,
    /// Where to send the result.
    pub response_tx: mpsc::Sender<TestCommandResult>,
}

/// The result of a test command.
pub enum TestCommandResult {
    /// JSON response.
    Json(serde_json::Value),
    /// Binary data (e.g., PNG screenshot).
    Bytes(Vec<u8>),
    /// Error message.
    Error(String),
}

/// The different commands the MCP server can send.
pub enum TestCommandKind {
    /// Capture a screenshot. Returns PNG bytes.
    Screenshot,
    /// Get the full DOM tree as JSON.
    DomTree,
    /// Query for nodes matching a selector.
    QuerySelector(String),
    /// Get details of a specific node by ID.
    GetNode(usize),
    /// Get text content of a node subtree.
    GetTextContent(usize),
    /// Inject a mouse click at (x, y).
    Click(f32, f32),
    /// Inject keyboard text input.
    TypeText(String),
    /// Wait for the next render frame.
    WaitFrame,
}

impl CommandSender {
    /// Create a new sender with the given channel and optional notify callback.
    pub fn new(tx: mpsc::Sender<TestCommand>, notify: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self { tx, notify }
    }

    /// Send a command and wait for the result.
    ///
    /// After sending the command, calls the notify callback (if set)
    /// to wake the event loop so it processes the command promptly.
    pub fn execute(&self, kind: TestCommandKind) -> Result<TestCommandResult, String> {
        let (response_tx, response_rx) = mpsc::channel();
        let cmd = TestCommand { kind, response_tx };
        self.tx
            .send(cmd)
            .map_err(|_| "Runtime shut down".to_string())?;

        // Wake the event loop
        if let Some(notify) = &self.notify {
            notify();
        }

        response_rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| "Timeout waiting for response".to_string())
    }
}
