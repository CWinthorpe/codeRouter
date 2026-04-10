//! Sidecar binary entry point.
//!
//! On first run this creates the default configuration and data directories
//! and writes a skeleton `config.json`. It then starts the proxy server.

use coderouter_proxy::proxy::server::start_server;
use std::fs;
use std::path::PathBuf;

/// Ensures the application directories and default config file exist.
///
/// Creates `~/.config/coderouter/`, `~/.local/share/coderouter/`, and a
/// default `config.json` if they are missing so the proxy can start cleanly
/// on a fresh install.
///
/// # Errors
///
/// Returns an error if directory or file creation fails due to I/O or
/// serialization problems.
fn ensure_first_run() -> anyhow::Result<()> {
    let config_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("coderouter");
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("coderouter");
    if !data_dir.exists() {
        fs::create_dir_all(&data_dir)?;
    }

    // Write a default config only when no config file exists yet
    let config_path = config_dir.join("config.json");
    if !config_path.exists() {
        let default_config = coderouter_proxy::config::models::AppConfig::default();
        let content = serde_json::to_string_pretty(&default_config)?;
        fs::write(&config_path, content)?;
    }

    Ok(())
}

/// Application entry point.
///
/// Initialises the first-run environment and then starts the proxy server.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ensure_first_run()?;
    start_server().await
}
