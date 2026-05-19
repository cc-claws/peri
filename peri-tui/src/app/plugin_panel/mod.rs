use std::any::Any;
use std::collections::HashSet;

use peri_middlewares::plugin::InstallScope;
use peri_middlewares::plugin::MarketplaceSource;
use peri_widgets::InputState;
use ratatui::layout::Rect;
use ratatui::Frame;
use tui_textarea::Input;

use super::panel_component::PanelComponent;
use super::panel_list::PanelList;
use super::panel_manager::{EventResult, PanelContext, PanelKind};
use super::App;

pub mod handlers;
pub mod types;

pub use types::*;

impl PluginPanel {
    pub fn new(entries: Vec<PluginEntry>) -> Self {
        let mut installed_list = PanelList::new();
        installed_list.set_items(entries.clone());
        Self {
            view: PluginPanelView::Installed,
            entries,
            installed_list,
            confirm_delete: None,
            detail_index: None,
            detail_cursor: 0,
            discover_plugins: Vec::new(),
            discover_search: InputState::new(),
            discover_searching: false,
            discover_list: PanelList::new(),
            discover_loading: false,
            discover_selected: HashSet::new(),
            discover_detail_index: None,
            discover_detail_cursor: 0,
            marketplace_entries: Vec::new(),
            marketplace_list: PanelList::new(),
            marketplace_confirm_delete: None,
            marketplace_updating: HashSet::new(),
            add_marketplace_input: InputState::new(),
            add_marketplace_active: false,
            installing: HashSet::new(),
            uninstalling: HashSet::new(),
        }
    }

    pub fn is_detail(&self) -> bool {
        self.detail_index.is_some()
            || self.discover_detail_index.is_some()
            || self.add_marketplace_active
    }

    /// 按搜索词过滤后的 Discover 插件列表
    pub fn discover_filtered_plugins(&self) -> Vec<&DiscoverPlugin> {
        let search = self.discover_search.value();
        if search.is_empty() {
            self.discover_plugins.iter().collect()
        } else {
            let query = search.to_lowercase();
            self.discover_plugins
                .iter()
                .filter(|p| {
                    p.name.to_lowercase().contains(&query)
                        || p.description.to_lowercase().contains(&query)
                        || p.marketplace.to_lowercase().contains(&query)
                })
                .collect()
        }
    }

    /// 获取当前光标处的 Discover 插件
    pub fn discover_current_plugin(&self) -> Option<&DiscoverPlugin> {
        let filtered = self.discover_filtered_plugins();
        filtered.get(self.discover_list.cursor()).copied()
    }

    /// 根据当前视图过滤后的可见条目索引列表
    pub fn visible_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match self.view {
                PluginPanelView::Installed => true,
                PluginPanelView::Errors => e.load_error.is_some(),
                PluginPanelView::Discover | PluginPanelView::Marketplaces => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn current_list_len(&self) -> usize {
        match self.view {
            PluginPanelView::Installed => self.installed_list.len(),
            PluginPanelView::Errors => {
                // Errors 视图过滤有 load_error 的条目
                self.entries
                    .iter()
                    .filter(|e| e.load_error.is_some())
                    .count()
            }
            PluginPanelView::Discover => self.discover_list.len(),
            PluginPanelView::Marketplaces => {
                // marketplace_cursor = 0 是 Add Marketplace，+ marketplace_entries.len()
                self.marketplace_entries.len() + 1
            }
        }
    }

    /// 根据当前视图返回 cursor
    pub fn cursor(&self) -> usize {
        match self.view {
            PluginPanelView::Installed => self.installed_list.cursor(),
            PluginPanelView::Discover => self.discover_list.cursor(),
            PluginPanelView::Marketplaces => self.marketplace_list.cursor(),
            PluginPanelView::Errors => self.installed_list.cursor(),
        }
    }

    /// 根据当前视图返回 scroll_offset
    pub fn scroll_offset(&self) -> u16 {
        match self.view {
            PluginPanelView::Installed => self.installed_list.scroll_offset(),
            PluginPanelView::Discover => self.discover_list.scroll_offset(),
            PluginPanelView::Marketplaces => self.marketplace_list.scroll_offset(),
            PluginPanelView::Errors => self.installed_list.scroll_offset(),
        }
    }

    /// 根据当前视图设置 scroll_offset
    pub fn set_scroll_offset(&mut self, offset: u16) {
        match self.view {
            PluginPanelView::Installed => self.installed_list.set_scroll_offset(offset),
            PluginPanelView::Discover => self.discover_list.set_scroll_offset(offset),
            PluginPanelView::Marketplaces => self.marketplace_list.set_scroll_offset(offset),
            PluginPanelView::Errors => self.installed_list.set_scroll_offset(offset),
        }
    }

    pub fn selected_entry(&self) -> Option<&PluginEntry> {
        let indices = self.visible_indices();
        indices
            .get(self.cursor())
            .and_then(|&i| self.entries.get(i))
    }

    /// 切换视图后同步当前视图的 PanelList items
    fn sync_current_view_items(&mut self) {
        match self.view {
            PluginPanelView::Installed => {
                // installed_list items 已在 new() 时设置，无需同步
            }
            PluginPanelView::Errors => {
                // Errors 视图：只显示有 load_error 的 entries
                let error_entries: Vec<PluginEntry> = self
                    .entries
                    .iter()
                    .filter(|e| e.load_error.is_some())
                    .cloned()
                    .collect();
                self.installed_list.set_items(error_entries);
            }
            PluginPanelView::Discover => {
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
            }
            PluginPanelView::Marketplaces => {
                // marketplace_list items 在 open_plugin_panel 中设置
            }
        }
    }
}

// ─── PanelComponent 实现 ──────────────────────────────────────────────────────

impl PanelComponent for PluginPanel {
    fn kind(&self) -> PanelKind {
        PanelKind::Plugin
    }

    fn handle_key(&mut self, input: Input, ctx: &mut PanelContext<'_>) -> EventResult {
        // 1. confirm_delete 模式
        if self.confirm_delete.is_some() {
            return self.handle_confirm_delete(input, ctx);
        }

        // 2. discover_searching 模式
        if self.discover_searching {
            return self.handle_discover_searching(input, ctx);
        }

        // 3. discover_detail_index 模式
        if self.discover_detail_index.is_some() {
            return self.handle_discover_detail(input, ctx);
        }

        // 4. detail_index 模式
        if self.detail_index.is_some() {
            return self.handle_installed_detail(input, ctx);
        }

        // 5. 列表视图（按 PluginPanelView 分发）
        match self.view {
            PluginPanelView::Discover => self.handle_discover_list(input, ctx),
            PluginPanelView::Marketplaces => self.handle_marketplaces_list(input, ctx),
            PluginPanelView::Installed | PluginPanelView::Errors => {
                self.handle_installed_list(input, ctx)
            }
        }
    }

    fn handle_paste(&mut self, text: &str, _ctx: &mut PanelContext<'_>) -> EventResult {
        if self.add_marketplace_active {
            for ch in text.chars() {
                self.add_marketplace_input.insert(ch);
            }
            return EventResult::Consumed;
        }
        if self.discover_searching {
            for ch in text.chars() {
                self.discover_search.insert(ch);
            }
            self.discover_list.set_items(
                self.discover_filtered_plugins()
                    .into_iter()
                    .cloned()
                    .collect(),
            );
            return EventResult::Consumed;
        }
        EventResult::Consumed
    }

    fn handle_scroll(&mut self, lines: i16, _ctx: &mut PanelContext<'_>) -> EventResult {
        match self.view {
            PluginPanelView::Installed => self.installed_list.handle_scroll(lines, 10),
            PluginPanelView::Discover => self.discover_list.handle_scroll(lines, 10),
            PluginPanelView::Marketplaces => self.marketplace_list.handle_scroll(lines, 10),
            PluginPanelView::Errors => self.installed_list.handle_scroll(lines, 10),
        }
        EventResult::Consumed
    }

    fn desired_height(&self, screen_height: u16, _screen_width: u16) -> u16 {
        screen_height * 70 / 100
    }

    fn render(&mut self, f: &mut Frame, app: &mut App, area: Rect) {
        crate::ui::main_ui::panels::plugin::render_plugin_panel(f, self, app, area);
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn status_bar_hints(&self, _lc: &crate::i18n::LcRegistry) -> Vec<(String, String)> {
        if self.confirm_delete.is_some() {
            return vec![
                ("Enter".to_string(), _lc.tr("hint-plugin-uninstall")),
                ("\u{5176}\u{4ed6}\u{952e}".to_string(), _lc.tr("key-cancel")),
            ];
        }
        if self.marketplace_confirm_delete.is_some() {
            return vec![
                ("Enter".to_string(), _lc.tr("hint-plugin-delete")),
                ("Esc".to_string(), _lc.tr("key-cancel")),
            ];
        }
        if self.add_marketplace_active {
            return vec![
                ("Enter".to_string(), _lc.tr("hint-plugin-add")),
                ("Esc".to_string(), _lc.tr("key-cancel")),
            ];
        }
        if self.discover_searching {
            return vec![
                (
                    "Esc/\u{2191}\u{2193}".to_string(),
                    _lc.tr("hint-plugin-exit-search"),
                ),
                ("\u{2190}\u{2192}".to_string(), _lc.tr("key-tab")),
                ("Enter".to_string(), _lc.tr("key-install")),
                ("Backspace".to_string(), _lc.tr("key-delete")),
            ];
        }
        if self.discover_detail_index.is_some() {
            return vec![
                ("\u{2191}\u{2193}".to_string(), _lc.tr("key-move")),
                ("Enter".to_string(), _lc.tr("key-execute")),
                ("Esc".to_string(), _lc.tr("key-back")),
            ];
        }
        if self.detail_index.is_some() {
            return vec![
                ("\u{2191}\u{2193}".to_string(), _lc.tr("key-move")),
                ("Enter".to_string(), _lc.tr("key-execute")),
                ("Esc".to_string(), _lc.tr("key-back")),
            ];
        }
        match self.view {
            PluginPanelView::Discover => vec![
                ("\u{2191}\u{2193}".to_string(), _lc.tr("key-select")),
                ("\u{8f93}\u{5165}".to_string(), _lc.tr("hint-plugin-search")),
                ("Enter".to_string(), _lc.tr("key-install")),
                ("\u{2190}\u{2192}/Tab".to_string(), _lc.tr("key-tab")),
                ("Esc".to_string(), _lc.tr("key-close")),
            ],
            PluginPanelView::Marketplaces => vec![
                ("\u{2191}\u{2193}".to_string(), _lc.tr("key-select")),
                ("Enter".to_string(), _lc.tr("hint-plugin-add")),
                ("Backspace".to_string(), _lc.tr("hint-plugin-remove")),
                ("\u{2190}\u{2192}/Tab".to_string(), _lc.tr("key-tab")),
                ("Esc".to_string(), _lc.tr("key-close")),
            ],
            PluginPanelView::Installed | PluginPanelView::Errors => vec![
                ("\u{2191}\u{2193}".to_string(), _lc.tr("key-move")),
                ("Space".to_string(), _lc.tr("key-switch")),
                ("Enter".to_string(), _lc.tr("key-detail")),
                ("\u{2190}\u{2192}/Tab".to_string(), _lc.tr("key-tab")),
                ("Esc".to_string(), _lc.tr("key-close")),
            ],
        }
    }
}

// ─── App 操作方法 ────────────────────────────────────────────────────────────

impl App {
    pub fn plugin_panel_move_up(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            match panel.view {
                PluginPanelView::Installed | PluginPanelView::Errors => {
                    panel.installed_list.move_cursor(-1);
                }
                PluginPanelView::Discover => {
                    panel.discover_list.move_cursor(-1);
                }
                PluginPanelView::Marketplaces => {
                    panel.marketplace_list.move_cursor(-1);
                }
            }
        }
    }

    pub fn plugin_panel_move_down(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            match panel.view {
                PluginPanelView::Installed | PluginPanelView::Errors => {
                    panel.installed_list.move_cursor(1);
                }
                PluginPanelView::Discover => {
                    panel.discover_list.move_cursor(1);
                }
                PluginPanelView::Marketplaces => {
                    panel.marketplace_list.move_cursor(1);
                }
            }
        }
    }

    pub fn plugin_panel_tab(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.view.next();
            panel.sync_current_view_items();
        }
    }

    pub fn plugin_panel_shift_tab(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.view.prev();
            panel.sync_current_view_items();
        }
    }

    pub fn plugin_panel_close(&mut self) {
        self.global_panels.close();
    }

    pub fn plugin_panel_request_delete(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if let Some(entry) = panel.selected_entry() {
                panel.confirm_delete = Some(entry.id.clone());
            }
        }
    }

    pub fn plugin_panel_cancel_delete(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.confirm_delete = None;
        }
    }

    pub fn plugin_panel_confirm_delete(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if let Some(id) = panel.confirm_delete.take() {
                panel.entries.retain(|p| p.id != id);
                panel.installed_list.set_items(panel.entries.clone());
            }
        }
    }

    pub fn plugin_panel_toggle_enabled(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if let Some(entry_idx) = panel.visible_indices().get(panel.cursor()).copied() {
                if let Some(entry) = panel.entries.get_mut(entry_idx) {
                    entry.enabled = !entry.enabled;
                    self.persist_plugin_enabled_state();
                }
            }
        }
    }

    /// 将当前面板中所有插件的启用状态持久化到 ~/.claude/settings.json
    fn persist_plugin_enabled_state(&self) {
        if let Some(panel) = self.global_panels.get::<PluginPanel>() {
            let states: Vec<(String, bool)> = panel
                .entries
                .iter()
                .map(|e| (e.id.clone(), e.enabled))
                .collect();
            if let Err(e) = peri_middlewares::plugin::save_claude_settings_enabled_plugins(
                &states,
                self.services.claude_settings_override.as_deref(),
            ) {
                tracing::warn!(error = %e, "保存 enabledPlugins 失败");
            }
        }
    }

    /// 进入选中插件的详情视图
    pub fn plugin_panel_enter_detail(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if let Some(&entry_idx) = panel.visible_indices().get(panel.cursor()) {
                panel.detail_index = Some(entry_idx);
                panel.detail_cursor = 0;
            }
        }
    }

    /// 退出详情视图回到列表
    pub fn plugin_panel_exit_detail(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.detail_index = None;
            panel.detail_cursor = 0;
        }
    }

    /// 详情页操作菜单上移
    pub fn plugin_panel_detail_up(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if panel.detail_cursor > 0 {
                panel.detail_cursor -= 1;
            }
        }
    }

    /// 详情页操作菜单下移
    pub fn plugin_panel_detail_down(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            let max = DetailAction::ALL.len().saturating_sub(1);
            if panel.detail_cursor < max {
                panel.detail_cursor += 1;
            }
        }
    }

    /// 执行详情页当前操作
    pub fn plugin_panel_detail_action(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            let action = DetailAction::ALL.get(panel.detail_cursor).copied();
            let entry_idx = panel.detail_index;
            match action {
                Some(DetailAction::ToggleEnabled) => {
                    if let Some(idx) = entry_idx {
                        if let Some(entry) = panel.entries.get_mut(idx) {
                            entry.enabled = !entry.enabled;
                        }
                        // 面板引用已释放，调用保存
                        let states: Vec<(String, bool)> = panel
                            .entries
                            .iter()
                            .map(|e| (e.id.clone(), e.enabled))
                            .collect();
                        if let Err(e) =
                            peri_middlewares::plugin::save_claude_settings_enabled_plugins(
                                &states,
                                self.services.claude_settings_override.as_deref(),
                            )
                        {
                            tracing::warn!(error = %e, "保存 enabledPlugins 失败");
                        }
                    }
                }
                Some(DetailAction::Uninstall) => {
                    if let Some(idx) = entry_idx {
                        let id = panel.entries.get(idx).map(|e| e.id.clone());
                        if let Some(id) = id {
                            panel.confirm_delete = Some(id);
                        }
                    }
                }
                Some(DetailAction::BackToList) => {
                    panel.detail_index = None;
                    panel.detail_cursor = 0;
                }
                None => {}
            }
        }
    }

    // ─── Discover 视图操作 ─────────────────────────────────────────────────────

    pub fn discover_move_up(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_list.move_cursor(-1);
        }
    }

    pub fn discover_move_down(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_list.move_cursor(1);
        }
    }

    pub fn discover_enter_search(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_searching = true;
        }
    }

    pub fn discover_exit_search(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_searching = false;
            panel.discover_list.set_items(
                panel
                    .discover_filtered_plugins()
                    .into_iter()
                    .cloned()
                    .collect(),
            );
        }
    }

    pub fn discover_search_input(&mut self, ch: char) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_search.insert(ch);
            panel.discover_list.set_items(
                panel
                    .discover_filtered_plugins()
                    .into_iter()
                    .cloned()
                    .collect(),
            );
        }
    }

    pub fn discover_search_backspace(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_search.backspace();
            panel.discover_list.set_items(
                panel
                    .discover_filtered_plugins()
                    .into_iter()
                    .cloned()
                    .collect(),
            );
        }
    }

    pub fn discover_enter_detail(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if panel.discover_current_plugin().is_some() {
                panel.discover_detail_index = Some(panel.discover_list.cursor());
                panel.discover_detail_cursor = 0;
            }
        }
    }

    pub fn discover_exit_detail(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.discover_detail_index = None;
            panel.discover_detail_cursor = 0;
        }
    }

    pub fn discover_detail_up(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if panel.discover_detail_cursor > 0 {
                panel.discover_detail_cursor -= 1;
            }
        }
    }

    pub fn discover_detail_down(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            let max = DiscoverDetailAction::ALL.len().saturating_sub(1);
            if panel.discover_detail_cursor < max {
                panel.discover_detail_cursor += 1;
            }
        }
    }

    /// 执行 Discover 详情页操作（安装或返回）
    pub fn discover_detail_action(&mut self) -> Option<(String, String, InstallScope)> {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            let action = DiscoverDetailAction::ALL
                .get(panel.discover_detail_cursor)
                .copied();
            let plugin_idx = panel.discover_detail_index;
            match action {
                Some(DiscoverDetailAction::InstallUser) => {
                    if let Some(dp) = plugin_idx.and_then(|i| panel.discover_plugins.get(i)) {
                        return Some((dp.name.clone(), dp.marketplace.clone(), InstallScope::User));
                    }
                }
                Some(DiscoverDetailAction::InstallProject) => {
                    if let Some(dp) = plugin_idx.and_then(|i| panel.discover_plugins.get(i)) {
                        return Some((
                            dp.name.clone(),
                            dp.marketplace.clone(),
                            InstallScope::Project,
                        ));
                    }
                }
                Some(DiscoverDetailAction::BackToList) => {
                    panel.discover_detail_index = None;
                    panel.discover_detail_cursor = 0;
                }
                None => {}
            }
        }
        None
    }

    // ─── Marketplaces 视图操作 ──────────────────────────────────────────────────

    pub fn marketplace_move_up(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.marketplace_list.move_cursor(-1);
        }
    }

    pub fn marketplace_move_down(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            // cursor = 0 是 Add Marketplace，最大值是 marketplace_entries.len()
            let max = panel.marketplace_entries.len();
            if panel.marketplace_list.cursor() < max {
                panel.marketplace_list.move_cursor(1);
            }
        }
    }

    /// 检查当前是否选中了 "Add Marketplace" 选项
    pub fn marketplace_is_add_selected(&self) -> bool {
        self.global_panels
            .get::<PluginPanel>()
            .map(|p| p.marketplace_list.cursor() == 0)
            .unwrap_or(false)
    }

    /// 获取当前选中的 marketplace 名称（如果选中 Add Marketplace 则返回 None）
    pub fn marketplace_current_name(&self) -> Option<String> {
        self.global_panels
            .get::<PluginPanel>()
            .filter(|p| p.marketplace_list.cursor() > 0)
            .and_then(|p| p.marketplace_entries.get(p.marketplace_list.cursor() - 1))
            .map(|m| m.name.clone())
    }

    /// 请求删除当前 marketplace（进入确认状态）
    pub fn marketplace_request_delete(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            // cursor = 0 是 Add Marketplace，不能删除
            if panel.marketplace_list.cursor() > 0 {
                let idx = panel.marketplace_list.cursor() - 1;
                if panel.marketplace_entries.get(idx).is_some() {
                    panel.marketplace_confirm_delete = Some(idx);
                }
            }
        }
    }

    /// 取消删除 marketplace
    pub fn marketplace_cancel_delete(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.marketplace_confirm_delete = None;
        }
    }

    /// 确认删除当前 marketplace，返回要删除的 marketplace 名称
    pub fn marketplace_confirm_delete(&mut self) -> Option<String> {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            if let Some(idx) = panel.marketplace_confirm_delete.take() {
                if let Some(entry) = panel.marketplace_entries.get(idx) {
                    let name = entry.name.clone();
                    // 从列表中移除
                    panel.marketplace_entries.remove(idx);
                    panel
                        .marketplace_list
                        .set_items(panel.marketplace_entries.clone());
                    return Some(name);
                }
            }
        }
        None
    }

    /// 请求更新当前 marketplace（添加到 updating 集合）
    pub fn marketplace_request_update(&mut self) -> Option<String> {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            // cursor = 0 是 Add Marketplace，不能更新
            if panel.marketplace_list.cursor() > 0 {
                let idx = panel.marketplace_list.cursor() - 1;
                if let Some(entry) = panel.marketplace_entries.get(idx) {
                    let name = entry.name.clone();
                    panel.marketplace_updating.insert(name.clone());
                    return Some(name);
                }
            }
        }
        None
    }

    /// 请求更新当前 marketplace，返回名称和 source
    pub fn marketplace_request_update_with_source(
        &mut self,
    ) -> Option<(String, MarketplaceSource)> {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            // cursor = 0 是 Add Marketplace，不能更新
            if panel.marketplace_list.cursor() > 0 {
                let idx = panel.marketplace_list.cursor() - 1;
                if let Some(entry) = panel.marketplace_entries.get(idx) {
                    let name = entry.name.clone();
                    let source = entry.source.clone();
                    panel.marketplace_updating.insert(name.clone());
                    return Some((name, source));
                }
            }
        }
        None
    }

    /// 标记 marketplace 更新完成
    pub fn marketplace_update_done(&mut self, name: &str) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.marketplace_updating.remove(name);
        }
    }

    /// 进入添加 marketplace 模式
    pub fn marketplace_enter_add(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.add_marketplace_input = InputState::new();
            panel.add_marketplace_active = true;
        }
    }

    /// 退出添加 marketplace 模式
    pub fn marketplace_exit_add(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.add_marketplace_active = false;
            panel.add_marketplace_input = InputState::new();
        }
    }

    /// 添加 marketplace 输入字符
    pub fn marketplace_add_input(&mut self, ch: char) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.add_marketplace_input.insert(ch);
        }
    }

    /// 添加 marketplace 退格
    pub fn marketplace_add_backspace(&mut self) {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            panel.add_marketplace_input.backspace();
        }
    }

    /// 确认添加 marketplace，返回输入的 source 字符串
    pub fn marketplace_add_confirm(&mut self) -> Option<String> {
        if let Some(panel) = self.global_panels.get_mut::<PluginPanel>() {
            let input = panel.add_marketplace_input.value().trim().to_string();
            panel.add_marketplace_active = false;
            panel.add_marketplace_input = InputState::new();
            if input.is_empty() {
                None
            } else {
                Some(input)
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("plugin_panel_test.rs");
}
