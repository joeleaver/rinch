use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{tool, tool_router, ErrorData as McpError};
use serde::Deserialize;
use std::sync::Mutex;

use crate::client::{DebugClient, DebugCommandKind, DebugResult};
use crate::discovery;

#[derive(Clone)]
pub struct RinchMcpServer {
    client: std::sync::Arc<Mutex<Option<DebugClient>>>,
    tool_router: ToolRouter<Self>,
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
pub struct TypeTextParams {
    /// The text to type
    pub text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LaunchAppParams {
    /// Cargo package name to run (e.g. "hello-rinch-dom" or "smyeditor")
    pub package: String,
    /// Additional cargo arguments (e.g. "--features debug")
    #[serde(default)]
    pub extra_args: Option<String>,
}

#[tool_router]
impl RinchMcpServer {
    pub fn new() -> Self {
        Self {
            client: std::sync::Arc::new(Mutex::new(None)),
            tool_router: Self::tool_router(),
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
                "No rinch applications found. Make sure your app has rinch-debug enabled and is running."
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
    async fn connect(
        &self,
        params: Parameters<ConnectParams>,
    ) -> Result<CallToolResult, McpError> {
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
                    ))]))
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
    async fn get_node(
        &self,
        params: Parameters<NodeIdParams>,
    ) -> Result<CallToolResult, McpError> {
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

    #[tool(description = "Wait for the next render frame.")]
    async fn wait_frame(&self) -> Result<CallToolResult, McpError> {
        self.forward_json_command(DebugCommandKind::WaitFrame).await
    }

    #[tool(description = "Close the connected rinch application. The app will exit gracefully.")]
    async fn close_app(&self) -> Result<CallToolResult, McpError> {
        match self.execute_command(DebugCommandKind::CloseApp) {
            Ok(DebugResult::Json { data }) => {
                // Clear the client connection since the app is closing
                *self.client.lock().unwrap() = None;
                Ok(CallToolResult::success(vec![Content::text(
                    format!("App closing: {}", data),
                )]))
            }
            Ok(DebugResult::Error { message }) => {
                Ok(CallToolResult::error(vec![Content::text(message)]))
            }
            Err(e) => {
                // Connection may have been lost because app exited
                *self.client.lock().unwrap() = None;
                Ok(CallToolResult::success(vec![Content::text(
                    format!("App closed (connection lost: {})", e),
                )]))
            }
            _ => {
                *self.client.lock().unwrap() = None;
                Ok(CallToolResult::success(vec![Content::text("App closed")]))
            }
        }
    }

    #[tool(description = "Disconnect from the currently connected rinch application without closing it.")]
    async fn disconnect(&self) -> Result<CallToolResult, McpError> {
        let mut client = self.client.lock().unwrap();
        if client.is_some() {
            *client = None;
            Ok(CallToolResult::success(vec![Content::text("Disconnected from app.")]))
        } else {
            Ok(CallToolResult::success(vec![Content::text("No app connected.")]))
        }
    }

    #[tool(description = "Launch a rinch application via cargo run. Waits for it to register with the debug server, then auto-connects.")]
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

        // Spawn detached
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();

                // Wait for the app to register with debug server (poll for up to 30 seconds)
                let mut connected = false;
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
                                connected = true;
                                return Ok(CallToolResult::success(vec![Content::text(msg)]));
                            }
                            Err(_) => continue,
                        }
                    }
                }

                if !connected {
                    Ok(CallToolResult::success(vec![Content::text(format!(
                        "Launched {} (PID {}) but could not auto-connect. The app may not have debug enabled. Use 'list_apps' to check.",
                        package, pid
                    ))]))
                } else {
                    unreachable!()
                }
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "Failed to launch {}: {}",
                package, e
            ))])),
        }
    }
}

impl RinchMcpServer {
    async fn forward_json_command(
        &self,
        cmd: DebugCommandKind,
    ) -> Result<CallToolResult, McpError> {
        match self.execute_command(cmd) {
            Ok(DebugResult::Json { data }) => {
                let text =
                    serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string());
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
