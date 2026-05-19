    fn make_entry(id: &str, name: &str, enabled: bool) -> PluginEntry {
        PluginEntry {
            id: id.into(),
            name: name.into(),
            plugin_type: PluginItemType::Plugin,
            marketplace: "test".into(),
            enabled,
            scope: InstallScope::User,
            version: "1.0.0".into(),
            install_path: std::path::PathBuf::new(),
            project_path: None,
            load_error: None,
            description: String::new(),
            author: None,
            commands: vec![],
            skills: vec![],
            agents: vec![],
            mcp_servers: vec![],
        }
    }

    #[test]
    fn test_plugin_panel_new() {
        let panel = PluginPanel::new(vec![]);
        assert_eq!(panel.cursor(), 0);
        assert_eq!(panel.view, PluginPanelView::Installed);
        assert!(panel.confirm_delete.is_none());
    }

    #[tokio::test]
    async fn test_plugin_panel_move_cursor() {
        let panel = PluginPanel::new(vec![
            make_entry("a@test", "a", true),
            make_entry("b@test", "b", true),
            make_entry("c@test", "c", true),
        ]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        for _ in 0..5 {
            app.plugin_panel_move_up();
        }
        assert_eq!(app.global_panels.get::<PluginPanel>().unwrap().cursor(), 0);

        for _ in 0..5 {
            app.plugin_panel_move_down();
        }
        assert_eq!(app.global_panels.get::<PluginPanel>().unwrap().cursor(), 2);
    }

    #[tokio::test]
    async fn test_plugin_panel_tab_cycles_views() {
        let panel = PluginPanel::new(vec![]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        app.plugin_panel_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Discover
        );
        app.plugin_panel_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Marketplaces
        );
        app.plugin_panel_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Errors
        );
        app.plugin_panel_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Installed
        );
    }

    #[tokio::test]
    async fn test_plugin_panel_shift_tab_cycles_back() {
        let panel = PluginPanel::new(vec![]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        app.plugin_panel_shift_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Errors
        );
        app.plugin_panel_shift_tab();
        assert_eq!(
            app.global_panels.get::<PluginPanel>().unwrap().view,
            PluginPanelView::Marketplaces
        );
    }

    #[tokio::test]
    async fn test_plugin_panel_close() {
        let panel = PluginPanel::new(vec![]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));
        app.plugin_panel_close();
        assert!(!app.global_panels.is_active(crate::app::PanelKind::Plugin));
    }

    #[tokio::test]
    async fn test_plugin_panel_request_cancel_delete() {
        let panel = PluginPanel::new(vec![make_entry("my-plugin@test", "my-plugin", true)]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        app.plugin_panel_request_delete();
        assert_eq!(
            app.global_panels
                .get::<PluginPanel>()
                .unwrap()
                .confirm_delete,
            Some("my-plugin@test".into())
        );

        app.plugin_panel_cancel_delete();
        assert!(app
            .global_panels
            .get::<PluginPanel>()
            .unwrap()
            .confirm_delete
            .is_none());
    }

    #[tokio::test]
    async fn test_plugin_panel_toggle_enabled() {
        let panel = PluginPanel::new(vec![make_entry("p@test", "p", true)]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        app.plugin_panel_toggle_enabled();
        assert!(!app.global_panels.get::<PluginPanel>().unwrap().entries[0].enabled);

        app.plugin_panel_toggle_enabled();
        assert!(app.global_panels.get::<PluginPanel>().unwrap().entries[0].enabled);
    }

    #[tokio::test]
    async fn test_plugin_panel_errors_view() {
        let mut entry = make_entry("bad@t", "bad-plugin", true);
        entry.load_error = Some("missing manifest".into());
        let panel = PluginPanel::new(vec![make_entry("good@t", "good-plugin", true), entry]);
        let (mut app, _handle) = crate::app::App::new_headless(80, 24).await;
        app.global_panels
            .open(crate::app::panel_manager::PanelState::Plugin(Box::new(
                panel,
            )));

        // Default view (Installed): 2 items
        assert_eq!(
            app.global_panels
                .get::<PluginPanel>()
                .unwrap()
                .current_list_len(),
            2
        );

        // Switch to Errors view: 1 item
        app.plugin_panel_tab(); // -> Discover
        app.plugin_panel_tab(); // -> Marketplaces
        app.plugin_panel_tab(); // -> Errors
        assert_eq!(
            app.global_panels
                .get::<PluginPanel>()
                .unwrap()
                .current_list_len(),
            1
        );
    }
