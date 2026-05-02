pub mod config;
pub mod transport;
pub mod client;
pub mod tool_bridge;
pub mod resource_tool;
pub mod middleware;

pub use config::{
    load_merged_config, remove_server_from_config, McpConfigError, McpConfigFile, McpServerConfig,
};
pub use transport::{TransportConfig, TransportError};
pub use client::{ClientStatus, McpClientHandle, McpClientPool, McpInitStatus, McpPoolError, ServerInfo};
pub use tool_bridge::{build_tool_bridges, McpToolBridge, ToolCallError};
pub use rmcp::model::{Resource, Tool};
pub use resource_tool::McpResourceTool;
pub use middleware::McpMiddleware;
