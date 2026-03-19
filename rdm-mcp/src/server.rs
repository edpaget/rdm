use std::path::PathBuf;

use rmcp::{
    ServerHandler, ServiceExt,
    model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo},
    transport::io::stdio,
};

/// MCP server backed by an rdm plan repo.
struct RdmMcpServer {
    #[allow(dead_code)]
    plan_root: PathBuf,
}

impl RdmMcpServer {
    fn new(plan_root: PathBuf) -> Self {
        Self { plan_root }
    }
}

impl ServerHandler for RdmMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::default(),
            server_info: Implementation {
                name: "rdm-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                ..Default::default()
            },
            instructions: Some("MCP server for managing rdm plan repos.".into()),
        }
    }
}

/// Start the MCP server on stdin/stdout.
///
/// # Errors
///
/// Returns an error if the transport fails to initialize or the server
/// encounters a fatal I/O error.
pub async fn run(plan_root: PathBuf) -> anyhow::Result<()> {
    let server = RdmMcpServer::new(plan_root);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
