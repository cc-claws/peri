use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;

use rmcp::model::{Resource, Tool};
use rmcp::service::{Peer, RoleClient, RunningService};

use super::config::McpServerConfig;
use super::transport::TransportConfig;

/// MCP 客户端连接状态
#[derive(Debug, Clone, PartialEq)]
pub enum ClientStatus {
    Connected,
    Failed(String),
    Disconnected,
}

/// MCP 连接池初始化状态
#[derive(Debug, Clone, PartialEq)]
pub enum McpInitStatus {
    /// 初始化尚未开始
    Pending,
    /// 正在连接中
    Initializing { connected: usize, total: usize },
    /// 初始化完成
    Ready { total: usize },
    /// 初始化失败
    Failed(String),
}

/// 单个 MCP 服务器的详细信息（用于 TUI 面板展示）
#[derive(Debug)]
pub struct ServerInfo {
    pub name: String,
    pub transport_type: String,
    pub status: ClientStatus,
    pub tool_count: usize,
    pub resource_count: usize,
}

/// 连接池级别错误
#[derive(Debug, Error)]
pub enum McpPoolError {
    #[error("MCP 服务器 \"{server}\" 连接失败: {reason}")]
    ConnectionFailed { server: String, reason: String },
    #[error("MCP 服务器 \"{server}\" 工具发现失败: {reason}")]
    ToolDiscoveryFailed { server: String, reason: String },
    #[error("MCP 服务器 \"{server}\" 未连接 (状态: {status:?})")]
    NotConnected { server: String, status: ClientStatus },
    #[error("MCP 服务器 \"{server}\" 调用超时")]
    CallTimeout { server: String },
}

/// 单个 MCP 服务器的客户端句柄
#[derive(Clone)]
pub struct McpClientHandle {
    pub name: String,
    /// None 表示未连接（Failed/Disconnected 状态）
    pub peer: Option<Peer<RoleClient>>,
    pub tools: Vec<Tool>,
    pub resources: Vec<Resource>,
    pub status: ClientStatus,
}

/// MCP 客户端连接池
pub struct McpClientPool {
    clients: parking_lot::RwLock<HashMap<String, Arc<McpClientHandle>>>,
    services: tokio::sync::Mutex<HashMap<String, RunningService<RoleClient, ()>>>,
    configs: parking_lot::RwLock<HashMap<String, McpServerConfig>>,
}

const STDIO_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl McpClientPool {
    /// 创建空的 pending 状态连接池（用于后台初始化）
    pub fn new_pending() -> Self {
        Self {
            clients: parking_lot::RwLock::new(HashMap::new()),
            services: tokio::sync::Mutex::new(HashMap::new()),
            configs: parking_lot::RwLock::new(HashMap::new()),
        }
    }

    /// 创建空连接池（用于测试）
    #[cfg(test)]
    pub fn new_empty() -> Self {
        Self::new_pending()
    }

    /// 后台初始化所有 MCP 服务器连接
    pub async fn run_initialize(
        pool: Arc<Self>,
        cwd: &Path,
        status_tx: tokio::sync::watch::Sender<McpInitStatus>,
    ) {
        let config = super::load_merged_config(cwd);

        let total = config.mcp_servers.len();
        if total == 0 {
            let _ = status_tx.send(McpInitStatus::Ready { total: 0 });
            return;
        }

        // 保存配置用于重连
        for (name, server_config) in &config.mcp_servers {
            pool.configs
                .write()
                .insert(name.clone(), server_config.clone());
        }

        let _ = status_tx.send(McpInitStatus::Initializing {
            connected: 0,
            total,
        });

        let mut connected = 0usize;
        for (name, server_config) in &config.mcp_servers {
            let transport_config = match TransportConfig::try_from(server_config) {
                Ok(tc) => tc,
                Err(e) => {
                    tracing::warn!(server = %name, error = %e, "MCP 服务器传输层构建失败，跳过");
                    Self::insert_failed(&pool, name, format!("传输层构建失败: {e}"));
                    continue;
                }
            };

            let timeout = if matches!(transport_config, TransportConfig::StreamableHttp { .. }) {
                HTTP_CONNECT_TIMEOUT
            } else {
                STDIO_CONNECT_TIMEOUT
            };

            let connect_result = match transport_config {
                TransportConfig::Stdio {
                    ref command,
                    ref args,
                    ref env,
                } => {
                    let child_result = spawn_stdio_transport(command, args, env);
                    match child_result {
                        Ok(transport) => {
                            tokio::time::timeout(
                                timeout,
                                rmcp::service::serve_client((), transport),
                            )
                            .await
                        }
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP stdio 子进程启动失败");
                            Self::insert_failed(&pool, name, format!("stdio 子进程启动失败: {e}"));
                            continue;
                        }
                    }
                }
                TransportConfig::StreamableHttp {
                    ref url,
                    ref headers,
                } => {
                    let transport = build_http_transport(url, headers);
                    tokio::time::timeout(timeout, rmcp::service::serve_client((), transport)).await
                }
            };

            match connect_result {
                Ok(Ok(running_service)) => {
                    let tools = match running_service.list_all_tools().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP 服务器工具发现失败");
                            vec![]
                        }
                    };
                    let resources = match running_service.list_all_resources().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP 服务器资源发现失败");
                            vec![]
                        }
                    };

                    tracing::info!(
                        server = %name,
                        tools_count = tools.len(),
                        resources_count = resources.len(),
                        "MCP 服务器连接成功"
                    );

                    let peer = running_service.peer().clone();
                    let handle = Arc::new(McpClientHandle {
                        name: name.clone(),
                        peer: Some(peer),
                        tools,
                        resources,
                        status: ClientStatus::Connected,
                    });
                    pool.clients.write().insert(name.clone(), handle);
                    pool.services
                        .lock()
                        .await
                        .insert(name.clone(), running_service);
                    connected += 1;
                    let _ = status_tx.send(McpInitStatus::Initializing {
                        connected,
                        total,
                    });
                }
                Ok(Err(e)) => {
                    tracing::warn!(server = %name, error = %e, "MCP 服务器连接失败，跳过");
                    Self::insert_failed(&pool, name, e.to_string());
                }
                Err(_) => {
                    tracing::warn!(server = %name, timeout_secs = timeout.as_secs(), "MCP 服务器连接超时，跳过");
                    Self::insert_failed(&pool, name, format!("连接超时 ({}s)", timeout.as_secs()));
                }
            }
        }

        if connected == 0 && total > 0 {
            let failed_names: Vec<String> = pool
                .clients
                .read()
                .iter()
                .filter(|(_, h)| matches!(h.status, ClientStatus::Failed(_)))
                .map(|(n, h)| {
                    if let ClientStatus::Failed(reason) = &h.status {
                        format!("{}: {}", n, reason)
                    } else {
                        n.clone()
                    }
                })
                .collect();
            let msg = format!("{} 个服务器连接失败: {}", total, failed_names.join("; "));
            let _ = status_tx.send(McpInitStatus::Failed(msg));
        } else {
            let _ = status_tx.send(McpInitStatus::Ready { total: connected });
        }
    }

    fn insert_failed(pool: &Arc<Self>, name: &str, reason: String) {
        pool.clients.write().insert(
            name.to_string(),
            Arc::new(McpClientHandle {
                name: name.to_string(),
                peer: None,
                tools: vec![],
                resources: vec![],
                status: ClientStatus::Failed(reason),
            }),
        );
    }

    /// 重连指定的 MCP 服务器
    pub async fn reconnect(self: &Arc<Self>, server_name: &str) -> Result<(), McpPoolError> {
        let server_config = {
            let configs = self.configs.read();
            configs.get(server_name).cloned().ok_or_else(|| {
                McpPoolError::NotConnected {
                    server: server_name.to_string(),
                    status: ClientStatus::Disconnected,
                }
            })?
        };

        // 关闭旧连接
        if let Some(mut service) = self.services.lock().await.remove(server_name) {
            let _ = service.close_with_timeout(SHUTDOWN_TIMEOUT).await;
        }
        self.clients.write().remove(server_name);

        // 重新连接
        let transport_config = TransportConfig::try_from(&server_config).map_err(|e| {
            McpPoolError::ConnectionFailed {
                server: server_name.to_string(),
                reason: format!("传输层构建失败: {e}"),
            }
        })?;

        let timeout = if matches!(transport_config, TransportConfig::StreamableHttp { .. }) {
            HTTP_CONNECT_TIMEOUT
        } else {
            STDIO_CONNECT_TIMEOUT
        };

        let connect_result = match &transport_config {
            TransportConfig::Stdio {
                command,
                args,
                env,
            } => {
                let child_result = spawn_stdio_transport(command, args, env);
                match child_result {
                    Ok(transport) => {
                        tokio::time::timeout(timeout, rmcp::service::serve_client((), transport)).await
                    }
                    Err(e) => {
                        Self::insert_failed(self, server_name, format!("stdio 子进程启动失败: {e}"));
                        return Err(McpPoolError::ConnectionFailed {
                            server: server_name.to_string(),
                            reason: format!("stdio 子进程启动失败: {e}"),
                        });
                    }
                }
            }
            TransportConfig::StreamableHttp { url, headers } => {
                let transport = build_http_transport(url, headers);
                tokio::time::timeout(timeout, rmcp::service::serve_client((), transport)).await
            }
        };

        match connect_result {
            Ok(Ok(running_service)) => {
                let tools = running_service.list_all_tools().await.map_err(|e| {
                    McpPoolError::ToolDiscoveryFailed {
                        server: server_name.to_string(),
                        reason: e.to_string(),
                    }
                })?;
                let resources = running_service.list_all_resources().await.unwrap_or_default();

                let peer = running_service.peer().clone();
                let handle = Arc::new(McpClientHandle {
                    name: server_name.to_string(),
                    peer: Some(peer),
                    tools,
                    resources,
                    status: ClientStatus::Connected,
                });
                self.clients
                    .write()
                    .insert(server_name.to_string(), handle);
                self.services
                    .lock()
                    .await
                    .insert(server_name.to_string(), running_service);
                tracing::info!(server = %server_name, "MCP 服务器重连成功");
                Ok(())
            }
            Ok(Err(e)) => {
                Self::insert_failed(self, server_name, e.to_string());
                Err(McpPoolError::ConnectionFailed {
                    server: server_name.to_string(),
                    reason: e.to_string(),
                })
            }
            Err(_) => {
                let msg = format!("连接超时 ({}s)", timeout.as_secs());
                Self::insert_failed(self, server_name, msg.clone());
                Err(McpPoolError::ConnectionFailed {
                    server: server_name.to_string(),
                    reason: msg,
                })
            }
        }
    }

    /// 删除指定的 MCP 服务器（从内存中移除，不修改配置文件）
    pub async fn remove_server(self: &Arc<Self>, server_name: &str) {
        self.clients.write().remove(server_name);
        if let Some(mut service) = self.services.lock().await.remove(server_name) {
            let _ = service.close_with_timeout(SHUTDOWN_TIMEOUT).await;
        }
        self.configs.write().remove(server_name);
    }

    /// 获取所有服务器的摘要信息（用于 TUI 面板渲染）
    pub fn server_infos(&self) -> Vec<ServerInfo> {
        let configs = self.configs.read();
        self.clients
            .read()
            .values()
            .map(|handle| {
                let transport_type = configs
                    .get(&handle.name)
                    .map(|cfg| {
                        if cfg.command.is_some() {
                            "stdio"
                        } else if cfg.url.is_some() {
                            "http"
                        } else {
                            "unknown"
                        }
                    })
                    .unwrap_or("unknown")
                    .to_string();
                ServerInfo {
                    name: handle.name.clone(),
                    transport_type,
                    status: handle.status.clone(),
                    tool_count: handle.tools.len(),
                    resource_count: handle.resources.len(),
                }
            })
            .collect()
    }

    /// 获取指定服务器的工具列表
    pub fn get_tools(&self, server_name: &str) -> Vec<Tool> {
        self.clients
            .read()
            .get(server_name)
            .map(|h| h.tools.clone())
            .unwrap_or_default()
    }

    /// 获取指定服务器的资源列表
    pub fn get_resources(&self, server_name: &str) -> Vec<Resource> {
        self.clients
            .read()
            .get(server_name)
            .map(|h| h.resources.clone())
            .unwrap_or_default()
    }

    /// 一次性初始化所有 MCP 服务器连接（同步阻塞，保留向后兼容）
    pub async fn initialize(cwd: &Path) -> Self {
        let config = super::load_merged_config(cwd);
        let pool = Arc::new(Self::new_pending());

        for (name, server_config) in &config.mcp_servers {
            pool.configs
                .write()
                .insert(name.clone(), server_config.clone());
        }

        for (name, server_config) in &config.mcp_servers {
            let transport_config = match TransportConfig::try_from(server_config) {
                Ok(tc) => tc,
                Err(e) => {
                    tracing::warn!(server = %name, error = %e, "MCP 服务器传输层构建失败，跳过");
                    Self::insert_failed(&pool, name, format!("传输层构建失败: {e}"));
                    continue;
                }
            };

            let timeout = if matches!(transport_config, TransportConfig::StreamableHttp { .. }) {
                HTTP_CONNECT_TIMEOUT
            } else {
                STDIO_CONNECT_TIMEOUT
            };

            let connect_result = match transport_config {
                TransportConfig::Stdio {
                    ref command,
                    ref args,
                    ref env,
                } => {
                    let child_result = spawn_stdio_transport(command, args, env);
                    match child_result {
                        Ok(transport) => {
                            tokio::time::timeout(
                                timeout,
                                rmcp::service::serve_client((), transport),
                            )
                            .await
                        }
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP stdio 子进程启动失败");
                            Self::insert_failed(&pool, name, format!("stdio 子进程启动失败: {e}"));
                            continue;
                        }
                    }
                }
                TransportConfig::StreamableHttp {
                    ref url,
                    ref headers,
                } => {
                    let transport = build_http_transport(url, headers);
                    tokio::time::timeout(timeout, rmcp::service::serve_client((), transport)).await
                }
            };

            match connect_result {
                Ok(Ok(running_service)) => {
                    let tools = match running_service.list_all_tools().await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP 服务器工具发现失败");
                            vec![]
                        }
                    };
                    let resources = match running_service.list_all_resources().await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!(server = %name, error = %e, "MCP 服务器资源发现失败");
                            vec![]
                        }
                    };

                    tracing::info!(
                        server = %name,
                        tools_count = tools.len(),
                        resources_count = resources.len(),
                        "MCP 服务器连接成功"
                    );

                    let peer = running_service.peer().clone();
                    let handle = Arc::new(McpClientHandle {
                        name: name.clone(),
                        peer: Some(peer),
                        tools,
                        resources,
                        status: ClientStatus::Connected,
                    });
                    pool.clients.write().insert(name.clone(), handle);
                    pool.services
                        .lock()
                        .await
                        .insert(name.clone(), running_service);
                }
                Ok(Err(e)) => {
                    tracing::warn!(server = %name, error = %e, "MCP 服务器连接失败，跳过");
                    Self::insert_failed(&pool, name, e.to_string());
                }
                Err(_) => {
                    tracing::warn!(server = %name, timeout_secs = timeout.as_secs(), "MCP 服务器连接超时，跳过");
                    Self::insert_failed(&pool, name, format!("连接超时 ({}s)", timeout.as_secs()));
                }
            }
        }

        Arc::try_unwrap(pool).unwrap_or_else(|arc| {
            let p = arc.as_ref();
            Self {
                clients: parking_lot::RwLock::new(p.clients.read().clone()),
                services: tokio::sync::Mutex::new(HashMap::new()),
                configs: parking_lot::RwLock::new(p.configs.read().clone()),
            }
        })
    }

    /// 获取指定名称的客户端句柄
    pub fn get_client(&self, name: &str) -> Option<Arc<McpClientHandle>> {
        self.clients.read().get(name).cloned()
    }

    /// 获取所有已连接的客户端句柄
    pub fn get_all_clients(&self) -> Vec<Arc<McpClientHandle>> {
        self.clients
            .read()
            .values()
            .filter(|c| matches!(c.status, ClientStatus::Connected))
            .cloned()
            .collect()
    }

    /// 判断是否有任何已连接的 server 提供资源
    pub fn has_resources(&self) -> bool {
        self.clients.read().values().any(|c| {
            matches!(c.status, ClientStatus::Connected) && !c.resources.is_empty()
        })
    }

    /// 获取所有已连接 server 的资源摘要
    pub fn resource_summary(&self) -> String {
        let mut lines = Vec::new();
        for client in self.clients.read().values() {
            if matches!(client.status, ClientStatus::Connected) && !client.resources.is_empty() {
                lines.push(format!(
                    "- server \"{}\": {} ({} resources)",
                    client.name,
                    client
                        .resources
                        .iter()
                        .map(|r| r.raw.uri.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    client.resources.len()
                ));
            }
        }
        lines.join("\n")
    }

    /// 关闭所有 MCP 服务器连接
    pub async fn shutdown(&self) {
        // 先记录关闭日志并更新状态
        let names: Vec<String> = self.clients.read().keys().cloned().collect();
        for name in &names {
            let mut clients = self.clients.write();
            if let Some(client) = clients.get_mut(name) {
                if matches!(client.status, ClientStatus::Connected) {
                    tracing::info!(server = %name, "关闭 MCP 服务器连接");
                }
                let handle = Arc::make_mut(client);
                handle.status = ClientStatus::Disconnected;
                handle.peer = None;
            }
        }
        let mut services = self.services.lock().await;
        for (name, mut service) in services.drain() {
            match service.close_with_timeout(SHUTDOWN_TIMEOUT).await {
                Ok(Some(reason)) => tracing::debug!(?reason, %name, "MCP 连接已关闭"),
                Ok(None) => tracing::warn!(%name, "MCP 连接关闭超时"),
                Err(e) => tracing::warn!(error = %e, %name, "MCP 连接关闭异常"),
            }
        }
    }
}

/// 创建 stdio transport（使用 tokio::process::Command）
fn spawn_stdio_transport(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
) -> std::io::Result<rmcp::transport::child_process::TokioChildProcess> {
    let mut child = tokio::process::Command::new(command);
    child.args(args);
    child.envs(env);
    child.stdin(std::process::Stdio::piped());
    child.stdout(std::process::Stdio::piped());
    child.stderr(std::process::Stdio::piped());
    rmcp::transport::child_process::TokioChildProcess::new(child)
}

/// 创建 HTTP transport，将自定义 headers（如 Authorization）注入 transport config
fn build_http_transport(
    url: &str,
    headers: &HashMap<String, String>,
) -> rmcp::transport::StreamableHttpClientTransport<reqwest::Client> {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let mut config = StreamableHttpClientTransportConfig::with_uri(url);

    let mut custom_headers = std::collections::HashMap::new();
    for (key, value) in headers {
        match reqwest::header::HeaderName::try_from(key.as_str()) {
            Ok(name) => match reqwest::header::HeaderValue::from_str(value) {
                Ok(val) => {
                    custom_headers.insert(name, val);
                }
                Err(e) => {
                    tracing::warn!(header = %key, error = %e, "MCP HTTP header 值无效，跳过");
                }
            },
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "MCP HTTP header 名称无效，跳过");
            }
        }
    }

    if !custom_headers.is_empty() {
        config = config.custom_headers(custom_headers);
    }

    let client = reqwest::Client::new();
    rmcp::transport::StreamableHttpClientTransport::with_client(client, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_get_all_clients_filters_disconnected() {
        let pool = McpClientPool::new_empty();
        assert!(pool.get_all_clients().is_empty());
    }

    #[test]
    fn test_pool_has_no_resources() {
        let pool = McpClientPool::new_empty();
        assert!(!pool.has_resources());
    }

    #[test]
    fn test_resource_summary_empty() {
        let pool = McpClientPool::new_empty();
        assert!(pool.resource_summary().is_empty());
    }

    #[test]
    fn test_client_status_equality() {
        assert_eq!(ClientStatus::Connected, ClientStatus::Connected);
        assert_ne!(
            ClientStatus::Failed("a".to_string()),
            ClientStatus::Failed("b".to_string())
        );
        assert_ne!(ClientStatus::Connected, ClientStatus::Disconnected);
    }

    #[test]
    fn test_mcp_init_status_equality() {
        assert_eq!(McpInitStatus::Pending, McpInitStatus::Pending);
        assert_eq!(
            McpInitStatus::Initializing {
                connected: 1,
                total: 2
            },
            McpInitStatus::Initializing {
                connected: 1,
                total: 2
            }
        );
        assert_ne!(
            McpInitStatus::Initializing {
                connected: 1,
                total: 2
            },
            McpInitStatus::Initializing {
                connected: 2,
                total: 2
            }
        );
        assert_eq!(
            McpInitStatus::Ready { total: 3 },
            McpInitStatus::Ready { total: 3 }
        );
        assert_ne!(
            McpInitStatus::Ready { total: 3 },
            McpInitStatus::Ready { total: 4 }
        );
        assert_ne!(
            McpInitStatus::Failed("a".to_string()),
            McpInitStatus::Failed("b".to_string())
        );
    }

    #[test]
    fn test_new_pending_creates_empty_pool() {
        let pool = McpClientPool::new_pending();
        assert!(pool.clients.read().is_empty());
        assert!(pool.configs.read().is_empty());
    }

    #[test]
    fn test_server_infos_empty_pool() {
        let pool = McpClientPool::new_pending();
        assert!(pool.server_infos().is_empty());
    }

    #[tokio::test]
    async fn test_insert_failed_creates_failed_handle() {
        let pool = Arc::new(McpClientPool::new_pending());
        McpClientPool::insert_failed(&pool, "test-server", "timeout".into());
        let infos = pool.server_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "test-server");
        assert_eq!(
            infos[0].status,
            ClientStatus::Failed("timeout".to_string())
        );
    }

    #[tokio::test]
    async fn test_remove_server() {
        let pool = Arc::new(McpClientPool::new_pending());
        pool.clients.write().insert(
            "server-a".to_string(),
            Arc::new(McpClientHandle {
                name: "server-a".to_string(),
                peer: None,
                tools: vec![],
                resources: vec![],
                status: ClientStatus::Connected,
            }),
        );
        pool.clients.write().insert(
            "server-b".to_string(),
            Arc::new(McpClientHandle {
                name: "server-b".to_string(),
                peer: None,
                tools: vec![],
                resources: vec![],
                status: ClientStatus::Failed("error".to_string()),
            }),
        );
        pool.remove_server("server-a").await;
        let infos = pool.server_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].name, "server-b");
    }

    #[tokio::test]
    async fn test_get_tools_resources() {
        let pool = McpClientPool::new_pending();
        // 插入一个没有 tools 和 resources 的 handle
        pool.clients.write().insert(
            "s".to_string(),
            Arc::new(McpClientHandle {
                name: "s".to_string(),
                peer: None,
                tools: vec![],
                resources: vec![],
                status: ClientStatus::Connected,
            }),
        );
        assert!(pool.get_tools("s").is_empty());
        assert!(pool.get_resources("s").is_empty());
        assert!(pool.get_tools("nonexistent").is_empty());
        assert!(pool.get_resources("nonexistent").is_empty());
    }
}
