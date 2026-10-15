//! everything-mcp-lite entry: serve the MCP server over stdio.
use everything_mcp_lite::server::EverythingTools;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // stdout is the protocol channel; nothing else may write there.
    let service = EverythingTools.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
