use tui_textarea::{Input, Key};

use peri_middlewares::plugin::InstallScope;
use peri_widgets::InputState;

use super::super::panel_manager::{EventResult, PanelContext};
use super::types::*;

impl PluginPanel {
    // ─── 内部 handle_key 分发方法 ─────────────────────────────────────────────

    pub(super) fn handle_confirm_delete(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input {
                key: Key::Enter, ..
            } => {
                let (plugin_id, project_path) = if let Some(id) = self.confirm_delete.clone() {
                    let entry = self.entries.iter().find(|e| e.id == id);
                    let project_path = entry.and_then(|e| e.project_path.clone());
                    (Some(id), project_path)
                } else {
                    (None, None)
                };

                if let Some(plugin_id) = plugin_id {
                    self.uninstalling.insert(plugin_id.clone());
                    self.confirm_delete = None;

                    let tx = ctx.services.bg_event_tx.clone();
                    let claude_dir = peri_middlewares::plugin::claude_home();
                    let project_dir = project_path.map(std::path::PathBuf::from);
                    tokio::spawn(async move {
                        let result = peri_middlewares::plugin::uninstall_plugin(
                            &plugin_id,
                            &claude_dir,
                            project_dir.as_deref(),
                        )
                        .await;
                        let success = result.is_ok();
                        let message = if let Err(e) = result {
                            format!("\u{5378}\u{8f7d}\u{5931}\u{8d25}: {e}")
                        } else {
                            "\u{5378}\u{8f7d}\u{6210}\u{529f}".to_string()
                        };
                        let _ = tx.try_send(super::super::AgentEvent::PluginActionCompleted {
                            plugin_id,
                            action: "uninstall".to_string(),
                            success,
                            message,
                        });
                    });
                } else {
                    self.confirm_delete = None;
                }
                EventResult::Consumed
            }
            _ => {
                self.confirm_delete = None;
                EventResult::Consumed
            }
        }
    }

    pub(super) fn handle_discover_searching(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input {
                key: Key::Char(c), ..
            } => {
                self.discover_search.insert(c);
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                EventResult::Consumed
            }
            Input {
                key: Key::Backspace,
                ..
            } => {
                self.discover_search.backspace();
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                EventResult::Consumed
            }
            Input { key: Key::Up, .. } => {
                self.discover_searching = false;
                self.discover_list.move_cursor(-1);
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                self.discover_searching = false;
                self.discover_list.move_cursor(1);
                EventResult::Consumed
            }
            Input { key: Key::Left, .. } => {
                self.discover_searching = false;
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                self.view.prev();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input {
                key: Key::Right, ..
            } => {
                self.discover_searching = false;
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                self.view.next();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => {
                self.discover_searching = false;
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                self.discover_searching = false;
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                self.spawn_install_current(ctx);
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    pub(super) fn handle_discover_detail(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input { key: Key::Up, .. } => {
                if self.discover_detail_cursor > 0 {
                    self.discover_detail_cursor -= 1;
                }
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                let max = DiscoverDetailAction::ALL.len().saturating_sub(1);
                if self.discover_detail_cursor < max {
                    self.discover_detail_cursor += 1;
                }
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                let action = DiscoverDetailAction::ALL
                    .get(self.discover_detail_cursor)
                    .copied();
                let plugin_idx = self.discover_detail_index;
                match action {
                    Some(DiscoverDetailAction::InstallUser) => {
                        if let Some(dp) = plugin_idx.and_then(|i| self.discover_plugins.get(i)) {
                            let name = dp.name.clone();
                            let marketplace = dp.marketplace.clone();
                            let plugin_id = format!("{}@{}", name, marketplace);
                            self.installing.insert(plugin_id.clone());
                            let project_dir = std::path::PathBuf::from(&ctx.services.cwd);
                            let claude_dir = peri_middlewares::plugin::claude_home();
                            let cache_dir = peri_middlewares::plugin::marketplaces_cache_dir();
                            let tx = ctx.services.bg_event_tx.clone();
                            tokio::spawn(async move {
                                let result = peri_middlewares::plugin::install_plugin(
                                    &name,
                                    &marketplace,
                                    InstallScope::User,
                                    &cache_dir,
                                    &claude_dir,
                                    Some(&project_dir),
                                )
                                .await;
                                let _ =
                                    tx.try_send(super::super::AgentEvent::PluginActionCompleted {
                                        plugin_id: format!("{}@{}", name, marketplace),
                                        action: "install".to_string(),
                                        success: result.is_ok(),
                                        message: result
                                            .map(|_| String::new())
                                            .unwrap_or_else(|e| e.to_string()),
                                    });
                            });
                        }
                        self.discover_detail_index = None;
                        self.discover_detail_cursor = 0;
                    }
                    Some(DiscoverDetailAction::InstallProject) => {
                        if let Some(dp) = plugin_idx.and_then(|i| self.discover_plugins.get(i)) {
                            let name = dp.name.clone();
                            let marketplace = dp.marketplace.clone();
                            let plugin_id = format!("{}@{}", name, marketplace);
                            self.installing.insert(plugin_id.clone());
                            let project_dir = std::path::PathBuf::from(&ctx.services.cwd);
                            let claude_dir = peri_middlewares::plugin::claude_home();
                            let cache_dir = peri_middlewares::plugin::marketplaces_cache_dir();
                            let tx = ctx.services.bg_event_tx.clone();
                            tokio::spawn(async move {
                                let result = peri_middlewares::plugin::install_plugin(
                                    &name,
                                    &marketplace,
                                    InstallScope::Project,
                                    &cache_dir,
                                    &claude_dir,
                                    Some(&project_dir),
                                )
                                .await;
                                let _ =
                                    tx.try_send(super::super::AgentEvent::PluginActionCompleted {
                                        plugin_id: format!("{}@{}", name, marketplace),
                                        action: "install".to_string(),
                                        success: result.is_ok(),
                                        message: result
                                            .map(|_| String::new())
                                            .unwrap_or_else(|e| e.to_string()),
                                    });
                            });
                        }
                        self.discover_detail_index = None;
                        self.discover_detail_cursor = 0;
                    }
                    Some(DiscoverDetailAction::BackToList) => {
                        self.discover_detail_index = None;
                        self.discover_detail_cursor = 0;
                    }
                    None => {}
                }
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => {
                self.discover_detail_index = None;
                self.discover_detail_cursor = 0;
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    pub(super) fn handle_installed_detail(
        &mut self,
        input: Input,
        ctx: &PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input { key: Key::Up, .. } => {
                if self.detail_cursor > 0 {
                    self.detail_cursor -= 1;
                }
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                let max = DetailAction::ALL.len().saturating_sub(1);
                if self.detail_cursor < max {
                    self.detail_cursor += 1;
                }
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                self.do_detail_action(ctx);
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => {
                self.detail_index = None;
                self.detail_cursor = 0;
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    pub(super) fn handle_installed_list(
        &mut self,
        input: Input,
        ctx: &PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input {
                key: Key::Right, ..
            }
            | Input { key: Key::Tab, .. } => {
                self.view.next();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Left, .. } => {
                self.view.prev();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Up, .. } => {
                self.installed_list.move_cursor(-1);
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                self.installed_list.move_cursor(1);
                EventResult::Consumed
            }
            Input {
                key: Key::Char(' '),
                ..
            } => {
                if let Some(&entry_idx) = self.visible_indices().get(self.installed_list.cursor()) {
                    if let Some(entry) = self.entries.get_mut(entry_idx) {
                        entry.enabled = !entry.enabled;
                    }
                }
                self.persist_enabled_state(ctx.services.claude_settings_override.as_ref());
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                if let Some(&entry_idx) = self.visible_indices().get(self.installed_list.cursor()) {
                    self.detail_index = Some(entry_idx);
                    self.detail_cursor = 0;
                }
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => EventResult::ClosePanel,
            _ => EventResult::Consumed,
        }
    }

    pub(super) fn handle_discover_list(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input {
                key: Key::Right, ..
            }
            | Input { key: Key::Tab, .. } => {
                self.view.next();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Left, .. } => {
                self.view.prev();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Up, .. } => {
                self.discover_list.move_cursor(-1);
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                self.discover_list.move_cursor(1);
                EventResult::Consumed
            }
            Input {
                key: Key::Char(c), ..
            } => {
                self.discover_searching = true;
                self.discover_search.insert(c);
                self.discover_list.set_items(
                    self.discover_filtered_plugins()
                        .into_iter()
                        .cloned()
                        .collect(),
                );
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                self.spawn_install_current(ctx);
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => EventResult::ClosePanel,
            _ => EventResult::Consumed,
        }
    }

    pub(super) fn handle_marketplaces_list(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        // marketplace_confirm_delete 子状态
        if self.marketplace_confirm_delete.is_some() {
            return self.handle_marketplace_confirm_delete(input, ctx);
        }

        // add_marketplace_active 子状态
        if self.add_marketplace_active {
            return self.handle_marketplace_add(input, ctx);
        }

        // 默认列表视图
        match input {
            Input {
                key: Key::Right, ..
            }
            | Input { key: Key::Tab, .. } => {
                self.view.next();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Left, .. } => {
                self.view.prev();
                self.sync_current_view_items();
                EventResult::Consumed
            }
            Input { key: Key::Up, .. } => {
                self.marketplace_list.move_cursor(-1);
                EventResult::Consumed
            }
            Input { key: Key::Down, .. } => {
                self.marketplace_list.move_cursor(1);
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                if self.marketplace_list.cursor() == 0 {
                    // Add Marketplace
                    self.add_marketplace_input = InputState::new();
                    self.add_marketplace_active = true;
                } else if let Some(entry) = self
                    .marketplace_entries
                    .get(self.marketplace_list.cursor() - 1)
                {
                    let name = entry.name.clone();
                    let source = entry.source.clone();
                    self.marketplace_updating.insert(name.clone());
                    let name_for_msg = name.clone();
                    let source_for_update = source.clone();
                    let tx = ctx.services.bg_event_tx.clone();
                    tokio::spawn(async move {
                        let result = peri_middlewares::plugin::marketplace::refresh_marketplace(
                            &source, &name,
                        )
                        .await;
                        match result {
                            Ok((_manifest, install_location)) => {
                                if let Ok(mut marketplaces) =
                                    peri_middlewares::plugin::load_known_marketplaces(None)
                                {
                                    if let Some(km) = marketplaces
                                        .iter_mut()
                                        .find(|km| km.source == source_for_update)
                                    {
                                        km.install_location = install_location;
                                        km.last_updated = chrono::Utc::now().to_rfc3339();
                                        let _ = peri_middlewares::plugin::save_known_marketplaces(
                                            &marketplaces,
                                            None,
                                        );
                                    }
                                }
                                let _ = tx
                                    .send(super::super::AgentEvent::PluginActionCompleted {
                                        plugin_id: name.clone(),
                                        action: "refresh".to_string(),
                                        success: true,
                                        message: format!(
                                            "Marketplace '{}' \u{5df2}\u{66f4}\u{65b0}",
                                            name
                                        ),
                                    })
                                    .await;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(super::super::AgentEvent::PluginActionCompleted {
                                        plugin_id: name.clone(),
                                        action: "refresh".to_string(),
                                        success: false,
                                        message: format!("\u{66f4}\u{65b0}\u{5931}\u{8d25}: {}", e),
                                    })
                                    .await;
                            }
                        }
                    });
                    ctx.session_mgr.sessions[ctx.session_mgr.active]
                        .messages
                        .push_system_note(ctx.services.lc.tr_args(
                            "app-plugin-updating",
                            &[("name".into(), name_for_msg.into())],
                        ));
                }
                EventResult::Consumed
            }
            Input {
                key: Key::Backspace,
                ..
            } => {
                if self.marketplace_list.cursor() > 0 {
                    let idx = self.marketplace_list.cursor() - 1;
                    if self.marketplace_entries.get(idx).is_some() {
                        self.marketplace_confirm_delete = Some(idx);
                    }
                }
                EventResult::Consumed
            }
            Input { key: Key::Esc, .. } => EventResult::ClosePanel,
            _ => EventResult::Consumed,
        }
    }

    fn handle_marketplace_confirm_delete(
        &mut self,
        input: Input,
        ctx: &mut PanelContext<'_>,
    ) -> EventResult {
        match input {
            Input { key: Key::Esc, .. } => {
                self.marketplace_confirm_delete = None;
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                if let Some(idx) = self.marketplace_confirm_delete.take() {
                    if let Some(entry) = self.marketplace_entries.get(idx) {
                        let name = entry.name.clone();
                        self.marketplace_entries.remove(idx);
                        self.marketplace_list
                            .set_items(self.marketplace_entries.clone());

                        // Persist delete
                        if let Err(e) = self.persist_marketplace_delete(&name) {
                            ctx.session_mgr.sessions[ctx.session_mgr.active]
                                .messages
                                .push_system_note(ctx.services.lc.tr_args(
                                    "app-plugin-delete-failed",
                                    &[("error".into(), e.to_string().into())],
                                ));
                        }
                    }
                }
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    fn handle_marketplace_add(&mut self, input: Input, ctx: &mut PanelContext<'_>) -> EventResult {
        match input {
            Input { key: Key::Esc, .. } => {
                self.add_marketplace_active = false;
                self.add_marketplace_input = InputState::new();
                EventResult::Consumed
            }
            Input {
                key: Key::Enter, ..
            } => {
                let input_str = self.add_marketplace_input.value().trim().to_string();
                self.add_marketplace_active = false;
                self.add_marketplace_input = InputState::new();
                if !input_str.is_empty() {
                    if let Err(e) = self.persist_marketplace_add(&input_str, ctx) {
                        ctx.session_mgr.sessions[ctx.session_mgr.active]
                            .messages
                            .push_system_note(ctx.services.lc.tr_args(
                                "app-plugin-add-failed",
                                &[("error".into(), e.to_string().into())],
                            ));
                    }
                }
                EventResult::Consumed
            }
            Input {
                key: Key::Backspace,
                ..
            } => {
                self.add_marketplace_input.backspace();
                EventResult::Consumed
            }
            Input {
                key: Key::Char(ch), ..
            } => {
                self.add_marketplace_input.insert(ch);
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    // ─── 辅助方法 ──────────────────────────────────────────────────────────

    /// 异步安装 Discover 视图中当前光标处的插件
    fn spawn_install_current(&mut self, ctx: &PanelContext<'_>) {
        let plugin = match self.discover_current_plugin() {
            Some(p) => p,
            None => return,
        };
        let name = plugin.name.clone();
        let marketplace = plugin.marketplace.clone();
        let plugin_id = plugin.plugin_id.clone();
        self.installing.insert(plugin_id.clone());

        let project_dir = std::path::PathBuf::from(&ctx.services.cwd);
        let claude_dir = peri_middlewares::plugin::claude_home();
        let cache_dir = peri_middlewares::plugin::marketplaces_cache_dir();
        let tx = ctx.services.bg_event_tx.clone();
        tokio::spawn(async move {
            let result = peri_middlewares::plugin::install_plugin(
                &name,
                &marketplace,
                InstallScope::User,
                &cache_dir,
                &claude_dir,
                Some(&project_dir),
            )
            .await;
            let _ = tx.try_send(super::super::AgentEvent::PluginActionCompleted {
                plugin_id,
                action: "install".to_string(),
                success: result.is_ok(),
                message: result
                    .map(|_| String::new())
                    .unwrap_or_else(|e| e.to_string()),
            });
        });
    }

    /// 执行详情页当前操作（ToggleEnabled/Uninstall/BackToList）
    fn do_detail_action(&mut self, ctx: &PanelContext<'_>) {
        let action = DetailAction::ALL.get(self.detail_cursor).copied();
        let entry_idx = self.detail_index;
        match action {
            Some(DetailAction::ToggleEnabled) => {
                if let Some(idx) = entry_idx {
                    if let Some(entry) = self.entries.get_mut(idx) {
                        entry.enabled = !entry.enabled;
                    }
                }
                self.persist_enabled_state(ctx.services.claude_settings_override.as_ref());
            }
            Some(DetailAction::Uninstall) => {
                if let Some(idx) = entry_idx {
                    let id = self.entries.get(idx).map(|e| e.id.clone());
                    if let Some(id) = id {
                        self.confirm_delete = Some(id);
                    }
                }
            }
            Some(DetailAction::BackToList) => {
                self.detail_index = None;
                self.detail_cursor = 0;
            }
            None => {}
        }
    }

    /// 持久化 enabled 状态到 Claude settings
    fn persist_enabled_state(&self, claude_settings_override: Option<&std::path::PathBuf>) {
        let states: Vec<(String, bool)> = self
            .entries
            .iter()
            .map(|e| (e.id.clone(), e.enabled))
            .collect();
        if let Err(e) = peri_middlewares::plugin::save_claude_settings_enabled_plugins(
            &states,
            claude_settings_override.map(|p| p.as_path()),
        ) {
            tracing::warn!(error = %e, "\u{4fdd}\u{5b58} enabledPlugins \u{5931}\u{8d25}");
        }
    }

    /// 持久化删除 marketplace
    fn persist_marketplace_delete(&self, name: &str) -> anyhow::Result<()> {
        use peri_middlewares::plugin::{
            load_known_marketplaces, save_known_marketplaces, MarketplaceSource,
        };
        let marketplaces = load_known_marketplaces(None).unwrap_or_default();
        let filtered: Vec<_> = marketplaces
            .into_iter()
            .filter(|km| {
                let km_name = match &km.source {
                    MarketplaceSource::GitHub { repo } => {
                        repo.split('/').next_back().unwrap_or(repo).to_string()
                    }
                    MarketplaceSource::Git { url } => url
                        .split('/')
                        .next_back()
                        .and_then(|s| s.strip_suffix(".git"))
                        .unwrap_or("marketplace")
                        .to_string(),
                    MarketplaceSource::Url { url } => {
                        let last = url.split('/').next_back().unwrap_or("marketplace");
                        last.strip_suffix(".json").unwrap_or(last).to_string()
                    }
                    MarketplaceSource::File { path } => std::path::Path::new(path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("marketplace")
                        .to_string(),
                    MarketplaceSource::Directory { path } => std::path::Path::new(path)
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("marketplace")
                        .to_string(),
                    MarketplaceSource::Npm { package } => {
                        package.split('@').next().unwrap_or(package).to_string()
                    }
                };
                km_name != name
            })
            .collect();
        save_known_marketplaces(&filtered, None)?;
        Ok(())
    }

    /// 持久化添加 marketplace
    fn persist_marketplace_add(
        &mut self,
        input: &str,
        ctx: &mut PanelContext<'_>,
    ) -> anyhow::Result<()> {
        use peri_middlewares::plugin::{
            load_known_marketplaces, parse_marketplace_input, save_known_marketplaces,
            KnownMarketplace, MarketplaceManager,
        };
        let source = parse_marketplace_input(input)
            .map_err(|e| anyhow::anyhow!("\u{89e3}\u{6790}\u{5931}\u{8d25}: {}", e))?;
        let mut marketplaces = load_known_marketplaces(None).unwrap_or_default();
        for existing in &marketplaces {
            if existing.source == source {
                anyhow::bail!("Marketplace \u{5df2}\u{5b58}\u{5728}");
            }
        }
        let name = MarketplaceManager::extract_name(&source);
        let new_entry = KnownMarketplace {
            source: source.clone(),
            install_location: String::new(),
            auto_update: false,
            last_updated: String::new(),
        };
        marketplaces.push(new_entry);
        save_known_marketplaces(&marketplaces, None)?;

        ctx.session_mgr.sessions[ctx.session_mgr.active]
            .messages
            .push_system_note(
                ctx.services
                    .lc
                    .tr_args("app-plugin-added", &[("name".into(), name.clone().into())]),
            );

        // Add placeholder entry to marketplace_entries
        self.marketplace_entries.push(MarketplaceViewEntry {
            name: name.clone(),
            source: source.clone(),
            source_label: format!("{:?}", source),
            plugin_count: 0,
            installed_count: 0,
            status: MarketplaceViewStatus::Fetching,
            last_updated: None,
            auto_update: false,
        });

        // Spawn background refresh
        let name_clone = name.clone();
        let tx = ctx.services.bg_event_tx.clone();
        tokio::spawn(async move {
            use peri_middlewares::plugin::marketplace::refresh_marketplace;
            match refresh_marketplace(&source, &name_clone).await {
                Ok((_manifest, install_location)) => {
                    if let Ok(mut mkt_places) =
                        peri_middlewares::plugin::load_known_marketplaces(None)
                    {
                        if let Some(entry) = mkt_places.iter_mut().find(|km| km.source == source) {
                            entry.install_location = install_location;
                            entry.last_updated = chrono::Utc::now().to_rfc3339();
                            let _ = peri_middlewares::plugin::save_known_marketplaces(
                                &mkt_places,
                                None,
                            );
                        }
                    }
                    let _ = tx
                        .send(super::super::AgentEvent::PluginActionCompleted {
                            plugin_id: name_clone.clone(),
                            action: "add".to_string(),
                            success: true,
                            message: format!(
                                "Marketplace '{}' \u{5185}\u{5bb9}\u{5df2}\u{83b7}\u{53d6}",
                                name_clone
                            ),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(super::super::AgentEvent::PluginActionCompleted {
                            plugin_id: name_clone.clone(),
                            action: "add".to_string(),
                            success: false,
                            message: format!(
                                "\u{83b7}\u{53d6}\u{5185}\u{5bb9}\u{5931}\u{8d25}: {}",
                                e
                            ),
                        })
                        .await;
                }
            }
        });

        Ok(())
    }
}
