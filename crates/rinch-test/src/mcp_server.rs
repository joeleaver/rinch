//! MCP server implementation exposing rinch test tools.

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::schemars;
use rmcp::{ErrorData as McpError, tool, tool_router};
use serde::Deserialize;

use crate::commands::{CommandSender, TestCommandKind, TestCommandResult};

/// MCP server that exposes rinch testing tools.
#[derive(Clone)]
pub struct RinchTestServer {
    cmd_tx: CommandSender,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
pub struct SelectorParams {
    /// CSS-like selector: tag name, [attr], or [attr=value]
    pub selector: String,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
pub struct NodeIdParams {
    /// The numeric node ID
    pub id: u64,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
pub struct ClickParams {
    /// X coordinate in pixels
    pub x: f64,
    /// Y coordinate in pixels
    pub y: f64,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
pub struct TypeTextParams {
    /// The text to type
    pub text: String,
}

#[tool_router]
impl RinchTestServer {
    pub fn new(cmd_tx: CommandSender) -> Self {
        Self {
            cmd_tx,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Take a screenshot of the rinch application window. Returns a base64-encoded PNG image."
    )]
    async fn screenshot(&self) -> Result<CallToolResult, McpError> {
        match self.cmd_tx.execute(TestCommandKind::Screenshot) {
            Ok(TestCommandResult::Bytes(png_bytes)) => {
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
                Ok(CallToolResult::success(vec![Content::image(
                    b64,
                    "image/png",
                )]))
            }
            Ok(TestCommandResult::Error(e)) => Ok(CallToolResult::error(vec![Content::text(e)])),
            Ok(TestCommandResult::Json(v)) => {
                Ok(CallToolResult::success(vec![Content::text(v.to_string())]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }

    #[tool(
        description = "Get the full DOM tree of the rinch application as JSON. Shows element tags, attributes, text content, and layout bounds (x, y, width, height) for every node."
    )]
    async fn dom_tree(&self) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::DomTree).await
    }

    #[tool(
        description = "Query DOM nodes by selector. Supports: tag name (e.g. 'div'), attribute existence ('[data-rid]'), attribute value ('[data-rid=5]'). Returns matching nodes with layout bounds."
    )]
    async fn query_selector(
        &self,
        params: Parameters<SelectorParams>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::QuerySelector(params.0.selector))
            .await
    }

    #[tool(
        description = "Get detailed information about a specific DOM node by its ID, including tag, attributes, layout bounds, children, and text content."
    )]
    async fn get_node(&self, params: Parameters<NodeIdParams>) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::GetNode(params.0.id as usize))
            .await
    }

    #[tool(description = "Get all text content within a DOM node subtree.")]
    async fn get_text_content(
        &self,
        params: Parameters<NodeIdParams>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::GetTextContent(params.0.id as usize))
            .await
    }

    #[tool(
        description = "Simulate a mouse click at the given screen coordinates within the rinch window."
    )]
    async fn click(&self, params: Parameters<ClickParams>) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::Click(params.0.x as f32, params.0.y as f32))
            .await
    }

    #[tool(description = "Simulate typing text into the rinch application.")]
    async fn type_text(
        &self,
        params: Parameters<TypeTextParams>,
    ) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::TypeText(params.0.text))
            .await
    }

    #[tool(
        description = "Wait for the rinch application to complete its next render frame. Use this after clicking or typing to ensure the UI has updated."
    )]
    async fn wait_frame(&self) -> Result<CallToolResult, McpError> {
        self.execute_json_command(TestCommandKind::WaitFrame).await
    }
}

impl rmcp::ServerHandler for RinchTestServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Rinch test harness - take screenshots, inspect DOM, inject input".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

impl RinchTestServer {
    async fn execute_json_command(
        &self,
        kind: TestCommandKind,
    ) -> Result<CallToolResult, McpError> {
        match self.cmd_tx.execute(kind) {
            Ok(TestCommandResult::Json(v)) => {
                let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
                Ok(CallToolResult::success(vec![Content::text(text)]))
            }
            Ok(TestCommandResult::Bytes(b)) => Ok(CallToolResult::success(vec![Content::text(
                format!("<{} bytes>", b.len()),
            )])),
            Ok(TestCommandResult::Error(e)) => Ok(CallToolResult::error(vec![Content::text(e)])),
            Err(e) => Ok(CallToolResult::error(vec![Content::text(e)])),
        }
    }
}

/// Start the MCP server on a background thread using stdio transport.
///
/// This spawns a tokio runtime and runs the MCP server, reading from stdin
/// and writing to stdout. The rinch app process becomes the MCP server.
pub fn start_mcp_server(cmd_tx: CommandSender) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for MCP server");

        rt.block_on(async move {
            let server = RinchTestServer::new(cmd_tx);
            let (stdin, stdout) = rmcp::transport::io::stdio();

            tracing::info!("Rinch test MCP server starting on stdio");

            match rmcp::ServiceExt::serve(server, (stdin, stdout)).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(e) => {
                    tracing::error!("MCP server failed to start: {:?}", e);
                }
            }
        });
    });
}
