//! Signaling error types.

use std::fmt;

/// Errors from signaling operations.
#[derive(Debug)]
pub enum SignalingError {
    /// The connection was closed.
    Closed,
    /// Failed to connect to the signaling server.
    ConnectionFailed(String),
    /// Failed to send or receive a message.
    Io(String),
    /// Failed to serialize or deserialize a message.
    Serialization(String),
}

impl fmt::Display for SignalingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalingError::Closed => write!(f, "signaling channel closed"),
            SignalingError::ConnectionFailed(msg) => {
                write!(f, "signaling connection failed: {msg}")
            }
            SignalingError::Io(msg) => write!(f, "signaling I/O error: {msg}"),
            SignalingError::Serialization(msg) => {
                write!(f, "signaling serialization error: {msg}")
            }
        }
    }
}

impl std::error::Error for SignalingError {}
