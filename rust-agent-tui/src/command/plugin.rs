use super::Command;
use crate::app::App;

pub struct PluginCommand;

impl Command for PluginCommand {
    fn name(&self) -> &str {
        "plugin"
    }
    fn description(&self) -> &str {
        "管理插件（浏览、安装、卸载）"
    }
    fn execute(&self, app: &mut App, _args: &str) {
        app.open_plugin_panel();
    }
}
