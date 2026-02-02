use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

mod client;
mod discovery;
mod mcp_server;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("rinch-mcp-server starting");

    let server = mcp_server::RinchMcpServer::new();
    let (stdin, stdout) = rmcp::transport::io::stdio();

    match server.serve((stdin, stdout)).await {
        Ok(running) => {
            let _ = running.waiting().await;
        }
        Err(e) => {
            tracing::error!("MCP server failed: {:?}", e);
            std::process::exit(1);
        }
    }
}
