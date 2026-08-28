//! Rinch capture - connects to running rinch app via debug protocol.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};
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
    /// Discovers apps via ~/.rinch/debug/*.json files.
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
        let handshake = json!({
            "protocol": "rinch-debug",
            "version": 1
        });
        send_message(&mut stream, &handshake)?;
        let _response = receive_message(&mut stream)?;

        Ok(Self {
            stream,
            request_id: 0,
        })
    }

    /// Capture a screenshot as PNG bytes.
    pub fn screenshot(&mut self) -> Result<Vec<u8>, CaptureError> {
        self.request_id += 1;
        let cmd = json!({ "id": self.request_id, "method": "screenshot" });
        send_message(&mut self.stream, &cmd)?;

        let response = receive_message(&mut self.stream)?;

        if let Some(err) = response_error(&response) {
            return Err(CaptureError::CommandFailed(err));
        }

        // Look for bytes response: {"id": N, "type": "bytes", "data": "base64..."}
        if let Some(Value::String(response_type)) = response.get("type") {
            if response_type == "bytes" {
                if let Some(Value::String(base64_data)) = response.get("data") {
                    use base64::Engine;
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(base64_data)
                        .map_err(|e| {
                            CaptureError::ProtocolError(format!("base64 decode: {}", e))
                        })?;
                    return Ok(bytes);
                }
            }
        }

        Err(CaptureError::CommandFailed(
            "Screenshot returned no data".into(),
        ))
    }

    /// Get the DOM tree as JSON.
    pub fn dom_tree(&mut self) -> Result<Value, CaptureError> {
        self.request_id += 1;
        // `DebugCommandKind` is an adjacently-tagged enum (`tag = "method"`,
        // `content = "params"`), so a struct variant needs its `params` key
        // present even when every field defaults.
        // `verbose` pulls each node's computed styles, without which the
        // exported HTML is an unstyled skeleton the browser renders blank.
        // `max_depth` overrides the server's shallow default of 3.
        let cmd = json!({
            "id": self.request_id,
            "method": "dom_tree",
            "params": { "max_depth": 1000, "verbose": true }
        });
        send_message(&mut self.stream, &cmd)?;

        let response = receive_message(&mut self.stream)?;

        if let Some(err) = response_error(&response) {
            return Err(CaptureError::CommandFailed(err));
        }

        // Look for json response: {"id": N, "type": "json", "data": {...}}
        if let Some(Value::String(response_type)) = response.get("type") {
            if response_type == "json" {
                if let Some(data) = response.get("data") {
                    return Ok(data.clone());
                }
            }
        }

        Err(CaptureError::CommandFailed(
            "DomTree returned no data".into(),
        ))
    }

    /// Click at coordinates.
    pub fn click(&mut self, x: f64, y: f64) -> Result<(), CaptureError> {
        self.request_id += 1;
        let cmd = json!({ "id": self.request_id, "method": "click", "params": { "x": x, "y": y } });
        send_message(&mut self.stream, &cmd)?;
        let response = receive_message(&mut self.stream)?;

        if let Some(err) = response_error(&response) {
            return Err(CaptureError::CommandFailed(err));
        }

        Ok(())
    }

    /// Wait for next render frame.
    pub fn wait_frame(&mut self) -> Result<(), CaptureError> {
        self.request_id += 1;
        let cmd = json!({ "id": self.request_id, "method": "wait_frame" });
        send_message(&mut self.stream, &cmd)?;
        let response = receive_message(&mut self.stream)?;

        if let Some(err) = response_error(&response) {
            return Err(CaptureError::CommandFailed(err));
        }

        Ok(())
    }
}

/// Discover a running rinch app.
fn discover_app() -> Result<AppInfo, CaptureError> {
    let debug_dir = dirs::home_dir()
        .ok_or(CaptureError::NoAppFound)?
        .join(".rinch")
        .join("debug");

    if !debug_dir.exists() {
        return Err(CaptureError::NoAppFound);
    }

    // Look for .json files
    for entry in std::fs::read_dir(&debug_dir).map_err(|_| CaptureError::NoAppFound)? {
        let entry = entry.map_err(|_| CaptureError::NoAppFound)?;
        let path = entry.path();

        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(info) = serde_json::from_str::<AppInfo>(&contents) {
                    // Verify process is still running
                    if is_process_running(info.pid) {
                        return Ok(info);
                    } else {
                        // Cleanup stale file
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    Err(CaptureError::NoAppFound)
}

/// Check if a process is running.
fn is_process_running(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{}", pid)).exists()
}

/// Extract the message from an error response.
///
/// The server answers with `{"type": "error", "message": "..."}`; older code
/// here looked for a bare `error` key, so every failure was misreported as a
/// missing payload instead of the reason the command failed.
fn response_error(response: &Value) -> Option<String> {
    if response.get("type").and_then(Value::as_str) == Some("error") {
        return Some(
            response
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error")
                .to_string(),
        );
    }
    if let Some(Value::String(msg)) = response.get("error") {
        return Some(msg.clone());
    }
    None
}

/// Send a length-prefixed JSON message.
fn send_message(stream: &mut TcpStream, msg: &Value) -> Result<(), CaptureError> {
    let json_bytes =
        serde_json::to_vec(msg).map_err(|e| CaptureError::ProtocolError(e.to_string()))?;

    let len = json_bytes.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&json_bytes)?;
    stream.flush()?;

    Ok(())
}

/// Receive a length-prefixed JSON message.
fn receive_message(stream: &mut TcpStream) -> Result<Value, CaptureError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;

    serde_json::from_slice(&buf).map_err(|e| CaptureError::ProtocolError(e.to_string()))
}
