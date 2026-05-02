use rust_agent_middlewares::mcp::{ClientStatus, Resource, ServerInfo, Tool};

/// MCP 管理面板
pub struct McpPanel {
    /// 服务器列表信息
    pub servers: Vec<ServerInfo>,
    /// 当前选中索引
    pub cursor: usize,
    /// 当前视图层级
    pub view: McpPanelView,
    /// 确认删除弹窗（server name），None 表示非确认状态
    pub confirm_delete: Option<String>,
}

/// 面板视图层级
pub enum McpPanelView {
    /// 服务器列表
    ServerList,
    /// 工具列表
    ToolList {
        server_name: String,
        tools: Vec<Tool>,
    },
    /// 资源列表
    ResourceList {
        server_name: String,
        resources: Vec<Resource>,
    },
}

impl McpPanel {
    pub fn new(servers: Vec<ServerInfo>) -> Self {
        Self {
            servers,
            cursor: 0,
            view: McpPanelView::ServerList,
            confirm_delete: None,
        }
    }
}

impl crate::app::App {
    pub fn mcp_panel_move_up(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            panel.cursor = panel.cursor.saturating_sub(1);
        }
    }

    pub fn mcp_panel_move_down(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            let max = panel.servers.len().saturating_sub(1);
            if panel.cursor < max {
                panel.cursor += 1;
            }
        }
    }

    pub fn mcp_panel_enter(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            if !matches!(panel.view, McpPanelView::ServerList) {
                return;
            }
            if panel.cursor >= panel.servers.len() {
                return;
            }
            let name = panel.servers[panel.cursor].name.clone();
            let tools = self
                .mcp_pool
                .as_ref()
                .map(|p| p.get_tools(&name))
                .unwrap_or_default();
            panel.view = McpPanelView::ToolList {
                server_name: name,
                tools,
            };
            panel.cursor = 0;
        }
    }

    pub fn mcp_panel_back(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            if matches!(panel.view, McpPanelView::ServerList) {
                return;
            }
            panel.view = McpPanelView::ServerList;
            panel.cursor = 0;
        }
    }

    pub fn mcp_panel_tab(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            match &panel.view {
                McpPanelView::ToolList { server_name, .. } => {
                    let name = server_name.clone();
                    let resources = self
                        .mcp_pool
                        .as_ref()
                        .map(|p| p.get_resources(&name))
                        .unwrap_or_default();
                    panel.view = McpPanelView::ResourceList {
                        server_name: name,
                        resources,
                    };
                    panel.cursor = 0;
                }
                McpPanelView::ResourceList { server_name, .. } => {
                    let name = server_name.clone();
                    let tools = self
                        .mcp_pool
                        .as_ref()
                        .map(|p| p.get_tools(&name))
                        .unwrap_or_default();
                    panel.view = McpPanelView::ToolList {
                        server_name: name,
                        tools,
                    };
                    panel.cursor = 0;
                }
                McpPanelView::ServerList => {}
            }
        }
    }

    pub fn mcp_panel_request_delete(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            if !matches!(panel.view, McpPanelView::ServerList) {
                return;
            }
            if panel.cursor >= panel.servers.len() {
                return;
            }
            panel.confirm_delete = Some(panel.servers[panel.cursor].name.clone());
        }
    }

    pub fn mcp_panel_confirm_delete(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            let name = match panel.confirm_delete.take() {
                Some(n) => n,
                None => return,
            };
            // 异步断开连接
            if let Some(pool) = self.mcp_pool.clone() {
                let name_clone = name.clone();
                tokio::spawn(async move {
                    pool.remove_server(&name_clone).await;
                });
            }
            // 持久化删除配置
            let _ = rust_agent_middlewares::mcp::remove_server_from_config(
                std::path::Path::new(&self.cwd),
                &name,
            );
            // 刷新列表
            panel.servers = self
                .mcp_pool
                .as_ref()
                .map(|p| p.server_infos())
                .unwrap_or_default();
            if panel.cursor >= panel.servers.len() && !panel.servers.is_empty() {
                panel.cursor = panel.servers.len() - 1;
            }
            if panel.servers.is_empty() {
                self.mcp_panel = None;
            }
        }
    }

    pub fn mcp_panel_cancel_delete(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            panel.confirm_delete = None;
        }
    }

    pub fn mcp_panel_reconnect(&mut self) {
        if let Some(ref mut panel) = self.mcp_panel {
            if !matches!(panel.view, McpPanelView::ServerList) {
                return;
            }
            if panel.cursor >= panel.servers.len() {
                return;
            }
            let status = &panel.servers[panel.cursor].status;
            if !matches!(status, ClientStatus::Failed(_)) {
                return;
            }
            let name = panel.servers[panel.cursor].name.clone();
            if let Some(pool) = self.mcp_pool.clone() {
                tokio::spawn(async move {
                    let _ = pool.reconnect(&name).await;
                });
            }
            // 刷新列表以反映重连状态
            panel.servers = self
                .mcp_pool
                .as_ref()
                .map(|p| p.server_infos())
                .unwrap_or_default();
        }
    }

    pub fn mcp_panel_close(&mut self) {
        self.mcp_panel = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server_info(name: &str, status: ClientStatus) -> ServerInfo {
        ServerInfo {
            name: name.to_string(),
            transport_type: "stdio".to_string(),
            status,
            tool_count: 0,
            resource_count: 0,
        }
    }

    #[tokio::test]
    async fn test_mcp_panel_new() {
        let panel = McpPanel::new(vec![]);
        assert_eq!(panel.cursor, 0);
        assert!(matches!(panel.view, McpPanelView::ServerList));
        assert!(panel.confirm_delete.is_none());

        let servers = vec![
            make_server_info("a", ClientStatus::Connected),
            make_server_info("b", ClientStatus::Failed("err".into())),
            make_server_info("c", ClientStatus::Connected),
        ];
        let panel = McpPanel::new(servers);
        assert_eq!(panel.servers.len(), 3);
    }

    #[tokio::test]
    async fn test_mcp_panel_move_cursor() {
        let (mut app, _handle) = crate::app::App::new_headless(80, 24);
        let servers = vec![
            make_server_info("a", ClientStatus::Connected),
            make_server_info("b", ClientStatus::Connected),
            make_server_info("c", ClientStatus::Connected),
        ];
        app.mcp_panel = Some(McpPanel::new(servers));

        // move_up 5 次，应停在 0
        for _ in 0..5 {
            app.mcp_panel_move_up();
        }
        assert_eq!(app.mcp_panel.as_ref().unwrap().cursor, 0);

        // move_down 5 次，应停在 2
        for _ in 0..5 {
            app.mcp_panel_move_down();
        }
        assert_eq!(app.mcp_panel.as_ref().unwrap().cursor, 2);
    }

    #[tokio::test]
    async fn test_mcp_panel_close() {
        let (mut app, _handle) = crate::app::App::new_headless(80, 24);
        app.mcp_panel = Some(McpPanel::new(vec![]));
        assert!(app.mcp_panel.is_some());
        app.mcp_panel_close();
        assert!(app.mcp_panel.is_none());
    }

    #[tokio::test]
    async fn test_mcp_panel_request_cancel_delete() {
        let (mut app, _handle) = crate::app::App::new_headless(80, 24);
        app.mcp_panel = Some(McpPanel::new(vec![
            make_server_info("test-srv", ClientStatus::Connected),
        ]));

        app.mcp_panel_request_delete();
        assert_eq!(
            app.mcp_panel.as_ref().unwrap().confirm_delete,
            Some("test-srv".to_string())
        );

        app.mcp_panel_cancel_delete();
        assert!(app.mcp_panel.as_ref().unwrap().confirm_delete.is_none());
    }
}
