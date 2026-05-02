use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    Frame,
};

use perihelion_widgets::{BorderedPanel, ScrollState, ScrollableArea};

use crate::app::{McpPanelView, App};
use crate::ui::main_ui::highlight_line_spans;
use crate::ui::theme;

/// MCP 管理面板渲染
pub(crate) fn render_mcp_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(panel) = &app.mcp_panel else {
        return;
    };

    let title = match &panel.view {
        McpPanelView::ServerList => " MCP 服务器 ".to_string(),
        McpPanelView::ToolList { server_name, .. } => {
            format!(" {} — 工具列表 ", server_name)
        }
        McpPanelView::ResourceList { server_name, .. } => {
            format!(" {} — 资源列表 ", server_name)
        }
    };

    let inner = BorderedPanel::new(Span::styled(
        title,
        Style::default()
            .fg(theme::THINKING)
            .add_modifier(Modifier::BOLD),
    ))
    .border_style(Style::default().fg(theme::BORDER))
    .render(f, area);

    let mut lines: Vec<Line> = Vec::new();

    match &panel.view {
        McpPanelView::ServerList => {
            for (i, server) in panel.servers.iter().enumerate() {
                let is_cursor = i == panel.cursor;
                let cursor_char = if is_cursor { "❯ " } else { "  " };

                let status_icon = match &server.status {
                    rust_agent_middlewares::mcp::ClientStatus::Connected => "●",
                    _ => "○",
                };
                let status_style = match &server.status {
                    rust_agent_middlewares::mcp::ClientStatus::Connected => {
                        Style::default().fg(theme::SAGE)
                    }
                    _ => Style::default().fg(theme::ERROR),
                };

                let status_text = match &server.status {
                    rust_agent_middlewares::mcp::ClientStatus::Connected => {
                        "Connected".to_string()
                    }
                    rust_agent_middlewares::mcp::ClientStatus::Failed(reason) => {
                        let truncated: String = reason.chars().take(20).collect();
                        if reason.len() > 20 {
                            format!("Failed({})…", truncated)
                        } else {
                            format!("Failed({})", truncated)
                        }
                    }
                    rust_agent_middlewares::mcp::ClientStatus::Disconnected => {
                        "Disconnected".to_string()
                    }
                };

                let count_text = match &server.status {
                    rust_agent_middlewares::mcp::ClientStatus::Connected => {
                        format!(
                            "{} tools, {} resources",
                            server.tool_count, server.resource_count
                        )
                    }
                    _ => "—".to_string(),
                };

                let name_style = if is_cursor {
                    Style::default()
                        .fg(theme::TEXT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::TEXT)
                };

                lines.push(Line::from(vec![
                    Span::styled(cursor_char.to_string(), Style::default().fg(theme::THINKING)),
                    Span::styled(status_icon.to_string(), status_style),
                    Span::styled(" ", Style::default()),
                    Span::styled(
                        format!("{:<20}", server.name),
                        name_style,
                    ),
                    Span::styled(
                        format!("[{}] ", server.transport_type),
                        Style::default().fg(theme::MUTED),
                    ),
                    Span::styled(
                        format!("{:<16} ", status_text),
                        status_style,
                    ),
                    Span::styled(count_text, Style::default().fg(theme::MUTED)),
                ]));
            }

            if panel.servers.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  （无 MCP 服务器配置，请编辑 .mcp.json 或 settings.json）",
                    Style::default().fg(theme::MUTED),
                )));
            }
        }
        McpPanelView::ToolList { tools, .. } => {
            for (i, tool) in tools.iter().enumerate() {
                let is_cursor = i == panel.cursor;
                let cursor_char = if is_cursor { "❯ " } else { "  " };
                let name = &tool.name;
                let desc = tool
                    .description
                    .as_ref()
                    .map(|s| s.as_ref())
                    .unwrap_or("");

                let available_width = inner.width.saturating_sub(6) as usize;
                let name_width = name.len();
                let desc_max = available_width.saturating_sub(name_width);
                let desc_display: String = desc.chars().take(desc_max).collect();

                lines.push(Line::from(vec![
                    Span::styled(cursor_char.to_string(), Style::default().fg(theme::THINKING)),
                    Span::styled(
                        format!("{:<24}", name),
                        Style::default().fg(theme::SAGE),
                    ),
                    Span::styled(desc_display, Style::default().fg(theme::MUTED)),
                ]));
            }

            if tools.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  （该服务器未暴露工具）",
                    Style::default().fg(theme::MUTED),
                )));
            }
        }
        McpPanelView::ResourceList { resources, .. } => {
            for (i, resource) in resources.iter().enumerate() {
                let is_cursor = i == panel.cursor;
                let cursor_char = if is_cursor { "❯ " } else { "  " };
                let uri = &resource.uri;
                let name = resource
                    .title
                    .as_deref()
                    .unwrap_or("");

                let available_width = inner.width.saturating_sub(6) as usize;
                let uri_width = uri.len();
                let name_max = available_width.saturating_sub(uri_width);
                let name_display: String = name.chars().take(name_max).collect();

                lines.push(Line::from(vec![
                    Span::styled(cursor_char.to_string(), Style::default().fg(theme::THINKING)),
                    Span::styled(
                        format!("{:<30}", uri),
                        Style::default().fg(theme::THINKING),
                    ),
                    Span::styled(name_display, Style::default().fg(theme::MUTED)),
                ]));
            }

            if resources.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "  （该服务器未暴露资源）",
                    Style::default().fg(theme::MUTED),
                )));
            }
        }
    }

    // 底部提示行
    lines.push(Line::from(""));
    if panel.confirm_delete.is_some() {
        let server_name = panel.confirm_delete.as_deref().unwrap_or("?");
        lines.push(Line::from(vec![
            Span::styled(
                " ⚠ 确定删除 ",
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                server_name,
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "？此操作将从配置文件中永久移除  ",
                Style::default().fg(theme::MUTED),
            ),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":确认  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                "其他键",
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(":取消", Style::default().fg(theme::MUTED)),
        ]));
    } else {
        match &panel.view {
            McpPanelView::ServerList => {
                lines.push(Line::from(vec![
                    Span::styled("↑↓", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":移动  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Enter", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":详情  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Ctrl+R", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":重连  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Ctrl+D", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":删除  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Esc", Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD)),
                    Span::styled(":关闭", Style::default().fg(theme::MUTED)),
                ]));
            }
            McpPanelView::ToolList { .. } | McpPanelView::ResourceList { .. } => {
                lines.push(Line::from(vec![
                    Span::styled("↑↓", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":移动  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Tab", Style::default().fg(theme::WARNING).add_modifier(Modifier::BOLD)),
                    Span::styled(":切换视图  ", Style::default().fg(theme::MUTED)),
                    Span::styled("Esc", Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD)),
                    Span::styled(":返回", Style::default().fg(theme::MUTED)),
                ]));
            }
        }
    }

    // 存储面板元数据供鼠标选区使用
    app.core.panel_area = Some(inner);
    app.core.panel_scroll_offset = 0;
    app.core.panel_plain_lines = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();

    // 应用面板选区高亮
    if app.core.panel_selection.is_active() {
        let sel = &app.core.panel_selection;
        if let (Some(start), Some(end)) = (sel.start, sel.end) {
            let ((sr, sc), (er, ec)) = if start <= end {
                (start, end)
            } else {
                (end, start)
            };
            let scroll = 0usize;
            let visible_end = inner.height as usize;
            for line_idx in sr as usize..=er as usize {
                if line_idx >= visible_end {
                    continue;
                }
                let visual_idx = line_idx - scroll;
                if visual_idx >= lines.len() {
                    continue;
                }
                let (cs, ce) = if line_idx == sr as usize && line_idx == er as usize {
                    (sc as usize, ec as usize)
                } else if line_idx == sr as usize {
                    (sc as usize, usize::MAX)
                } else if line_idx == er as usize {
                    (0, ec as usize)
                } else {
                    (0, usize::MAX)
                };
                let spans = std::mem::take(&mut lines[visual_idx].spans);
                lines[visual_idx] = Line::from(highlight_line_spans(spans, cs, ce));
            }
        }
    }

    let mut scroll_state = ScrollState::with_offset(0);
    ScrollableArea::new(Text::from(lines))
        .scrollbar_style(Style::default().fg(theme::MUTED))
        .render(f, inner, &mut scroll_state);
}

#[cfg(test)]
mod tests {
    use rust_agent_middlewares::mcp::{ClientStatus, ServerInfo};

    use crate::app::{McpPanel, App};

    fn make_server(name: &str, status: ClientStatus) -> ServerInfo {
        ServerInfo {
            name: name.to_string(),
            transport_type: "stdio".to_string(),
            status,
            tool_count: 3,
            resource_count: 2,
        }
    }

    fn render_mcp_panel(servers: Vec<ServerInfo>) -> crate::ui::headless::HeadlessHandle {
        let (mut app, mut handle) = App::new_headless(120, 30);
        app.mcp_panel = Some(McpPanel::new(servers));
        handle
            .terminal
            .draw(|f| crate::ui::main_ui::render(f, &mut app))
            .unwrap();
        handle
    }

    #[tokio::test]
    async fn test_mcp_panel_empty_server_list() {
        let handle = render_mcp_panel(vec![]);
        let snap = handle.snapshot().join("\n");
        assert!(
            snap.contains(".mcp.json"),
            "空 MCP 面板应显示配置引导文字"
        );
    }

    #[tokio::test]
    async fn test_mcp_panel_server_list_with_items() {
        let handle = render_mcp_panel(vec![
            make_server("test-connected", ClientStatus::Connected),
            make_server("test-failed", ClientStatus::Failed("timeout".into())),
        ]);
        let snap = handle.snapshot().join("\n");
        assert!(
            snap.contains("test-connected"),
            "MCP 面板应显示服务器名称"
        );
        // Connected 显示 ●，Failed 显示 ○
        assert!(
            snap.contains("Connected"),
            "MCP 面板应显示 Connected 状态"
        );
    }
}
