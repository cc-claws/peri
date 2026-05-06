use rust_agent_middlewares::plugin::load_installed_plugins;

fn main() {
    let result = load_installed_plugins(None).unwrap();
    println!("Loaded {} plugins", result.plugins.len());
    for p in &result.plugins {
        println!("  - {} ({})", p.id, p.install_path.display());
    }
}
