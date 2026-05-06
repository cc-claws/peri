use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

use perihelion_widgets::{BorderedPanel, ScrollState, ScrollableArea};

use crate::app::plugin_panel::{
    DetailAction, DiscoverDetailAction, MarketplaceViewStatus, PluginItemType, PluginPanelView,
};
use crate::app::App;
use crate::ui::theme;

use rust_agent_middlewares::plugin::InstallScope;

pub fn render_plugin_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let is_detail = app.plugin_panel.as_ref().is_some_and(|p| p.is_detail());

    if is_detail {
        let is_discover_detail = app
            .plugin_panel
            .as_ref()
            .is_some_and(|p| p.discover_detail_index.is_some());
        if is_discover_detail {
            render_discover_detail(f, app, area);
        } else {
            render_detail(f, app, area);
        }
    } else {
        // Discover 视图使用固定搜索框布局
        let is_discover = app
            .plugin_panel
            .as_ref()
            .is_some_and(|p| p.view == PluginPanelView::Discover);
        if is_discover {
            render_discover_list(f, app, area);
        } else {
            render_list(f, app, area);
        }
    }
}

fn render_list(f: &mut Frame, app: &mut App, area: Rect) {
    let (lines, scroll_offset, cursor_row) = {
        let panel = match &app.plugin_panel {
            Some(p) => p,
            None => return,
        };
        let scroll_offset = panel.scroll_offset;
        let mut lines: Vec<Line> = Vec::new();
        let mut cursor_row = 0; // 光标所在行号（不含 Tab 行）

        // Tab 行
        let tab_labels: Vec<Span> = PluginPanelView::ALL
            .iter()
            .map(|v| {
                let label = v.label();
                let is_active = panel.view == *v;
                let style = if is_active {
                    Style::default()
                        .fg(theme::TEXT)
                        .bg(theme::THINKING)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                Span::styled(format!(" {} ", label), style)
            })
            .collect();
        lines.push(Line::from(tab_labels));
        lines.push(Line::from(""));

        // 根据视图渲染内容
        match panel.view {
            PluginPanelView::Installed => {
                let indices = panel.visible_indices();
                let cursor_idx = indices.get(panel.cursor).copied();
                let table_header_height = 3; // 表头行 + 空行

                if indices.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  No plugins installed".to_string(),
                        Style::default().fg(theme::MUTED),
                    )));
                } else {
                    // 表头
                    lines.push(Line::from(vec![
                        Span::styled(
                            "  Plugin",
                            Style::default()
                                .fg(theme::MUTED)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "                  Type  Scope      Status  Marketplace",
                            Style::default().fg(theme::MUTED),
                        ),
                    ]));
                    lines.push(Line::from(""));

                    // 直接遍历所有可见条目，不分组显示标题
                    for (row_idx, &idx) in indices.iter().enumerate() {
                        if let Some(entry) = panel.entries.get(idx) {
                            let is_cursor = cursor_idx == Some(idx);
                            if is_cursor {
                                cursor_row = table_header_height + row_idx;
                            }
                            let cursor_char = if is_cursor { "❯ " } else { "  " };

                            let type_label = match entry.plugin_type {
                                PluginItemType::Plugin => "Plugin",
                                PluginItemType::Mcp => "MCP    ",
                            };

                            let (status_icon, status_style) = if entry.enabled {
                                ("✔ ", Style::default().fg(theme::SAGE))
                            } else {
                                ("  ", Style::default().fg(theme::MUTED))
                            };

                            let name_style = if is_cursor {
                                Style::default()
                                    .fg(theme::THINKING)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme::TEXT)
                            };

                            let scope_label = match entry.scope {
                                InstallScope::User => "User  ",
                                InstallScope::Project => "Project",
                                InstallScope::Local => "Local ",
                            };

                            // 表格行：光标 + 名称 + 类型 + 作用域 + 状态 + marketplace（右对齐）
                            let name_width = 18;
                            let display_name = truncate_display(&entry.name, name_width);
                            let name_padding = " ".repeat(name_width.saturating_sub(
                                unicode_width::UnicodeWidthStr::width(display_name.as_str()),
                            ));

                            let marketplace_text = if !entry.marketplace.is_empty() {
                                entry.marketplace.clone()
                            } else {
                                String::new()
                            };

                            lines.push(Line::from(vec![
                                Span::styled(
                                    cursor_char.to_string(),
                                    Style::default().fg(theme::THINKING),
                                ),
                                Span::styled(display_name, name_style),
                                Span::styled(name_padding, Style::default()),
                                Span::styled(type_label, Style::default().fg(theme::MUTED)),
                                Span::styled("  ", Style::default()),
                                Span::styled(scope_label, Style::default().fg(theme::MUTED)),
                                Span::styled("  ", Style::default()),
                                Span::styled(status_icon.to_string(), status_style),
                                Span::styled("  ", Style::default()),
                                // Marketplace 右对齐（通过填充剩余空间）
                                Span::styled(marketplace_text, Style::default().fg(theme::MUTED)),
                            ]));
                        }
                    }
                }
            }
            PluginPanelView::Errors => {
                let indices = panel.visible_indices();
                let cursor_idx = indices.get(panel.cursor).copied();
                let table_header_height = 3; // 表头行 + 空行

                if indices.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  No errors".to_string(),
                        Style::default().fg(theme::SAGE),
                    )));
                } else {
                    // 表头
                    lines.push(Line::from(vec![
                        Span::styled(
                            "  Plugin",
                            Style::default()
                                .fg(theme::MUTED)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            "                  Scope  Error",
                            Style::default().fg(theme::MUTED),
                        ),
                    ]));
                    lines.push(Line::from(""));

                    for (row_idx, &idx) in indices.iter().enumerate() {
                        if let Some(entry) = panel.entries.get(idx) {
                            let is_cursor = cursor_idx == Some(idx);
                            if is_cursor {
                                cursor_row = table_header_height + row_idx;
                            }
                            let cursor_char = if is_cursor { "❯ " } else { "  " };

                            let name_style = if is_cursor {
                                Style::default()
                                    .fg(theme::THINKING)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(theme::TEXT)
                            };

                            let scope_label = match entry.scope {
                                InstallScope::User => "User  ",
                                InstallScope::Project => "Project",
                                InstallScope::Local => "Local ",
                            };

                            let error_text = entry.load_error.as_deref().unwrap_or("Unknown error");

                            lines.push(Line::from(vec![
                                Span::styled(
                                    cursor_char.to_string(),
                                    Style::default().fg(theme::THINKING),
                                ),
                                Span::styled(truncate_display(&entry.name, 18), name_style),
                                Span::styled("  ", Style::default()),
                                Span::styled(scope_label, Style::default().fg(theme::MUTED)),
                                Span::styled("  ", Style::default()),
                                Span::styled(
                                    error_text.to_string(),
                                    Style::default().fg(theme::ERROR),
                                ),
                            ]));
                        }
                    }
                }
            }
            PluginPanelView::Discover => {
                // Discover 由 render_discover_list 单独处理，此处不应到达
                lines.push(Line::from(Span::styled(
                    "  Discover",
                    Style::default().fg(theme::MUTED),
                )));
            }
            PluginPanelView::Marketplaces => {
                if panel.marketplace_entries.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  No marketplaces configured",
                        Style::default().fg(theme::MUTED),
                    )));
                } else {
                    for (i, mkt) in panel.marketplace_entries.iter().enumerate() {
                        let is_cursor = i == panel.marketplace_cursor;
                        let cursor_char = if is_cursor { "\u{276F} " } else { "  " };

                        let name_style = if is_cursor {
                            Style::default()
                                .fg(theme::THINKING)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::TEXT)
                        };

                        // 状态指示
                        let (status_text, status_style) = match mkt.status {
                            MarketplaceViewStatus::Fresh | MarketplaceViewStatus::Cached => {
                                ("cached", Style::default().fg(theme::SAGE))
                            }
                            MarketplaceViewStatus::Fetching => {
                                ("fetching\u{2026}", Style::default().fg(theme::WARNING))
                            }
                            MarketplaceViewStatus::Stale => {
                                ("stale", Style::default().fg(theme::WARNING))
                            }
                            MarketplaceViewStatus::Failed => {
                                ("failed", Style::default().fg(theme::ERROR))
                            }
                        };

                        lines.push(Line::from(vec![
                            Span::styled(
                                cursor_char.to_string(),
                                Style::default().fg(theme::THINKING),
                            ),
                            Span::styled(mkt.name.clone(), name_style),
                        ]));

                        // 详情行
                        let mut detail_parts = vec![
                            Span::styled("     ".to_string(), Style::default()),
                            Span::styled(
                                mkt.source_label.clone(),
                                Style::default().fg(theme::MUTED),
                            ),
                        ];

                        // 插件数
                        detail_parts.push(Span::styled(
                            format!(" \u{00B7} {} available", mkt.plugin_count),
                            Style::default().fg(theme::MUTED),
                        ));

                        // 已安装数
                        if mkt.installed_count > 0 {
                            detail_parts.push(Span::styled(
                                format!(" \u{00B7} {} installed", mkt.installed_count),
                                Style::default().fg(theme::SAGE),
                            ));
                        }

                        lines.push(Line::from(detail_parts));

                        // 状态行
                        let mut status_parts = vec![
                            Span::styled("     ", Style::default()),
                            Span::styled(status_text.to_string(), status_style),
                        ];

                        // 最后更新时间
                        if let Some(ref updated) = mkt.last_updated {
                            status_parts.push(Span::styled(
                                format!(" \u{00B7} Updated {}", updated),
                                Style::default().fg(theme::MUTED),
                            ));
                        }

                        // auto-update
                        let auto_label = if mkt.auto_update { "on" } else { "off" };
                        status_parts.push(Span::styled(
                            format!(" \u{00B7} auto-update: {}", auto_label),
                            Style::default().fg(theme::MUTED),
                        ));

                        lines.push(Line::from(status_parts));
                        lines.push(Line::from(""));
                    }
                }
            }
        }

        (lines, scroll_offset, cursor_row)
    };

    let inner = BorderedPanel::new(Span::styled(
        " Plugins ",
        Style::default()
            .fg(theme::THINKING)
            .add_modifier(Modifier::BOLD),
    ))
    .border_style(Style::default().fg(theme::BORDER))
    .render(f, area);

    app.sessions[app.active].core.panel_area = Some(inner);
    app.sessions[app.active].core.panel_scroll_offset = 0;
    app.sessions[app.active].core.panel_plain_lines = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();

    let visible_height = inner.height.saturating_sub(1);
    let mut scroll_state = ScrollState::with_offset(scroll_offset);
    // 确保光标行在可视区域内
    scroll_state.ensure_visible(cursor_row as u16, visible_height);

    // 回写 scroll offset
    if let Some(p) = &mut app.plugin_panel {
        p.scroll_offset = scroll_state.offset();
    }

    ScrollableArea::new(Text::from(lines))
        .scrollbar_style(Style::default().fg(theme::MUTED))
        .render(f, inner, &mut scroll_state);
}

fn render_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let (lines, scroll_offset) = {
        let panel = match &app.plugin_panel {
            Some(p) => p,
            None => return,
        };
        let entry_idx = match panel.detail_index {
            Some(i) => i,
            None => return,
        };
        let entry = match panel.entries.get(entry_idx) {
            Some(e) => e,
            None => return,
        };
        let scroll_offset = panel.scroll_offset;
        let detail_cursor = panel.detail_cursor;
        let mut lines: Vec<Line> = Vec::new();

        // Header: name @ marketplace
        let header_text = if entry.marketplace.is_empty() {
            entry.name.clone()
        } else {
            format!("{} @ {}", entry.name, entry.marketplace)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}", header_text),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )));

        // Scope
        let scope_label = match entry.scope {
            InstallScope::Project => "project",
            InstallScope::Local => "local",
            InstallScope::User => "user",
        };
        lines.push(detail_kv_line("Scope:", scope_label));
        lines.push(detail_kv_line("Version:", &entry.version));

        // Description
        if !entry.description.is_empty() {
            lines.push(Line::from(""));
            for desc_line in entry.description.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", desc_line),
                    Style::default().fg(theme::MUTED),
                )));
            }
        }

        // Author
        if let Some(ref author) = entry.author {
            lines.push(Line::from(""));
            lines.push(detail_kv_line("Author:", author));
        }

        // Status
        lines.push(Line::from(""));
        let is_uninstalling = panel.uninstalling.contains(&entry.id);
        let (status_icon, status_style, status_text) = if is_uninstalling {
            (
                "\u{26A0}",
                Style::default().fg(theme::WARNING),
                "Uninstalling\u{2026}",
            )
        } else if entry.enabled {
            ("✔", Style::default().fg(theme::SAGE), "Enabled")
        } else {
            ("◯", Style::default().fg(theme::MUTED), "Disabled")
        };
        lines.push(Line::from(vec![
            Span::styled("  Status: ".to_string(), Style::default().fg(theme::MUTED)),
            Span::styled(format!("{} {}", status_icon, status_text), status_style),
        ]));

        // Installed components
        let has_components = !entry.commands.is_empty()
            || !entry.skills.is_empty()
            || !entry.agents.is_empty()
            || !entry.mcp_servers.is_empty();

        if has_components {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Installed components:".to_string(),
                Style::default().fg(theme::MUTED),
            )));

            if !entry.commands.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  • Commands: {}", entry.commands.join(", ")),
                    Style::default().fg(theme::TEXT),
                )));
            }
            if !entry.skills.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  • Skills: {}", entry.skills.join(", ")),
                    Style::default().fg(theme::TEXT),
                )));
            }
            if !entry.agents.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  • Agents: {}", entry.agents.join(", ")),
                    Style::default().fg(theme::TEXT),
                )));
            }
            if !entry.mcp_servers.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  • MCP servers: {}", entry.mcp_servers.join(", ")),
                    Style::default().fg(theme::TEXT),
                )));
            }
        }

        // Action menu
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        for (i, action) in DetailAction::ALL.iter().enumerate() {
            let is_cursor = i == detail_cursor;
            let cursor_char = if is_cursor { "❯ " } else { "  " };
            let label = action.label(entry.enabled);
            let style = if is_cursor {
                Style::default()
                    .fg(theme::THINKING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    cursor_char.to_string(),
                    Style::default().fg(theme::THINKING),
                ),
                Span::styled(label.to_string(), style),
            ]));
        }

        (lines, scroll_offset)
    };

    let inner = BorderedPanel::new(Span::styled(
        " Plugins ",
        Style::default()
            .fg(theme::THINKING)
            .add_modifier(Modifier::BOLD),
    ))
    .border_style(Style::default().fg(theme::BORDER))
    .render(f, area);

    app.sessions[app.active].core.panel_area = Some(inner);
    app.sessions[app.active].core.panel_scroll_offset = 0;
    app.sessions[app.active].core.panel_plain_lines = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();

    let mut scroll_state = ScrollState::with_offset(scroll_offset);
    ScrollableArea::new(Text::from(lines))
        .scrollbar_style(Style::default().fg(theme::MUTED))
        .render(f, inner, &mut scroll_state);
}

fn detail_kv_line<'a>(key: &str, value: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {}: ", key), Style::default().fg(theme::MUTED)),
        Span::styled(value.to_string(), Style::default().fg(theme::TEXT)),
    ])
}

fn render_discover_detail(f: &mut Frame, app: &mut App, area: Rect) {
    let (lines, scroll_offset) = {
        let panel = match &app.plugin_panel {
            Some(p) => p,
            None => return,
        };
        let plugin_idx = match panel.discover_detail_index {
            Some(i) => i,
            None => return,
        };
        let filtered = panel.discover_filtered_plugins();
        let plugin = match filtered.get(plugin_idx) {
            Some(p) => p,
            None => return,
        };
        let scroll_offset = panel.discover_scroll;
        let detail_cursor = panel.discover_detail_cursor;
        let mut lines: Vec<Line> = Vec::new();

        // Header
        let header_text = if plugin.marketplace.is_empty() {
            plugin.name.clone()
        } else {
            format!("{} @ {}", plugin.name, plugin.marketplace)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}", header_text),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )));

        // Version
        lines.push(detail_kv_line("Version:", &plugin.version));

        // Description
        if !plugin.description.is_empty() {
            lines.push(Line::from(""));
            for desc_line in plugin.description.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", desc_line),
                    Style::default().fg(theme::MUTED),
                )));
            }
        }

        // Author
        if let Some(ref author) = plugin.author {
            lines.push(Line::from(""));
            lines.push(detail_kv_line("Author:", author));
        }

        // Status
        lines.push(Line::from(""));
        let (status_icon, status_style, status_text) = if plugin.installed {
            ("\u{2714}", Style::default().fg(theme::SAGE), "Installed")
        } else {
            (
                "\u{25CB}",
                Style::default().fg(theme::MUTED),
                "Not installed",
            )
        };
        lines.push(Line::from(vec![
            Span::styled("  Status: ".to_string(), Style::default().fg(theme::MUTED)),
            Span::styled(format!("{} {}", status_icon, status_text), status_style),
        ]));

        // Action menu
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        let actions = if plugin.installed {
            // 已安装的只显示返回
            &[DiscoverDetailAction::BackToList] as &[DiscoverDetailAction]
        } else {
            &DiscoverDetailAction::ALL
        };

        for (i, action) in actions.iter().enumerate() {
            let is_cursor = i == detail_cursor;
            let cursor_char = if is_cursor { "\u{276F} " } else { "  " };
            let label = action.label();
            let style = if is_cursor {
                Style::default()
                    .fg(theme::THINKING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    cursor_char.to_string(),
                    Style::default().fg(theme::THINKING),
                ),
                Span::styled(label.to_string(), style),
            ]));
        }

        (lines, scroll_offset)
    };

    let inner = BorderedPanel::new(Span::styled(
        " Plugins ",
        Style::default()
            .fg(theme::THINKING)
            .add_modifier(Modifier::BOLD),
    ))
    .border_style(Style::default().fg(theme::BORDER))
    .render(f, area);

    app.sessions[app.active].core.panel_area = Some(inner);
    app.sessions[app.active].core.panel_scroll_offset = 0;
    app.sessions[app.active].core.panel_plain_lines = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();

    let mut scroll_state = ScrollState::with_offset(scroll_offset);
    ScrollableArea::new(Text::from(lines))
        .scrollbar_style(Style::default().fg(theme::MUTED))
        .render(f, inner, &mut scroll_state);
}

/// Tab 行占用的固定高度（Tab 行 + 空行）
const DISCOVER_TAB_OVERHEAD: u16 = 2;
/// 搜索框占用的固定高度（搜索框 3 行 + 空行 1 行）
const DISCOVER_SEARCH_OVERHEAD: u16 = 4;
/// Tab + 搜索框合计固定高度
const DISCOVER_FIXED_OVERHEAD: u16 = DISCOVER_TAB_OVERHEAD + DISCOVER_SEARCH_OVERHEAD; // 6

/// 渲染搜索框到固定区域（不参与滚动）
fn render_discover_search_box(
    f: &mut Frame,
    panel: &crate::app::plugin_panel::PluginPanel,
    area: Rect,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }

    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if panel.discover_searching {
            theme::ACCENT
        } else {
            theme::DIM
        }));

    let search_inner = search_block.inner(area);

    let query_val = panel.discover_search.value();
    let content_line = if query_val.is_empty() && !panel.discover_searching {
        Line::from(vec![
            Span::styled(" \u{2315} ", Style::default().fg(theme::MUTED)),
            Span::styled("Search plugins\u{2026}", Style::default().fg(theme::DIM)),
        ])
    } else {
        let mut spans = vec![
            Span::styled(" \u{2315} ", Style::default().fg(theme::MUTED)),
            Span::styled(
                panel.discover_search.display_text('\u{2022}'),
                Style::default().fg(theme::TEXT),
            ),
        ];
        if panel.discover_searching {
            spans.push(Span::styled("\u{2588}", Style::default().fg(theme::TEXT)));
        }
        Line::from(spans)
    };

    let search_para = Paragraph::new(content_line);
    f.render_widget(search_block, area);
    f.render_widget(search_para, search_inner);
}

/// Discover 视图：Tab 行 → 搜索框（固定） → 可滚动插件列表（带跟随）
fn render_discover_list(f: &mut Frame, app: &mut App, area: Rect) {
    // 克隆面板数据避免 borrow 冲突
    let panel = match &app.plugin_panel {
        Some(p) => p.clone(),
        None => return,
    };

    // Tab 行 Spans
    let tab_labels: Vec<Span> = PluginPanelView::ALL
        .iter()
        .map(|v| {
            let label = v.label();
            let is_active = panel.view == *v;
            let style = if is_active {
                Style::default()
                    .fg(theme::TEXT)
                    .bg(theme::THINKING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            Span::styled(format!(" {} ", label), style)
        })
        .collect();

    let title_text = if panel.discover_loading {
        " Plugins \u{2026} "
    } else {
        " Plugins "
    };

    let inner = BorderedPanel::new(Span::styled(
        title_text,
        Style::default()
            .fg(theme::THINKING)
            .add_modifier(Modifier::BOLD),
    ))
    .border_style(Style::default().fg(theme::BORDER))
    .render(f, area);

    // ── 布局：Tab 行(2行) → 搜索框(4行) → 列表(剩余) ──
    // Tab 行直接渲染到 inner 顶部
    let tab_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: DISCOVER_TAB_OVERHEAD,
    };
    // 搜索框在 Tab 行下方
    let search_area = Rect {
        x: inner.x + 1,
        y: inner.y + DISCOVER_TAB_OVERHEAD,
        width: inner.width.saturating_sub(2),
        height: 3,
    };
    // 列表在搜索框下方
    let list_area = Rect {
        x: inner.x,
        y: inner.y + DISCOVER_FIXED_OVERHEAD,
        width: inner.width,
        height: inner.height.saturating_sub(DISCOVER_FIXED_OVERHEAD),
    };

    // ── 1. 渲染 Tab 行（固定） ──
    let tab_para = Paragraph::new(vec![Line::from(tab_labels), Line::from("")]);
    f.render_widget(tab_para, tab_area);

    // ── 2. 渲染搜索框（固定） ──
    render_discover_search_box(f, &panel, search_area);

    // ── 3. 构建列表内容（纯插件列表，不含 Tab/搜索框） ──
    let mut lines: Vec<Line> = Vec::new();

    let filtered = panel.discover_filtered_plugins();
    let max_name_width = list_area.width.saturating_sub(8) as usize;

    if panel.discover_loading && filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Loading marketplace data\u{2026}",
            Style::default().fg(theme::MUTED),
        )));
    } else if filtered.is_empty() {
        let msg = if panel.discover_search.value().is_empty() {
            "  No plugins available"
        } else {
            "  No matching plugins"
        };
        lines.push(Line::from(Span::styled(
            msg.to_string(),
            Style::default().fg(theme::MUTED),
        )));
    } else {
        // 计算光标所在的逻辑行号（每个插件占 2 行：名称行 + 描述行）
        for (i, plugin) in filtered.iter().enumerate() {
            let is_cursor = i == panel.discover_cursor;
            let is_selected = panel.discover_selected.contains(&plugin.plugin_id);
            let is_installing = panel.installing.contains(&plugin.plugin_id);
            let is_uninstalling = panel.uninstalling.contains(&plugin.plugin_id);
            let cursor_char = if is_cursor { "\u{276F} " } else { "  " };
            let check_char = if is_selected { "\u{25C9}" } else { "\u{25CB}" };

            let name_style = if is_cursor {
                Style::default()
                    .fg(theme::THINKING)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::TEXT)
            };

            let display_name = truncate_display(&plugin.name, max_name_width);

            // 第一行：cursor + checkbox + name · marketplace
            let mut spans = vec![
                Span::styled(
                    cursor_char.to_string(),
                    Style::default().fg(theme::THINKING),
                ),
                Span::styled(
                    format!("{} ", check_char),
                    if is_selected {
                        Style::default().fg(theme::ACCENT)
                    } else {
                        Style::default().fg(theme::MUTED)
                    },
                ),
                Span::styled(display_name.clone(), name_style),
            ];

            if !plugin.marketplace.is_empty() {
                spans.push(Span::styled(
                    format!(" \u{00B7} {}", plugin.marketplace),
                    Style::default().fg(theme::MUTED),
                ));
            }

            // 计算安装状态标识（放在右侧）
            let status_spans = if is_installing {
                Some(Span::styled(
                    "installing\u{2026}",
                    Style::default().fg(theme::WARNING),
                ))
            } else if is_uninstalling {
                Some(Span::styled(
                    "uninstalling\u{2026}",
                    Style::default().fg(theme::WARNING),
                ))
            } else if plugin.installed {
                Some(Span::styled("✔", Style::default().fg(theme::SAGE)))
            } else {
                None
            };

            // 如果有状态标识，计算填充空格使其右对齐
            if let Some(status_span) = status_spans {
                // 计算当前内容的显示宽度
                let content_width: usize = spans
                    .iter()
                    .map(|s| unicode_width::UnicodeWidthStr::width(&*s.content))
                    .sum();
                let status_width = unicode_width::UnicodeWidthStr::width(&*status_span.content);
                let available_width = list_area.width.saturating_sub(2) as usize; // 减去光标占用的空间
                let padding = if content_width + status_width < available_width {
                    " ".repeat(available_width.saturating_sub(content_width + status_width))
                } else {
                    " ".repeat(2) // 最小间隔
                };
                spans.push(Span::styled(padding, Style::default()));
                spans.push(status_span);
            }

            lines.push(Line::from(spans));

            // 第二行：描述（缩进，截断）
            let desc_width = list_area.width.saturating_sub(6) as usize;
            let desc = if plugin.description.is_empty() {
                String::new()
            } else {
                truncate_display(&plugin.description, desc_width)
            };
            if !desc.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("     ", Style::default()),
                    Span::styled(desc, Style::default().fg(theme::MUTED)),
                ]));
            } else {
                // 即使没有描述也要占一行，保持每个插件固定 2 行
                lines.push(Line::from(""));
            }
        }
    }

    // ── 跟随机制：确保光标行在可视区域内 ──
    let cursor_row = (panel.discover_cursor * 2) as u16;
    let visible_height = list_area.height;
    let mut scroll_state = ScrollState::with_offset(panel.discover_scroll);
    scroll_state.ensure_visible(cursor_row, visible_height);

    // 回写 scroll offset 供下次渲染使用
    if let Some(p) = &mut app.plugin_panel {
        p.discover_scroll = scroll_state.offset();
    }

    app.sessions[app.active].core.panel_area = Some(inner);
    app.sessions[app.active].core.panel_scroll_offset = 0;
    app.sessions[app.active].core.panel_plain_lines = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();

    ScrollableArea::new(Text::from(lines))
        .scrollbar_style(Style::default().fg(theme::MUTED))
        .render(f, list_area, &mut scroll_state);
}

/// 基于显示宽度的安全截断
fn truncate_display(s: &str, max_width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if UnicodeWidthStr::width(s) <= max_width {
        s.to_string()
    } else {
        let mut width = 0;
        let end = s
            .char_indices()
            .find(|&(_, c)| {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
                width > max_width.saturating_sub(1)
            })
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}\u{2026}", &s[..end])
    }
}
