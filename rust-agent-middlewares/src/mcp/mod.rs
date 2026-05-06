pub mod auth_store;
pub mod callback_server;
pub mod client;
pub mod config;
pub mod middleware;
pub mod oauth_flow;
pub mod resource_tool;
pub mod tool_bridge;
pub mod transport;

pub use auth_store::{AuthStoreError, FileCredentialStore, PerServerCredentialStore};
pub use callback_server::{parse_code_from_url, CallbackError, OAuthCallbackServer};
pub use client::{
    ClientStatus, McpClientHandle, McpClientPool, McpInitStatus, McpPoolError, OAuthStatus,
    ServerInfo,
};
pub use config::{
    expand_env_vars_with_context, expand_server_config_with_context, load_merged_config,
    remove_server_from_config, server_config_hash, set_server_disabled, ConfigSource,
    McpConfigError, McpConfigFile, McpServerConfig, OAuthConfig,
};
pub use middleware::McpMiddleware;
pub use oauth_flow::{OAuthCallbackResult, OAuthFlowError, OAuthFlowEvent, OAuthFlowManager};
pub use resource_tool::McpResourceTool;
pub use rmcp::model::{Resource, Tool};
pub use tool_bridge::{assemble_tool_pool, build_tool_bridges, McpToolBridge, ToolCallError};
pub use transport::{TransportConfig, TransportError};
