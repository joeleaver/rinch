use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{ErrorData as McpError, tool, tool_handler, tool_router};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::client::{DebugClient, DebugCommandKind, DebugResult};
use crate::discovery;

#[derive(Clone)]
pub struct RinchMcpServer {
    client: std::sync::Arc<Mutex<Option<DebugClient>>>,
    tool_router: ToolRouter<Self>,
    spawned_processes: std::sync::Arc<Mutex<HashMap<u32, std::process::Child>>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectParams {
    /// App name or PID to connect to. Leave empty to auto-connect to the only running app.
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SelectorParams {
    /// CSS-like selector: tag name, [attr], or [attr=value]
    pub selector: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NodeIdParams {
    /// The numeric node ID
    pub id: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClickParams {
    /// X coordinate in pixels
    pub x: f64,
    /// Y coordinate in pixels
    pub y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MouseMoveParams {
    /// X coordinate in pixels
    pub x: f64,
    /// Y coordinate in pixels
    pub y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MouseDownParams {
    /// X coordinate in pixels
    pub x: f64,
    /// Y coordinate in pixels
    pub y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MouseUpParams {
    /// X coordinate in pixels
    pub x: f64,
    /// Y coordinate in pixels
    pub y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScrollParams {
    /// X coordinate where the scroll occurs
    pub x: f64,
    /// Y coordinate where the scroll occurs
    pub y: f64,
    /// Horizontal scroll delta in pixels (positive = right)
    #[serde(default)]
    pub delta_x: f64,
    /// Vertical scroll delta in pixels (positive = scroll down, negative = scroll up)
    pub delta_y: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TypeTextParams {
    /// The text to type
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct KeyPressParams {
    /// Key name: ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Home, End, Enter, Backspace, Delete
    pub key: String,
    /// Shift modifier
    #[serde(default)]
    pub shift: bool,
    /// Ctrl modifier
    #[serde(default)]
    pub ctrl: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LaunchAppParams {
    /// Cargo package name to run (e.g. "hello-rinch-dom" or "ui-zoo-desktop")
    pub package: String,
    /// Additional cargo arguments (e.g. "--features debug")
    #[serde(default)]
    pub extra_args: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CaretPositionParams {
    /// The DOM node ID
    pub node_id: u64,
    /// Byte offset in the node's text content
    pub byte_offset: u64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GlyphBoundsParams {
    /// The DOM node ID
    pub node_id: u64,
    /// Byte offset in the node's text content
    pub byte_offset: u64,
}

#[tool_router]
impl RinchMcpServer {
    pub fn new() -> Self {
        Self {
            client: std::sync::Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
            spawned_processes: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ensure we have a connection, auto-connecting if needed
    fn ensure_connected(&self) -> Result<(), String> {
        let mut client = self.client.lock().unwrap();
        if client.is_some() {
            return Ok(());
        }

        let apps = discovery::list_apps();
        if apps.is_empty() {
            return Err("No rinch applications found. Make sure your app has rinch-debug enabled and is running.".into());
        }
        if apps.len() == 1 {
            let app = &apps[0];
            let c = DebugClient::connect(app.port)?;
            tracing::info!("Auto-connected to {} (PID {})", c.app_name, c.pid);
            *client = Some(c);
            return Ok(());
        }

        Err(format!(
            "Multiple rinch apps found. Use the 'connect' tool to choose one:\n{}",
            apps.iter()
                .map(|a| format!("  - {} (PID {}, port {})", a.app_name, a.pid, a.port))
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }

    fn execute_command(&self, cmd: DebugCommandKind) -> Result<DebugResult, String> {
        self.ensure_connected()?;
        let client = self.client.lock().unwrap();
        client.as_ref().unwrap().execute(cmd)
    }

    #[tool(description = "List all running rinch applications that have debug enabled.")]
    async fn list_apps(&self) -> Result<CallToolResult, McpError> {
        let apps = discovery::list_apps();
        if apps.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No rinch applications found. Make sure your app has rinch-debug enabled and is running.",
            )]));
        }
        let info: Vec<serde_json::Value> = apps
            .iter()
            .map(|a| {
                serde_json::json!({
                    "app_name": a.app_name,
                    "pid": a.pid,
                    "port": a.port,
                    "started_at": a.started_at,
                })
            })
            .collect();
        let text = serde_json::to_string_pretty(&info).unwrap();
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(description = "Connect to a specific rinch application by name or PID.")]
    async fn connect(&self, params: Parameters<ConnectParams>) -> Result<CallToolResult, McpError> {
        let apps = discovery::list_apps();
        if apps.is_empty() {
            return Ok(CallToolResult::error(vec![Content::text(
                "No rinch applications found.",
            )]));
        }

        let target = params.0.target;
        let app = if let Some(ref t) = target {
            // Try PID first, then name
            let by_pid = t
                .parse::<u32>()
                .ok()
                .and_then(|pid| apps.iter().find(|a| a.pid == pid));
            let found = by_pid.or_else(|| apps.iter().find(|a| a.app_name == *t));
            match found {
                Some(a) => a,
                None => {
                    return Ok(CallToolResult::error(vec![Content::text(format!(
                        "No app matching '{}' found",
                        t
                    ))]));
                }
            }
        } else if apps.len() == 1 {
            &apps[0]
        } else {
            return Ok(CallToolResult::error(vec![Content::text(
                "Multiple apps running. Specify a target name or PID.",
            )]));
        };

        match DebugClient::connect(app.port) {
            Ok(c) => {
                let msg = format!("Connected to {} (PID {})", c.app_name, c.pid);
                *self.client.lock().unwrap() = Some(c);
                Ok(CallToolResult::success(vec![Content::text(msg)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }

    #[tool(
        description = "Take a screenshot of the rinch application window. Returns a base64-encoded PNG image."
    )]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        match self.execute_command(DebugCommandKind::Screenshot) {
            Ok(DebugResult::Bytes { data }) => Ok(CallToolResult::success(vec![Content::image(
                data,
                "image/png",
            )])),
            Ok(DebugResult::Json { data }) => Ok(CallToolResult::success(vec![Content::text(
                data.to_string(),
            )])),
            Ok(DebugResult::Error { message }) => {
                Ok(CallToolResult::error(vec![Content::text(message)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }

    #[tool(description = "Get the full DOM tree of the rinch application as JSON.")]
    async fn dom_tree(&self) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::DomTree).await
    }

    #[tool(description = "Query DOM nodes by selector.")]
    async fn query_selector(
        &self,
        params: Parameters<SelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::QuerySelector {
            selector: params.0.selector,
        })
        .await
    }

    #[tool(description = "Get detailed information about a specific DOM node by ID.")]
    async fn get_node(&self, params: Parameters<NodeIdParams>) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::GetNode {
            id: params.0.id as usize,
        })
        .await
    }

    #[tool(description = "Get all text content within a DOM node subtree.")]
    async fn get_text_content(
        &self,
        params: Parameters<NodeIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::GetTextContent {
            id: params.0.id as usize,
        })
        .await
    }

    #[tool(description = "Simulate a mouse click at the given coordinates.")]
    async fn click(&self, params: Parameters<ClickParams>) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::Click {
            x: params.0.x as f32,
            y: params.0.y as f32,
        })
        .await
    }

    #[tool(
        description = "Simulate mouse movement to the given coordinates. Triggers hover effects."
    )]
    async fn mouse_move(
        &self,
        params: Parameters<MouseMoveParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::MouseMove {
            x: params.0.x as f32,
            y: params.0.y as f32,
        })
        .await
    }

    #[tool(description = "Simulate a mouse button press (without release) at the given coordinates. Use with mouse_move and mouse_up for drag operations.")]
    async fn mouse_down(
        &self,
        params: Parameters<MouseDownParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::MouseDown {
            x: params.0.x as f32,
            y: params.0.y as f32,
        })
        .await
    }

    #[tool(description = "Simulate a mouse button release at the given coordinates. Use after mouse_down to complete a click or drag.")]
    async fn mouse_up(
        &self,
        params: Parameters<MouseUpParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::MouseUp {
            x: params.0.x as f32,
            y: params.0.y as f32,
        })
        .await
    }

    #[tool(description = "Simulate a scroll wheel event at the given coordinates.")]
    async fn scroll(&self, params: Parameters<ScrollParams>) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::Scroll {
            x: params.0.x as f32,
            y: params.0.y as f32,
            delta_x: params.0.delta_x,
            delta_y: params.0.delta_y,
        })
        .await
    }

    #[tool(description = "Simulate typing text into the rinch application.")]
    async fn type_text(
        &self,
        params: Parameters<TypeTextParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::TypeText {
            text: params.0.text,
        })
        .await
    }

    #[tool(
        description = "Simulate a key press (for special keys like arrows, Enter, Backspace, etc)."
    )]
    async fn key_press(
        &self,
        params: Parameters<KeyPressParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::KeyPress {
            key: params.0.key,
            shift: params.0.shift,
            ctrl: params.0.ctrl,
        })
        .await
    }

    #[tool(description = "Get computed CSS styles for a specific DOM node.")]
    async fn get_computed_styles(
        &self,
        params: Parameters<NodeIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::GetComputedStyles {
            id: params.0.id as usize,
        })
        .await
    }

    #[tool(description = "Wait for the next render frame.")]
    async fn wait_frame(&self) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::WaitFrame).await
    }

    #[tool(description = "Close the connected rinch application. The app will exit gracefully.")]
    async fn close_app(&self) -> Result<CallToolResult, McpError> {
        // Get the connected app's PID before we clear the client
        let pid = {
            let client = self.client.lock().unwrap();
            client.as_ref().map(|c| c.pid)
        };

        // Try graceful TCP close first
        let tcp_result = self.execute_command(DebugCommandKind::CloseApp);

        // Clear the client connection
        *self.client.lock().unwrap() = None;

        // Give the app a moment to exit gracefully
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // Kill and reap the spawned process if we launched it
        if let Some(pid) = pid {
            let mut procs = self.spawned_processes.lock().unwrap();
            if let Some(mut child) = procs.remove(&pid) {
                match child.try_wait() {
                    Ok(Some(_)) => {} // Already exited
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait(); // Reap zombie
                    }
                }
            }
        }

        match tcp_result {
            Ok(DebugResult::Json { data }) => Ok(CallToolResult::success(vec![Content::text(
                format!("App closed: {}", data),
            )])),
            Ok(DebugResult::Error { message }) => {
                Ok(CallToolResult::error(vec![Content::text(message)]))
            }
            Err(_) => Ok(CallToolResult::success(vec![Content::text("App closed")])),
            _ => Ok(CallToolResult::success(vec![Content::text("App closed")])),
        }
    }

    #[tool(
        description = "Disconnect from the currently connected rinch application without closing it."
    )]
    async fn disconnect(&self) -> Result<CallToolResult, McpError> {
        let mut client = self.client.lock().unwrap();
        if client.is_some() {
            *client = None;
            Ok(CallToolResult::success(vec![Content::text(
                "Disconnected from app.",
            )]))
        } else {
            Ok(CallToolResult::success(vec![Content::text(
                "No app connected.",
            )]))
        }
    }

    #[tool(
        description = "Launch a rinch application via cargo run. Waits for it to register with the debug server, then auto-connects."
    )]
    async fn launch_app(
        &self,
        params: Parameters<LaunchAppParams>,
    ) -> Result<CallToolResult, McpError> {
        use std::process::Command;

        let package = &params.0.package;

        // Build cargo run command
        let mut cmd = Command::new("cargo");
        cmd.arg("run").arg("-p").arg(package);
        if let Some(extra) = &params.0.extra_args {
            for arg in extra.split_whitespace() {
                cmd.arg(arg);
            }
        }

        // Set env to enable debug
        cmd.env("RINCH_DEBUG", "1");

        // Forward DISPLAY for headless environments (Xvfb)
        if let Ok(display) = std::env::var("DISPLAY") {
            cmd.env("DISPLAY", display);
        }

        // Spawn detached
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();

                // Store the child process handle
                self.spawned_processes.lock().unwrap().insert(pid, child);

                // Wait for the app to register with debug server (poll for up to 30 seconds)
                for _ in 0..60 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let apps = discovery::list_apps();
                    if let Some(app) = apps.iter().find(|a| a.pid == pid) {
                        // Try to connect
                        match DebugClient::connect(app.port) {
                            Ok(c) => {
                                let msg = format!(
                                    "Launched {} (PID {}) and connected on port {}",
                                    c.app_name, pid, app.port
                                );
                                *self.client.lock().unwrap() = Some(c);
                                return Ok(CallToolResult::success(vec![Content::text(msg)]));
                            }
                            Err(_) => continue,
                        }
                    }
                }

                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Launched {} (PID {}) but could not auto-connect. The app may not have debug enabled. Use 'list_apps' to check.",
                    package, pid
                ))]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to launch {}: {}",
                package, e
            ))])),
        }
    }

    #[tool(
        description = "Get the screen position of a text caret at a given byte offset in a node"
    )]
    async fn get_caret_position(
        &self,
        params: Parameters<CaretPositionParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::GetCaretPosition {
            node_id: params.0.node_id as usize,
            byte_offset: params.0.byte_offset as usize,
        })
        .await
    }

    #[tool(
        description = "Get the bounding box of a glyph cluster at a given byte offset in a node"
    )]
    async fn get_glyph_bounds(
        &self,
        params: Parameters<GlyphBoundsParams>,
    ) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::GetGlyphBounds {
            node_id: params.0.node_id as usize,
            byte_offset: params.0.byte_offset as usize,
        })
        .await
    }
}

impl RinchMcpServer {
    async fn forward_json_command(
        &self,
        cmd: DebugCommandKind,
    ) -> Result<CallToolResult, McpError> {
        match self.execute_command(cmd) {
            Ok(DebugResult::Json { data }) => {
                let text = serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string());
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Ok(DebugResult::Bytes { data }) => Ok(CallToolResult::success(vec![Content::text(
                format!("<{} bytes>", data.len()),
            )])),
            Ok(DebugResult::Error { message }) => {
                Ok(CallToolResult::error(vec![Content::text(message)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}

#[tool_handler]
impl rmcp::ServerHandler for RinchMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Rinch debug server - inspect DOM, take screenshots, inject input into any running rinch application".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

impl Drop for RinchMcpServer {
    fn drop(&mut self) {
        let mut procs = self.spawned_processes.lock().unwrap();
        for (pid, mut child) in procs.drain() {
            tracing::info!("Cleaning up spawned process {}", pid);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
