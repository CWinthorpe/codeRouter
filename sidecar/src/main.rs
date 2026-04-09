use coderouter_proxy::proxy::server::start_server;
use std::fs;
use std::path::PathBuf;

fn ensure_first_run() -> anyhow::Result<()> {
    // Create ~/.config/coderouter/
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("coderouter");
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    // Create ~/.local/share/coderouter/
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("coderouter");
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)?;
    }

    // Create default config.json if not present
    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        let default_config = coderouter_proxy::config::models::AppConfig::default();
        let content = serde_json::to_string_pretty(&default_config)?;
        fs::write(&config_path, content)?;
    }

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ensure_first_run()?;
    start_server().await
}
