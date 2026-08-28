//! Rinch capture - connects to running rinch app via debug protocol.
//!
//! The wire types are `rinch_debug::protocol`'s own, not a local mirror. A
//! mirror is what produced the two protocol bugs this client used to have: the
//! `params` key omitted from an adjacently-tagged struct variant, and errors
//! looked for under a bare `error` key when they arrive as
//! `{"type": "error", "message": ...}`. Typed round-tripping makes both
//! unrepresentable.

use rinch_debug::protocol::{
    DebugCommandKind, DebugResult, HandshakeRequest, Request, Response, read_frame, write_frame,
};
use serde::Deserialize;
use std::net::TcpStream;
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("No rinch app found - is an app running with debug feature enabled?")]
    NoAppFound,

    #[error("Failed to connect to rinch app: {0}")]
    ConnectionFailed(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Command failed: {0}")]
    CommandFailed(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Discovery info for a running rinch app.
///
/// A subset of `rinch_debug::discovery::DiscoveryEntry` — deserialized
/// permissively so an older or newer app's file still parses.
#[derive(Debug, Deserialize)]
struct AppInfo {
    port: u16,
    pid: u32,
    #[allow(dead_code)]
    app_name: String,
}

/// Rinch debug client for capturing screenshots and DOM.
pub struct RinchCapture {
    stream: TcpStream,
    request_id: u64,
}

impl RinchCapture {
    /// Connect to a running rinch app.
    ///
    /// Discovers apps via the same directory `rinch-debug` writes to.
    pub fn connect() -> Result<Self, CaptureError> {
        let app = discover_app()?;
        Self::connect_to_port(app.port)
    }

    /// Connect to a specific port.
    pub fn connect_to_port(port: u16) -> Result<Self, CaptureError> {
        let addr = format!("127.0.0.1:{}", port);
        let mut stream = TcpStream::connect(&addr)
            .map_err(|e| CaptureError::ConnectionFailed(format!("{}: {}", addr, e)))?;

        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;

        // Handshake
        let handshake = HandshakeRequest {
            protocol: "rinch-debug".to_string(),
            version: 1,
        };
        let bytes = serde_json::to_vec(&handshake)
            .map_err(|e| CaptureError::ProtocolError(e.to_string()))?;
        write_frame(&mut stream, &bytes)?;
        let _response = read_frame(&mut stream)?;

        Ok(Self {
            stream,
            request_id: 0,
        })
    }

    /// Send one command and return its result, or the server's error message.
    fn execute(&mut self, command: DebugCommandKind) -> Result<DebugResult, CaptureError> {
        self.request_id += 1;
        let request = Request {
            id: self.request_id,
            command,
        };
        let bytes =
            serde_json::to_vec(&request).map_err(|e| CaptureError::ProtocolError(e.to_string()))?;
        write_frame(&mut self.stream, &bytes)?;

        let frame = read_frame(&mut self.stream)?;
        let response: Response = serde_json::from_slice(&frame)
            .map_err(|e| CaptureError::ProtocolError(e.to_string()))?;

        match response.result {
            DebugResult::Error { message } => Err(CaptureError::CommandFailed(message)),
            other => Ok(other),
        }
    }

    /// Capture a screenshot as PNG bytes.
    pub fn screenshot(&mut self) -> Result<Vec<u8>, CaptureError> {
        match self.execute(DebugCommandKind::Screenshot)? {
            DebugResult::Bytes { data } => {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .map_err(|e| CaptureError::ProtocolError(format!("base64 decode: {}", e)))
            }
            _ => Err(CaptureError::CommandFailed(
                "Screenshot returned no data".into(),
            )),
        }
    }

    /// Get the DOM tree as JSON.
    ///
    /// `verbose` pulls each node's computed styles, without which the exported
    /// HTML is an unstyled skeleton the browser renders blank. `max_depth`
    /// overrides the server's shallow default of 3.
    pub fn dom_tree(&mut self) -> Result<serde_json::Value, CaptureError> {
        match self.execute(DebugCommandKind::DomTree {
            max_depth: Some(1000),
            root_id: None,
            verbose: true,
        })? {
            DebugResult::Json { data } => Ok(data),
            _ => Err(CaptureError::CommandFailed(
                "DomTree returned no data".into(),
            )),
        }
    }

    /// Click at coordinates.
    pub fn click(&mut self, x: f64, y: f64) -> Result<(), CaptureError> {
        self.execute(DebugCommandKind::Click {
            x: x as f32,
            y: y as f32,
            button: None,
        })?;
        Ok(())
    }

    /// Wait for next render frame.
    pub fn wait_frame(&mut self) -> Result<(), CaptureError> {
        self.execute(DebugCommandKind::WaitFrame)?;
        Ok(())
    }
}

/// Discover a running rinch app.
fn discover_app() -> Result<AppInfo, CaptureError> {
    let debug_dir = rinch_debug::discovery::discovery_dir();

    if !debug_dir.exists() {
        return Err(CaptureError::NoAppFound);
    }

    // Look for .json files
    for entry in std::fs::read_dir(&debug_dir).map_err(|_| CaptureError::NoAppFound)? {
        let entry = entry.map_err(|_| CaptureError::NoAppFound)?;
        let path = entry.path();

        if path.extension().map(|e| e == "json").unwrap_or(false)
            && let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(info) = serde_json::from_str::<AppInfo>(&contents)
        {
            // Verify process is still running
            if is_process_running(info.pid) {
                return Ok(info);
            } else {
                // Cleanup stale file
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    Err(CaptureError::NoAppFound)
}

/// Check if a process is running.
fn is_process_running(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}
